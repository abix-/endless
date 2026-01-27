//! Endless ECS - GDExtension bridge between Godot, Bevy ECS, and GPU compute.
//! See docs/ for architecture documentation.

// ============================================================================
// MODULES
// ============================================================================

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
use godot::classes::{RenderingServer, QuadMesh, INode2D};

use components::*;
use constants::*;
use gpu::GpuCompute;
use messages::*;
use resources::*;
use systems::*;
use world::*;

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
       .add_message::<SpawnGuardMsg>()
       .add_message::<SpawnFarmerMsg>()
       .add_message::<SpawnRaiderMsg>()
       .add_message::<ArrivalMsg>()
       .add_message::<DamageMsg>()
       .init_resource::<NpcCount>()
       .init_resource::<GpuData>()
       .init_resource::<NpcEntityMap>()
       .init_resource::<world::WorldData>()
       .init_resource::<world::BedOccupancy>()
       .init_resource::<world::FarmOccupancy>()
       // Chain phases with explicit command flush between Spawn and Combat
       .configure_sets(Update, (Step::Drain, Step::Spawn, Step::Combat, Step::Behavior).chain())
       // Flush commands after Spawn so Combat sees new entities
       .add_systems(Update, bevy::ecs::schedule::ApplyDeferred.after(Step::Spawn).before(Step::Combat))
       // Drain: reset + drain queues
       .add_systems(Update, (
           reset_bevy_system,
           drain_spawn_queue,
           drain_target_queue,
           drain_guard_queue,
           drain_farmer_queue,
           drain_raider_queue,
           drain_arrival_queue,
           drain_damage_queue,
       ).in_set(Step::Drain))
       // Spawn: create entities
       .add_systems(Update, (
           spawn_npc_system,
           spawn_guard_system,
           spawn_farmer_system,
           spawn_raider_system,
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
       // Behavior: energy, patrol, rest, work
       .add_systems(Update, (
           handle_arrival_system,
           energy_system,
           tired_system,
           resume_patrol_system,
           resume_work_system,
           patrol_system,
       ).in_set(Step::Behavior));
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
            prev_arrivals: vec![false; MAX_NPC_COUNT],
            proj_multimesh_rid: Rid::Invalid,
            proj_canvas_item: Rid::Invalid,
            proj_mesh: None,
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
    }

    fn process(&mut self, delta: f64) {
        // Update FRAME_DELTA for Bevy combat systems
        if let Ok(mut d) = FRAME_DELTA.lock() {
            *d = delta as f32;
        }

        let gpu = match self.gpu.as_mut() {
            Some(g) => g,
            None => return,
        };

        // GPU-FIRST: Get npc_count from GPU_READ_STATE
        let npc_count = GPU_READ_STATE.lock().map(|s| s.npc_count).unwrap_or(0);

        // GPU-FIRST: Single queue drain for all GPU updates
        if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
            for update in queue.drain(..) {
                match update {
                    GpuUpdate::SetTarget { idx, x, y } => {
                        if idx < npc_count {
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
                        if idx < npc_count {
                            let new_health = (gpu.healths[idx] - amount).max(0.0);
                            gpu.healths[idx] = new_health;
                            let health_bytes: Vec<u8> = new_health.to_le_bytes().to_vec();
                            let health_packed = PackedByteArray::from(health_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.health_buffer, (idx * 4) as u32, 4, &health_packed);
                        }
                    }
                    GpuUpdate::HideNpc { idx } => {
                        if idx < npc_count {
                            let hide_pos: Vec<u8> = [-9999.0f32, -9999.0f32].iter()
                                .flat_map(|f| f.to_le_bytes()).collect();
                            let hide_packed = PackedByteArray::from(hide_pos.as_slice());
                            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &hide_packed);
                            gpu.positions[idx * 2] = -9999.0;
                            gpu.positions[idx * 2 + 1] = -9999.0;
                        }
                    }
                    GpuUpdate::SetFaction { idx, faction } => {
                        if idx < npc_count {
                            gpu.factions[idx] = faction;
                            let faction_bytes: Vec<u8> = faction.to_le_bytes().to_vec();
                            let faction_packed = PackedByteArray::from(faction_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.faction_buffer, (idx * 4) as u32, 4, &faction_packed);
                        }
                    }
                    GpuUpdate::SetHealth { idx, health } => {
                        if idx < npc_count {
                            gpu.healths[idx] = health;
                            let health_bytes: Vec<u8> = health.to_le_bytes().to_vec();
                            let health_packed = PackedByteArray::from(health_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.health_buffer, (idx * 4) as u32, 4, &health_packed);
                        }
                    }
                    GpuUpdate::SetPosition { idx, x, y } => {
                        if idx < npc_count {
                            gpu.positions[idx * 2] = x;
                            gpu.positions[idx * 2 + 1] = y;
                            let pos_bytes: Vec<u8> = [x, y].iter()
                                .flat_map(|f| f.to_le_bytes()).collect();
                            let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &pos_packed);
                        }
                    }
                    GpuUpdate::SetSpeed { idx, speed } => {
                        if idx < npc_count {
                            let speed_bytes: Vec<u8> = speed.to_le_bytes().to_vec();
                            let speed_packed = PackedByteArray::from(speed_bytes.as_slice());
                            gpu.rd.buffer_update(gpu.speed_buffer, (idx * 4) as u32, 4, &speed_packed);
                        }
                    }
                    GpuUpdate::SetColor { idx, r, g, b, a } => {
                        if idx < npc_count {
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

            // === PROJECTILE PROCESSING ===
            let proj_count = gpu.proj_count;
            if proj_count > 0 {
                // Dispatch projectile compute shader
                gpu.dispatch_projectiles(proj_count, npc_count, delta as f32);

                // Read hit results and route to damage queue
                let hits = gpu.read_projectile_hits();
                for (proj_idx, npc_idx, damage) in hits {
                    // Queue damage for Bevy to process
                    if let Ok(mut queue) = DAMAGE_QUEUE.lock() {
                        queue.push(DamageMsg {
                            npc_index: npc_idx,
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

        rs.multimesh_allocate_data_ex(
            self.multimesh_rid,
            max_count,
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
        ).color_format(true).done();

        let count = max_count as usize;
        let mut init_buffer = vec![0.0f32; count * FLOATS_PER_INSTANCE];
        for i in 0..count {
            let base = i * FLOATS_PER_INSTANCE;
            init_buffer[base + 0] = 1.0;
            init_buffer[base + 5] = 1.0;
            init_buffer[base + 11] = 1.0;
        }
        let packed = PackedFloat32Array::from(init_buffer.as_slice());
        rs.multimesh_set_buffer(self.multimesh_rid, &packed);

        self.canvas_item = rs.canvas_item_create();
        let parent_canvas = self.base().get_canvas_item();
        rs.canvas_item_set_parent(self.canvas_item, parent_canvas);
        rs.canvas_item_add_multimesh(self.canvas_item, self.multimesh_rid);

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

    /// Allocate an NPC slot: reuse a free slot or allocate new.
    /// Returns None if at capacity.
    fn allocate_slot() -> Option<usize> {
        // Try to reuse a free slot first
        if let Ok(mut free) = FREE_SLOTS.lock() {
            if let Some(recycled) = free.pop() {
                return Some(recycled);
            }
        }
        // No free slots, allocate new
        if let Ok(mut state) = GPU_READ_STATE.lock() {
            if state.npc_count < MAX_NPC_COUNT {
                let idx = state.npc_count;
                state.npc_count += 1;
                return Some(idx);
            }
        }
        None
    }

    #[func]
    fn spawn_npc(&mut self, x: f32, y: f32, job: i32) {
        let idx = match Self::allocate_slot() {
            Some(i) => i,
            None => return,
        };

        if let Ok(mut queue) = SPAWN_QUEUE.lock() {
            queue.push(SpawnNpcMsg { x, y, job });
        }

        if let Some(gpu) = self.gpu.as_mut() {
            let (r, g, b, a) = Job::from_i32(job).color();

            let pos_bytes: Vec<u8> = [x, y].iter().flat_map(|f| f.to_le_bytes()).collect();
            let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &pos_packed);
            gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &pos_packed);

            // CRITICAL: Update CPU cache so grid is built correctly
            gpu.positions[idx * 2] = x;
            gpu.positions[idx * 2 + 1] = y;

            let color_bytes: Vec<u8> = [r, g, b, a].iter().flat_map(|f| f.to_le_bytes()).collect();
            let color_packed = PackedByteArray::from(color_bytes.as_slice());
            gpu.rd.buffer_update(gpu.color_buffer, (idx * 16) as u32, 16, &color_packed);

            gpu.colors[idx * 4] = r;
            gpu.colors[idx * 4 + 1] = g;
            gpu.colors[idx * 4 + 2] = b;
            gpu.colors[idx * 4 + 3] = a;

            let speed_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let speed_packed = PackedByteArray::from(speed_bytes.as_slice());
            gpu.rd.buffer_update(gpu.speed_buffer, (idx * 4) as u32, 4, &speed_packed);

            let zero_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let zero_packed = PackedByteArray::from(zero_bytes.as_slice());
            gpu.rd.buffer_update(gpu.arrival_buffer, (idx * 4) as u32, 4, &zero_packed);
            gpu.rd.buffer_update(gpu.backoff_buffer, (idx * 4) as u32, 4, &zero_packed);
        }
    }

    #[func]
    fn spawn_guard(&mut self, x: f32, y: f32, town_idx: i32, home_x: f32, home_y: f32) {
        let idx = match Self::allocate_slot() {
            Some(i) => i,
            None => return,
        };

        if let Ok(mut queue) = GUARD_QUEUE.lock() {
            queue.push(SpawnGuardMsg {
                x, y,
                town_idx: town_idx as u32,
                home_x, home_y,
                starting_post: 0,
            });
        }

        if let Some(gpu) = self.gpu.as_mut() {
            let (r, g, b, a) = Job::Guard.color();

            let pos_bytes: Vec<u8> = [x, y].iter().flat_map(|f| f.to_le_bytes()).collect();
            let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &pos_packed);
            gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &pos_packed);

            // CRITICAL: Update CPU cache so grid is built correctly
            gpu.positions[idx * 2] = x;
            gpu.positions[idx * 2 + 1] = y;

            let color_bytes: Vec<u8> = [r, g, b, a].iter().flat_map(|f| f.to_le_bytes()).collect();
            let color_packed = PackedByteArray::from(color_bytes.as_slice());
            gpu.rd.buffer_update(gpu.color_buffer, (idx * 16) as u32, 16, &color_packed);

            gpu.colors[idx * 4] = r;
            gpu.colors[idx * 4 + 1] = g;
            gpu.colors[idx * 4 + 2] = b;
            gpu.colors[idx * 4 + 3] = a;

            let speed_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let speed_packed = PackedByteArray::from(speed_bytes.as_slice());
            gpu.rd.buffer_update(gpu.speed_buffer, (idx * 4) as u32, 4, &speed_packed);

            let zero_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let zero_packed = PackedByteArray::from(zero_bytes.as_slice());
            gpu.rd.buffer_update(gpu.arrival_buffer, (idx * 4) as u32, 4, &zero_packed);
            gpu.rd.buffer_update(gpu.backoff_buffer, (idx * 4) as u32, 4, &zero_packed);

            // Upload faction (villager = 0)
            gpu.rd.buffer_update(gpu.faction_buffer, (idx * 4) as u32, 4, &zero_packed);
            gpu.factions[idx] = 0;

            // Upload health
            let health_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let health_packed = PackedByteArray::from(health_bytes.as_slice());
            gpu.rd.buffer_update(gpu.health_buffer, (idx * 4) as u32, 4, &health_packed);
            gpu.healths[idx] = 100.0;
        }

        if let Ok(world) = WORLD_DATA.lock() {
            if let Some(post) = world.guard_posts.iter()
                .find(|p| p.town_idx == town_idx as u32 && p.patrol_order == 0)
            {
                self.set_target(idx as i32, post.position.x, post.position.y);
            }
        }
    }

    #[func]
    fn spawn_guard_at_post(&mut self, x: f32, y: f32, town_idx: i32, home_x: f32, home_y: f32, starting_post: i32) {
        let idx = match Self::allocate_slot() {
            Some(i) => i,
            None => return,
        };

        if let Ok(mut queue) = GUARD_QUEUE.lock() {
            queue.push(SpawnGuardMsg {
                x, y,
                town_idx: town_idx as u32,
                home_x, home_y,
                starting_post: starting_post as u32,
            });
        }

        if let Some(gpu) = self.gpu.as_mut() {
            let (r, g, b, a) = Job::Guard.color();

            let pos_bytes: Vec<u8> = [x, y].iter().flat_map(|f| f.to_le_bytes()).collect();
            let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &pos_packed);
            gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &pos_packed);

            // CRITICAL: Update CPU cache so grid is built correctly
            gpu.positions[idx * 2] = x;
            gpu.positions[idx * 2 + 1] = y;

            let color_bytes: Vec<u8> = [r, g, b, a].iter().flat_map(|f| f.to_le_bytes()).collect();
            let color_packed = PackedByteArray::from(color_bytes.as_slice());
            gpu.rd.buffer_update(gpu.color_buffer, (idx * 16) as u32, 16, &color_packed);

            gpu.colors[idx * 4] = r;
            gpu.colors[idx * 4 + 1] = g;
            gpu.colors[idx * 4 + 2] = b;
            gpu.colors[idx * 4 + 3] = a;

            let speed_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let speed_packed = PackedByteArray::from(speed_bytes.as_slice());
            gpu.rd.buffer_update(gpu.speed_buffer, (idx * 4) as u32, 4, &speed_packed);

            let one_bytes: Vec<u8> = 1i32.to_le_bytes().to_vec();
            let one_packed = PackedByteArray::from(one_bytes.as_slice());
            gpu.rd.buffer_update(gpu.arrival_buffer, (idx * 4) as u32, 4, &one_packed);

            let zero_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let zero_packed = PackedByteArray::from(zero_bytes.as_slice());
            gpu.rd.buffer_update(gpu.backoff_buffer, (idx * 4) as u32, 4, &zero_packed);

            // Upload faction (villager = 0)
            gpu.rd.buffer_update(gpu.faction_buffer, (idx * 4) as u32, 4, &zero_packed);
            gpu.factions[idx] = 0;

            // Upload health
            let health_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let health_packed = PackedByteArray::from(health_bytes.as_slice());
            gpu.rd.buffer_update(gpu.health_buffer, (idx * 4) as u32, 4, &health_packed);
            gpu.healths[idx] = 100.0;
        }

        self.prev_arrivals[idx] = true;
    }

    #[func]
    fn spawn_farmer(&mut self, x: f32, y: f32, town_idx: i32, home_x: f32, home_y: f32, work_x: f32, work_y: f32) {
        let idx = match Self::allocate_slot() {
            Some(i) => i,
            None => return,
        };

        if let Ok(mut queue) = FARMER_QUEUE.lock() {
            queue.push(SpawnFarmerMsg {
                x, y,
                town_idx: town_idx as u32,
                home_x, home_y,
                work_x, work_y,
            });
        }

        if let Some(gpu) = self.gpu.as_mut() {
            let (r, g, b, a) = Job::Farmer.color();

            let pos_bytes: Vec<u8> = [x, y].iter().flat_map(|f| f.to_le_bytes()).collect();
            let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &pos_packed);

            // CRITICAL: Update CPU cache so grid is built correctly
            gpu.positions[idx * 2] = x;
            gpu.positions[idx * 2 + 1] = y;

            let work_bytes: Vec<u8> = [work_x, work_y].iter().flat_map(|f| f.to_le_bytes()).collect();
            let work_packed = PackedByteArray::from(work_bytes.as_slice());
            gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &work_packed);

            let color_bytes: Vec<u8> = [r, g, b, a].iter().flat_map(|f| f.to_le_bytes()).collect();
            let color_packed = PackedByteArray::from(color_bytes.as_slice());
            gpu.rd.buffer_update(gpu.color_buffer, (idx * 16) as u32, 16, &color_packed);

            gpu.colors[idx * 4] = r;
            gpu.colors[idx * 4 + 1] = g;
            gpu.colors[idx * 4 + 2] = b;
            gpu.colors[idx * 4 + 3] = a;

            let speed_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let speed_packed = PackedByteArray::from(speed_bytes.as_slice());
            gpu.rd.buffer_update(gpu.speed_buffer, (idx * 4) as u32, 4, &speed_packed);

            let zero_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let zero_packed = PackedByteArray::from(zero_bytes.as_slice());
            gpu.rd.buffer_update(gpu.arrival_buffer, (idx * 4) as u32, 4, &zero_packed);
            gpu.rd.buffer_update(gpu.backoff_buffer, (idx * 4) as u32, 4, &zero_packed);

            // Upload faction (villager)
            let faction_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let faction_packed = PackedByteArray::from(faction_bytes.as_slice());
            gpu.rd.buffer_update(gpu.faction_buffer, (idx * 4) as u32, 4, &faction_packed);
            gpu.factions[idx] = 0;

            // Upload health
            let health_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let health_packed = PackedByteArray::from(health_bytes.as_slice());
            gpu.rd.buffer_update(gpu.health_buffer, (idx * 4) as u32, 4, &health_packed);
            gpu.healths[idx] = 100.0;
        }
    }

    #[func]
    fn spawn_raider(&mut self, x: f32, y: f32, camp_x: f32, camp_y: f32) {
        let idx = match Self::allocate_slot() {
            Some(i) => i,
            None => return,
        };

        if let Ok(mut queue) = RAIDER_QUEUE.lock() {
            queue.push(SpawnRaiderMsg { x, y, camp_x, camp_y });
        }

        if let Some(gpu) = self.gpu.as_mut() {
            let (r, g, b, a) = Job::Raider.color();

            let pos_bytes: Vec<u8> = [x, y].iter().flat_map(|f| f.to_le_bytes()).collect();
            let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &pos_packed);
            gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &pos_packed);

            // CRITICAL: Update CPU cache so grid is built correctly
            gpu.positions[idx * 2] = x;
            gpu.positions[idx * 2 + 1] = y;

            let color_bytes: Vec<u8> = [r, g, b, a].iter().flat_map(|f| f.to_le_bytes()).collect();
            let color_packed = PackedByteArray::from(color_bytes.as_slice());
            gpu.rd.buffer_update(gpu.color_buffer, (idx * 16) as u32, 16, &color_packed);

            gpu.colors[idx * 4] = r;
            gpu.colors[idx * 4 + 1] = g;
            gpu.colors[idx * 4 + 2] = b;
            gpu.colors[idx * 4 + 3] = a;

            let speed_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let speed_packed = PackedByteArray::from(speed_bytes.as_slice());
            gpu.rd.buffer_update(gpu.speed_buffer, (idx * 4) as u32, 4, &speed_packed);

            let zero_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let zero_packed = PackedByteArray::from(zero_bytes.as_slice());
            gpu.rd.buffer_update(gpu.arrival_buffer, (idx * 4) as u32, 4, &zero_packed);
            gpu.rd.buffer_update(gpu.backoff_buffer, (idx * 4) as u32, 4, &zero_packed);

            // Upload faction (raider = 1)
            let faction_bytes: Vec<u8> = 1i32.to_le_bytes().to_vec();
            let faction_packed = PackedByteArray::from(faction_bytes.as_slice());
            gpu.rd.buffer_update(gpu.faction_buffer, (idx * 4) as u32, 4, &faction_packed);
            gpu.factions[idx] = 1;

            // Upload health
            let health_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let health_packed = PackedByteArray::from(health_bytes.as_slice());
            gpu.rd.buffer_update(gpu.health_buffer, (idx * 4) as u32, 4, &health_packed);
            gpu.healths[idx] = 100.0;
        }
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
        None  // Will be set by fire_projectile based on gpu.proj_count
    }

    /// Fire a projectile from one position toward another.
    /// Returns the projectile index, or -1 if at capacity.
    #[func]
    fn fire_projectile(
        &mut self,
        from_x: f32, from_y: f32,
        to_x: f32, to_y: f32,
        damage: f32,
        faction: i32,
        shooter: i32,
    ) -> i32 {
        let gpu = match self.gpu.as_mut() {
            Some(g) => g,
            None => return -1,
        };

        // Allocate slot
        let idx = if let Some(recycled) = Self::allocate_proj_slot() {
            recycled
        } else if gpu.proj_count < MAX_PROJECTILES {
            let i = gpu.proj_count;
            gpu.proj_count += 1;
            i
        } else {
            return -1;  // At capacity
        };

        // Calculate velocity
        let dx = to_x - from_x;
        let dy = to_y - from_y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < 0.001 {
            return -1;  // No direction
        }
        let vx = (dx / dist) * PROJECTILE_SPEED;
        let vy = (dy / dist) * PROJECTILE_SPEED;

        // Upload to GPU
        gpu.upload_projectile(idx, from_x, from_y, vx, vy, damage, faction, shooter);

        idx as i32
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
        if let Ok(mut queue) = TARGET_QUEUE.lock() {
            queue.push(SetTargetMsg { npc_index: npc_index as usize, x, y });
        }

        if let Some(gpu) = self.gpu.as_mut() {
            let idx = npc_index as usize;
            let npc_count = GPU_READ_STATE.lock().map(|s| s.npc_count).unwrap_or(0);
            if idx < npc_count {
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
        if let Ok(mut queue) = DAMAGE_QUEUE.lock() {
            queue.push(DamageMsg {
                npc_index: npc_index as usize,
                amount,
            });
        }
    }

    // ========================================================================
    // QUERY API
    // ========================================================================

    #[func]
    fn get_npc_count(&self) -> i32 {
        GPU_READ_STATE.lock().map(|s| s.npc_count as i32).unwrap_or(0)
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
            let npc_count = GPU_READ_STATE.lock().map(|s| s.npc_count).unwrap_or(0);
            if idx < npc_count {
                let x = gpu.positions.get(idx * 2).copied().unwrap_or(0.0);
                let y = gpu.positions.get(idx * 2 + 1).copied().unwrap_or(0.0);
                return Vector2::new(x, y);
            }
        }
        Vector2::ZERO
    }

    #[func]
    fn get_combat_debug(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Ok(debug) = systems::COMBAT_DEBUG.lock() {
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
            dict.set("combat_target_5", debug.sample_combat_target_5);
            dict.set("pos_0_x", debug.sample_pos_0.0);
            dict.set("pos_0_y", debug.sample_pos_0.1);
            dict.set("pos_5_x", debug.sample_pos_5.0);
            dict.set("pos_5_y", debug.sample_pos_5.1);
        }
        // Add grid debug info from GPU cache
        if let Some(gpu) = &self.gpu {
            let npc_count = GPU_READ_STATE.lock().map(|s| s.npc_count).unwrap_or(0);
            // Count NPCs in grid cells near spawn location (cell 5,4 for test 10)
            // cell_size=64, CENTER=(400,300) -> guards at 375,300 -> cell 5,4
            // raiders at 425,300 -> cell 6,4
            let cell_5_4 = 4 * GRID_WIDTH + 5;
            let cell_6_4 = 4 * GRID_WIDTH + 6;
            dict.set("grid_cell_5_4", gpu.grid.counts.get(cell_5_4).copied().unwrap_or(-1));
            dict.set("grid_cell_6_4", gpu.grid.counts.get(cell_6_4).copied().unwrap_or(-1));
            // Sample factions from cache
            dict.set("faction_0", gpu.factions.get(0).copied().unwrap_or(-99));
            dict.set("faction_5", gpu.factions.get(5).copied().unwrap_or(-99));
            // Sample healths from cache
            dict.set("health_0", gpu.healths.get(0).copied().unwrap_or(-99.0) as f32);
            dict.set("health_5", gpu.healths.get(5).copied().unwrap_or(-99.0) as f32);
            // CPU-side position cache
            dict.set("cpu_pos_0_x", gpu.positions.get(0).copied().unwrap_or(-999.0));
            dict.set("cpu_pos_0_y", gpu.positions.get(1).copied().unwrap_or(-999.0));
            dict.set("cpu_pos_5_x", gpu.positions.get(10).copied().unwrap_or(-999.0));
            dict.set("cpu_pos_5_y", gpu.positions.get(11).copied().unwrap_or(-999.0));
            dict.set("npc_count", npc_count as i32);
        }
        dict
    }

    #[func]
    fn get_health_debug(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Ok(debug) = HEALTH_DEBUG.lock() {
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
        dict
    }

    // ========================================================================
    // RESET API
    // ========================================================================

    #[func]
    fn reset(&mut self) {
        // GPU-FIRST: Reset GPU read state
        if let Ok(mut state) = GPU_READ_STATE.lock() {
            state.positions.clear();
            state.combat_targets.clear();
            state.health.clear();
            state.factions.clear();
            state.npc_count = 0;
        }

        // Clear Bevy message queues
        if let Ok(mut queue) = SPAWN_QUEUE.lock() { queue.clear(); }
        if let Ok(mut queue) = TARGET_QUEUE.lock() { queue.clear(); }
        if let Ok(mut queue) = GUARD_QUEUE.lock() { queue.clear(); }
        if let Ok(mut queue) = FARMER_QUEUE.lock() { queue.clear(); }
        if let Ok(mut queue) = RAIDER_QUEUE.lock() { queue.clear(); }
        if let Ok(mut queue) = ARRIVAL_QUEUE.lock() { queue.clear(); }
        if let Ok(mut queue) = DAMAGE_QUEUE.lock() { queue.clear(); }

        // GPU-FIRST: Clear consolidated GPU update queue
        if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() { queue.clear(); }

        // SLOT REUSE: Clear free slot pools
        if let Ok(mut free) = FREE_SLOTS.lock() { free.clear(); }
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

        if let Ok(mut world) = WORLD_DATA.lock() {
            world.towns.clear();
            world.farms.clear();
            world.beds.clear();
            world.guard_posts.clear();
        }
        if let Ok(mut beds) = BED_OCCUPANCY.lock() { beds.occupant_npc.clear(); }
        if let Ok(mut farms) = FARM_OCCUPANCY.lock() { farms.occupant_count.clear(); }

        self.prev_arrivals.fill(false);

        if let Ok(mut flag) = RESET_BEVY.lock() { *flag = true; }
    }

    // ========================================================================
    // WORLD DATA API
    // ========================================================================

    #[func]
    fn init_world(&mut self, town_count: i32) {
        if let Ok(mut world) = WORLD_DATA.lock() {
            world.towns = Vec::with_capacity(town_count as usize);
            world.farms = Vec::new();
            world.beds = Vec::new();
            world.guard_posts = Vec::new();
        }
        if let Ok(mut beds) = BED_OCCUPANCY.lock() { beds.occupant_npc = Vec::new(); }
        if let Ok(mut farms) = FARM_OCCUPANCY.lock() { farms.occupant_count = Vec::new(); }
    }

    #[func]
    fn add_town(&mut self, name: GString, center_x: f32, center_y: f32, camp_x: f32, camp_y: f32) {
        if let Ok(mut world) = WORLD_DATA.lock() {
            world.towns.push(Town {
                name: name.to_string(),
                center: Vector2::new(center_x, center_y),
                camp_position: Vector2::new(camp_x, camp_y),
            });
        }
    }

    #[func]
    fn add_farm(&mut self, x: f32, y: f32, town_idx: i32) {
        if let Ok(mut world) = WORLD_DATA.lock() {
            world.farms.push(Farm {
                position: Vector2::new(x, y),
                town_idx: town_idx as u32,
            });
        }
        if let Ok(mut farms) = FARM_OCCUPANCY.lock() { farms.occupant_count.push(0); }
    }

    #[func]
    fn add_bed(&mut self, x: f32, y: f32, town_idx: i32) {
        if let Ok(mut world) = WORLD_DATA.lock() {
            world.beds.push(Bed {
                position: Vector2::new(x, y),
                town_idx: town_idx as u32,
            });
        }
        if let Ok(mut beds) = BED_OCCUPANCY.lock() { beds.occupant_npc.push(-1); }
    }

    #[func]
    fn add_guard_post(&mut self, x: f32, y: f32, town_idx: i32, patrol_order: i32) {
        if let Ok(mut world) = WORLD_DATA.lock() {
            world.guard_posts.push(GuardPost {
                position: Vector2::new(x, y),
                town_idx: town_idx as u32,
                patrol_order: patrol_order as u32,
            });
        }
    }

    // ========================================================================
    // WORLD QUERY API
    // ========================================================================

    #[func]
    fn get_town_center(&self, town_idx: i32) -> Vector2 {
        if let Ok(world) = WORLD_DATA.lock() {
            if let Some(town) = world.towns.get(town_idx as usize) {
                return town.center;
            }
        }
        Vector2::ZERO
    }

    #[func]
    fn get_camp_position(&self, town_idx: i32) -> Vector2 {
        if let Ok(world) = WORLD_DATA.lock() {
            if let Some(town) = world.towns.get(town_idx as usize) {
                return town.camp_position;
            }
        }
        Vector2::ZERO
    }

    #[func]
    fn get_patrol_post(&self, town_idx: i32, patrol_order: i32) -> Vector2 {
        if let Ok(world) = WORLD_DATA.lock() {
            for post in &world.guard_posts {
                if post.town_idx == town_idx as u32 && post.patrol_order == patrol_order as u32 {
                    return post.position;
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

        if let (Ok(world), Ok(beds)) = (WORLD_DATA.lock(), BED_OCCUPANCY.lock()) {
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
        best_idx
    }

    #[func]
    fn get_nearest_free_farm(&self, town_idx: i32, x: f32, y: f32) -> i32 {
        let pos = Vector2::new(x, y);
        let mut best_idx: i32 = -1;
        let mut best_dist = f32::MAX;

        if let (Ok(world), Ok(farms)) = (WORLD_DATA.lock(), FARM_OCCUPANCY.lock()) {
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
        best_idx
    }

    #[func]
    fn reserve_bed(&mut self, bed_idx: i32, npc_idx: i32) -> bool {
        if let Ok(mut beds) = BED_OCCUPANCY.lock() {
            let idx = bed_idx as usize;
            if idx < beds.occupant_npc.len() && beds.occupant_npc[idx] < 0 {
                beds.occupant_npc[idx] = npc_idx;
                return true;
            }
        }
        false
    }

    #[func]
    fn release_bed(&mut self, bed_idx: i32) {
        if let Ok(mut beds) = BED_OCCUPANCY.lock() {
            let idx = bed_idx as usize;
            if idx < beds.occupant_npc.len() { beds.occupant_npc[idx] = -1; }
        }
    }

    #[func]
    fn reserve_farm(&mut self, farm_idx: i32) -> bool {
        if let Ok(mut farms) = FARM_OCCUPANCY.lock() {
            let idx = farm_idx as usize;
            if idx < farms.occupant_count.len() && farms.occupant_count[idx] < 1 {
                farms.occupant_count[idx] += 1;
                return true;
            }
        }
        false
    }

    #[func]
    fn release_farm(&mut self, farm_idx: i32) {
        if let Ok(mut farms) = FARM_OCCUPANCY.lock() {
            let idx = farm_idx as usize;
            if idx < farms.occupant_count.len() && farms.occupant_count[idx] > 0 {
                farms.occupant_count[idx] -= 1;
            }
        }
    }

    #[func]
    fn get_world_stats(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Ok(world) = WORLD_DATA.lock() {
            dict.set("town_count", world.towns.len() as i32);
            dict.set("farm_count", world.farms.len() as i32);
            dict.set("bed_count", world.beds.len() as i32);
            dict.set("guard_post_count", world.guard_posts.len() as i32);
        }
        if let Ok(beds) = BED_OCCUPANCY.lock() {
            let free_beds = beds.occupant_npc.iter().filter(|&&x| x < 0).count();
            dict.set("free_beds", free_beds as i32);
        }
        if let Ok(farms) = FARM_OCCUPANCY.lock() {
            let free_farms = farms.occupant_count.iter().filter(|&&x| x < 1).count();
            dict.set("free_farms", free_farms as i32);
        }
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
}
