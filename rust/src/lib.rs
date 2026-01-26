//! # Endless ECS - GPU-Accelerated NPC Physics
//!
//! This module implements a hybrid CPU/GPU architecture for managing thousands of NPCs:
//!
//! ## Architecture Overview
//!
//! ```text
//! ┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
//! │    GDScript     │────▶│   Bevy ECS      │────▶│   GPU Compute   │
//! │  (game logic)   │     │ (logical state) │     │   (physics)     │
//! └─────────────────┘     └─────────────────┘     └─────────────────┘
//!         │                       │                       │
//!         │ spawn_npc()           │ Components            │ Positions
//!         │ set_target()          │ Messages              │ Velocities
//!         ▼                       ▼                       ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        GPU Buffers                               │
//! │  [positions] [targets] [colors] [speeds] [grid] [arrivals]      │
//! └─────────────────────────────────────────────────────────────────┘
//!                                 │
//!                                 ▼
//!                         ┌─────────────────┐
//!                         │   MultiMesh     │
//!                         │  (rendering)    │
//!                         └─────────────────┘
//! ```
//!
//! ## Data Flow
//!
//! 1. **GDScript** calls `spawn_npc()` or `set_target()` on EcsNpcManager
//! 2. Commands are queued in static Mutex queues (thread-safe)
//! 3. **Bevy ECS** drains queues each frame and updates logical state
//! 4. **GPU Compute** runs separation physics on all NPCs in parallel
//! 5. **MultiMesh** receives position/color data for batch rendering
//!
//! ## Why This Architecture?
//!
//! - **Bevy ECS**: Handles game logic (state machines, decisions) with cache-friendly DOD
//! - **GPU Compute**: Runs O(n) neighbor queries in parallel (10,000 NPCs @ 140fps)
//! - **Static Mutexes**: Bridge between Godot's single-threaded calls and Bevy's systems
//! - **RenderingDevice**: Godot's GPU abstraction (not Send-safe, so owned by EcsNpcManager)
//!
//! ## Module Structure
//!
//! - `components` - ECS components (NpcIndex, Job, Energy, Health, state markers)
//! - `constants` - Tuning parameters (grid size, separation strength, energy rates)
//! - `resources` - Bevy resources (NpcCount, GpuData)
//! - `world` - World data structs (Town, Farm, Bed, GuardPost) and static storage
//! - `messages` - Message types and static queues for GDScript→Bevy communication
//! - `gpu` - GPU compute shader dispatch and buffer management
//! - `systems` - Bevy systems (spawn, movement, energy, behavior, health)

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

/// Build the Bevy application. Called once at startup by godot-bevy.
#[bevy_app]
fn build_app(app: &mut bevy::prelude::App) {
    app.add_message::<SpawnNpcMsg>()
       .add_message::<SetTargetMsg>()
       .add_message::<SpawnGuardMsg>()
       .add_message::<SpawnFarmerMsg>()
       .add_message::<SpawnRaiderMsg>()
       .add_message::<ArrivalMsg>()
       .add_message::<DamageMsg>()
       .init_resource::<NpcCount>()
       .init_resource::<GpuData>()
       .init_resource::<world::WorldData>()
       .init_resource::<world::BedOccupancy>()
       .init_resource::<world::FarmOccupancy>()
       // Systems run in groups to avoid tuple size limit
       // Group 1: Reset and drain queues
       .add_systems(bevy::prelude::Update, (
           reset_bevy_system,
           drain_spawn_queue,
           drain_target_queue,
           drain_guard_queue,
           drain_farmer_queue,
           drain_raider_queue,
           drain_arrival_queue,
           drain_damage_queue,
       ).chain())
       // Group 2: Spawn systems
       .add_systems(bevy::prelude::Update, (
           spawn_npc_system,
           spawn_guard_system,
           spawn_farmer_system,
           spawn_raider_system,
           apply_targets_system,
       ).chain())
       // Group 3: Combat and health
       .add_systems(bevy::prelude::Update, (
           cooldown_system,
           attack_system,
           damage_system,
           death_system,
           death_cleanup_system,
       ).chain())
       // Group 4: Behavior systems
       .add_systems(bevy::prelude::Update, (
           handle_arrival_system,
           energy_system,
           tired_system,
           resume_patrol_system,
           resume_work_system,
           patrol_system,
       ).chain());
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
        }
    }

    fn ready(&mut self) {
        self.gpu = GpuCompute::new();
        if self.gpu.is_none() {
            godot_error!("[EcsNpcManager] Failed to initialize GPU compute");
            return;
        }
        self.setup_multimesh(MAX_NPC_COUNT as i32);
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

        let npc_count = GPU_NPC_COUNT.lock().map(|c| *c).unwrap_or(0);

        // Drain GPU target queue and upload to GPU buffers
        if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
            for msg in queue.drain(..) {
                if msg.npc_index < npc_count {
                    let target_bytes: Vec<u8> = [msg.x, msg.y].iter()
                        .flat_map(|f| f.to_le_bytes()).collect();
                    let target_packed = PackedByteArray::from(target_bytes.as_slice());
                    gpu.rd.buffer_update(
                        gpu.target_buffer,
                        (msg.npc_index * 8) as u32,
                        8,
                        &target_packed
                    );

                    let arrival_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
                    let arrival_packed = PackedByteArray::from(arrival_bytes.as_slice());
                    gpu.rd.buffer_update(
                        gpu.arrival_buffer,
                        (msg.npc_index * 4) as u32,
                        4,
                        &arrival_packed
                    );

                    let backoff_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
                    let backoff_packed = PackedByteArray::from(backoff_bytes.as_slice());
                    gpu.rd.buffer_update(
                        gpu.backoff_buffer,
                        (msg.npc_index * 4) as u32,
                        4,
                        &backoff_packed
                    );

                    self.prev_arrivals[msg.npc_index] = false;
                }
            }
        }

        if npc_count > 0 {
            // Upload factions and healths before dispatch (for GPU targeting)
            gpu.upload_factions(npc_count);
            gpu.upload_healths(npc_count);

            gpu.dispatch(npc_count, delta as f32);
            gpu.read_positions_from_gpu(npc_count);

            // Read combat targets from GPU
            gpu.read_combat_targets(npc_count);

            // Copy to static for Bevy access
            if let Ok(mut targets) = GPU_COMBAT_TARGETS.lock() {
                targets.clear();
                targets.extend_from_slice(&gpu.combat_targets[..npc_count]);
            }
            if let Ok(mut positions) = GPU_POSITIONS.lock() {
                positions.clear();
                positions.extend_from_slice(&gpu.positions[..(npc_count * 2)]);
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

    // ========================================================================
    // SPAWN API
    // ========================================================================

    #[func]
    fn spawn_npc(&mut self, x: f32, y: f32, job: i32) {
        let idx = {
            let mut guard = GPU_NPC_COUNT.lock().unwrap();
            let idx = *guard;
            if idx < MAX_NPC_COUNT {
                *guard += 1;
            }
            idx
        };

        if idx >= MAX_NPC_COUNT {
            return;
        }

        if let Ok(mut queue) = SPAWN_QUEUE.lock() {
            queue.push(SpawnNpcMsg { x, y, job });
        }

        if let Some(gpu) = self.gpu.as_mut() {
            let (r, g, b, a) = Job::from_i32(job).color();

            let pos_bytes: Vec<u8> = [x, y].iter().flat_map(|f| f.to_le_bytes()).collect();
            let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &pos_packed);
            gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &pos_packed);

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
        let idx = {
            let mut guard = GPU_NPC_COUNT.lock().unwrap();
            let idx = *guard;
            if idx < MAX_NPC_COUNT {
                *guard += 1;
            }
            idx
        };

        if idx >= MAX_NPC_COUNT {
            return;
        }

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

            // Upload health
            let health_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let health_packed = PackedByteArray::from(health_bytes.as_slice());
            gpu.rd.buffer_update(gpu.health_buffer, (idx * 4) as u32, 4, &health_packed);
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
        let idx = {
            let mut guard = GPU_NPC_COUNT.lock().unwrap();
            let idx = *guard;
            if idx < MAX_NPC_COUNT {
                *guard += 1;
            }
            idx
        };

        if idx >= MAX_NPC_COUNT {
            return;
        }

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

            // Upload health
            let health_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let health_packed = PackedByteArray::from(health_bytes.as_slice());
            gpu.rd.buffer_update(gpu.health_buffer, (idx * 4) as u32, 4, &health_packed);
        }

        self.prev_arrivals[idx] = true;
    }

    #[func]
    fn spawn_farmer(&mut self, x: f32, y: f32, town_idx: i32, home_x: f32, home_y: f32, work_x: f32, work_y: f32) {
        let idx = {
            let mut guard = GPU_NPC_COUNT.lock().unwrap();
            let idx = *guard;
            if idx < MAX_NPC_COUNT {
                *guard += 1;
            }
            idx
        };

        if idx >= MAX_NPC_COUNT {
            return;
        }

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

            // Upload health
            let health_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let health_packed = PackedByteArray::from(health_bytes.as_slice());
            gpu.rd.buffer_update(gpu.health_buffer, (idx * 4) as u32, 4, &health_packed);
        }
    }

    #[func]
    fn spawn_raider(&mut self, x: f32, y: f32, camp_x: f32, camp_y: f32) {
        let idx = {
            let mut guard = GPU_NPC_COUNT.lock().unwrap();
            let idx = *guard;
            if idx < MAX_NPC_COUNT {
                *guard += 1;
            }
            idx
        };

        if idx >= MAX_NPC_COUNT {
            return;
        }

        if let Ok(mut queue) = RAIDER_QUEUE.lock() {
            queue.push(SpawnRaiderMsg { x, y, camp_x, camp_y });
        }

        if let Some(gpu) = self.gpu.as_mut() {
            let (r, g, b, a) = Job::Raider.color();

            let pos_bytes: Vec<u8> = [x, y].iter().flat_map(|f| f.to_le_bytes()).collect();
            let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &pos_packed);
            gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &pos_packed);

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

            // Upload health
            let health_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let health_packed = PackedByteArray::from(health_bytes.as_slice());
            gpu.rd.buffer_update(gpu.health_buffer, (idx * 4) as u32, 4, &health_packed);
        }
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
            let npc_count = GPU_NPC_COUNT.lock().map(|c| *c).unwrap_or(0);
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
        GPU_NPC_COUNT.lock().map(|c| *c as i32).unwrap_or(0)
    }

    #[func]
    fn get_build_info(&self) -> GString {
        let timestamp = option_env!("BUILD_TIMESTAMP").unwrap_or("unknown");
        let commit = option_env!("BUILD_COMMIT").unwrap_or("unknown");
        GString::from(&format!("BUILD: {} ({})", timestamp, commit))
    }

    #[func]
    fn get_debug_stats(&mut self) -> Dictionary {
        let mut dict = Dictionary::new();
        if let Some(gpu) = &mut self.gpu {
            let npc_count = GPU_NPC_COUNT.lock().map(|c| *c).unwrap_or(0);

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
            let npc_count = GPU_NPC_COUNT.lock().map(|c| *c).unwrap_or(0);
            if idx < npc_count {
                let x = gpu.positions.get(idx * 2).copied().unwrap_or(0.0);
                let y = gpu.positions.get(idx * 2 + 1).copied().unwrap_or(0.0);
                return Vector2::new(x, y);
            }
        }
        Vector2::ZERO
    }

    #[func]
    fn get_health_debug(&self) -> Dictionary {
        let mut dict = Dictionary::new();
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
        if let Ok(mut count) = GPU_NPC_COUNT.lock() {
            *count = 0;
        }

        if let Ok(mut queue) = SPAWN_QUEUE.lock() { queue.clear(); }
        if let Ok(mut queue) = TARGET_QUEUE.lock() { queue.clear(); }
        if let Ok(mut queue) = GUARD_QUEUE.lock() { queue.clear(); }
        if let Ok(mut queue) = FARMER_QUEUE.lock() { queue.clear(); }
        if let Ok(mut queue) = RAIDER_QUEUE.lock() { queue.clear(); }
        if let Ok(mut queue) = ARRIVAL_QUEUE.lock() { queue.clear(); }
        if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() { queue.clear(); }
        if let Ok(mut queue) = DAMAGE_QUEUE.lock() { queue.clear(); }

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
    fn get_world_stats(&self) -> Dictionary {
        let mut dict = Dictionary::new();
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
    fn get_guard_debug(&mut self) -> Dictionary {
        let mut dict = Dictionary::new();
        if let Some(gpu) = &mut self.gpu {
            let npc_count = GPU_NPC_COUNT.lock().map(|c| *c).unwrap_or(0);

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
