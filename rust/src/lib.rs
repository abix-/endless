//! Endless ECS - GDExtension bridge between Godot, Bevy ECS, and GPU compute.
//! See docs/ for architecture documentation.

// ============================================================================
// MODULES
// ============================================================================

pub mod channels;
pub mod components;
pub mod constants;
pub mod gpu;
pub mod messages;
pub mod resources;
pub mod systems;
pub mod world;

// ============================================================================
// IMPORTS
// ============================================================================

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;
use godot_bevy::prelude::*;
use godot::classes::{RenderingServer, QuadMesh, INode2D, ShaderMaterial, ResourceLoader};

use constants::*;
use gpu::GpuCompute;
use messages::*;
use resources::*;
use systems::*;
use components::*;

// ============================================================================
// HELPERS
// ============================================================================

/// Derive NPC state name from ECS components (no cache needed).
fn derive_npc_state(world: &World, entity: Entity) -> &'static str {
    if world.get::<Dead>(entity).is_some() { return "Dead"; }
    if world.get::<InCombat>(entity).is_some() { return "Fighting"; }
    if world.get::<Recovering>(entity).is_some() { return "Recovering"; }
    if world.get::<Resting>(entity).is_some() { return "Resting"; }
    if world.get::<Working>(entity).is_some() { return "Working"; }
    if world.get::<OnDuty>(entity).is_some() { return "On Duty"; }
    if world.get::<Patrolling>(entity).is_some() { return "Patrolling"; }
    if world.get::<GoingToRest>(entity).is_some() { return "Going to Rest"; }
    if world.get::<GoingToWork>(entity).is_some() { return "Going to Work"; }
    if world.get::<Raiding>(entity).is_some() { return "Raiding"; }
    if world.get::<Returning>(entity).is_some() { return "Returning"; }
    "Idle"
}

/// Get job name from job ID.
fn job_name(job: i32) -> &'static str {
    match job {
        0 => "Farmer",
        1 => "Guard",
        2 => "Raider",
        3 => "Fighter",
        _ => "Unknown",
    }
}

/// Get trait name from trait ID.
fn trait_name(trait_id: i32) -> &'static str {
    match trait_id {
        0 => "",
        1 => "Brave",
        2 => "Coward",
        3 => "Efficient",
        4 => "Hardy",
        5 => "Lazy",
        6 => "Strong",
        7 => "Swift",
        8 => "Sharpshot",
        9 => "Berserker",
        _ => "",
    }
}

// ============================================================================
// BEVY APP - Initializes ECS world and systems
// ============================================================================

/// System execution phases. Chained sets get automatic apply_deferred between them.
#[derive(bevy::ecs::schedule::SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Step {
    Drain,    // Reset + drain message queues
    Spawn,    // Create entities + apply targets
    Combat,   // Cooldowns, attacks, damage, death
    Behavior, // Energy, patrol, rest, work
}

/// Build the Bevy application. Called once at startup by godot-bevy.
#[bevy_app]
fn build_app(app: &mut bevy::prelude::App) {
    use bevy::prelude::Update;

    app.add_message::<SpawnNpcMsg>()
       .add_message::<SetTargetMsg>()
       .add_message::<ArrivalMsg>()
       .add_message::<DamageMsg>()
       .add_message::<GpuUpdateMsg>()
       .init_resource::<NpcCount>()
       .init_resource::<NpcEntityMap>()
       .init_resource::<PopulationStats>()
       .init_resource::<GameConfig>()
       .init_resource::<resources::GameTime>()
       .init_resource::<RespawnTimers>()
       .init_resource::<world::WorldData>()
       .init_resource::<world::BedOccupancy>()
       .init_resource::<world::FarmOccupancy>()
       .init_resource::<resources::HealthDebug>()
       .init_resource::<resources::CombatDebug>()
       .init_resource::<resources::KillStats>()
       .init_resource::<resources::SelectedNpc>()
       .init_resource::<resources::NpcMetaCache>()
       .init_resource::<resources::NpcEnergyCache>()
       .init_resource::<resources::NpcsByTownCache>()
       .init_resource::<resources::NpcLogCache>()
       .init_resource::<resources::FoodEvents>()
       .init_resource::<resources::ResetFlag>()
       .init_resource::<resources::GpuReadState>()
       .init_resource::<resources::GpuDispatchCount>()
       .init_resource::<resources::SlotAllocator>()
       .init_resource::<resources::ProjSlotAllocator>()
       .init_resource::<resources::FoodStorage>()
       // Chain phases with explicit command flush between Spawn and Combat
       .configure_sets(Update, (Step::Drain, Step::Spawn, Step::Combat, Step::Behavior).chain())
       // Flush commands after Spawn so Combat sees new entities
       .add_systems(Update, bevy::ecs::schedule::ApplyDeferred.after(Step::Spawn).before(Step::Combat))
       // Drain: reset + drain queues (channels + legacy mutexes still used by lib.rs)
       .add_systems(Update, (
           reset_bevy_system,
           gpu_position_readback, // Phase 11: GPU → Bevy position sync
           sync_gpu_state_to_bevy, // Sync GPU_READ_STATE static to Bevy resource
           godot_to_bevy_read,    // Phase 11: lock-free channel (spawn, target, damage)
           drain_arrival_queue,   // Still needed: lib.rs pushes arrivals from GPU
           drain_game_config,
       ).in_set(Step::Drain))
       // Spawn: create entities
       .add_systems(Update, (
           spawn_npc_system,
           apply_targets_system,
       ).in_set(Step::Spawn))
       // Combat: cooldowns, attacks, damage, death
       .add_systems(Update, (
           cooldown_system,
           attack_system,
           damage_system,
           death_system,
           death_cleanup_system,
       ).chain().in_set(Step::Combat))
       // Behavior: energy, patrol, rest, work, stealing, combat escape, economy
       .add_systems(Update, (
           arrival_system,
           energy_system,
           healing_system,
           flee_system,
           leash_system,
           recovery_system,
           patrol_system,
           economy_tick_system,
           decision_system,
       ).in_set(Step::Behavior))
       // Collect GPU updates at end of frame (single Mutex lock point)
       .add_systems(Update, collect_gpu_updates.after(Step::Behavior))
       // Phase 11: Write changed state to BevyToGodot outbox
       .add_systems(Update, bevy_to_godot_write.after(Step::Behavior));
}

// ============================================================================
// GODOT CLASS - Bridge between GDScript and the ECS/GPU systems
// ============================================================================

/// Main interface for GDScript to interact with the NPC system.
#[derive(GodotClass)]
#[class(base=Node2D)]
pub struct EcsNpcManager {
    base: Base<Node2D>,

    /// GPU compute context. None if initialization failed.
    gpu: Option<GpuCompute>,

    /// MultiMesh resource ID for batch rendering all NPCs.
    multimesh_rid: Rid,

    /// Canvas item for attaching the MultiMesh to the scene.
    canvas_item: Rid,

    /// Keep mesh alive (Godot reference counting)
    #[allow(dead_code)]
    mesh: Option<Gd<QuadMesh>>,

    /// Keep material alive (Godot reference counting)
    #[allow(dead_code)]
    material: Option<Gd<ShaderMaterial>>,

    /// Previous frame's arrival states (to detect new arrivals).
    prev_arrivals: Vec<bool>,

    // === Projectile Rendering ===
    /// MultiMesh for projectiles
    proj_multimesh_rid: Rid,
    /// Canvas item for projectile MultiMesh
    proj_canvas_item: Rid,
    /// Keep projectile mesh alive
    #[allow(dead_code)]
    proj_mesh: Option<Gd<QuadMesh>>,

    // === Channels (Phase 11) ===
    /// Sender for Godot → Bevy messages
    godot_to_bevy: Option<channels::GodotToBevySender>,
    /// Receiver for Bevy → Godot messages
    bevy_to_godot: Option<channels::BevyToGodotReceiver>,
}

#[godot_api]
impl INode2D for EcsNpcManager {
    fn init(base: Base<Node2D>) -> Self {
        Self {
            base,
            gpu: None,
            multimesh_rid: Rid::Invalid,
            canvas_item: Rid::Invalid,
            mesh: None,
            material: None,
            prev_arrivals: vec![false; MAX_NPC_COUNT],
            proj_multimesh_rid: Rid::Invalid,
            proj_canvas_item: Rid::Invalid,
            proj_mesh: None,
            godot_to_bevy: None,
            bevy_to_godot: None,
        }
    }

    fn ready(&mut self) {
        self.gpu = GpuCompute::new();
        if self.gpu.is_none() {
            godot_error!("[EcsNpcManager] Failed to initialize GPU compute");
            return;
        }
        self.setup_multimesh(MAX_NPC_COUNT as i32);
        self.setup_proj_multimesh(MAX_PROJECTILES as i32);

        // Create channels and register Bevy resources
        let channels = channels::create_channels();
        self.godot_to_bevy = Some(channels.godot_to_bevy_sender);
        self.bevy_to_godot = Some(channels.bevy_to_godot_receiver);

        // Insert channel resources into Bevy app
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                app.world_mut().insert_resource(channels.godot_to_bevy_receiver);
                app.world_mut().insert_resource(channels.bevy_to_godot_sender);
            }
        }
    }

    fn process(&mut self, delta: f64) {
        let gpu = match self.gpu.as_mut() {
            Some(g) => g,
            None => return,
        };

        // Get dispatch count: only includes NPCs with initialized GPU buffers
        let npc_count = GPU_DISPATCH_COUNT.lock().map(|c| *c).unwrap_or(0);

        // Drain GPU update queue. Guard uses MAX_NPC_COUNT (buffer size) not
        // npc_count, so spawn data for newly-allocated slots can be written
        // before GPU_DISPATCH_COUNT catches up next frame.
        if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
            for update in queue.drain(..) {
                match update {
                    GpuUpdate::SetTarget { idx, x, y } => {
                        if idx < MAX_NPC_COUNT {
                            // Update CPU cache
                            gpu.targets[idx * 2] = x;
                            gpu.targets[idx * 2 + 1] = y;

                            let target_bytes: Vec<u8> = [x, y].iter()
                                .flat_map(|f| f.to_le_bytes()).collect();
                            let target_packed = PackedByteArray::from(target_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &target_packed);

                            let zero_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
                            let zero_packed = PackedByteArray::from(zero_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.arrival_buffer, (idx * 4) as u32, 4, &zero_packed);
                            gpu.rd.buffer_update(gpu.backoff_buffer, (idx * 4) as u32, 4, &zero_packed);
                            self.prev_arrivals[idx] = false;
                        }
                    }
                    GpuUpdate::ApplyDamage { idx, amount } => {
                        if idx < MAX_NPC_COUNT {
                            let new_health = (gpu.healths[idx] - amount).max(0.0);
                            gpu.healths[idx] = new_health;
                            let health_bytes: Vec<u8> = new_health.to_le_bytes().to_vec();
                            let health_packed = PackedByteArray::from(health_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.health_buffer, (idx * 4) as u32, 4, &health_packed);
                        }
                    }
                    GpuUpdate::HideNpc { idx } => {
                        if idx < MAX_NPC_COUNT {
                            // Set position to offscreen
                            let hide_pos: Vec<u8> = [-9999.0f32, -9999.0f32].iter()
                                .flat_map(|f| f.to_le_bytes()).collect();
                            let hide_packed = PackedByteArray::from(hide_pos.as_slice());
                            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &hide_packed);
                            gpu.positions[idx * 2] = -9999.0;
                            gpu.positions[idx * 2 + 1] = -9999.0;

                            // Also set target to offscreen so NPC doesn't try to move
                            gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &hide_packed);
                            gpu.targets[idx * 2] = -9999.0;
                            gpu.targets[idx * 2 + 1] = -9999.0;

                            // Mark as arrived so GPU doesn't compute movement
                            let one_bytes: Vec<u8> = 1i32.to_le_bytes().to_vec();
                            let one_packed = PackedByteArray::from(one_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.arrival_buffer, (idx * 4) as u32, 4, &one_packed);

                            // Set health to 0 so click detection skips this slot
                            gpu.healths[idx] = 0.0;
                            let zero_health: Vec<u8> = 0.0f32.to_le_bytes().to_vec();
                            let zero_packed = PackedByteArray::from(zero_health.as_slice());
                            gpu.rd.buffer_update(gpu.health_buffer, (idx * 4) as u32, 4, &zero_packed);
                        }
                    }
                    GpuUpdate::SetFaction { idx, faction } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.factions[idx] = faction;
                            let faction_bytes: Vec<u8> = faction.to_le_bytes().to_vec();
                            let faction_packed = PackedByteArray::from(faction_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.faction_buffer, (idx * 4) as u32, 4, &faction_packed);
                        }
                    }
                    GpuUpdate::SetHealth { idx, health } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.healths[idx] = health;
                            let health_bytes: Vec<u8> = health.to_le_bytes().to_vec();
                            let health_packed = PackedByteArray::from(health_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.health_buffer, (idx * 4) as u32, 4, &health_packed);
                        }
                    }
                    GpuUpdate::SetPosition { idx, x, y } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.positions[idx * 2] = x;
                            gpu.positions[idx * 2 + 1] = y;
                            let pos_bytes: Vec<u8> = [x, y].iter()
                                .flat_map(|f| f.to_le_bytes()).collect();
                            let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &pos_packed);
                        }
                    }
                    GpuUpdate::SetSpeed { idx, speed } => {
                        if idx < MAX_NPC_COUNT {
                            let speed_bytes: Vec<u8> = speed.to_le_bytes().to_vec();
                            let speed_packed = PackedByteArray::from(speed_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.speed_buffer, (idx * 4) as u32, 4, &speed_packed);
                        }
                    }
                    GpuUpdate::SetColor { idx, r, g, b, a } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.colors[idx * 4] = r;
                            gpu.colors[idx * 4 + 1] = g;
                            gpu.colors[idx * 4 + 2] = b;
                            gpu.colors[idx * 4 + 3] = a;
                            let color_bytes: Vec<u8> = [r, g, b, a].iter()
                                .flat_map(|f| f.to_le_bytes()).collect();
                            let color_packed = PackedByteArray::from(color_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.color_buffer, (idx * 16) as u32, 16, &color_packed);
                        }
                    }
                    GpuUpdate::SetSpriteFrame { idx, col, row } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.sprite_frames[idx * 2] = col;
                            gpu.sprite_frames[idx * 2 + 1] = row;
                            let frame_bytes: Vec<u8> = [col, row].iter()
                                .flat_map(|f| f.to_le_bytes()).collect();
                            let frame_packed = PackedByteArray::from(frame_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.sprite_frame_buffer, (idx * 8) as u32, 8, &frame_packed);
                        }
                    }
                    GpuUpdate::SetHealing { idx, healing } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.healing_flags[idx] = healing;
                        }
                    }
                }
            }
        }

        if npc_count > 0 {
            gpu.dispatch(npc_count, delta as f32);
            gpu.read_positions_from_gpu(npc_count);

            // Read combat targets from GPU
            gpu.read_combat_targets(npc_count);

            // GPU-FIRST: Single state update for all GPU reads
            if let Ok(mut state) = GPU_READ_STATE.lock() {
                state.npc_count = npc_count;

                state.positions.clear();
                state.positions.extend_from_slice(&gpu.positions[..(npc_count * 2)]);

                state.combat_targets.clear();
                state.combat_targets.extend_from_slice(&gpu.combat_targets[..npc_count]);

                state.health.clear();
                state.health.extend_from_slice(&gpu.healths[..npc_count]);

                state.factions.clear();
                state.factions.extend_from_slice(&gpu.factions[..npc_count]);
            }

            // Detect arrivals
            let arrival_bytes = gpu.rd.buffer_get_data(gpu.arrival_buffer);
            let arrival_slice = arrival_bytes.as_slice();
            if let Ok(mut queue) = ARRIVAL_QUEUE.lock() {
                for i in 0..npc_count {
                    if arrival_slice.len() >= (i + 1) * 4 {
                        let arrived = i32::from_le_bytes([
                            arrival_slice[i * 4],
                            arrival_slice[i * 4 + 1],
                            arrival_slice[i * 4 + 2],
                            arrival_slice[i * 4 + 3],
                        ]) > 0;

                        if arrived && !self.prev_arrivals[i] {
                            queue.push(ArrivalMsg { npc_index: i });
                        }
                        self.prev_arrivals[i] = arrived;
                    }
                }
            }

            // Update MultiMesh
            let buffer = gpu.build_multimesh_from_cache(&gpu.colors, npc_count, MAX_NPC_COUNT);
            let mut rs = RenderingServer::singleton();
            rs.multimesh_set_buffer(self.multimesh_rid, &buffer);

            // === DRAIN BEVY OUTBOX FOR PROJECTILE FIRE (via channel) ===
            if let Some(ref receiver) = self.bevy_to_godot {
                while let Ok(outbox_msg) = receiver.0.try_recv() {
                    if let channels::BevyToGodotMsg::FireProjectile {
                        from_x, from_y, to_x, to_y, damage, faction, shooter, speed, lifetime
                    } = outbox_msg {
                        // Allocate slot
                        let idx = if let Some(recycled) = Self::allocate_proj_slot() {
                            recycled
                        } else if gpu.proj_count < MAX_PROJECTILES {
                            let i = gpu.proj_count;
                            gpu.proj_count += 1;
                            i
                        } else {
                            continue; // At capacity, drop this projectile
                        };

                        // Calculate velocity from direction + speed
                        let dx = to_x - from_x;
                        let dy = to_y - from_y;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist < 0.001 { continue; }
                        let vx = (dx / dist) * speed;
                        let vy = (dy / dist) * speed;

                        gpu.upload_projectile(
                            idx, from_x, from_y, vx, vy,
                            damage, faction, shooter as i32, lifetime,
                        );
                    }
                    // Other outbox messages can be ignored or handled here
                }
            }

            // === PROJECTILE PROCESSING ===
            let proj_count = gpu.proj_count;
            if proj_count > 0 {
                // Dispatch projectile compute shader
                gpu.dispatch_projectiles(proj_count, npc_count, delta as f32);

                // Read hit results and route via channel
                let hits = gpu.read_projectile_hits();
                for (proj_idx, npc_idx, damage) in hits {
                    // Send damage via channel
                    if let Some(ref sender) = self.godot_to_bevy {
                        let _ = sender.0.send(channels::GodotToBevyMsg::ApplyDamage {
                            slot: npc_idx,
                            amount: damage,
                        });
                    }
                    // Return projectile slot to pool
                    if let Ok(mut free) = FREE_PROJ_SLOTS.lock() {
                        free.push(proj_idx);
                    }
                }

                // Read updated positions for rendering
                gpu.read_projectile_positions();
                gpu.read_projectile_active();

                // Update projectile MultiMesh (sized to proj_count, not MAX)
                let current_count = rs.multimesh_get_instance_count(self.proj_multimesh_rid) as usize;
                if current_count != proj_count {
                    rs.multimesh_allocate_data_ex(
                        self.proj_multimesh_rid,
                        proj_count as i32,
                        godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
                    ).color_format(true).done();
                }
                let proj_buffer = gpu.build_proj_multimesh(proj_count);
                rs.multimesh_set_buffer(self.proj_multimesh_rid, &proj_buffer);
            }
        }
    }
}

#[godot_api]
impl EcsNpcManager {
    fn setup_multimesh(&mut self, max_count: i32) {
        let mut rs = RenderingServer::singleton();

        self.multimesh_rid = rs.multimesh_create();

        let mut mesh = QuadMesh::new_gd();
        mesh.set_size(Vector2::new(16.0, 16.0));
        let mesh_rid = mesh.get_rid();
        rs.multimesh_set_mesh(self.multimesh_rid, mesh_rid);

        // Enable custom_data for sprite shader (health, flash, sprite frame)
        rs.multimesh_allocate_data_ex(
            self.multimesh_rid,
            max_count,
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
        ).color_format(true).custom_data_format(true).done();

        let count = max_count as usize;
        let mut init_buffer = vec![0.0f32; count * FLOATS_PER_INSTANCE];
        for i in 0..count {
            let base = i * FLOATS_PER_INSTANCE;
            init_buffer[base + 0] = 1.0;   // scale x
            init_buffer[base + 5] = 1.0;   // scale y
            init_buffer[base + 11] = 1.0;  // color alpha
            init_buffer[base + 12] = 1.0;  // custom_data.r = health (100%)
        }
        let packed = PackedFloat32Array::from(init_buffer.as_slice());
        rs.multimesh_set_buffer(self.multimesh_rid, &packed);

        self.canvas_item = rs.canvas_item_create();
        let parent_canvas = self.base().get_canvas_item();
        rs.canvas_item_set_parent(self.canvas_item, parent_canvas);

        // Load and apply sprite shader material
        // IMPORTANT: Store material in struct to keep it alive (RenderingServer expects references to be kept around)
        let mut loader = ResourceLoader::singleton();
        if let Some(shader) = loader.load("res://shaders/npc_sprite.gdshader") {
            if let Some(texture) = loader.load("res://assets/roguelikeChar_transparent.png") {
                let mut material = ShaderMaterial::new_gd();
                let shader_res: Gd<godot::classes::Shader> = shader.cast();
                material.set_shader(&shader_res);
                material.set_shader_parameter("sprite_sheet", &texture.to_variant());
                material.set_shader_parameter("sheet_size", &Vector2::new(918.0, 203.0).to_variant());
                material.set_shader_parameter("sprite_size", &Vector2::new(16.0, 16.0).to_variant());
                material.set_shader_parameter("margin", &1.0f32.to_variant());
                material.set_shader_parameter("hp_bar_mode", &1i32.to_variant());
                rs.canvas_item_set_material(self.canvas_item, material.get_rid());
                self.material = Some(material);
            }
        }

        rs.canvas_item_add_multimesh(self.canvas_item, self.multimesh_rid);
        // Disable visibility culling — Godot's auto AABB is wrong for world-spanning MultiMesh
        rs.canvas_item_set_custom_rect_ex(self.canvas_item, true).rect(Rect2::new(Vector2::new(-100000.0, -100000.0), Vector2::new(200000.0, 200000.0))).done();

        self.mesh = Some(mesh);
    }

    fn setup_proj_multimesh(&mut self, _max_count: i32) {
        let mut rs = RenderingServer::singleton();

        self.proj_multimesh_rid = rs.multimesh_create();

        let mut mesh = QuadMesh::new_gd();
        mesh.set_size(Vector2::new(12.0, 4.0));
        let mesh_rid = mesh.get_rid();
        rs.multimesh_set_mesh(self.proj_multimesh_rid, mesh_rid);

        rs.multimesh_allocate_data_ex(
            self.proj_multimesh_rid,
            0,  // Start empty, resized dynamically per-frame
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
        ).color_format(true).done();

        // Share NPC canvas item (second canvas_item_create doesn't render - Godot quirk)
        // Projectiles draw on top since this add_multimesh is called after NPC's
        self.proj_canvas_item = self.canvas_item;
        rs.canvas_item_add_multimesh(self.proj_canvas_item, self.proj_multimesh_rid);

        self.proj_mesh = Some(mesh);
    }

    // ========================================================================
    // SPAWN API
    // ========================================================================

    /// Allocate an NPC slot from Bevy's SlotAllocator resource.
    /// Returns None if at capacity or if Bevy app unavailable.
    fn allocate_slot(&mut self) -> Option<usize> {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut slots) = app.world_mut().get_resource_mut::<resources::SlotAllocator>() {
                    return slots.alloc();
                }
            }
        }
        None
    }

    /// Unified spawn API. Job determines component template.
    /// No direct GPU writes — all go through GPU_UPDATE_QUEUE.
    #[func]
    fn spawn_npc(&mut self, x: f32, y: f32, job: i32, faction: i32, opts: VarDictionary) -> i32 {
        let idx = match self.allocate_slot() {
            Some(i) => i,
            None => return -1,
        };

        // Extract optional params from dictionary (defaults = "not set")
        let home_x: f32 = opts.get("home_x").and_then(|v| v.try_to::<f32>().ok()).unwrap_or(-1.0);
        let home_y: f32 = opts.get("home_y").and_then(|v| v.try_to::<f32>().ok()).unwrap_or(-1.0);
        let work_x: f32 = opts.get("work_x").and_then(|v| v.try_to::<f32>().ok()).unwrap_or(-1.0);
        let work_y: f32 = opts.get("work_y").and_then(|v| v.try_to::<f32>().ok()).unwrap_or(-1.0);
        let town_idx: i32 = opts.get("town_idx").and_then(|v| v.try_to::<i32>().ok()).unwrap_or(-1);
        let starting_post: i32 = opts.get("starting_post").and_then(|v| v.try_to::<i32>().ok()).unwrap_or(-1);
        let attack_type: i32 = opts.get("attack_type").and_then(|v| v.try_to::<i32>().ok()).unwrap_or(0);

        // Send via channel instead of static queue
        if let Some(ref sender) = self.godot_to_bevy {
            let _ = sender.0.send(channels::GodotToBevyMsg::SpawnNpc {
                slot_idx: idx,
                x, y,
                job: job as u8,
                faction: faction as u8,
                town_idx, home_x, home_y, work_x, work_y, starting_post,
                attack_type: attack_type as u8,
            });
        }

        idx as i32
    }

    // ========================================================================
    // PROJECTILE API
    // ========================================================================

    /// Allocate a projectile slot: reuse a free slot or allocate new.
    /// Returns None if at capacity.
    fn allocate_proj_slot() -> Option<usize> {
        // Try to reuse a free slot first
        if let Ok(mut free) = FREE_PROJ_SLOTS.lock() {
            if let Some(recycled) = free.pop() {
                return Some(recycled);
            }
        }
        // No free slots, check capacity
        None  // Caller handles proj_count increment
    }

    /// Get number of active projectiles.
    #[func]
    fn get_projectile_count(&self) -> i32 {
        if let Some(gpu) = &self.gpu {
            gpu.proj_count as i32
        } else {
            0
        }
    }

    /// Trace raw GPU projectile state - reads directly from GPU buffers (not CPU cache).
    /// Returns a string with lifetime, active, pos, hit for each projectile.
    #[func]
    fn get_projectile_trace(&mut self) -> GString {
        if let Some(gpu) = self.gpu.as_mut() {
            let traces = gpu.trace_projectile_gpu_state(5);
            let lines: Vec<String> = traces.iter().enumerate()
                .map(|(i, (lt, act, px, py, hit_npc, hit_proc))| {
                    format!("[{}] lt={:.2} act={} pos=({:.0},{:.0}) hit=({},{})",
                        i, lt, act, px, py, hit_npc, hit_proc)
                })
                .collect();
            let active_count = gpu.proj_active.iter().take(gpu.proj_count).filter(|&&a| a == 1).count();
            let header = format!("proj_count={} active={}/{}",
                gpu.proj_count, active_count, gpu.proj_count);

            // Debug: check RIDs and dump multimesh floats
            let rs = RenderingServer::singleton();
            let proj_mm_valid = self.proj_multimesh_rid.is_valid();
            let proj_ci_valid = self.proj_canvas_item.is_valid();
            let npc_mm_valid = self.multimesh_rid.is_valid();
            let proj_inst_count = if proj_mm_valid {
                rs.multimesh_get_instance_count(self.proj_multimesh_rid)
            } else { -1 };
            let proj_vis_count = if proj_mm_valid {
                rs.multimesh_get_visible_instances(self.proj_multimesh_rid)
            } else { -1 };
            let proj_mesh_valid = if proj_mm_valid {
                rs.multimesh_get_mesh(self.proj_multimesh_rid).is_valid()
            } else { false };

            let mm = gpu.build_proj_multimesh(gpu.proj_count);
            let mm_slice = mm.as_slice();
            let mut mm_debug = String::new();
            for i in 0..gpu.proj_count.min(2) {
                let base = i * PROJ_FLOATS_PER_INSTANCE;
                if base + 12 <= mm_slice.len() {
                    mm_debug += &format!("mm[{}]: [{:.1},{:.1},{:.1},{:.0}, {:.1},{:.1},{:.1},{:.0}, {:.1},{:.1},{:.1},{:.1}]\n",
                        i,
                        mm_slice[base], mm_slice[base+1], mm_slice[base+2], mm_slice[base+3],
                        mm_slice[base+4], mm_slice[base+5], mm_slice[base+6], mm_slice[base+7],
                        mm_slice[base+8], mm_slice[base+9], mm_slice[base+10], mm_slice[base+11]);
                }
            }

            // Grid debug: check raider position (NPC 1) and guard position (NPC 0)
            let npc0_x = gpu.positions.get(0).copied().unwrap_or(-1.0);
            let npc0_y = gpu.positions.get(1).copied().unwrap_or(-1.0);
            let npc1_x = gpu.positions.get(2).copied().unwrap_or(-1.0);
            let npc1_y = gpu.positions.get(3).copied().unwrap_or(-1.0);
            let (g0cx, g0cy, g0ct) = gpu.trace_grid_cell(npc0_x, npc0_y);
            let (g1cx, g1cy, g1ct) = gpu.trace_grid_cell(npc1_x, npc1_y);
            let grid_debug = format!("npc0=({:.0},{:.0}) cell({},{})={} npc1=({:.0},{:.0}) cell({},{})={}",
                npc0_x, npc0_y, g0cx, g0cy, g0ct,
                npc1_x, npc1_y, g1cx, g1cy, g1ct);

            let rid_debug = format!("mm={} ci={} npc_mm={} inst={} vis={} mesh={}",
                proj_mm_valid, proj_ci_valid, npc_mm_valid, proj_inst_count, proj_vis_count, proj_mesh_valid);
            GString::from(&format!("{}\n{}\n{}\n{}\n{}", header, rid_debug, grid_debug, lines.join("\n"), mm_debug))
        } else {
            GString::from("no gpu")
        }
    }

    /// Get projectile debug info.
    #[func]
    fn get_projectile_debug(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Some(gpu) = &self.gpu {
            dict.set("proj_count", gpu.proj_count as i32);
            let active = gpu.proj_active.iter().take(gpu.proj_count).filter(|&&x| x == 1).count();
            dict.set("active", active as i32);
            // Check if shader pipeline is valid
            dict.set("pipeline_valid", if gpu.proj_pipeline.is_valid() { 1 } else { 0 });
            // Sample first projectile
            if gpu.proj_count > 0 {
                dict.set("pos_0_x", gpu.proj_positions.get(0).copied().unwrap_or(-999.0));
                dict.set("pos_0_y", gpu.proj_positions.get(1).copied().unwrap_or(-999.0));
                dict.set("vel_0_x", gpu.proj_velocities.get(0).copied().unwrap_or(0.0));
                dict.set("vel_0_y", gpu.proj_velocities.get(1).copied().unwrap_or(0.0));
                dict.set("active_0", gpu.proj_active.get(0).copied().unwrap_or(-1));
                dict.set("damage_0", gpu.proj_damages.get(0).copied().unwrap_or(-1.0));
            }
            // Count how many have valid positions (not -9999)
            let visible = gpu.proj_positions.chunks(2)
                .take(gpu.proj_count)
                .filter(|p| p.len() == 2 && p[0] > -9000.0)
                .count();
            dict.set("visible", visible as i32);
        }
        dict
    }

    // ========================================================================
    // TARGET API
    // ========================================================================

    #[func]
    fn set_target(&mut self, npc_index: i32, x: f32, y: f32) {
        // Send via channel instead of static queue
        if let Some(ref sender) = self.godot_to_bevy {
            let _ = sender.0.send(channels::GodotToBevyMsg::SetTarget {
                slot: npc_index as usize, x, y,
            });
        }

        if let Some(gpu) = self.gpu.as_mut() {
            let idx = npc_index as usize;
            let slot_count = NPC_SLOT_COUNTER.lock().map(|c| *c).unwrap_or(0);
            if idx < slot_count {
                let target_bytes: Vec<u8> = [x, y].iter()
                    .flat_map(|f| f.to_le_bytes()).collect();
                let target_packed = PackedByteArray::from(target_bytes.as_slice());
                gpu.rd.buffer_update(
                    gpu.target_buffer,
                    (idx * 8) as u32,
                    target_packed.len() as u32,
                    &target_packed
                );

                let zero_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
                let zero_packed = PackedByteArray::from(zero_bytes.as_slice());
                gpu.rd.buffer_update(gpu.arrival_buffer, (idx * 4) as u32, 4, &zero_packed);
                gpu.rd.buffer_update(gpu.backoff_buffer, (idx * 4) as u32, 4, &zero_packed);
            }
        }
    }

    // ========================================================================
    // HEALTH API
    // ========================================================================

    /// Deal damage to an NPC.
    #[func]
    fn apply_damage(&mut self, npc_index: i32, amount: f32) {
        // Send via channel instead of static queue
        if let Some(ref sender) = self.godot_to_bevy {
            let _ = sender.0.send(channels::GodotToBevyMsg::ApplyDamage {
                slot: npc_index as usize,
                amount,
            });
        }
    }

    // ========================================================================
    // QUERY API
    // ========================================================================

    #[func]
    fn get_npc_count(&self) -> i32 {
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(slots) = app.world().get_resource::<resources::SlotAllocator>() {
                    return slots.count() as i32;
                }
            }
        }
        0
    }

    #[func]
    fn get_build_info(&self) -> GString {
        let timestamp = option_env!("BUILD_TIMESTAMP").unwrap_or("unknown");
        let commit = option_env!("BUILD_COMMIT").unwrap_or("unknown");
        GString::from(&format!("BUILD: {} ({})", timestamp, commit))
    }

    #[func]
    fn get_debug_stats(&mut self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Some(gpu) = &mut self.gpu {
            let npc_count = GPU_READ_STATE.lock().map(|s| s.npc_count).unwrap_or(0);

            let arrival_bytes = gpu.rd.buffer_get_data(gpu.arrival_buffer);
            let arrival_slice = arrival_bytes.as_slice();
            let mut arrived_count = 0;
            for i in 0..npc_count {
                if arrival_slice.len() >= (i + 1) * 4 {
                    let val = i32::from_le_bytes([
                        arrival_slice[i * 4],
                        arrival_slice[i * 4 + 1],
                        arrival_slice[i * 4 + 2],
                        arrival_slice[i * 4 + 3],
                    ]);
                    if val > 0 { arrived_count += 1; }
                }
            }

            let backoff_bytes = gpu.rd.buffer_get_data(gpu.backoff_buffer);
            let backoff_slice = backoff_bytes.as_slice();
            let mut total_backoff = 0i32;
            let mut max_backoff = 0i32;

            for i in 0..npc_count {
                if backoff_slice.len() >= (i + 1) * 4 {
                    let val = i32::from_le_bytes([
                        backoff_slice[i * 4],
                        backoff_slice[i * 4 + 1],
                        backoff_slice[i * 4 + 2],
                        backoff_slice[i * 4 + 3],
                    ]);
                    total_backoff += val;
                    if val > max_backoff { max_backoff = val; }
                }
            }

            let mut cells_with_npcs = 0;
            let mut max_per_cell = 0i32;
            for count in gpu.grid.counts.iter() {
                if *count > 0 {
                    cells_with_npcs += 1;
                    if *count > max_per_cell {
                        max_per_cell = *count;
                    }
                }
            }

            dict.set("npc_count", npc_count as i32);
            dict.set("arrived_count", arrived_count);
            dict.set("avg_backoff", if npc_count > 0 { total_backoff / npc_count as i32 } else { 0 });
            dict.set("max_backoff", max_backoff);
            dict.set("cells_used", cells_with_npcs);
            dict.set("max_per_cell", max_per_cell);
        }
        dict
    }

    #[func]
    fn get_npc_position(&self, npc_index: i32) -> Vector2 {
        if let Some(gpu) = &self.gpu {
            let idx = npc_index as usize;
            // Use slot counter (high-water mark) not dispatch count to avoid timing issues
            let slot_count = NPC_SLOT_COUNTER.lock().map(|c| *c).unwrap_or(0);
            if idx < slot_count {
                let x = gpu.positions.get(idx * 2).copied().unwrap_or(0.0);
                let y = gpu.positions.get(idx * 2 + 1).copied().unwrap_or(0.0);
                return Vector2::new(x, y);
            }
        }
        Vector2::ZERO
    }

    #[func]
    fn get_npc_target(&self, npc_index: i32) -> Vector2 {
        if let Some(gpu) = &self.gpu {
            let idx = npc_index as usize;
            if idx < MAX_NPC_COUNT {
                let x = gpu.targets.get(idx * 2).copied().unwrap_or(0.0);
                let y = gpu.targets.get(idx * 2 + 1).copied().unwrap_or(0.0);
                return Vector2::new(x, y);
            }
        }
        Vector2::ZERO
    }

    #[func]
    fn get_npc_health(&self, npc_index: i32) -> f32 {
        if let Some(gpu) = &self.gpu {
            let idx = npc_index as usize;
            // Use slot counter (high-water mark) not dispatch count to avoid timing issues
            let slot_count = NPC_SLOT_COUNTER.lock().map(|c| *c).unwrap_or(0);
            if idx < slot_count {
                return gpu.healths.get(idx).copied().unwrap_or(0.0);
            }
        }
        0.0
    }

    #[func]
    fn get_combat_debug(&mut self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(debug) = app.world().get_resource::<resources::CombatDebug>() {
                    dict.set("attackers", debug.attackers_queried as i32);
                    dict.set("targets_found", debug.targets_found as i32);
                    dict.set("attacks", debug.attacks_made as i32);
                    dict.set("chases", debug.chases_started as i32);
                    dict.set("in_combat_added", debug.in_combat_added as i32);
                    dict.set("sample_target", debug.sample_target_idx);
                    dict.set("positions_len", debug.positions_len as i32);
                    dict.set("combat_targets_len", debug.combat_targets_len as i32);
                    dict.set("bounds_fail", debug.bounds_failures as i32);
                    dict.set("sample_dist", debug.sample_dist);
                    dict.set("in_range", debug.in_range_count as i32);
                    dict.set("timer_ready", debug.timer_ready_count as i32);
                    dict.set("sample_timer", debug.sample_timer);
                    dict.set("cooldown_entities", debug.cooldown_entities as i32);
                    dict.set("frame_delta", debug.frame_delta);
                    // Enhanced debug data
                    dict.set("combat_target_0", debug.sample_combat_target_0);
                    dict.set("combat_target_1", debug.sample_combat_target_1);
                    dict.set("pos_0_x", debug.sample_pos_0.0);
                    dict.set("pos_0_y", debug.sample_pos_0.1);
                    dict.set("pos_1_x", debug.sample_pos_1.0);
                    dict.set("pos_1_y", debug.sample_pos_1.1);
                }
            }
        }
        // Read combat targets for indices 2-3 directly from GPU_READ_STATE
        if let Ok(state) = GPU_READ_STATE.lock() {
            dict.set("combat_target_2", state.combat_targets.get(2).copied().unwrap_or(-99));
            dict.set("combat_target_3", state.combat_targets.get(3).copied().unwrap_or(-99));
        }
        // GPU cache and buffer debug for NPC 0 and NPC 1
        if let Some(gpu) = &mut self.gpu {
            let npc_count = GPU_READ_STATE.lock().map(|s| s.npc_count).unwrap_or(0);
            dict.set("npc_count", npc_count as i32);

            // CPU cache: positions
            let p0x = gpu.positions.get(0).copied().unwrap_or(-999.0);
            let p0y = gpu.positions.get(1).copied().unwrap_or(-999.0);
            let p1x = gpu.positions.get(2).copied().unwrap_or(-999.0);
            let p1y = gpu.positions.get(3).copied().unwrap_or(-999.0);
            dict.set("cpu_pos_0_x", p0x);
            dict.set("cpu_pos_0_y", p0y);
            dict.set("cpu_pos_1_x", p1x);
            dict.set("cpu_pos_1_y", p1y);

            // CPU cache: factions, healths
            dict.set("faction_0", gpu.factions.get(0).copied().unwrap_or(-99));
            dict.set("faction_1", gpu.factions.get(1).copied().unwrap_or(-99));
            dict.set("health_0", gpu.healths.get(0).copied().unwrap_or(-99.0) as f32);
            dict.set("health_1", gpu.healths.get(1).copied().unwrap_or(-99.0) as f32);

            // Grid cells for each NPC's position
            let grid_cell = |px: f32, py: f32| -> (i32, i32, i32) {
                let cx = (px / CELL_SIZE) as usize;
                let cy = (py / CELL_SIZE) as usize;
                if cx < GRID_WIDTH && cy < GRID_HEIGHT {
                    let cell_idx = cy * GRID_WIDTH + cx;
                    (cx as i32, cy as i32, gpu.grid.counts.get(cell_idx).copied().unwrap_or(-1))
                } else {
                    (-1, -1, -1)
                }
            };
            let (cx0, cy0, gc0) = grid_cell(p0x, p0y);
            let (cx1, cy1, gc1) = grid_cell(p1x, p1y);
            dict.set("grid_cx_0", cx0);
            dict.set("grid_cy_0", cy0);
            dict.set("grid_cell_0", gc0);
            dict.set("grid_cx_1", cx1);
            dict.set("grid_cy_1", cy1);
            dict.set("grid_cell_1", gc1);

            // Direct GPU buffer reads (not CPU cache)
            let fac_bytes = gpu.rd.buffer_get_data(gpu.faction_buffer);
            let fac_slice = fac_bytes.as_slice();
            let hp_bytes = gpu.rd.buffer_get_data(gpu.health_buffer);
            let hp_slice = hp_bytes.as_slice();

            let read_i32 = |slice: &[u8], idx: usize| -> i32 {
                let o = idx * 4;
                if o + 4 <= slice.len() {
                    i32::from_le_bytes([slice[o], slice[o+1], slice[o+2], slice[o+3]])
                } else { -99 }
            };
            let read_f32 = |slice: &[u8], idx: usize| -> f32 {
                let o = idx * 4;
                if o + 4 <= slice.len() {
                    f32::from_le_bytes([slice[o], slice[o+1], slice[o+2], slice[o+3]])
                } else { -99.0 }
            };

            dict.set("gpu_faction_0", read_i32(fac_slice, 0));
            dict.set("gpu_faction_1", read_i32(fac_slice, 1));
            dict.set("gpu_health_0", read_f32(hp_slice, 0));
            dict.set("gpu_health_1", read_f32(hp_slice, 1));
            dict.set("gpu_health_2", read_f32(hp_slice, 2));
            dict.set("gpu_health_3", read_f32(hp_slice, 3));
        }
        dict
    }

    #[func]
    fn get_health_debug(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(debug) = app.world().get_resource::<resources::HealthDebug>() {
                    dict.set("damage_processed", debug.damage_processed as i32);
                    dict.set("deaths_this_frame", debug.deaths_this_frame as i32);
                    dict.set("despawned_this_frame", debug.despawned_this_frame as i32);
                    dict.set("bevy_entity_count", debug.bevy_entity_count as i32);

                    // Health samples as string for easy display
                    let samples: Vec<String> = debug.health_samples.iter()
                        .map(|(idx, hp)| format!("{}:{:.0}", idx, hp))
                        .collect();
                    dict.set("health_samples", GString::from(&samples.join(" ")));
                }
            }
        }
        dict
    }

    // ========================================================================
    // RESET API
    // ========================================================================

    #[func]
    fn reset(&mut self) {
        // Reset GPU dispatch count (SlotAllocator reset happens below with Bevy resources)
        if let Ok(mut c) = GPU_DISPATCH_COUNT.lock() { *c = 0; }

        // Reset GPU read state
        if let Ok(mut state) = GPU_READ_STATE.lock() {
            state.positions.clear();
            state.combat_targets.clear();
            state.health.clear();
            state.factions.clear();
            state.npc_count = 0;
        }

        // Clear remaining message queue (arrivals still use static)
        if let Ok(mut queue) = ARRIVAL_QUEUE.lock() { queue.clear(); }

        // GPU-FIRST: Clear consolidated GPU update queue
        if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() { queue.clear(); }

        // SLOT REUSE: Clear free projectile slot pool (NPC slots now in Bevy SlotAllocator)
        if let Ok(mut free) = FREE_PROJ_SLOTS.lock() { free.clear(); }

        // Reset projectile state
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.proj_count = 0;
            gpu.proj_positions.fill(0.0);
            gpu.proj_velocities.fill(0.0);
            gpu.proj_damages.fill(0.0);
            gpu.proj_factions.fill(0);
            gpu.proj_active.fill(0);
        }

        self.prev_arrivals.fill(false);

        // Reset Bevy Resources (world data, occupancy, stats, UI caches)
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut world) = app.world_mut().get_resource_mut::<world::WorldData>() {
                    world.towns.clear();
                    world.farms.clear();
                    world.beds.clear();
                    world.guard_posts.clear();
                }
                if let Some(mut beds) = app.world_mut().get_resource_mut::<world::BedOccupancy>() {
                    beds.occupant_npc.clear();
                }
                if let Some(mut farms) = app.world_mut().get_resource_mut::<world::FarmOccupancy>() {
                    farms.occupant_count.clear();
                }
                if let Some(mut kills) = app.world_mut().get_resource_mut::<resources::KillStats>() {
                    *kills = resources::KillStats::default();
                }
                if let Some(mut selected) = app.world_mut().get_resource_mut::<resources::SelectedNpc>() {
                    selected.0 = -1;
                }
                // Reset UI caches
                if let Some(mut meta) = app.world_mut().get_resource_mut::<resources::NpcMetaCache>() {
                    for m in meta.0.iter_mut() {
                        *m = resources::NpcMeta::default();
                    }
                }
                if let Some(mut energies) = app.world_mut().get_resource_mut::<resources::NpcEnergyCache>() {
                    energies.0.fill(100.0);
                }
                if let Some(mut by_town) = app.world_mut().get_resource_mut::<resources::NpcsByTownCache>() {
                    by_town.0.clear();
                }
                // Reset slot allocator
                if let Some(mut slots) = app.world_mut().get_resource_mut::<resources::SlotAllocator>() {
                    slots.reset();
                }
            }
        }

        // Send reset via channel
        if let Some(ref sender) = self.godot_to_bevy {
            let _ = sender.0.send(channels::GodotToBevyMsg::Reset);
        }
    }

    // ========================================================================
    // WORLD DATA API
    // ========================================================================

    #[func]
    fn init_world(&mut self, town_count: i32) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut world) = app.world_mut().get_resource_mut::<world::WorldData>() {
                    world.towns = Vec::with_capacity(town_count as usize);
                    world.farms = Vec::new();
                    world.beds = Vec::new();
                    world.guard_posts = Vec::new();
                }
                if let Some(mut beds) = app.world_mut().get_resource_mut::<world::BedOccupancy>() {
                    beds.occupant_npc = Vec::new();
                }
                if let Some(mut farms) = app.world_mut().get_resource_mut::<world::FarmOccupancy>() {
                    farms.occupant_count = Vec::new();
                }
                // Initialize per-town NPC lists for UI queries
                if let Some(mut by_town) = app.world_mut().get_resource_mut::<resources::NpcsByTownCache>() {
                    by_town.0.clear();
                    for _ in 0..town_count {
                        by_town.0.push(Vec::new());
                    }
                }
            }
        }
    }

    /// Add a town (villager or raider settlement).
    /// faction: 0=Villager, 1=Raider
    #[func]
    fn add_town(&mut self, name: GString, center_x: f32, center_y: f32, faction: i32) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut world) = app.world_mut().get_resource_mut::<world::WorldData>() {
                    world.towns.push(world::Town {
                        name: name.to_string(),
                        center: Vector2::new(center_x, center_y),
                        faction,
                    });
                }
            }
        }
    }

    #[func]
    fn add_farm(&mut self, x: f32, y: f32, town_idx: i32) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut world) = app.world_mut().get_resource_mut::<world::WorldData>() {
                    world.farms.push(world::Farm {
                        position: Vector2::new(x, y),
                        town_idx: town_idx as u32,
                    });
                }
                if let Some(mut farms) = app.world_mut().get_resource_mut::<world::FarmOccupancy>() {
                    farms.occupant_count.push(0);
                }
            }
        }
    }

    #[func]
    fn add_bed(&mut self, x: f32, y: f32, town_idx: i32) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut world) = app.world_mut().get_resource_mut::<world::WorldData>() {
                    world.beds.push(world::Bed {
                        position: Vector2::new(x, y),
                        town_idx: town_idx as u32,
                    });
                }
                if let Some(mut beds) = app.world_mut().get_resource_mut::<world::BedOccupancy>() {
                    beds.occupant_npc.push(-1);
                }
            }
        }
    }

    #[func]
    fn add_guard_post(&mut self, x: f32, y: f32, town_idx: i32, patrol_order: i32) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut world) = app.world_mut().get_resource_mut::<world::WorldData>() {
                    world.guard_posts.push(world::GuardPost {
                        position: Vector2::new(x, y),
                        town_idx: town_idx as u32,
                        patrol_order: patrol_order as u32,
                    });
                }
            }
        }
    }

    // ========================================================================
    // WORLD QUERY API
    // ========================================================================

    #[func]
    fn get_town_center(&self, town_idx: i32) -> Vector2 {
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(world) = app.world().get_resource::<world::WorldData>() {
                    if let Some(town) = world.towns.get(town_idx as usize) {
                        return town.center;
                    }
                }
            }
        }
        Vector2::ZERO
    }

    #[func]
    fn get_patrol_post(&self, town_idx: i32, patrol_order: i32) -> Vector2 {
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(world) = app.world().get_resource::<world::WorldData>() {
                    for post in &world.guard_posts {
                        if post.town_idx == town_idx as u32 && post.patrol_order == patrol_order as u32 {
                            return post.position;
                        }
                    }
                }
            }
        }
        Vector2::ZERO
    }

    #[func]
    fn get_nearest_free_bed(&self, town_idx: i32, x: f32, y: f32) -> i32 {
        let pos = Vector2::new(x, y);
        let mut best_idx: i32 = -1;
        let mut best_dist = f32::MAX;

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                let world = app.world().get_resource::<world::WorldData>();
                let beds = app.world().get_resource::<world::BedOccupancy>();
                if let (Some(world), Some(beds)) = (world, beds) {
                    for (i, bed) in world.beds.iter().enumerate() {
                        if bed.town_idx != town_idx as u32 { continue; }
                        if i >= beds.occupant_npc.len() { continue; }
                        if beds.occupant_npc[i] >= 0 { continue; }
                        let dist = pos.distance_to(bed.position);
                        if dist < best_dist {
                            best_dist = dist;
                            best_idx = i as i32;
                        }
                    }
                }
            }
        }
        best_idx
    }

    #[func]
    fn get_nearest_free_farm(&self, town_idx: i32, x: f32, y: f32) -> i32 {
        let pos = Vector2::new(x, y);
        let mut best_idx: i32 = -1;
        let mut best_dist = f32::MAX;

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                let world = app.world().get_resource::<world::WorldData>();
                let farms = app.world().get_resource::<world::FarmOccupancy>();
                if let (Some(world), Some(farms)) = (world, farms) {
                    for (i, farm) in world.farms.iter().enumerate() {
                        if farm.town_idx != town_idx as u32 { continue; }
                        if i >= farms.occupant_count.len() { continue; }
                        if farms.occupant_count[i] >= 1 { continue; }
                        let dist = pos.distance_to(farm.position);
                        if dist < best_dist {
                            best_dist = dist;
                            best_idx = i as i32;
                        }
                    }
                }
            }
        }
        best_idx
    }

    #[func]
    fn reserve_bed(&mut self, bed_idx: i32, npc_idx: i32) -> bool {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut beds) = app.world_mut().get_resource_mut::<world::BedOccupancy>() {
                    let idx = bed_idx as usize;
                    if idx < beds.occupant_npc.len() && beds.occupant_npc[idx] < 0 {
                        beds.occupant_npc[idx] = npc_idx;
                        return true;
                    }
                }
            }
        }
        false
    }

    #[func]
    fn release_bed(&mut self, bed_idx: i32) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut beds) = app.world_mut().get_resource_mut::<world::BedOccupancy>() {
                    let idx = bed_idx as usize;
                    if idx < beds.occupant_npc.len() {
                        beds.occupant_npc[idx] = -1;
                    }
                }
            }
        }
    }

    #[func]
    fn reserve_farm(&mut self, farm_idx: i32) -> bool {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut farms) = app.world_mut().get_resource_mut::<world::FarmOccupancy>() {
                    let idx = farm_idx as usize;
                    if idx < farms.occupant_count.len() && farms.occupant_count[idx] < 1 {
                        farms.occupant_count[idx] += 1;
                        return true;
                    }
                }
            }
        }
        false
    }

    #[func]
    fn release_farm(&mut self, farm_idx: i32) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut farms) = app.world_mut().get_resource_mut::<world::FarmOccupancy>() {
                    let idx = farm_idx as usize;
                    if idx < farms.occupant_count.len() && farms.occupant_count[idx] > 0 {
                        farms.occupant_count[idx] -= 1;
                    }
                }
            }
        }
    }

    // ========================================================================
    // FOOD STORAGE API
    // ========================================================================

    /// Initialize food storage for all towns (villager and raider).
    #[func]
    fn init_food_storage(&self, total_town_count: i32) {
        if let Ok(mut food) = FOOD_STORAGE.lock() {
            food.food = vec![0; total_town_count as usize];
        }
    }

    /// Add food to a town (called when farmer produces food).
    #[func]
    fn add_town_food(&self, town_idx: i32, amount: i32) {
        if let Ok(mut food) = FOOD_STORAGE.lock() {
            let idx = town_idx as usize;
            if idx < food.food.len() {
                food.food[idx] += amount;
            }
        }
    }

    /// Get food count for a town (works for both villager and raider towns).
    #[func]
    fn get_town_food(&self, town_idx: i32) -> i32 {
        if let Ok(food) = FOOD_STORAGE.lock() {
            let idx = town_idx as usize;
            if idx < food.food.len() {
                return food.food[idx];
            }
        }
        0
    }

    /// Get food events since last call (deliveries and consumption).
    #[func]
    fn get_food_events(&mut self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let mut deliveries = 0i32;
        let mut consumed = 0i32;
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut events) = app.world_mut().get_resource_mut::<resources::FoodEvents>() {
                    deliveries = events.delivered.len() as i32;
                    consumed = events.consumed.len() as i32;
                    events.delivered.clear();
                    events.consumed.clear();
                }
            }
        }
        dict.set("deliveries", deliveries);
        dict.set("consumed", consumed);
        dict
    }

    #[func]
    fn get_world_stats(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(world) = app.world().get_resource::<world::WorldData>() {
                    dict.set("town_count", world.towns.len() as i32);
                    dict.set("farm_count", world.farms.len() as i32);
                    dict.set("bed_count", world.beds.len() as i32);
                    dict.set("guard_post_count", world.guard_posts.len() as i32);
                }
                if let Some(beds) = app.world().get_resource::<world::BedOccupancy>() {
                    let free_beds = beds.occupant_npc.iter().filter(|&&x| x < 0).count();
                    dict.set("free_beds", free_beds as i32);
                }
                if let Some(farms) = app.world().get_resource::<world::FarmOccupancy>() {
                    let free_farms = farms.occupant_count.iter().filter(|&&x| x < 1).count();
                    dict.set("free_farms", free_farms as i32);
                }
            }
        }
        dict
    }

    // ========================================================================
    // TIME API
    // ========================================================================

    /// Get the BevyApp autoload node.
    fn get_bevy_app(&self) -> Option<Gd<godot_bevy::app::BevyApp>> {
        let tree = self.base().get_tree()?;
        let root = tree.get_root()?;
        // Window inherits from Node, use upcast to access try_get_node_as
        let root_node: Gd<godot::classes::Node> = root.upcast();
        root_node.try_get_node_as::<godot_bevy::app::BevyApp>("BevyAppSingleton")
    }

    /// Get current game time.
    #[func]
    fn get_game_time(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(time) = app.world().get_resource::<resources::GameTime>() {
                    dict.set("day", time.day());
                    dict.set("hour", time.hour());
                    dict.set("minute", time.minute());
                    dict.set("is_daytime", time.is_daytime());
                    dict.set("time_scale", time.time_scale);
                    dict.set("paused", time.paused);
                }
            }
        }
        dict
    }

    /// Set game time scale (1.0 = normal, 2.0 = 2x speed).
    #[func]
    fn set_time_scale(&mut self, scale: f32) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut time) = app.world_mut().get_resource_mut::<resources::GameTime>() {
                    time.time_scale = scale.max(0.0);
                }
            }
        }
    }

    /// Pause or unpause game time.
    #[func]
    fn set_paused(&mut self, paused: bool) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut time) = app.world_mut().get_resource_mut::<resources::GameTime>() {
                    time.paused = paused;
                }
            }
        }
    }

    // ========================================================================
    // UI QUERY API (Phase 9.4)
    // ========================================================================

    /// Get population statistics for UI display.
    #[func]
    fn get_population_stats(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let mut farmers_alive = 0i32;
        let mut guards_alive = 0i32;
        let mut raiders_alive = 0i32;

        // Debug counters
        let mut debug_towns = 0i32;
        let mut debug_total_in_cache = 0i32;
        let mut debug_health_len = 0i32;

        // Count alive NPCs from NpcMetaCache + GPU health
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let (Some(by_town), Some(meta)) = (
                    app.world().get_resource::<resources::NpcsByTownCache>(),
                    app.world().get_resource::<resources::NpcMetaCache>(),
                ) {
                    debug_towns = by_town.0.len() as i32;
                    for town_npcs in by_town.0.iter() {
                        debug_total_in_cache += town_npcs.len() as i32;
                    }

                    if let Ok(state) = GPU_READ_STATE.lock() {
                        debug_health_len = state.health.len() as i32;
                        for town_npcs in by_town.0.iter() {
                            for &idx in town_npcs {
                                if idx < state.health.len() && state.health[idx] > 0.0 {
                                    match meta.0[idx].job {
                                        0 => farmers_alive += 1,
                                        1 => guards_alive += 1,
                                        2 => raiders_alive += 1,
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(kills) = app.world().get_resource::<resources::KillStats>() {
                    dict.set("guard_kills", kills.guard_kills);
                    dict.set("villager_kills", kills.villager_kills);
                }
            }
        }

        dict.set("farmers_alive", farmers_alive);
        dict.set("guards_alive", guards_alive);
        dict.set("raiders_alive", raiders_alive);
        // Debug info
        dict.set("_debug_towns", debug_towns);
        dict.set("_debug_cache_total", debug_total_in_cache);
        dict.set("_debug_health_len", debug_health_len);
        dict
    }

    /// Get population for a specific town.
    #[func]
    fn get_town_population(&self, town_idx: i32) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let mut farmer_count = 0i32;
        let mut guard_count = 0i32;
        let mut raider_count = 0i32;

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let (Some(by_town), Some(meta)) = (
                    app.world().get_resource::<resources::NpcsByTownCache>(),
                    app.world().get_resource::<resources::NpcMetaCache>(),
                ) {
                    if let Ok(state) = GPU_READ_STATE.lock() {
                        if (town_idx as usize) < by_town.0.len() {
                            for &idx in &by_town.0[town_idx as usize] {
                                if idx < state.health.len() && state.health[idx] > 0.0 {
                                    match meta.0[idx].job {
                                        0 => farmer_count += 1,
                                        1 => guard_count += 1,
                                        2 => raider_count += 1,
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        dict.set("farmer_count", farmer_count);
        dict.set("guard_count", guard_count);
        dict.set("raider_count", raider_count);
        dict
    }

    /// Get detailed info for a single NPC (for inspector panel).
    #[func]
    fn get_npc_info(&self, idx: i32) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let i = idx as usize;

        if let Ok(state) = GPU_READ_STATE.lock() {
            if i < state.npc_count {
                dict.set("hp", state.health.get(i).copied().unwrap_or(0.0));
                dict.set("x", state.positions.get(i * 2).copied().unwrap_or(0.0));
                dict.set("y", state.positions.get(i * 2 + 1).copied().unwrap_or(0.0));
                dict.set("faction", state.factions.get(i).copied().unwrap_or(0));
                dict.set("target_idx", state.combat_targets.get(i).copied().unwrap_or(-1));
            }
        }

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(meta) = app.world().get_resource::<resources::NpcMetaCache>() {
                    if i < meta.0.len() {
                        dict.set("name", GString::from(&meta.0[i].name));
                        dict.set("level", meta.0[i].level);
                        dict.set("xp", meta.0[i].xp);
                        dict.set("trait", GString::from(trait_name(meta.0[i].trait_id)));
                        dict.set("town_id", meta.0[i].town_id);
                        dict.set("job", GString::from(job_name(meta.0[i].job)));
                    }
                }
                if let Some(npc_map) = app.world().get_resource::<NpcEntityMap>() {
                    if let Some(&entity) = npc_map.0.get(&i) {
                        dict.set("state", GString::from(derive_npc_state(app.world(), entity)));
                    }
                }
                if let Some(energies) = app.world().get_resource::<resources::NpcEnergyCache>() {
                    dict.set("energy", energies.0.get(i).copied().unwrap_or(0.0));
                }
            }
        }

        dict.set("max_hp", 100.0);
        dict
    }

    /// Get activity log for an NPC (decisions, state changes, combat events).
    /// Returns array of dicts with {day, hour, minute, message} for last N entries.
    #[func]
    fn get_npc_log(&self, idx: i32, limit: i32) -> VarArray {
        let mut result = VarArray::new();
        let i = idx as usize;
        let limit = limit.max(1) as usize;

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(logs) = app.world().get_resource::<resources::NpcLogCache>() {
                    if let Some(log) = logs.0.get(i) {
                        // Get last `limit` entries (most recent first)
                        let entries: Vec<_> = log.iter().collect();
                        let start = entries.len().saturating_sub(limit);
                        for entry in entries[start..].iter().rev() {
                            let mut entry_dict = VarDictionary::new();
                            entry_dict.set("day", entry.day);
                            entry_dict.set("hour", entry.hour);
                            entry_dict.set("minute", entry.minute);
                            entry_dict.set("message", GString::from(&entry.message));
                            result.push(&entry_dict.to_variant());
                        }
                    }
                }
            }
        }

        result
    }

    /// Get list of NPCs in a town (for roster panel).
    #[func]
    fn get_npcs_by_town(&self, town_idx: i32, filter: i32) -> VarArray {
        let mut result = VarArray::new();

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let (Some(by_town), Some(meta), Some(npc_map)) = (
                    app.world().get_resource::<resources::NpcsByTownCache>(),
                    app.world().get_resource::<resources::NpcMetaCache>(),
                    app.world().get_resource::<NpcEntityMap>(),
                ) {
                    if let Ok(gpu_state) = GPU_READ_STATE.lock() {
                        if (town_idx as usize) < by_town.0.len() {
                            for &idx in &by_town.0[town_idx as usize] {
                                // Skip dead NPCs
                                if idx >= gpu_state.health.len() || gpu_state.health[idx] <= 0.0 {
                                    continue;
                                }

                                // Apply job filter (-1 = all)
                                let job = meta.0[idx].job;
                                if filter >= 0 && job != filter {
                                    continue;
                                }

                                let state = npc_map.0.get(&idx)
                                    .map(|&e| derive_npc_state(app.world(), e))
                                    .unwrap_or("Idle");

                                let mut npc_dict = VarDictionary::new();
                                npc_dict.set("idx", idx as i32);
                                npc_dict.set("name", GString::from(&meta.0[idx].name));
                                npc_dict.set("job", GString::from(job_name(job)));
                                npc_dict.set("level", meta.0[idx].level);
                                npc_dict.set("hp", gpu_state.health[idx]);
                                npc_dict.set("max_hp", 100.0f32);
                                npc_dict.set("state", GString::from(state));
                                npc_dict.set("trait", GString::from(trait_name(meta.0[idx].trait_id)));

                                result.push(&npc_dict.to_variant());
                            }
                        }
                    }
                }
            }
        }

        result
    }

    /// Get currently selected NPC index.
    #[func]
    fn get_selected_npc(&self) -> i32 {
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(selected) = app.world().get_resource::<resources::SelectedNpc>() {
                    return selected.0;
                }
            }
        }
        -1
    }

    /// Set currently selected NPC index.
    #[func]
    fn set_selected_npc(&mut self, idx: i32) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut selected) = app.world_mut().get_resource_mut::<resources::SelectedNpc>() {
                    selected.0 = idx;
                }
            }
        }
    }

    /// Get NPC name by index.
    #[func]
    fn get_npc_name(&self, idx: i32) -> GString {
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(meta) = app.world().get_resource::<resources::NpcMetaCache>() {
                    if (idx as usize) < meta.0.len() {
                        return GString::from(&meta.0[idx as usize].name);
                    }
                }
            }
        }
        GString::new()
    }

    /// Get NPC trait by index.
    #[func]
    fn get_npc_trait(&self, idx: i32) -> i32 {
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(meta) = app.world().get_resource::<resources::NpcMetaCache>() {
                    if (idx as usize) < meta.0.len() {
                        return meta.0[idx as usize].trait_id;
                    }
                }
            }
        }
        0
    }

    /// Set NPC name (for rename feature).
    #[func]
    fn set_npc_name(&mut self, idx: i32, name: GString) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut meta) = app.world_mut().get_resource_mut::<resources::NpcMetaCache>() {
                    if (idx as usize) < meta.0.len() {
                        meta.0[idx as usize].name = name.to_string();
                    }
                }
            }
        }
    }

    /// Find nearest NPC at a position within radius (for click selection).
    #[func]
    fn get_npc_at_position(&self, x: f32, y: f32, radius: f32) -> i32 {
        let mut best_idx: i32 = -1;
        let mut best_dist = radius;

        // Use slot counter (high-water mark) not dispatch count to avoid timing issues
        let slot_count = NPC_SLOT_COUNTER.lock().map(|c| *c).unwrap_or(0);

        if let Some(gpu) = &self.gpu {
            for i in 0..slot_count {
                // Skip dead NPCs (health <= 0)
                let health = gpu.healths.get(i).copied().unwrap_or(0.0);
                if health <= 0.0 {
                    continue;
                }

                let px = gpu.positions.get(i * 2).copied().unwrap_or(0.0);
                let py = gpu.positions.get(i * 2 + 1).copied().unwrap_or(0.0);
                let dx = px - x;
                let dy = py - y;
                let dist = (dx * dx + dy * dy).sqrt();

                if dist < best_dist {
                    best_dist = dist;
                    best_idx = i as i32;
                }
            }
        }

        best_idx
    }

    /// Get bed statistics for a town.
    #[func]
    fn get_bed_stats(&self, town_idx: i32) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let mut total = 0i32;
        let mut free = 0i32;

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let (Some(world), Some(beds)) = (
                    app.world().get_resource::<world::WorldData>(),
                    app.world().get_resource::<world::BedOccupancy>(),
                ) {
                    for (i, bed) in world.beds.iter().enumerate() {
                        if bed.town_idx == town_idx as u32 {
                            total += 1;
                            if i < beds.occupant_npc.len() && beds.occupant_npc[i] < 0 {
                                free += 1;
                            }
                        }
                    }
                }
            }
        }

        dict.set("total_beds", total);
        dict.set("free_beds", free);
        dict
    }

    #[func]
    fn get_guard_debug(&mut self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Some(gpu) = &mut self.gpu {
            let npc_count = GPU_READ_STATE.lock().map(|s| s.npc_count).unwrap_or(0);

            let arrival_bytes = gpu.rd.buffer_get_data(gpu.arrival_buffer);
            let arrival_slice = arrival_bytes.as_slice();
            let mut arrived_flags = 0;
            for i in 0..npc_count {
                if arrival_slice.len() >= (i + 1) * 4 {
                    let val = i32::from_le_bytes([
                        arrival_slice[i * 4],
                        arrival_slice[i * 4 + 1],
                        arrival_slice[i * 4 + 2],
                        arrival_slice[i * 4 + 3],
                    ]);
                    if val > 0 { arrived_flags += 1; }
                }
            }

            let prev_true = self.prev_arrivals.iter().take(npc_count).filter(|&&x| x).count();
            let queue_len = ARRIVAL_QUEUE.lock().map(|q| q.len()).unwrap_or(0);

            dict.set("arrived_flags", arrived_flags as i32);
            dict.set("prev_arrivals_true", prev_true as i32);
            dict.set("arrival_queue_len", queue_len as i32);
        }
        dict
    }

    // ========================================================================
    // CHANNEL API (Phase 11)
    // ========================================================================

    /// Send message to Bevy via channel.
    /// Commands: spawn, target, damage, select, reset, pause, time_scale
    #[func]
    fn godot_to_bevy(&mut self, cmd: GString, data: VarDictionary) {
        // For spawn, allocate slot first (needs &mut self)
        let spawn_slot = if cmd.to_string() == "spawn" {
            match self.allocate_slot() {
                Some(i) => Some(i),
                None => return,
            }
        } else {
            None
        };

        let sender = match &self.godot_to_bevy {
            Some(s) => s,
            None => return,
        };

        let msg = match cmd.to_string().as_str() {
            "spawn" => {
                let slot_idx = spawn_slot.unwrap();
                channels::GodotToBevyMsg::SpawnNpc {
                    slot_idx,
                    x: data.get("x").and_then(|v| v.try_to().ok()).unwrap_or(0.0),
                    y: data.get("y").and_then(|v| v.try_to().ok()).unwrap_or(0.0),
                    job: data.get("job").and_then(|v| v.try_to::<i32>().ok()).unwrap_or(0) as u8,
                    faction: data.get("faction").and_then(|v| v.try_to::<i32>().ok()).unwrap_or(0) as u8,
                    town_idx: data.get("town_idx").and_then(|v| v.try_to().ok()).unwrap_or(-1),
                    home_x: data.get("home_x").and_then(|v| v.try_to().ok()).unwrap_or(-1.0),
                    home_y: data.get("home_y").and_then(|v| v.try_to().ok()).unwrap_or(-1.0),
                    work_x: data.get("work_x").and_then(|v| v.try_to().ok()).unwrap_or(-1.0),
                    work_y: data.get("work_y").and_then(|v| v.try_to().ok()).unwrap_or(-1.0),
                    starting_post: data.get("starting_post").and_then(|v| v.try_to().ok()).unwrap_or(-1),
                    attack_type: data.get("attack_type").and_then(|v| v.try_to::<i32>().ok()).unwrap_or(0) as u8,
                }
            },
            "target" => channels::GodotToBevyMsg::SetTarget {
                slot: data.get("slot").and_then(|v| v.try_to::<i32>().ok()).unwrap_or(0) as usize,
                x: data.get("x").and_then(|v| v.try_to().ok()).unwrap_or(0.0),
                y: data.get("y").and_then(|v| v.try_to().ok()).unwrap_or(0.0),
            },
            "damage" => channels::GodotToBevyMsg::ApplyDamage {
                slot: data.get("slot").and_then(|v| v.try_to::<i32>().ok()).unwrap_or(0) as usize,
                amount: data.get("amount").and_then(|v| v.try_to().ok()).unwrap_or(0.0),
            },
            "select" => channels::GodotToBevyMsg::SelectNpc {
                slot: data.get("slot").and_then(|v| v.try_to().ok()).unwrap_or(-1),
            },
            "click" => channels::GodotToBevyMsg::PlayerClick {
                x: data.get("x").and_then(|v| v.try_to().ok()).unwrap_or(0.0),
                y: data.get("y").and_then(|v| v.try_to().ok()).unwrap_or(0.0),
            },
            "reset" => channels::GodotToBevyMsg::Reset,
            "pause" => channels::GodotToBevyMsg::SetPaused(
                data.get("paused").and_then(|v| v.try_to().ok()).unwrap_or(false)
            ),
            "time_scale" => channels::GodotToBevyMsg::SetTimeScale(
                data.get("scale").and_then(|v| v.try_to().ok()).unwrap_or(1.0)
            ),
            _ => return,
        };

        let _ = sender.0.send(msg);
    }

    /// Poll messages from Bevy. Call every frame.
    #[func]
    fn bevy_to_godot(&self) -> VarArray {
        let mut result = VarArray::new();

        let receiver = match &self.bevy_to_godot {
            Some(r) => r,
            None => return result,
        };

        while let Ok(msg) = receiver.0.try_recv() {
            let mut d = VarDictionary::new();
            match msg {
                channels::BevyToGodotMsg::SpawnView { slot, job, x, y } => {
                    d.set("type", "spawn_view");
                    d.set("slot", slot as i32);
                    d.set("job", job as i32);
                    d.set("x", x);
                    d.set("y", y);
                }
                channels::BevyToGodotMsg::DespawnView { slot } => {
                    d.set("type", "despawn_view");
                    d.set("slot", slot as i32);
                }
                channels::BevyToGodotMsg::SyncTransform { slot, x, y } => {
                    d.set("type", "sync_transform");
                    d.set("slot", slot as i32);
                    d.set("x", x);
                    d.set("y", y);
                }
                channels::BevyToGodotMsg::SyncHealth { slot, hp, max_hp } => {
                    d.set("type", "sync_health");
                    d.set("slot", slot as i32);
                    d.set("hp", hp);
                    d.set("max_hp", max_hp);
                }
                channels::BevyToGodotMsg::SyncColor { slot, r, g, b, a } => {
                    d.set("type", "sync_color");
                    d.set("slot", slot as i32);
                    d.set("r", r);
                    d.set("g", g);
                    d.set("b", b);
                    d.set("a", a);
                }
                channels::BevyToGodotMsg::SyncSprite { slot, col, row } => {
                    d.set("type", "sync_sprite");
                    d.set("slot", slot as i32);
                    d.set("col", col);
                    d.set("row", row);
                }
                channels::BevyToGodotMsg::FireProjectile {
                    from_x, from_y, to_x, to_y, speed, damage, faction, shooter, lifetime,
                } => {
                    d.set("type", "fire_projectile");
                    d.set("from_x", from_x);
                    d.set("from_y", from_y);
                    d.set("to_x", to_x);
                    d.set("to_y", to_y);
                    d.set("speed", speed);
                    d.set("damage", damage);
                    d.set("faction", faction);
                    d.set("shooter", shooter as i32);
                    d.set("lifetime", lifetime);
                }
            }
            result.push(&d.to_variant());
        }

        result
    }
}
