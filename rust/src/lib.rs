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
use gpu::{GpuCompute, DirtyRange};
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
    if world.get::<Wandering>(entity).is_some() { return "Wandering"; }
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

fn bevy_timer_start(mut timer: ResMut<resources::BevyFrameTimer>) {
    timer.start = Some(std::time::Instant::now());
}

fn bevy_timer_end(timer: Res<resources::BevyFrameTimer>) {
    if let Some(start) = timer.start {
        let elapsed = start.elapsed().as_secs_f32() * 1000.0;
        if let Ok(mut stats) = resources::PERF_STATS.lock() {
            stats.bevy_ms = elapsed;
        }
    }
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
       .init_resource::<resources::FarmStates>()
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
       .init_resource::<resources::FactionStats>()
       .init_resource::<resources::BevyFrameTimer>()
       // Chain phases with explicit command flush between Spawn and Combat
       .configure_sets(Update, (Step::Drain, Step::Spawn, Step::Combat, Step::Behavior).chain())
       // Timing: start timer before everything
       .add_systems(Update, bevy_timer_start.before(Step::Drain))
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
       // Behavior: energy, economy, unified decisions
       .add_systems(Update, (
           arrival_system,
           energy_system,
           healing_system,
           on_duty_tick_system,
           game_time_system,
           farm_growth_system,
           decision_system,
       ).in_set(Step::Behavior))
       // Collect GPU updates at end of frame (single Mutex lock point)
       .add_systems(Update, collect_gpu_updates.after(Step::Behavior))
       // Phase 11: Write changed state to BevyToGodot outbox
       .add_systems(Update, bevy_to_godot_write.after(Step::Behavior))
       // Timing: record Bevy frame time
       .add_systems(Update, bevy_timer_end.after(bevy_to_godot_write));
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

    // === Carried Item Rendering ===
    /// MultiMesh for carried items (above NPC heads) and farm progress icons
    item_multimesh_rid: Rid,
    /// Canvas item for item MultiMesh (above NPCs)
    item_canvas_item: Rid,
    /// Keep item mesh alive
    #[allow(dead_code)]
    item_mesh: Option<Gd<QuadMesh>>,
    /// Keep item material alive
    #[allow(dead_code)]
    item_material: Option<Gd<ShaderMaterial>>,

    // === Location Rendering (static buildings) ===
    /// MultiMesh for locations (farms, beds, posts, fountains, camps)
    location_multimesh_rid: Rid,
    /// Canvas item for location MultiMesh (behind NPCs)
    location_canvas_item: Rid,
    /// Keep location mesh alive
    #[allow(dead_code)]
    location_mesh: Option<Gd<QuadMesh>>,
    /// Keep location material alive
    #[allow(dead_code)]
    location_material: Option<Gd<ShaderMaterial>>,
    /// Sprite count for location MultiMesh
    location_sprite_count: usize,

    // === Channels (Phase 11) ===
    /// Sender for Godot → Bevy messages
    godot_to_bevy: Option<channels::GodotToBevySender>,
    /// Receiver for Bevy → Godot messages
    bevy_to_godot: Option<channels::BevyToGodotReceiver>,

    // === Timing ===
    /// Last process() call timestamp for frame timing
    last_process_time: Option<std::time::Instant>,

    // === BevyApp Cache ===
    /// Cached reference to BevyApp to avoid scene tree traversal every frame
    bevy_app_cache: Option<Gd<godot_bevy::app::BevyApp>>,
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
            item_multimesh_rid: Rid::Invalid,
            item_canvas_item: Rid::Invalid,
            item_mesh: None,
            item_material: None,
            location_multimesh_rid: Rid::Invalid,
            location_canvas_item: Rid::Invalid,
            location_mesh: None,
            location_material: None,
            location_sprite_count: 0,
            godot_to_bevy: None,
            bevy_to_godot: None,
            last_process_time: None,
            bevy_app_cache: None,
        }
    }

    fn ready(&mut self) {
        godot_print!("[EcsNpcManager] DLL built: {} {}", compile_time::date_str!(), compile_time::time_str!());
        self.gpu = GpuCompute::new();
        if self.gpu.is_none() {
            godot_error!("[EcsNpcManager] Failed to initialize GPU compute");
            return;
        }
        self.setup_multimesh(MAX_NPC_COUNT as i32);
        self.setup_proj_multimesh(MAX_PROJECTILES as i32);
        self.setup_item_multimesh((MAX_NPC_COUNT + MAX_FARMS) as i32);
        self.setup_location_multimesh();

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

        // Cache BevyApp reference to avoid scene tree traversal every frame
        self.bevy_app_cache = self.get_bevy_app();
    }

    fn process(&mut self, delta: f64) {
        let frame_start = std::time::Instant::now();

        // Calculate full frame time (includes Godot rendering from previous frame)
        if let Some(last) = self.last_process_time {
            let frame_ms = last.elapsed().as_secs_f32() * 1000.0;
            if let Ok(mut stats) = resources::PERF_STATS.lock() {
                stats.frame_ms = frame_ms;
            }
        }
        self.last_process_time = Some(frame_start);

        // Get farm data for item rendering BEFORE borrowing gpu mutably
        // Use cached BevyApp reference to avoid scene tree traversal every frame
        // Tuple: (x, y, progress) where progress 0.0-1.0, 1.0 = ready
        let farm_data: Vec<(f32, f32, f32)> = if let Some(bevy_app) = self.get_bevy_app_cached() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                let world_data = app.world().get_resource::<world::WorldData>();
                let farm_states = app.world().get_resource::<resources::FarmStates>();
                if let (Some(wd), Some(fs)) = (world_data, farm_states) {
                    wd.farms.iter().enumerate().map(|(i, farm)| {
                        let progress = if i < fs.progress.len() { fs.progress[i] } else { 0.0 };
                        (farm.position.x, farm.position.y, progress)
                    }).collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let gpu = match self.gpu.as_mut() {
            Some(g) => g,
            None => return,
        };

        // Get dispatch count: only includes NPCs with initialized GPU buffers
        let npc_count = GPU_DISPATCH_COUNT.lock().map(|c| *c).unwrap_or(0);

        // Drain GPU update queue with BATCHED uploads.
        // Instead of individual buffer_update() calls per message, we:
        // 1. Update CPU caches and track dirty ranges
        // 2. Batch upload each dirty buffer once at the end
        let mut target_dirty = DirtyRange::new();
        let mut arrival_dirty = DirtyRange::new();
        let mut backoff_dirty = DirtyRange::new();
        let mut health_dirty = DirtyRange::new();
        let mut position_dirty = DirtyRange::new();
        let mut color_dirty = DirtyRange::new();
        let mut faction_dirty = DirtyRange::new();
        let mut speed_dirty = DirtyRange::new();

        if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
            for update in queue.drain(..) {
                match update {
                    GpuUpdate::SetTarget { idx, x, y } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.targets[idx * 2] = x;
                            gpu.targets[idx * 2 + 1] = y;
                            gpu.arrivals[idx] = 0;
                            gpu.backoffs[idx] = 0;
                            target_dirty.mark(idx);
                            arrival_dirty.mark(idx);
                            backoff_dirty.mark(idx);
                            self.prev_arrivals[idx] = false;
                        }
                    }
                    GpuUpdate::ApplyDamage { idx, amount } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.healths[idx] = (gpu.healths[idx] - amount).max(0.0);
                            health_dirty.mark(idx);
                        }
                    }
                    GpuUpdate::HideNpc { idx } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.positions[idx * 2] = -9999.0;
                            gpu.positions[idx * 2 + 1] = -9999.0;
                            gpu.targets[idx * 2] = -9999.0;
                            gpu.targets[idx * 2 + 1] = -9999.0;
                            gpu.arrivals[idx] = 1;
                            gpu.healths[idx] = 0.0;
                            position_dirty.mark(idx);
                            target_dirty.mark(idx);
                            arrival_dirty.mark(idx);
                            health_dirty.mark(idx);
                        }
                    }
                    GpuUpdate::SetFaction { idx, faction } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.factions[idx] = faction;
                            faction_dirty.mark(idx);
                        }
                    }
                    GpuUpdate::SetHealth { idx, health } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.healths[idx] = health;
                            health_dirty.mark(idx);
                        }
                    }
                    GpuUpdate::SetPosition { idx, x, y } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.positions[idx * 2] = x;
                            gpu.positions[idx * 2 + 1] = y;
                            position_dirty.mark(idx);
                        }
                    }
                    GpuUpdate::SetSpeed { idx, speed } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.speeds[idx] = speed;
                            speed_dirty.mark(idx);
                        }
                    }
                    GpuUpdate::SetColor { idx, r, g, b, a } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.colors[idx * 4] = r;
                            gpu.colors[idx * 4 + 1] = g;
                            gpu.colors[idx * 4 + 2] = b;
                            gpu.colors[idx * 4 + 3] = a;
                            color_dirty.mark(idx);
                        }
                    }
                    GpuUpdate::SetSpriteFrame { idx, col, row } => {
                        if idx < MAX_NPC_COUNT {
                            // CPU cache only - used by build_multimesh_from_cache()
                            gpu.sprite_frames[idx * 2] = col;
                            gpu.sprite_frames[idx * 2 + 1] = row;
                        }
                    }
                    GpuUpdate::SetHealing { idx, healing } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.healing_flags[idx] = healing;
                        }
                    }
                    GpuUpdate::SetCarriedItem { idx, item_id } => {
                        if idx < MAX_NPC_COUNT {
                            gpu.carried_items[idx] = item_id;
                        }
                    }
                }
            }
        }

        // Batch upload dirty ranges (one buffer_update per buffer type)
        gpu.upload_targets_range(&target_dirty);
        gpu.upload_arrivals_range(&arrival_dirty);
        gpu.upload_backoffs_range(&backoff_dirty);
        gpu.upload_healths_range(&health_dirty);
        gpu.upload_positions_range(&position_dirty);
        gpu.upload_colors_range(&color_dirty);
        gpu.upload_factions_range(&faction_dirty);
        gpu.upload_speeds_range(&speed_dirty);

        let t_queue = frame_start.elapsed();
        if npc_count > 0 {
            let t0 = std::time::Instant::now();
            gpu.dispatch(npc_count, delta as f32);
            let t_dispatch = t0.elapsed();

            let t1 = std::time::Instant::now();
            gpu.read_positions_from_gpu(npc_count);
            let t_readpos = t1.elapsed();

            // Read combat targets from GPU
            let t2 = std::time::Instant::now();
            gpu.read_combat_targets(npc_count);
            let t_combat = t2.elapsed();

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

            // Detect arrivals + cache debug stats (avoids extra GPU reads later)
            let arrival_bytes = gpu.rd.buffer_get_data(gpu.arrival_buffer);
            let arrival_slice = arrival_bytes.as_slice();
            let backoff_bytes = gpu.rd.buffer_get_data(gpu.backoff_buffer);
            let backoff_slice = backoff_bytes.as_slice();

            let mut arrived_count = 0i32;
            let mut total_backoff = 0i32;
            let mut max_backoff = 0i32;

            if let Ok(mut queue) = ARRIVAL_QUEUE.lock() {
                for i in 0..npc_count {
                    if arrival_slice.len() >= (i + 1) * 4 {
                        let arrived = i32::from_le_bytes([
                            arrival_slice[i * 4],
                            arrival_slice[i * 4 + 1],
                            arrival_slice[i * 4 + 2],
                            arrival_slice[i * 4 + 3],
                        ]) > 0;

                        if arrived {
                            arrived_count += 1;
                        }
                        if arrived && !self.prev_arrivals[i] {
                            queue.push(ArrivalMsg { npc_index: i });
                        }
                        self.prev_arrivals[i] = arrived;
                    }
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
            }

            // Cache debug stats
            if let Ok(mut stats) = resources::PERF_STATS.lock() {
                stats.arrived_count = arrived_count;
                stats.avg_backoff = if npc_count > 0 { total_backoff / npc_count as i32 } else { 0 };
                stats.max_backoff = max_backoff;
            }

            // Update NPC MultiMesh
            let t3 = std::time::Instant::now();
            let buffer = gpu.build_multimesh_from_cache(&gpu.colors, npc_count, MAX_NPC_COUNT);
            let t_build = t3.elapsed();

            let t4 = std::time::Instant::now();
            let mut rs = RenderingServer::singleton();
            rs.multimesh_set_buffer(self.multimesh_rid, &buffer);
            let t_upload = t4.elapsed();

            // Update carried item MultiMesh (includes farm food icons)
            let item_buffer = gpu.build_item_multimesh(npc_count, MAX_NPC_COUNT, &farm_data);
            rs.multimesh_set_buffer(self.item_multimesh_rid, &item_buffer);

            // Update perf stats
            if let Ok(mut stats) = resources::PERF_STATS.lock() {
                stats.queue_ms = t_queue.as_secs_f32() * 1000.0;
                stats.dispatch_ms = t_dispatch.as_secs_f32() * 1000.0;
                stats.readpos_ms = t_readpos.as_secs_f32() * 1000.0;
                stats.combat_ms = t_combat.as_secs_f32() * 1000.0;
                stats.build_ms = t_build.as_secs_f32() * 1000.0;
                stats.upload_ms = t_upload.as_secs_f32() * 1000.0;
            }

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
    /// Get DLL build timestamp for version checking.
    #[func]
    fn get_build_time(&self) -> GString {
        GString::from(format!("{} {}", compile_time::date_str!(), compile_time::time_str!()).as_str())
    }

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

    fn setup_item_multimesh(&mut self, max_count: i32) {
        let mut rs = RenderingServer::singleton();

        self.item_multimesh_rid = rs.multimesh_create();

        // Small quad for carried item icon (8x8, rendered above NPC head)
        let mut mesh = QuadMesh::new_gd();
        mesh.set_size(Vector2::new(8.0, 8.0));
        let mesh_rid = mesh.get_rid();
        rs.multimesh_set_mesh(self.item_multimesh_rid, mesh_rid);

        // Color + custom_data for progress bar (INSTANCE_CUSTOM.r = progress)
        rs.multimesh_allocate_data_ex(
            self.item_multimesh_rid,
            max_count,
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
        ).color_format(true).custom_data_format(true).done();

        // Initialize all items as hidden (position -9999)
        let count = max_count as usize;
        let mut init_buffer = vec![0.0f32; count * 16]; // Transform2D(8) + Color(4) + CustomData(4)
        for i in 0..count {
            let base = i * 16;
            init_buffer[base + 0] = 1.0;      // scale x
            init_buffer[base + 3] = -9999.0;  // pos x (hidden)
            init_buffer[base + 5] = 1.0;      // scale y
            init_buffer[base + 7] = -9999.0;  // pos y (hidden)
            init_buffer[base + 11] = 1.0;     // color alpha
            // CustomData[0-3] = 0.0 (progress = 0)
        }
        let packed = PackedFloat32Array::from(init_buffer.as_slice());
        rs.multimesh_set_buffer(self.item_multimesh_rid, &packed);

        // Create canvas item for items (above NPCs)
        self.item_canvas_item = rs.canvas_item_create();
        let parent_canvas = self.base().get_canvas_item();
        rs.canvas_item_set_parent(self.item_canvas_item, parent_canvas);
        rs.canvas_item_set_z_index(self.item_canvas_item, 10);  // Above NPCs (z=0)

        // Load and apply item icon shader with sprite sheet
        let mut loader = ResourceLoader::singleton();
        if let Some(shader) = loader.load("res://shaders/item_icon.gdshader") {
            if let Some(texture) = loader.load("res://assets/roguelikeSheet_transparent.png") {
                let mut material = ShaderMaterial::new_gd();
                let shader_res: Gd<godot::classes::Shader> = shader.cast();
                material.set_shader(&shader_res);
                material.set_shader_parameter("sprite_sheet", &texture.to_variant());
                rs.canvas_item_set_material(self.item_canvas_item, material.get_rid());
                self.item_material = Some(material);
            }
        }

        rs.canvas_item_add_multimesh(self.item_canvas_item, self.item_multimesh_rid);
        // Disable visibility culling for world-spanning MultiMesh
        rs.canvas_item_set_custom_rect_ex(self.item_canvas_item, true).rect(Rect2::new(Vector2::new(-100000.0, -100000.0), Vector2::new(200000.0, 200000.0))).done();

        self.item_mesh = Some(mesh);
    }

    /// Max location sprites (farms, beds, posts, fountains, camps).
    /// Each multi-cell sprite (2x2 farm) uses multiple slots.
    const MAX_LOCATION_SPRITES: usize = 10_000;

    fn setup_location_multimesh(&mut self) {
        let mut rs = RenderingServer::singleton();

        self.location_multimesh_rid = rs.multimesh_create();

        let mut mesh = QuadMesh::new_gd();
        mesh.set_size(Vector2::new(world::SPRITE_SIZE, world::SPRITE_SIZE));
        let mesh_rid = mesh.get_rid();
        rs.multimesh_set_mesh(self.location_multimesh_rid, mesh_rid);

        // Allocate max capacity upfront (like NPC MultiMesh)
        rs.multimesh_allocate_data_ex(
            self.location_multimesh_rid,
            Self::MAX_LOCATION_SPRITES as i32,
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
        ).custom_data_format(true).done();

        // Initialize all as hidden
        let mut init_buffer = vec![0.0f32; Self::MAX_LOCATION_SPRITES * 12];
        for i in 0..Self::MAX_LOCATION_SPRITES {
            let base = i * 12;
            init_buffer[base + 0] = 1.0;       // scale x
            init_buffer[base + 3] = -99999.0;  // pos x (hidden)
            init_buffer[base + 5] = 1.0;       // scale y
            init_buffer[base + 7] = -99999.0;  // pos y (hidden)
            init_buffer[base + 11] = 1.0;      // custom_data.a
        }
        let packed = PackedFloat32Array::from(init_buffer.as_slice());
        rs.multimesh_set_buffer(self.location_multimesh_rid, &packed);

        // Create canvas item for locations (behind NPCs)
        self.location_canvas_item = rs.canvas_item_create();
        let parent_canvas = self.base().get_canvas_item();
        rs.canvas_item_set_parent(self.location_canvas_item, parent_canvas);
        rs.canvas_item_set_z_index(self.location_canvas_item, -100);  // Behind NPCs

        // Load terrain_sprite shader and roguelikeSheet texture
        let mut loader = ResourceLoader::singleton();
        if let Some(shader) = loader.load("res://world/terrain_sprite.gdshader") {
            if let Some(texture) = loader.load("res://assets/roguelikeSheet_transparent.png") {
                let mut material = ShaderMaterial::new_gd();
                let shader_res: Gd<godot::classes::Shader> = shader.cast();
                material.set_shader(&shader_res);
                material.set_shader_parameter("spritesheet", &texture.to_variant());
                rs.canvas_item_set_material(self.location_canvas_item, material.get_rid());
                self.location_material = Some(material);
            }
        }

        rs.canvas_item_add_multimesh(self.location_canvas_item, self.location_multimesh_rid);
        // Disable visibility culling for world-spanning MultiMesh
        rs.canvas_item_set_custom_rect_ex(self.location_canvas_item, true)
            .rect(Rect2::new(Vector2::new(-100000.0, -100000.0), Vector2::new(200000.0, 200000.0)))
            .done();

        self.location_mesh = Some(mesh);
    }

    /// Build location MultiMesh buffer from WorldData.
    /// Call once after all add_farm/add_bed/add_guard_post/add_town calls complete.
    fn build_location_buffer(&mut self) {
        // Get sprites from WorldData
        let sprites: Vec<world::SpriteInstance> = if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(world_data) = app.world().get_resource::<world::WorldData>() {
                    world_data.get_all_sprites()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        if sprites.is_empty() {
            return;
        }

        let count = sprites.len().min(Self::MAX_LOCATION_SPRITES);
        self.location_sprite_count = count;

        // Build buffer: Transform2D (8) + CustomData (4) = 12 floats per instance
        let mut buffer = vec![0.0f32; Self::MAX_LOCATION_SPRITES * 12];

        for (i, sprite) in sprites.iter().take(count).enumerate() {
            let base = i * 12;

            // Transform2D
            buffer[base + 0] = sprite.scale;  // scale x
            buffer[base + 1] = 0.0;           // skew y
            buffer[base + 2] = 0.0;           // unused
            buffer[base + 3] = sprite.pos.x;  // pos x
            buffer[base + 4] = 0.0;           // skew x
            buffer[base + 5] = sprite.scale;  // scale y
            buffer[base + 6] = 0.0;           // unused
            buffer[base + 7] = sprite.pos.y;  // pos y

            // CustomData: UV for terrain_sprite shader
            let uv_x = (sprite.uv.0 as f32 * world::CELL) / world::SHEET_SIZE.0;
            let uv_y = (sprite.uv.1 as f32 * world::CELL) / world::SHEET_SIZE.1;
            let uv_width = world::SPRITE_SIZE / world::SHEET_SIZE.0;

            buffer[base + 8] = uv_x;      // r = u
            buffer[base + 9] = uv_y;      // g = v
            buffer[base + 10] = uv_width; // b = width
            buffer[base + 11] = 1.0;      // a = tile_count
        }

        // Hide remaining slots
        for i in count..Self::MAX_LOCATION_SPRITES {
            let base = i * 12;
            buffer[base + 0] = 1.0;
            buffer[base + 3] = -99999.0;
            buffer[base + 5] = 1.0;
            buffer[base + 7] = -99999.0;
            buffer[base + 11] = 1.0;
        }

        let mut rs = RenderingServer::singleton();
        let packed = PackedFloat32Array::from(buffer.as_slice());
        rs.multimesh_set_buffer(self.location_multimesh_rid, &packed);

        godot_print!("[EcsNpcManager] Built location MultiMesh: {} sprites", count);
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

        // Direct GPU update for immediate responsiveness (GDScript API calls are rare)
        if let Some(gpu) = self.gpu.as_mut() {
            let idx = npc_index as usize;
            if idx < MAX_NPC_COUNT {
                // Update CPU caches
                gpu.targets[idx * 2] = x;
                gpu.targets[idx * 2 + 1] = y;
                gpu.arrivals[idx] = 0;
                gpu.backoffs[idx] = 0;

                // Immediate GPU upload (not batched - for responsiveness)
                let target_bytes: Vec<u8> = [x, y].iter()
                    .flat_map(|f| f.to_le_bytes()).collect();
                let target_packed = PackedByteArray::from(target_bytes.as_slice());
                gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &target_packed);

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
    fn get_debug_stats(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let npc_count = GPU_READ_STATE.lock().map(|s| s.npc_count).unwrap_or(0);

        // Use cached stats from process() - no GPU reads!
        if let Ok(stats) = resources::PERF_STATS.lock() {
            dict.set("npc_count", npc_count as i32);
            dict.set("arrived_count", stats.arrived_count);
            dict.set("avg_backoff", stats.avg_backoff);
            dict.set("max_backoff", stats.max_backoff);
            dict.set("cells_used", 0);  // Grid now built on GPU, no CPU-side count
            dict.set("max_per_cell", 0);
        }
        dict
    }

    #[func]
    fn get_perf_stats(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Ok(mut stats) = resources::PERF_STATS.lock() {
            dict.set("queue_ms", stats.queue_ms);
            dict.set("dispatch_ms", stats.dispatch_ms);
            dict.set("readpos_ms", stats.readpos_ms);
            dict.set("combat_ms", stats.combat_ms);
            dict.set("build_ms", stats.build_ms);
            dict.set("upload_ms", stats.upload_ms);
            dict.set("bevy_ms", stats.bevy_ms);
            let gpu_total = stats.queue_ms + stats.dispatch_ms + stats.readpos_ms + stats.combat_ms + stats.build_ms + stats.upload_ms;
            dict.set("gpu_total_ms", gpu_total);
            let ecs_total = gpu_total + stats.bevy_ms;
            dict.set("ecs_total_ms", ecs_total);
            dict.set("frame_ms", stats.frame_ms);
            // Use PREVIOUS frame's ECS time: frame_ms spans last process() to this process(),
            // so it includes last frame's ECS + Godot time. Subtracting this frame's ECS is wrong.
            let godot_ms = (stats.frame_ms - stats.prev_ecs_total_ms).max(0.0);
            dict.set("godot_ms", godot_ms);
            // Save current ECS total for next frame's godot_ms calculation
            stats.prev_ecs_total_ms = ecs_total;
        }
        dict
    }

    #[func]
    fn get_npc_position(&self, npc_index: i32) -> Vector2 {
        if let Some(gpu) = &self.gpu {
            let idx = npc_index as usize;
            // Check against GPU array bounds (pre-allocated to MAX_NPC_COUNT)
            if idx * 2 + 1 < gpu.positions.len() {
                let x = gpu.positions[idx * 2];
                let y = gpu.positions[idx * 2 + 1];
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
            // Check against GPU array bounds (pre-allocated to MAX_NPC_COUNT)
            if idx < gpu.healths.len() {
                return gpu.healths[idx];
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

                    // Healing debug
                    dict.set("healing_npcs_checked", debug.healing_npcs_checked as i32);
                    dict.set("healing_positions_len", debug.healing_positions_len as i32);
                    dict.set("healing_towns_count", debug.healing_towns_count as i32);
                    dict.set("healing_in_zone_count", debug.healing_in_zone_count as i32);
                    dict.set("healing_healed_count", debug.healing_healed_count as i32);
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
                    beds.occupants.clear();
                }
                if let Some(mut farms) = app.world_mut().get_resource_mut::<world::FarmOccupancy>() {
                    farms.occupants.clear();
                }
                if let Some(mut farm_states) = app.world_mut().get_resource_mut::<resources::FarmStates>() {
                    farm_states.states.clear();
                    farm_states.progress.clear();
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
                    beds.occupants.clear();
                }
                if let Some(mut farms) = app.world_mut().get_resource_mut::<world::FarmOccupancy>() {
                    farms.occupants.clear();
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

    /// Unified location API. Adds any location type and updates sprite rendering.
    /// loc_type: "farm", "bed", "guard_post", "town_center"
    /// opts: { "patrol_order": i32 (guard_post), "name": String (fountain), "faction": i32 (fountain) }
    /// Returns: index of added location within its type, or -1 on error.
    #[func]
    fn add_location(&mut self, loc_type: GString, x: f32, y: f32, town_idx: i32, opts: VarDictionary) -> i32 {
        let type_str = loc_type.to_string();
        let pos = Vector2::new(x, y);
        let mut index: i32 = -1;

        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                match type_str.as_str() {
                    "farm" => {
                        if let Some(mut world) = app.world_mut().get_resource_mut::<world::WorldData>() {
                            index = world.farms.len() as i32;
                            world.farms.push(world::Farm {
                                position: pos,
                                town_idx: town_idx as u32,
                            });
                        }
                        // Initialize occupancy for this farm position
                        if let Some(mut farms) = app.world_mut().get_resource_mut::<world::FarmOccupancy>() {
                            let key = world::pos_to_key(pos);
                            farms.occupants.insert(key, 0);
                        }
                        // Initialize farm growth state (starts as Growing)
                        if let Some(mut farm_states) = app.world_mut().get_resource_mut::<resources::FarmStates>() {
                            farm_states.push_farm();
                        }
                    }
                    "bed" => {
                        if let Some(mut world) = app.world_mut().get_resource_mut::<world::WorldData>() {
                            index = world.beds.len() as i32;
                            world.beds.push(world::Bed {
                                position: pos,
                                town_idx: town_idx as u32,
                            });
                        }
                        // Initialize occupancy for this bed position
                        if let Some(mut beds) = app.world_mut().get_resource_mut::<world::BedOccupancy>() {
                            let key = world::pos_to_key(pos);
                            beds.occupants.insert(key, -1);
                        }
                    }
                    "guard_post" => {
                        let patrol_order = opts.get("patrol_order")
                            .map(|v| v.to::<i32>())
                            .unwrap_or(0);
                        if let Some(mut world) = app.world_mut().get_resource_mut::<world::WorldData>() {
                            index = world.guard_posts.len() as i32;
                            world.guard_posts.push(world::GuardPost {
                                position: pos,
                                town_idx: town_idx as u32,
                                patrol_order: patrol_order as u32,
                            });
                        }
                        // Rebuild patrol routes for all guards in this town (clockwise order)
                        let new_posts: Vec<Vector2> = {
                            let world = app.world().get_resource::<world::WorldData>();
                            world.map(|w| {
                                // Get town center for angle calculation
                                let center = w.towns.get(town_idx as usize)
                                    .map(|t| t.center)
                                    .unwrap_or(Vector2::ZERO);

                                let mut posts: Vec<_> = w.guard_posts.iter()
                                    .filter(|p| p.town_idx == town_idx as u32)
                                    .map(|p| {
                                        let dx = p.position.x - center.x;
                                        let dy = p.position.y - center.y;
                                        // atan2 gives angle, negate for clockwise
                                        let angle = -dy.atan2(dx);
                                        (angle, p.position)
                                    })
                                    .collect();
                                // Sort by angle for clockwise order
                                posts.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                                posts.into_iter().map(|(_, pos)| pos).collect()
                            }).unwrap_or_default()
                        };
                        let mut to_update = Vec::new();
                        {
                            let mut query = app.world_mut().query::<(Entity, &TownId, &PatrolRoute)>();
                            for (entity, tid, _) in query.iter(app.world()) {
                                if tid.0 == town_idx {
                                    to_update.push(entity);
                                }
                            }
                        }
                        for entity in to_update {
                            if let Some(mut patrol) = app.world_mut().get_mut::<PatrolRoute>(entity) {
                                patrol.posts = new_posts.clone();
                            }
                        }
                    }
                    "town_center" => {
                        let name = opts.get("name")
                            .map(|v| v.to::<GString>().to_string())
                            .unwrap_or_else(|| format!("Town {}", town_idx));
                        let faction = opts.get("faction")
                            .map(|v| v.to::<i32>())
                            .unwrap_or(0);
                        // Default sprite: fountain for villagers (faction 0), tent for raiders
                        let sprite_type = opts.get("sprite_type")
                            .map(|v| v.to::<i32>())
                            .unwrap_or(if faction == 0 { 0 } else { 1 });
                        if let Some(mut world) = app.world_mut().get_resource_mut::<world::WorldData>() {
                            index = world.towns.len() as i32;
                            world.towns.push(world::Town {
                                name,
                                center: pos,
                                faction,
                                sprite_type,
                            });
                        }
                    }
                    _ => {
                        godot_warn!("[EcsNpcManager] Unknown location type: {}", type_str);
                    }
                }
            }
        }

        index
    }

    /// Rebuild location MultiMesh. Call once after all add_location() calls complete.
    #[func]
    fn build_locations(&mut self) {
        self.build_location_buffer();
    }

    /// Remove a location by type and position. Returns true if found and removed.
    /// Evicts any NPCs assigned to the location.
    #[func]
    fn remove_location(&mut self, loc_type: GString, x: f64, y: f64) -> bool {
        let target_pos = Vector2::new(x as f32, y as f32);
        let target_key = world::pos_to_key(target_pos);
        let type_str = loc_type.to_string();
        let mut found = false;
        let mut town_idx_for_patrol: Option<u32> = None;

        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                match type_str.as_str() {
                    "farm" => {
                        // Find and remove farm
                        let farm_idx = {
                            let world = app.world().get_resource::<world::WorldData>();
                            world.and_then(|w| w.farms.iter().position(|f| world::pos_to_key(f.position) == target_key))
                        };
                        if let Some(idx) = farm_idx {
                            // Remove from WorldData
                            if let Some(mut world) = app.world_mut().get_resource_mut::<world::WorldData>() {
                                world.farms.remove(idx);
                            }
                            // Remove from occupancy
                            if let Some(mut farms) = app.world_mut().get_resource_mut::<world::FarmOccupancy>() {
                                farms.occupants.remove(&target_key);
                            }
                            // Remove from FarmStates (keep indices aligned)
                            if let Some(mut farm_states) = app.world_mut().get_resource_mut::<resources::FarmStates>() {
                                if idx < farm_states.states.len() {
                                    farm_states.states.remove(idx);
                                    farm_states.progress.remove(idx);
                                }
                            }
                            found = true;
                        }
                        // Evict NPCs working at this farm
                        if found {
                            let mut to_evict = Vec::new();
                            {
                                let mut query = app.world_mut().query::<(Entity, &AssignedFarm)>();
                                for (entity, assigned) in query.iter(app.world()) {
                                    if world::pos_to_key(assigned.0) == target_key {
                                        to_evict.push(entity);
                                    }
                                }
                            }
                            for entity in to_evict {
                                app.world_mut().entity_mut(entity).remove::<Working>();
                                app.world_mut().entity_mut(entity).remove::<AssignedFarm>();
                            }
                        }
                    }
                    "bed" => {
                        // Find and remove bed
                        let bed_idx = {
                            let world = app.world().get_resource::<world::WorldData>();
                            world.and_then(|w| w.beds.iter().position(|b| world::pos_to_key(b.position) == target_key))
                        };
                        if let Some(idx) = bed_idx {
                            if let Some(mut world) = app.world_mut().get_resource_mut::<world::WorldData>() {
                                world.beds.remove(idx);
                            }
                            if let Some(mut beds) = app.world_mut().get_resource_mut::<world::BedOccupancy>() {
                                beds.occupants.remove(&target_key);
                            }
                            found = true;
                        }
                        // Evict NPCs using this bed
                        if found {
                            let mut to_evict = Vec::new();
                            {
                                let mut query = app.world_mut().query::<(Entity, &Home)>();
                                for (entity, home) in query.iter(app.world()) {
                                    if world::pos_to_key(home.0) == target_key {
                                        to_evict.push(entity);
                                    }
                                }
                            }
                            for entity in to_evict {
                                app.world_mut().entity_mut(entity).insert(Home(Vector2::new(-1.0, -1.0)));
                                app.world_mut().entity_mut(entity).remove::<Resting>();
                            }
                        }
                    }
                    "guard_post" => {
                        // Find and remove guard post
                        let post_info = {
                            let world = app.world().get_resource::<world::WorldData>();
                            world.and_then(|w| {
                                w.guard_posts.iter()
                                    .position(|p| world::pos_to_key(p.position) == target_key)
                                    .map(|idx| (idx, w.guard_posts[idx].town_idx))
                            })
                        };
                        if let Some((idx, town_idx)) = post_info {
                            if let Some(mut world) = app.world_mut().get_resource_mut::<world::WorldData>() {
                                world.guard_posts.remove(idx);
                            }
                            town_idx_for_patrol = Some(town_idx);
                            found = true;
                        }
                    }
                    _ => {}
                }

                // Rebuild patrol routes if guard post was removed (clockwise order)
                if let Some(town_idx) = town_idx_for_patrol {
                    // Get remaining posts for this town sorted clockwise
                    let new_posts: Vec<Vector2> = {
                        let world = app.world().get_resource::<world::WorldData>();
                        world.map(|w| {
                            // Get town center for angle calculation
                            let center = w.towns.get(town_idx as usize)
                                .map(|t| t.center)
                                .unwrap_or(Vector2::ZERO);

                            let mut posts: Vec<_> = w.guard_posts.iter()
                                .filter(|p| p.town_idx == town_idx)
                                .map(|p| {
                                    let dx = p.position.x - center.x;
                                    let dy = p.position.y - center.y;
                                    let angle = -dy.atan2(dx);
                                    (angle, p.position)
                                })
                                .collect();
                            posts.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                            posts.into_iter().map(|(_, pos)| pos).collect()
                        }).unwrap_or_default()
                    };

                    // Update all guards in this town with new patrol routes
                    let mut to_update = Vec::new();
                    {
                        let mut query = app.world_mut().query::<(Entity, &TownId, &PatrolRoute)>();
                        for (entity, tid, _) in query.iter(app.world()) {
                            if tid.0 == town_idx as i32 {
                                to_update.push(entity);
                            }
                        }
                    }
                    for entity in to_update {
                        if let Some(mut patrol) = app.world_mut().get_mut::<PatrolRoute>(entity) {
                            patrol.posts = new_posts.clone();
                            if patrol.current >= patrol.posts.len() && !patrol.posts.is_empty() {
                                patrol.current = 0;
                            }
                        }
                    }
                }
            }
        }
        found
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
                        // Check occupancy by position
                        let key = world::pos_to_key(bed.position);
                        let occupant = beds.occupants.get(&key).copied().unwrap_or(-1);
                        if occupant >= 0 { continue; }
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
                        // Check occupancy by position
                        let key = world::pos_to_key(farm.position);
                        let count = farms.occupants.get(&key).copied().unwrap_or(0);
                        if count >= 1 { continue; }
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
                // Get bed position from WorldData
                let bed_pos = {
                    let world = app.world().get_resource::<world::WorldData>();
                    world.and_then(|w| w.beds.get(bed_idx as usize).map(|b| b.position))
                };
                if let Some(pos) = bed_pos {
                    let key = world::pos_to_key(pos);
                    if let Some(mut beds) = app.world_mut().get_resource_mut::<world::BedOccupancy>() {
                        let occupant = beds.occupants.get(&key).copied().unwrap_or(-1);
                        if occupant < 0 {
                            beds.occupants.insert(key, npc_idx);
                            return true;
                        }
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
                // Get bed position from WorldData
                let bed_pos = {
                    let world = app.world().get_resource::<world::WorldData>();
                    world.and_then(|w| w.beds.get(bed_idx as usize).map(|b| b.position))
                };
                if let Some(pos) = bed_pos {
                    let key = world::pos_to_key(pos);
                    if let Some(mut beds) = app.world_mut().get_resource_mut::<world::BedOccupancy>() {
                        beds.occupants.insert(key, -1);
                    }
                }
            }
        }
    }

    #[func]
    fn reserve_farm(&mut self, farm_idx: i32) -> bool {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                // Get farm position from WorldData
                let farm_pos = {
                    let world = app.world().get_resource::<world::WorldData>();
                    world.and_then(|w| w.farms.get(farm_idx as usize).map(|f| f.position))
                };
                if let Some(pos) = farm_pos {
                    let key = world::pos_to_key(pos);
                    if let Some(mut farms) = app.world_mut().get_resource_mut::<world::FarmOccupancy>() {
                        let count = farms.occupants.get(&key).copied().unwrap_or(0);
                        if count < 1 {
                            *farms.occupants.entry(key).or_insert(0) += 1;
                            return true;
                        }
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
                // Get farm position from WorldData
                let farm_pos = {
                    let world = app.world().get_resource::<world::WorldData>();
                    world.and_then(|w| w.farms.get(farm_idx as usize).map(|f| f.position))
                };
                if let Some(pos) = farm_pos {
                    let key = world::pos_to_key(pos);
                    if let Some(mut farms) = app.world_mut().get_resource_mut::<world::FarmOccupancy>() {
                        if let Some(count) = farms.occupants.get_mut(&key) {
                            if *count > 0 {
                                *count -= 1;
                            }
                        }
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
    fn init_food_storage(&mut self, total_town_count: i32) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut food) = app.world_mut().get_resource_mut::<resources::FoodStorage>() {
                    food.init(total_town_count as usize);
                }
            }
        }
    }

    /// Add food to a town (called from GDScript when needed).
    #[func]
    fn add_town_food(&mut self, town_idx: i32, amount: i32) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut food) = app.world_mut().get_resource_mut::<resources::FoodStorage>() {
                    let idx = town_idx as usize;
                    if idx < food.food.len() {
                        food.food[idx] += amount;
                    }
                }
            }
        }
    }

    /// Get food count for a town (works for both villager and raider towns).
    #[func]
    fn get_town_food(&self, town_idx: i32) -> i32 {
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(food) = app.world().get_resource::<resources::FoodStorage>() {
                    let idx = town_idx as usize;
                    if idx < food.food.len() {
                        return food.food[idx];
                    }
                }
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

    // ========================================================================
    // FACTION STATS API
    // ========================================================================

    /// Initialize faction stats for all factions (villager + raiders).
    /// Call after init_food_storage with total_faction_count = num_towns + num_camps.
    #[func]
    fn init_faction_stats(&mut self, total_faction_count: i32) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut stats) = app.world_mut().get_resource_mut::<resources::FactionStats>() {
                    stats.init(total_faction_count as usize);
                }
            }
        }
    }

    /// Get stats for a specific faction. Returns dict with alive, dead, kills.
    #[func]
    fn get_faction_stats(&self, faction_id: i32) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("alive", 0i32);
        dict.set("dead", 0i32);
        dict.set("kills", 0i32);

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(stats) = app.world().get_resource::<resources::FactionStats>() {
                    if let Some(s) = stats.stats.get(faction_id as usize) {
                        dict.set("alive", s.alive);
                        dict.set("dead", s.dead);
                        dict.set("kills", s.kills);
                    }
                }
            }
        }
        dict
    }

    /// Get all faction stats as array of dicts. Index = faction_id.
    #[func]
    fn get_all_faction_stats(&self) -> VarArray {
        let mut arr = VarArray::new();

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(stats) = app.world().get_resource::<resources::FactionStats>() {
                    for s in &stats.stats {
                        let mut dict = VarDictionary::new();
                        dict.set("alive", s.alive);
                        dict.set("dead", s.dead);
                        dict.set("kills", s.kills);
                        arr.push(&dict.to_variant());
                    }
                }
            }
        }
        arr
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
                    let free_beds = beds.occupants.values().filter(|&&x| x < 0).count();
                    dict.set("free_beds", free_beds as i32);
                }
                if let Some(farms) = app.world().get_resource::<world::FarmOccupancy>() {
                    let free_farms = farms.occupants.values().filter(|&&x| x < 1).count();
                    dict.set("free_farms", free_farms as i32);
                }
            }
        }
        dict
    }

    // ========================================================================
    // TIME API
    // ========================================================================

    /// Get the BevyApp autoload node (does scene tree traversal).
    fn get_bevy_app(&self) -> Option<Gd<godot_bevy::app::BevyApp>> {
        let tree = self.base().get_tree()?;
        let root = tree.get_root()?;
        // Window inherits from Node, use upcast to access try_get_node_as
        let root_node: Gd<godot::classes::Node> = root.upcast();
        root_node.try_get_node_as::<godot_bevy::app::BevyApp>("BevyAppSingleton")
    }

    /// Get cached BevyApp reference (no tree traversal, use in hot paths like process()).
    fn get_bevy_app_cached(&self) -> Option<Gd<godot_bevy::app::BevyApp>> {
        self.bevy_app_cache.clone()
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

        // Count alive NPCs from NpcsByTownCache + GPU health
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let (Some(by_town), Some(meta)) = (
                    app.world().get_resource::<resources::NpcsByTownCache>(),
                    app.world().get_resource::<resources::NpcMetaCache>(),
                ) {
                    if let Ok(state) = GPU_READ_STATE.lock() {
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
                // Get dead counts from PopulationStats (tracks by job)
                if let Some(pop) = app.world().get_resource::<resources::PopulationStats>() {
                    let mut farmers_dead = 0i32;
                    let mut guards_dead = 0i32;
                    let mut raiders_dead = 0i32;
                    for ((job, _clan), stats) in pop.0.iter() {
                        match job {
                            0 => farmers_dead += stats.dead,
                            1 => guards_dead += stats.dead,
                            2 => raiders_dead += stats.dead,
                            _ => {}
                        }
                    }
                    dict.set("farmers_dead", farmers_dead);
                    dict.set("guards_dead", guards_dead);
                    dict.set("raiders_dead", raiders_dead);
                }
            }
        }

        dict.set("farmers_alive", farmers_alive);
        dict.set("guards_alive", guards_alive);
        dict.set("raiders_alive", raiders_alive);
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

        // GPU data: positions, targets, factions (these come from GPU compute)
        if let Ok(state) = GPU_READ_STATE.lock() {
            if i < state.npc_count {
                dict.set("x", state.positions.get(i * 2).copied().unwrap_or(0.0));
                dict.set("y", state.positions.get(i * 2 + 1).copied().unwrap_or(0.0));
                dict.set("faction", state.factions.get(i).copied().unwrap_or(0));
                dict.set("target_idx", state.combat_targets.get(i).copied().unwrap_or(-1));
            }
        }

        // Bevy data: components and resources
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                // Meta cache (name, level, trait, etc.)
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
                // Entity components (HP, Energy, State)
                if let Some(npc_map) = app.world().get_resource::<NpcEntityMap>() {
                    if let Some(&entity) = npc_map.0.get(&i) {
                        dict.set("state", GString::from(derive_npc_state(app.world(), entity)));
                        // Read HP directly from component
                        if let Some(health) = app.world().get::<components::Health>(entity) {
                            dict.set("hp", health.0);
                        }
                        // Read Energy directly from component
                        if let Some(energy) = app.world().get::<components::Energy>(entity) {
                            dict.set("energy", energy.0);
                        }
                    }
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

        let Some(bevy_app) = self.get_bevy_app() else { return result; };
        let app_ref = bevy_app.bind();
        let Some(app) = app_ref.get_app() else { return result; };
        let Some(logs) = app.world().get_resource::<resources::NpcLogCache>() else { return result; };

        if let Some(log) = logs.0.get(i) {
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

    /// Get selected NPC data: { idx, position, target } in one FFI call.
    #[func]
    fn get_selected_npc(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let mut idx = -1i32;
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(selected) = app.world().get_resource::<resources::SelectedNpc>() {
                    idx = selected.0;
                }
            }
        }
        dict.set("idx", idx);
        if idx >= 0 {
            dict.set("position", self.get_npc_position(idx));
            dict.set("target", self.get_npc_target(idx));
        }
        dict
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

        // Use Bevy SlotAllocator count (high-water mark) for click detection
        let slot_count = if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                app.world().get_resource::<resources::SlotAllocator>()
                    .map(|s| s.count())
                    .unwrap_or(0)
            } else { 0 }
        } else { 0 };

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

    /// Find nearest location at a position within radius (for click selection).
    /// Returns: { type: "farm"|"bed"|"guard_post"|"fountain"|"", index: i32, x: f32, y: f32, town_idx: i32 }
    #[func]
    fn get_location_at_position(&self, x: f32, y: f32, radius: f32) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("type", "");
        dict.set("index", -1);
        dict.set("x", 0.0f32);
        dict.set("y", 0.0f32);
        dict.set("town_idx", -1);

        let mut best_dist = radius;

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(world) = app.world().get_resource::<world::WorldData>() {
                    // Check farms
                    for (i, farm) in world.farms.iter().enumerate() {
                        let dx = farm.position.x - x;
                        let dy = farm.position.y - y;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist < best_dist {
                            best_dist = dist;
                            dict.set("type", "farm");
                            dict.set("index", i as i32);
                            dict.set("x", farm.position.x);
                            dict.set("y", farm.position.y);
                            dict.set("town_idx", farm.town_idx as i32);
                        }
                    }
                    // Check beds
                    for (i, bed) in world.beds.iter().enumerate() {
                        let dx = bed.position.x - x;
                        let dy = bed.position.y - y;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist < best_dist {
                            best_dist = dist;
                            dict.set("type", "bed");
                            dict.set("index", i as i32);
                            dict.set("x", bed.position.x);
                            dict.set("y", bed.position.y);
                            dict.set("town_idx", bed.town_idx as i32);
                        }
                    }
                    // Check guard posts
                    for (i, post) in world.guard_posts.iter().enumerate() {
                        let dx = post.position.x - x;
                        let dy = post.position.y - y;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist < best_dist {
                            best_dist = dist;
                            dict.set("type", "guard_post");
                            dict.set("index", i as i32);
                            dict.set("x", post.position.x);
                            dict.set("y", post.position.y);
                            dict.set("town_idx", post.town_idx as i32);
                        }
                    }
                    // Check town centers (fountains)
                    for (i, town) in world.towns.iter().enumerate() {
                        let dx = town.center.x - x;
                        let dy = town.center.y - y;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist < best_dist {
                            best_dist = dist;
                            dict.set("type", "fountain");
                            dict.set("index", i as i32);
                            dict.set("x", town.center.x);
                            dict.set("y", town.center.y);
                            dict.set("town_idx", i as i32);
                        }
                    }
                }
            }
        }

        dict
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
                    for bed in world.beds.iter() {
                        if bed.town_idx == town_idx as u32 {
                            total += 1;
                            let key = world::pos_to_key(bed.position);
                            let occupant = beds.occupants.get(&key).copied().unwrap_or(-1);
                            if occupant < 0 {
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
