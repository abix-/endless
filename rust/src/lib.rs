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

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;
use godot_bevy::prelude::*;
use godot::classes::{RenderingServer, RenderingDevice, RdUniform, QuadMesh, INode2D};
use godot::classes::rendering_device::UniformType;
use godot::classes::ResourceLoader;
use std::sync::{Mutex, LazyLock};

// ============================================================================
// CONSTANTS - Tuning parameters for the NPC system
// ============================================================================

/// Maximum NPCs the system can handle. Buffers are pre-allocated to this size.
/// Higher = more memory usage, but allows larger crowds.
const MAX_NPC_COUNT: usize = 10000;

/// Spatial grid dimensions. The world is divided into GRID_WIDTH × GRID_HEIGHT cells.
/// Each cell is CELL_SIZE pixels wide. Total coverage: 128 × 64 = 8192 pixels per axis.
const GRID_WIDTH: usize = 128;
const GRID_HEIGHT: usize = 128;
const GRID_CELLS: usize = GRID_WIDTH * GRID_HEIGHT;

/// Maximum NPCs per grid cell. If exceeded, additional NPCs are ignored for collision.
/// Too low = missed collisions. Too high = wasted memory and slower lookups.
const MAX_PER_CELL: usize = 48;

/// Size of each grid cell in pixels. Should be >= SEPARATION_RADIUS for correct neighbor detection.
/// NPCs check their own cell plus 8 neighbors (3×3 area).
const CELL_SIZE: f32 = 64.0;

/// Minimum distance NPCs try to maintain from each other.
/// If two NPCs are closer than this, separation forces push them apart.
const SEPARATION_RADIUS: f32 = 20.0;

/// How strongly NPCs push away from neighbors. Higher = faster separation but more jittery.
/// This is multiplied by the overlap distance, so closer NPCs get pushed harder.
const SEPARATION_STRENGTH: f32 = 50.0;

/// Distance from target at which an NPC is considered "arrived".
/// Once within this distance, the NPC stops moving toward target.
const ARRIVAL_THRESHOLD: f32 = 8.0;

/// Floats per NPC instance in the MultiMesh buffer.
/// Transform2D (8 floats) + Color (4 floats) = 12 floats per NPC.
const FLOATS_PER_INSTANCE: usize = 12;

/// Size of push constants passed to the compute shader.
/// Must match the PushConstants struct in npc_compute.glsl (48 bytes with padding).
const PUSH_CONSTANTS_SIZE: usize = 48;

// ============================================================================
// GUARD BEHAVIOR CONSTANTS
// ============================================================================

/// Energy threshold below which guards go rest.
const ENERGY_HUNGRY: f32 = 50.0;

/// Energy threshold above which guards resume patrol.
const ENERGY_RESTED: f32 = 80.0;

/// Ticks a guard waits at a post before moving to next (60 = ~1 second at 60fps).
const GUARD_PATROL_WAIT: u32 = 60;

/// Energy drain per tick while active.
/// At 0.02/tick with 60-tick patrol wait, guards do ~10 rotations before rest.
const ENERGY_DRAIN_RATE: f32 = 0.02;

/// Energy recovery per tick while resting.
const ENERGY_RECOVER_RATE: f32 = 0.2;

// ============================================================================
// WORLD DATA STRUCTS - Towns, farms, beds, guard posts
// ============================================================================

/// A town with its center position and associated raider camp.
#[derive(Clone, Debug)]
pub struct Town {
    pub name: String,
    pub center: Vector2,
    pub camp_position: Vector2,
}

/// A farm building that farmers work at.
#[derive(Clone, Debug)]
pub struct Farm {
    pub position: Vector2,
    pub town_idx: u32,
}

/// A bed where NPCs sleep.
#[derive(Clone, Debug)]
pub struct Bed {
    pub position: Vector2,
    pub town_idx: u32,
}

/// A guard post where guards patrol.
#[derive(Clone, Debug)]
pub struct GuardPost {
    pub position: Vector2,
    pub town_idx: u32,
    /// Patrol order (0-3 for clockwise perimeter)
    pub patrol_order: u32,
}

// ============================================================================
// WORLD RESOURCES - Shared world state accessible by all systems
// ============================================================================

/// Contains all world layout data (immutable after init).
#[derive(Resource, Default)]
pub struct WorldData {
    pub towns: Vec<Town>,
    pub farms: Vec<Farm>,
    pub beds: Vec<Bed>,
    pub guard_posts: Vec<GuardPost>,
}

/// Tracks which NPCs occupy each bed (-1 = free).
#[derive(Resource, Default)]
pub struct BedOccupancy {
    pub occupant_npc: Vec<i32>,
}

/// Tracks how many NPCs are working at each farm.
#[derive(Resource, Default)]
pub struct FarmOccupancy {
    pub occupant_count: Vec<i32>,
}

// ============================================================================
// ECS COMPONENTS - Bevy entities have these attached
// ============================================================================

/// Links a Bevy entity to its index in the GPU buffers.
/// When spawning an NPC, we create an entity with NpcIndex(n) where n is the buffer slot.
#[derive(Component, Clone, Copy)]
pub struct NpcIndex(pub usize);

/// NPC's job determines behavior and color.
/// - Farmer (green): works at farms, avoids combat
/// - Guard (blue): patrols and fights raiders
/// - Raider (red): attacks guards, steals from farms
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Job {
    Farmer,
    Guard,
    Raider,
}

impl Job {
    /// Convert from GDScript integer (0=Farmer, 1=Guard, 2=Raider)
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Job::Guard,
            2 => Job::Raider,
            _ => Job::Farmer,
        }
    }

    /// RGBA color for this job type. Alpha=1.0 means "has target" on GPU.
    pub fn color(&self) -> (f32, f32, f32, f32) {
        match self {
            Job::Farmer => (0.2, 0.8, 0.2, 1.0),  // Green
            Job::Guard => (0.2, 0.4, 0.9, 1.0),   // Blue
            Job::Raider => (0.9, 0.2, 0.2, 1.0),  // Red
        }
    }
}

/// Marker component: this NPC has an active target to move toward.
/// Added when set_target() is called, could be removed when arrived.
#[derive(Component)]
pub struct HasTarget;

/// Movement speed in pixels per second.
#[derive(Component, Clone, Copy)]
pub struct Speed(pub f32);

impl Default for Speed {
    fn default() -> Self {
        Self(100.0)  // 100 pixels/second base speed
    }
}

// ============================================================================
// GUARD COMPONENTS - State machine for guard behavior
// ============================================================================

/// Guard marker - identifies NPC as a guard (for queries).
/// Patrol data is now in PatrolRoute component.
#[derive(Component)]
pub struct Guard {
    pub town_idx: u32,
}

/// Farmer marker - identifies NPC as a farmer.
#[derive(Component)]
pub struct Farmer {
    pub town_idx: u32,
}

/// NPC energy level (0-100). Drains while active, recovers while resting.
#[derive(Component)]
pub struct Energy(pub f32);

impl Default for Energy {
    fn default() -> Self {
        Self(100.0)
    }
}

/// Where the NPC goes to rest (bed position).
#[derive(Component)]
pub struct Home(pub Vector2);

/// Patrol route for guards (or any NPC that patrols).
#[derive(Component)]
pub struct PatrolRoute {
    pub posts: Vec<Vector2>,
    pub current: usize,
}

/// Work position for farmers (or any NPC that works at a location).
#[derive(Component)]
pub struct WorkPosition(pub Vector2);

// --- State Markers (mutually exclusive) ---

/// Guard is moving toward next patrol post.
#[derive(Component)]
pub struct Patrolling;

/// Guard is standing at a post, waiting before moving to next.
#[derive(Component)]
pub struct OnDuty {
    pub ticks_waiting: u32,
}

/// NPC is at home/bed recovering energy.
#[derive(Component)]
pub struct Resting;

/// NPC is walking home to rest.
#[derive(Component)]
pub struct GoingToRest;

/// NPC is at work position, working.
#[derive(Component)]
pub struct Working;

/// NPC is walking to work position.
#[derive(Component)]
pub struct GoingToWork;

// ============================================================================
// ECS MESSAGES - Commands sent from GDScript to Bevy
// ============================================================================

/// Request to spawn a new NPC at position (x, y) with the given job type.
#[derive(Message, Clone)]
pub struct SpawnNpcMsg {
    pub x: f32,
    pub y: f32,
    pub job: i32,
}

/// Request to set an NPC's movement target.
#[derive(Message, Clone)]
pub struct SetTargetMsg {
    pub npc_index: usize,
    pub x: f32,
    pub y: f32,
}

/// Request to spawn a guard with home position and town assignment.
#[derive(Message, Clone)]
pub struct SpawnGuardMsg {
    pub x: f32,
    pub y: f32,
    pub town_idx: u32,
    pub home_x: f32,
    pub home_y: f32,
    pub starting_post: u32,
}

/// Notification that an NPC has arrived at its target.
#[derive(Message, Clone)]
pub struct ArrivalMsg {
    pub npc_index: usize,
}

/// Request to spawn a farmer with home and work positions.
#[derive(Message, Clone)]
pub struct SpawnFarmerMsg {
    pub x: f32,
    pub y: f32,
    pub town_idx: u32,
    pub home_x: f32,
    pub home_y: f32,
    pub work_x: f32,
    pub work_y: f32,
}

// ============================================================================
// ECS RESOURCES - Shared state accessible by all systems
// ============================================================================

/// Tracks total number of active NPCs.
#[derive(Resource, Default)]
pub struct NpcCount(pub usize);

/// CPU-side copy of GPU data, used for uploading to GPU buffers.
/// When `dirty` is true, the data needs to be re-uploaded.
#[derive(Resource)]
pub struct GpuData {
    /// Position data: [x0, y0, x1, y1, ...] - 2 floats per NPC
    pub positions: Vec<f32>,
    /// Target positions: [tx0, ty0, tx1, ty1, ...] - 2 floats per NPC
    pub targets: Vec<f32>,
    /// Colors: [r0, g0, b0, a0, r1, g1, b1, a1, ...] - 4 floats per NPC
    /// Alpha channel doubles as "has target" flag (a > 0 means seeking target)
    pub colors: Vec<f32>,
    /// Movement speeds: one float per NPC
    pub speeds: Vec<f32>,
    /// Current NPC count (may differ from GPU_NPC_COUNT during spawn frame)
    pub npc_count: usize,
    /// True if data changed and needs GPU upload
    pub dirty: bool,
}

impl Default for GpuData {
    fn default() -> Self {
        Self {
            positions: vec![0.0; MAX_NPC_COUNT * 2],
            targets: vec![0.0; MAX_NPC_COUNT * 2],
            colors: vec![0.0; MAX_NPC_COUNT * 4],
            speeds: vec![0.0; MAX_NPC_COUNT],
            npc_count: 0,
            dirty: false,
        }
    }
}

// ============================================================================
// STATIC QUEUES - Thread-safe communication from Godot to Bevy
// ============================================================================
//
// Why static Mutexes?
// - Godot calls (spawn_npc, set_target) happen on main thread
// - Bevy systems run in their own scheduling context
// - We can't pass references between them, so we use global queues
// - Mutex ensures thread-safety (even though Godot is single-threaded, Bevy isn't)

/// Queue of pending spawn requests. Drained each frame by drain_spawn_queue system.
static SPAWN_QUEUE: Mutex<Vec<SpawnNpcMsg>> = Mutex::new(Vec::new());

/// Queue of pending target updates. Drained each frame by drain_target_queue system.
static TARGET_QUEUE: Mutex<Vec<SetTargetMsg>> = Mutex::new(Vec::new());

/// Queue of pending guard spawn requests.
static GUARD_QUEUE: Mutex<Vec<SpawnGuardMsg>> = Mutex::new(Vec::new());

/// Queue of pending farmer spawn requests.
static FARMER_QUEUE: Mutex<Vec<SpawnFarmerMsg>> = Mutex::new(Vec::new());

/// Queue of arrival notifications (NPC index that just arrived).
static ARRIVAL_QUEUE: Mutex<Vec<ArrivalMsg>> = Mutex::new(Vec::new());

/// Queue of target updates that need to be uploaded to GPU.
/// Bevy systems push here, process() drains and uploads.
static GPU_TARGET_QUEUE: Mutex<Vec<SetTargetMsg>> = Mutex::new(Vec::new());

/// Authoritative NPC count. Updated immediately on spawn (not waiting for Bevy).
/// This ensures GPU gets correct count even before Bevy processes the spawn message.
static GPU_NPC_COUNT: Mutex<usize> = Mutex::new(0);

/// World data (towns, farms, beds, guard posts). Initialized once from GDScript.
static WORLD_DATA: LazyLock<Mutex<WorldData>> = LazyLock::new(|| Mutex::new(WorldData::default()));

/// Bed occupancy tracking (-1 = free, >= 0 = NPC index).
static BED_OCCUPANCY: LazyLock<Mutex<BedOccupancy>> = LazyLock::new(|| Mutex::new(BedOccupancy::default()));

/// Farm occupancy tracking (count of NPCs working at each farm).
static FARM_OCCUPANCY: LazyLock<Mutex<FarmOccupancy>> = LazyLock::new(|| Mutex::new(FarmOccupancy::default()));

/// Flag to trigger Bevy entity despawn on next frame.
static RESET_BEVY: Mutex<bool> = Mutex::new(false);

// ============================================================================
// SPATIAL GRID - O(n) neighbor lookup for collision detection
// ============================================================================
//
// Without spatial partitioning, checking all pairs is O(n²) - 100M checks for 10K NPCs!
// With a grid, each NPC only checks its 3×3 neighborhood - typically < 100 checks.
//
// How it works:
// 1. Each frame, clear the grid
// 2. Insert each NPC into the cell containing its position
// 3. Upload grid to GPU
// 4. Shader reads neighbors from grid instead of checking all NPCs

/// Spatial partitioning grid for efficient neighbor queries.
struct SpatialGrid {
    /// Number of NPCs in each cell: counts[cell_idx] = n
    counts: Vec<i32>,
    /// NPC indices in each cell: data[cell_idx * MAX_PER_CELL + n] = npc_index
    /// Stored as flat array for GPU upload efficiency.
    data: Vec<i32>,
}

impl SpatialGrid {
    fn new() -> Self {
        Self {
            counts: vec![0i32; GRID_CELLS],
            data: vec![0i32; GRID_CELLS * MAX_PER_CELL],
        }
    }

    /// Reset all cell counts to zero (called each frame before rebuilding)
    fn clear(&mut self) {
        self.counts.fill(0);
    }

    /// Add an NPC to the grid cell containing position (x, y)
    fn insert(&mut self, x: f32, y: f32, npc_idx: i32) {
        // Clamp to grid bounds (handles NPCs outside expected world area)
        let cx = ((x / CELL_SIZE) as usize).min(GRID_WIDTH - 1);
        let cy = ((y / CELL_SIZE) as usize).min(GRID_HEIGHT - 1);
        let cell_idx = cy * GRID_WIDTH + cx;

        let count = self.counts[cell_idx] as usize;
        if count < MAX_PER_CELL {
            self.data[cell_idx * MAX_PER_CELL + count] = npc_idx;
            self.counts[cell_idx] += 1;
        }
        // If cell is full, NPC is silently ignored for collision
        // This is a tradeoff: prevents buffer overflow, but may miss collisions in crowds
    }
}

// ============================================================================
// GPU COMPUTE - Runs physics on thousands of NPCs in parallel
// ============================================================================
//
// Why GPU compute instead of CPU?
// - GPU has thousands of cores vs CPU's ~8-16
// - Separation physics is "embarrassingly parallel" - each NPC is independent
// - Memory bandwidth: GPU can read/write 10K positions in microseconds
//
// Architecture:
// - 9 GPU buffers store all NPC state (positions, targets, colors, etc.)
// - Compute shader runs once per NPC per frame
// - Shader reads neighbors from spatial grid, calculates forces, writes new position
// - MultiMesh buffer is written directly by shader - zero CPU copy for rendering!

/// GPU compute context - owns RenderingDevice and all GPU buffers.
/// Note: RenderingDevice is not Send-safe, so this must stay on main thread.
struct GpuCompute {
    /// Godot's GPU abstraction. Must be kept alive for buffer operations.
    rd: Gd<RenderingDevice>,

    /// Compiled compute shader (kept alive but not used after pipeline creation)
    #[allow(dead_code)]
    shader: Rid,

    /// Compute pipeline - the "program" we dispatch each frame
    pipeline: Rid,

    // === GPU Buffers (binding numbers match npc_compute.glsl) ===

    /// Binding 0: NPC positions [x, y] pairs. Read/write - GPU owns authoritative positions.
    position_buffer: Rid,

    /// Binding 1: Target positions [x, y] pairs. Read-only on GPU, written by CPU.
    target_buffer: Rid,

    /// Binding 2: NPC colors [r, g, b, a]. Alpha > 0 means "seeking target".
    color_buffer: Rid,

    /// Binding 3: Movement speeds, one float per NPC.
    speed_buffer: Rid,

    /// Binding 4: Grid cell counts - how many NPCs in each cell.
    grid_counts_buffer: Rid,

    /// Binding 5: Grid data - which NPCs are in each cell.
    grid_data_buffer: Rid,

    /// Binding 6: MultiMesh output - shader writes transform+color directly here.
    /// This is read by Godot's renderer, achieving zero-copy GPU rendering!
    multimesh_buffer: Rid,

    /// Binding 7: Arrival flags - set to 1 when NPC reaches target or gives up.
    arrival_buffer: Rid,

    /// Binding 8: Backoff counters - TCP-style collision avoidance.
    /// Incremented when NPC is blocked, decremented when making progress.
    /// When backoff > threshold, NPC "gives up" and stops pursuing target.
    backoff_buffer: Rid,

    /// Uniform set - groups all buffers together for binding to shader.
    uniform_set: Rid,

    /// CPU-side spatial grid, rebuilt and uploaded each frame.
    grid: SpatialGrid,

    /// Cached positions read back from GPU for GDScript queries (get_npc_position).
    positions: Vec<f32>,

    /// Cached colors for building multimesh on CPU (colors don't change after spawn).
    colors: Vec<f32>,
}

impl GpuCompute {
    /// Initialize GPU compute: create device, compile shader, allocate buffers.
    /// Returns None if GPU compute isn't available (falls back to CPU would go here).
    fn new() -> Option<Self> {
        // Create a local rendering device (separate from Godot's main renderer)
        let rs = RenderingServer::singleton();
        let mut rd = rs.create_local_rendering_device()?;

        // Load and compile the compute shader from .glsl file
        let shader_file = ResourceLoader::singleton()
            .load("res://shaders/npc_compute.glsl")?;
        let shader_file = shader_file.cast::<godot::classes::RdShaderFile>();
        let spirv = shader_file.get_spirv()?;

        let shader = rd.shader_create_from_spirv(&spirv);
        if !shader.is_valid() {
            godot_error!("[GPU] Failed to create shader");
            return None;
        }

        // Create compute pipeline (combines shader + configuration)
        let pipeline = rd.compute_pipeline_create(shader);
        if !pipeline.is_valid() {
            godot_error!("[GPU] Failed to create pipeline");
            return None;
        }

        // Allocate GPU buffers
        // Position buffer: 2 floats (x, y) × 4 bytes × MAX_NPC_COUNT
        let position_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 8) as u32);
        let target_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 8) as u32);
        // Color buffer: 4 floats (r, g, b, a) × 4 bytes × MAX_NPC_COUNT
        let color_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 16) as u32);
        // Speed buffer: 1 float × 4 bytes × MAX_NPC_COUNT
        let speed_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 4) as u32);
        // Grid buffers
        let grid_counts_buffer = rd.storage_buffer_create((GRID_CELLS * 4) as u32);
        let grid_data_buffer = rd.storage_buffer_create((GRID_CELLS * MAX_PER_CELL * 4) as u32);
        // MultiMesh buffer: 12 floats × 4 bytes × MAX_NPC_COUNT
        let multimesh_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * FLOATS_PER_INSTANCE * 4) as u32);
        // Arrival and backoff: 1 int × 4 bytes × MAX_NPC_COUNT
        let arrival_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 4) as u32);
        let backoff_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 4) as u32);

        // Create uniform set (groups buffers for shader binding)
        let uniform_set = Self::create_uniform_set(
            &mut rd, shader,
            position_buffer, target_buffer, color_buffer, speed_buffer,
            grid_counts_buffer, grid_data_buffer, multimesh_buffer, arrival_buffer,
            backoff_buffer,
        )?;

        // Initialization complete (no console logging)

        Some(Self {
            rd,
            shader,
            pipeline,
            position_buffer,
            target_buffer,
            color_buffer,
            speed_buffer,
            grid_counts_buffer,
            grid_data_buffer,
            multimesh_buffer,
            arrival_buffer,
            backoff_buffer,
            uniform_set,
            grid: SpatialGrid::new(),
            positions: vec![0.0; MAX_NPC_COUNT * 2],
            colors: vec![0.0; MAX_NPC_COUNT * 4],
        })
    }

    /// Create a uniform set binding all 9 buffers to their shader bindings.
    fn create_uniform_set(
        rd: &mut Gd<RenderingDevice>,
        shader: Rid,
        position_buffer: Rid,
        target_buffer: Rid,
        color_buffer: Rid,
        speed_buffer: Rid,
        grid_counts_buffer: Rid,
        grid_data_buffer: Rid,
        multimesh_buffer: Rid,
        arrival_buffer: Rid,
        backoff_buffer: Rid,
    ) -> Option<Rid> {
        let mut uniforms = Array::new();

        // Map buffer -> binding number (must match npc_compute.glsl layout declarations)
        let buffers = [
            (0, position_buffer),   // layout(binding = 0) buffer PositionBuffer
            (1, target_buffer),     // layout(binding = 1) buffer TargetBuffer
            (2, color_buffer),      // layout(binding = 2) buffer ColorBuffer
            (3, speed_buffer),      // layout(binding = 3) buffer SpeedBuffer
            (4, grid_counts_buffer),// layout(binding = 4) buffer GridCounts
            (5, grid_data_buffer),  // layout(binding = 5) buffer GridData
            (6, multimesh_buffer),  // layout(binding = 6) buffer MultiMeshBuffer
            (7, arrival_buffer),    // layout(binding = 7) buffer ArrivalBuffer
            (8, backoff_buffer),    // layout(binding = 8) buffer BackoffBuffer
        ];

        for (binding, buffer) in buffers {
            let mut uniform = RdUniform::new_gd();
            uniform.set_uniform_type(UniformType::STORAGE_BUFFER);
            uniform.set_binding(binding);
            uniform.add_id(buffer);
            uniforms.push(&uniform);
        }

        let uniform_set = rd.uniform_set_create(&uniforms, shader, 0);
        if uniform_set.is_valid() {
            Some(uniform_set)
        } else {
            None
        }
    }

    /// Rebuild spatial grid from CACHED positions and upload to GPU.
    /// Uses positions from last frame (1-frame delay is acceptable for collision grid).
    /// This eliminates the position readback BEFORE the shader, reducing stalls.
    fn build_and_upload_grid(&mut self, npc_count: usize) {
        // Use cached positions (from last frame's readback)
        // 1-frame delay is acceptable for spatial grid - NPCs don't move far in one frame
        self.grid.clear();
        for i in 0..npc_count {
            let x = self.positions[i * 2];
            let y = self.positions[i * 2 + 1];
            self.grid.insert(x, y, i as i32);
        }

        // Upload grid counts to GPU
        let counts_bytes: Vec<u8> = self.grid.counts.iter()
            .flat_map(|i| i.to_le_bytes()).collect();
        let counts_packed = PackedByteArray::from(counts_bytes.as_slice());
        self.rd.buffer_update(self.grid_counts_buffer, 0, counts_packed.len() as u32, &counts_packed);

        // Upload grid data to GPU
        let data_bytes: Vec<u8> = self.grid.data.iter()
            .flat_map(|i| i.to_le_bytes()).collect();
        let data_packed = PackedByteArray::from(data_bytes.as_slice());
        self.rd.buffer_update(self.grid_data_buffer, 0, data_packed.len() as u32, &data_packed);
    }

    /// Dispatch the compute shader: run physics for all NPCs in parallel.
    fn dispatch(&mut self, npc_count: usize, delta: f32) {
        if npc_count == 0 {
            return;
        }

        // Step 1: Rebuild spatial grid (CPU) and upload to GPU
        self.build_and_upload_grid(npc_count);

        // Step 2: Build push constants (shader parameters)
        // These are small, fast-path data passed directly to shader (no buffer needed)
        let mut push_data = vec![0u8; PUSH_CONSTANTS_SIZE];
        push_data[0..4].copy_from_slice(&(npc_count as u32).to_le_bytes());
        push_data[4..8].copy_from_slice(&SEPARATION_RADIUS.to_le_bytes());
        push_data[8..12].copy_from_slice(&SEPARATION_STRENGTH.to_le_bytes());
        push_data[12..16].copy_from_slice(&delta.to_le_bytes());
        push_data[16..20].copy_from_slice(&(GRID_WIDTH as u32).to_le_bytes());
        push_data[20..24].copy_from_slice(&(GRID_HEIGHT as u32).to_le_bytes());
        push_data[24..28].copy_from_slice(&CELL_SIZE.to_le_bytes());
        push_data[28..32].copy_from_slice(&(MAX_PER_CELL as u32).to_le_bytes());
        push_data[32..36].copy_from_slice(&ARRIVAL_THRESHOLD.to_le_bytes());
        // Padding for 48-byte alignment (GPU requires specific alignment)
        push_data[36..40].copy_from_slice(&0.0f32.to_le_bytes());
        push_data[40..44].copy_from_slice(&0.0f32.to_le_bytes());
        push_data[44..48].copy_from_slice(&0.0f32.to_le_bytes());
        let push_constants = PackedByteArray::from(push_data.as_slice());

        // Step 3: Dispatch compute shader
        // Workgroups: each workgroup processes 64 NPCs (local_size_x = 64 in shader)
        // So we need ceil(npc_count / 64) workgroups
        let compute_list = self.rd.compute_list_begin();
        self.rd.compute_list_bind_compute_pipeline(compute_list, self.pipeline);
        self.rd.compute_list_bind_uniform_set(compute_list, self.uniform_set, 0);
        self.rd.compute_list_set_push_constant(compute_list, &push_constants, PUSH_CONSTANTS_SIZE as u32);

        let workgroups = ((npc_count + 63) / 64) as u32;
        self.rd.compute_list_dispatch(compute_list, workgroups, 1, 1);
        self.rd.compute_list_end();

        // Step 4: Submit and wait for GPU
        // sync() blocks until GPU finishes - needed because we read results immediately
        self.rd.submit();
        self.rd.sync();
    }

    /// Build MultiMesh buffer from cached positions and colors (no GPU readback).
    /// This is 5x faster than reading the 480KB multimesh buffer from GPU.
    fn build_multimesh_from_cache(&self, colors: &[f32], npc_count: usize, max_count: usize) -> PackedFloat32Array {
        let float_count = max_count * FLOATS_PER_INSTANCE;
        let mut floats = vec![0.0f32; float_count];

        // Initialize all instances to identity transform at off-screen position
        // This hides unused instances (beyond npc_count)
        for i in 0..max_count {
            let base = i * FLOATS_PER_INSTANCE;
            floats[base + 0] = 1.0;     // Transform scale X
            floats[base + 5] = 1.0;     // Transform scale Y
            floats[base + 3] = -9999.0; // Position X (off-screen)
            floats[base + 7] = -9999.0; // Position Y (off-screen)
        }

        // Build active NPC data from cached positions and colors
        for i in 0..npc_count {
            let base = i * FLOATS_PER_INSTANCE;
            // Transform2D: [a.x, b.x, 0, origin.x, a.y, b.y, 0, origin.y]
            floats[base + 0] = 1.0;                      // scale x
            floats[base + 1] = 0.0;                      // shear
            floats[base + 2] = 0.0;                      // unused
            floats[base + 3] = self.positions[i * 2];    // position x
            floats[base + 4] = 0.0;                      // shear
            floats[base + 5] = 1.0;                      // scale y
            floats[base + 6] = 0.0;                      // unused
            floats[base + 7] = self.positions[i * 2 + 1];// position y
            // Color from cached colors
            floats[base + 8] = colors[i * 4];            // r
            floats[base + 9] = colors[i * 4 + 1];        // g
            floats[base + 10] = colors[i * 4 + 2];       // b
            floats[base + 11] = colors[i * 4 + 3];       // a
        }

        PackedFloat32Array::from(floats.as_slice())
    }

    /// Read positions from GPU after shader runs (for next frame's grid AND current multimesh).
    fn read_positions_from_gpu(&mut self, npc_count: usize) {
        let bytes = self.rd.buffer_get_data(self.position_buffer);
        let byte_slice = bytes.as_slice();
        for i in 0..(npc_count * 2) {
            let offset = i * 4;
            if offset + 4 <= byte_slice.len() {
                self.positions[i] = f32::from_le_bytes([
                    byte_slice[offset],
                    byte_slice[offset + 1],
                    byte_slice[offset + 2],
                    byte_slice[offset + 3],
                ]);
            }
        }
    }
}

// ============================================================================
// ECS SYSTEMS - Bevy functions that run each frame
// ============================================================================

/// Drain the spawn queue and convert to Bevy messages.
/// Runs first in the system chain to ensure spawns are processed before other systems.
fn drain_spawn_queue(mut messages: MessageWriter<SpawnNpcMsg>) {
    if let Ok(mut queue) = SPAWN_QUEUE.lock() {
        for msg in queue.drain(..) {
            messages.write(msg);
        }
    }
}

/// Drain the target queue and convert to Bevy messages.
fn drain_target_queue(mut messages: MessageWriter<SetTargetMsg>) {
    if let Ok(mut queue) = TARGET_QUEUE.lock() {
        for msg in queue.drain(..) {
            messages.write(msg);
        }
    }
}

/// Process spawn messages: create Bevy entities and initialize GPU data.
fn spawn_npc_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnNpcMsg>,
    mut count: ResMut<NpcCount>,
    mut gpu_data: ResMut<GpuData>,
) {
    for event in events.read() {
        let idx = gpu_data.npc_count;
        if idx >= MAX_NPC_COUNT {
            continue;
        }

        let job = Job::from_i32(event.job);
        let (r, g, b, a) = job.color();
        let speed = Speed::default().0;

        // Initialize GPU data (CPU-side copy)
        gpu_data.positions[idx * 2] = event.x;
        gpu_data.positions[idx * 2 + 1] = event.y;
        // Target starts at spawn position (no movement until set_target called)
        gpu_data.targets[idx * 2] = event.x;
        gpu_data.targets[idx * 2 + 1] = event.y;
        gpu_data.colors[idx * 4] = r;
        gpu_data.colors[idx * 4 + 1] = g;
        gpu_data.colors[idx * 4 + 2] = b;
        gpu_data.colors[idx * 4 + 3] = a;
        gpu_data.speeds[idx] = speed;
        gpu_data.npc_count += 1;
        gpu_data.dirty = true;

        // Create Bevy entity with components
        commands.spawn((
            NpcIndex(idx),
            job,
            Speed::default(),
        ));
        count.0 += 1;
    }
}

/// Process target messages: update GPU data and add HasTarget component.
fn apply_targets_system(
    mut commands: Commands,
    mut events: MessageReader<SetTargetMsg>,
    mut gpu_data: ResMut<GpuData>,
    query: Query<(Entity, &NpcIndex), Without<HasTarget>>,
) {
    for event in events.read() {
        if event.npc_index < gpu_data.npc_count {
            // Update target in GPU data
            gpu_data.targets[event.npc_index * 2] = event.x;
            gpu_data.targets[event.npc_index * 2 + 1] = event.y;
            gpu_data.dirty = true;

            // Add HasTarget component to entity (if not already present)
            for (entity, npc_idx) in query.iter() {
                if npc_idx.0 == event.npc_index {
                    commands.entity(entity).insert(HasTarget);
                    break;
                }
            }
        }
    }
}

// ============================================================================
// GUARD SYSTEMS - Patrol, rest, energy management
// ============================================================================

/// Drain the guard spawn queue.
fn drain_guard_queue(mut messages: MessageWriter<SpawnGuardMsg>) {
    if let Ok(mut queue) = GUARD_QUEUE.lock() {
        for msg in queue.drain(..) {
            messages.write(msg);
        }
    }
}

/// Drain the farmer spawn queue.
fn drain_farmer_queue(mut messages: MessageWriter<SpawnFarmerMsg>) {
    if let Ok(mut queue) = FARMER_QUEUE.lock() {
        for msg in queue.drain(..) {
            messages.write(msg);
        }
    }
}

/// Process guard spawn messages: create guard entities with full component set.
fn spawn_guard_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnGuardMsg>,
    mut count: ResMut<NpcCount>,
    mut gpu_data: ResMut<GpuData>,
) {
    for event in events.read() {
        let idx = gpu_data.npc_count;
        if idx >= MAX_NPC_COUNT {
            continue;
        }

        let (r, g, b, a) = Job::Guard.color();
        let speed = Speed::default().0;

        // Initialize GPU data
        gpu_data.positions[idx * 2] = event.x;
        gpu_data.positions[idx * 2 + 1] = event.y;
        gpu_data.targets[idx * 2] = event.x;
        gpu_data.targets[idx * 2 + 1] = event.y;
        gpu_data.colors[idx * 4] = r;
        gpu_data.colors[idx * 4 + 1] = g;
        gpu_data.colors[idx * 4 + 2] = b;
        gpu_data.colors[idx * 4 + 3] = a;
        gpu_data.speeds[idx] = speed;
        gpu_data.npc_count += 1;
        gpu_data.dirty = true;

        // Build patrol route from world data
        let patrol_posts: Vec<Vector2> = if let Ok(world) = WORLD_DATA.lock() {
            let mut posts: Vec<(u32, Vector2)> = world.guard_posts.iter()
                .filter(|p| p.town_idx == event.town_idx)
                .map(|p| (p.patrol_order, p.position))
                .collect();
            posts.sort_by_key(|(order, _)| *order);
            posts.into_iter().map(|(_, pos)| pos).collect()
        } else {
            Vec::new()
        };

        // Create guard entity with full component set
        commands.spawn((
            NpcIndex(idx),
            Job::Guard,
            Speed::default(),
            Energy::default(),
            Guard { town_idx: event.town_idx },
            Home(Vector2::new(event.home_x, event.home_y)),
            PatrolRoute {
                posts: patrol_posts,
                current: event.starting_post as usize,
            },
            OnDuty { ticks_waiting: 0 },  // Start on duty at their post
        ));
        count.0 += 1;
    }
}

/// Process farmer spawn messages: create farmer entities with WorkPosition + Home.
fn spawn_farmer_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnFarmerMsg>,
    mut count: ResMut<NpcCount>,
    mut gpu_data: ResMut<GpuData>,
) {
    for event in events.read() {
        let idx = gpu_data.npc_count;
        if idx >= MAX_NPC_COUNT {
            continue;
        }

        let (r, g, b, a) = Job::Farmer.color();
        let speed = Speed::default().0;

        // Initialize GPU data
        gpu_data.positions[idx * 2] = event.x;
        gpu_data.positions[idx * 2 + 1] = event.y;
        gpu_data.targets[idx * 2] = event.x;
        gpu_data.targets[idx * 2 + 1] = event.y;
        gpu_data.colors[idx * 4] = r;
        gpu_data.colors[idx * 4 + 1] = g;
        gpu_data.colors[idx * 4 + 2] = b;
        gpu_data.colors[idx * 4 + 3] = a;
        gpu_data.speeds[idx] = speed;
        gpu_data.npc_count += 1;
        gpu_data.dirty = true;

        // Create farmer entity with behavior components
        commands.spawn((
            NpcIndex(idx),
            Job::Farmer,
            Speed::default(),
            Energy::default(),
            Farmer { town_idx: event.town_idx },
            Home(Vector2::new(event.home_x, event.home_y)),
            WorkPosition(Vector2::new(event.work_x, event.work_y)),
            GoingToWork,  // Start walking to work
        ));
        count.0 += 1;
    }
}

/// Energy system: drain while active, recover while resting.
fn energy_system(
    mut query: Query<(&mut Energy, Option<&Resting>)>,
) {
    for (mut energy, resting) in query.iter_mut() {
        if resting.is_some() {
            energy.0 = (energy.0 + ENERGY_RECOVER_RATE).min(100.0);
        } else {
            energy.0 = (energy.0 - ENERGY_DRAIN_RATE).max(0.0);
        }
    }
}

/// Tired system: anyone with Home + Energy below threshold goes to rest.
fn tired_system(
    mut commands: Commands,
    query: Query<(Entity, &Energy, &NpcIndex, &Home),
                 (Without<GoingToRest>, Without<Resting>)>,
) {
    for (entity, energy, npc_idx, home) in query.iter() {
        if energy.0 < ENERGY_HUNGRY {
            // Low energy - go rest
            commands.entity(entity)
                .remove::<OnDuty>()
                .remove::<Working>()
                .insert(GoingToRest);

            // Set target to home position (push to GPU queue)
            if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
                queue.push(SetTargetMsg {
                    npc_index: npc_idx.0,
                    x: home.0.x,
                    y: home.0.y,
                });
            }
        }
    }
}

/// Resume patrol when energy recovered (anyone with PatrolRoute + Resting).
fn resume_patrol_system(
    mut commands: Commands,
    query: Query<(Entity, &PatrolRoute, &Energy, &NpcIndex), With<Resting>>,
) {
    for (entity, patrol, energy, npc_idx) in query.iter() {
        if energy.0 >= ENERGY_RESTED {
            // Rested enough - go patrol
            commands.entity(entity)
                .remove::<Resting>()
                .insert(Patrolling);

            // Get current patrol post and set target
            if let Some(pos) = patrol.posts.get(patrol.current) {
                if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
                    queue.push(SetTargetMsg {
                        npc_index: npc_idx.0,
                        x: pos.x,
                        y: pos.y,
                    });
                }
            }
        }
    }
}

/// Resume work when energy recovered (anyone with WorkPosition + Resting).
fn resume_work_system(
    mut commands: Commands,
    query: Query<(Entity, &WorkPosition, &Energy, &NpcIndex), With<Resting>>,
) {
    for (entity, work_pos, energy, npc_idx) in query.iter() {
        if energy.0 >= ENERGY_RESTED {
            // Rested enough - go to work
            commands.entity(entity)
                .remove::<Resting>()
                .insert(GoingToWork);

            // Set target to work position
            if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
                queue.push(SetTargetMsg {
                    npc_index: npc_idx.0,
                    x: work_pos.0.x,
                    y: work_pos.0.y,
                });
            }
        }
    }
}

/// Patrol system: count ticks at post and move to next (anyone with PatrolRoute + OnDuty).
fn patrol_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut PatrolRoute, &mut OnDuty, &NpcIndex)>,
) {
    for (entity, mut patrol, mut on_duty, npc_idx) in query.iter_mut() {
        on_duty.ticks_waiting += 1;

        if on_duty.ticks_waiting >= GUARD_PATROL_WAIT {
            // Time to move to next post
            if !patrol.posts.is_empty() {
                patrol.current = (patrol.current + 1) % patrol.posts.len();
            }

            commands.entity(entity)
                .remove::<OnDuty>()
                .insert(Patrolling);

            // Set target to next patrol post
            if let Some(pos) = patrol.posts.get(patrol.current) {
                if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
                    queue.push(SetTargetMsg {
                        npc_index: npc_idx.0,
                        x: pos.x,
                        y: pos.y,
                    });
                }
            }
        }
    }
}

/// Drain arrival queue and convert to Bevy messages.
fn drain_arrival_queue(mut messages: MessageWriter<ArrivalMsg>) {
    if let Ok(mut queue) = ARRIVAL_QUEUE.lock() {
        for msg in queue.drain(..) {
            messages.write(msg);
        }
    }
}

/// Handle arrivals: transition states based on what the NPC was doing.
/// - Patrolling → OnDuty (arrived at patrol post)
/// - GoingToRest → Resting (arrived at home)
/// - GoingToWork → Working (arrived at work position)
fn handle_arrival_system(
    mut commands: Commands,
    mut events: MessageReader<ArrivalMsg>,
    patrolling_query: Query<(Entity, &NpcIndex), With<Patrolling>>,
    going_to_rest_query: Query<(Entity, &NpcIndex), With<GoingToRest>>,
    going_to_work_query: Query<(Entity, &NpcIndex), With<GoingToWork>>,
) {
    for event in events.read() {
        // Check if a patrolling NPC arrived at post
        for (entity, npc_idx) in patrolling_query.iter() {
            if npc_idx.0 == event.npc_index {
                commands.entity(entity)
                    .remove::<Patrolling>()
                    .insert(OnDuty { ticks_waiting: 0 });
                break;
            }
        }

        // Check if an NPC going to rest arrived at home
        for (entity, npc_idx) in going_to_rest_query.iter() {
            if npc_idx.0 == event.npc_index {
                commands.entity(entity)
                    .remove::<GoingToRest>()
                    .insert(Resting);
                break;
            }
        }

        // Check if an NPC going to work arrived
        for (entity, npc_idx) in going_to_work_query.iter() {
            if npc_idx.0 == event.npc_index {
                commands.entity(entity)
                    .remove::<GoingToWork>()
                    .insert(Working);
                break;
            }
        }
    }
}

// ============================================================================
// BEVY APP - Initializes ECS world and systems
// ============================================================================

/// Despawn all Bevy entities when RESET_BEVY flag is set.
fn reset_bevy_system(
    mut commands: Commands,
    query: Query<Entity, With<NpcIndex>>,
    mut count: ResMut<NpcCount>,
    mut gpu_data: ResMut<GpuData>,
) {
    let should_reset = RESET_BEVY.lock().map(|mut f| {
        let val = *f;
        *f = false;  // Clear flag
        val
    }).unwrap_or(false);

    if should_reset {
        // Despawn all NPC entities
        for entity in query.iter() {
            commands.entity(entity).despawn();
        }
        count.0 = 0;
        gpu_data.npc_count = 0;
        gpu_data.dirty = false;
    }
}

/// Build the Bevy application. Called once at startup by godot-bevy.
#[bevy_app]
fn build_app(app: &mut bevy::prelude::App) {
    app.add_message::<SpawnNpcMsg>()
       .add_message::<SetTargetMsg>()
       .add_message::<SpawnGuardMsg>()
       .add_message::<SpawnFarmerMsg>()
       .add_message::<ArrivalMsg>()
       .init_resource::<NpcCount>()
       .init_resource::<GpuData>()
       .init_resource::<WorldData>()
       .init_resource::<BedOccupancy>()
       .init_resource::<FarmOccupancy>()
       // Systems run in order: reset -> drain queues -> spawn -> arrivals -> behaviors
       .add_systems(bevy::prelude::Update, (
           reset_bevy_system,
           drain_spawn_queue,
           drain_target_queue,
           drain_guard_queue,
           drain_farmer_queue,
           drain_arrival_queue,
           spawn_npc_system,
           spawn_guard_system,
           spawn_farmer_system,
           apply_targets_system,
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
///
/// Usage from GDScript:
/// ```gdscript
/// var ecs = ClassDB.instantiate("EcsNpcManager")
/// add_child(ecs)
/// ecs.spawn_npc(100, 200, 0)  # Spawn farmer at (100, 200)
/// ecs.set_target(0, 400, 300)  # Move NPC 0 to (400, 300)
/// ```
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

    /// Called when node enters the scene tree. Initializes GPU and rendering.
    fn ready(&mut self) {
        self.gpu = GpuCompute::new();
        if self.gpu.is_none() {
            godot_error!("[EcsNpcManager] Failed to initialize GPU compute");
            return;
        }

        self.setup_multimesh(MAX_NPC_COUNT as i32);
    }

    /// Called every frame. Dispatches GPU compute and updates rendering.
    fn process(&mut self, delta: f64) {
        let gpu = match self.gpu.as_mut() {
            Some(g) => g,
            None => return,
        };

        let npc_count = GPU_NPC_COUNT.lock().map(|c| *c).unwrap_or(0);

        // Drain GPU target queue and upload to GPU buffers
        // (This handles target updates from Bevy systems like guard_on_duty_system)
        if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
            for msg in queue.drain(..) {
                if msg.npc_index < npc_count {
                    // Update target position on GPU
                    let target_bytes: Vec<u8> = [msg.x, msg.y].iter()
                        .flat_map(|f| f.to_le_bytes()).collect();
                    let target_packed = PackedByteArray::from(target_bytes.as_slice());
                    gpu.rd.buffer_update(
                        gpu.target_buffer,
                        (msg.npc_index * 8) as u32,
                        8,
                        &target_packed
                    );

                    // Reset arrival flag so NPC moves toward new target
                    let arrival_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
                    let arrival_packed = PackedByteArray::from(arrival_bytes.as_slice());
                    gpu.rd.buffer_update(
                        gpu.arrival_buffer,
                        (msg.npc_index * 4) as u32,
                        4,
                        &arrival_packed
                    );

                    // Reset backoff counter
                    let backoff_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
                    let backoff_packed = PackedByteArray::from(backoff_bytes.as_slice());
                    gpu.rd.buffer_update(
                        gpu.backoff_buffer,
                        (msg.npc_index * 4) as u32,
                        4,
                        &backoff_packed
                    );

                    // Reset prev_arrivals so we can detect arrival at new target
                    self.prev_arrivals[msg.npc_index] = false;
                }
            }
        }

        if npc_count > 0 {
            // Run physics on GPU
            gpu.dispatch(npc_count, delta as f32);

            // Read positions from GPU (for next frame's grid AND current multimesh)
            // This is the ONLY position readback - 80KB vs previous 80KB + 480KB multimesh
            gpu.read_positions_from_gpu(npc_count);

            // Detect arrivals: read GPU arrival buffer, queue messages for new arrivals
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

                        // Queue message only on transition from not-arrived to arrived
                        if arrived && !self.prev_arrivals[i] {
                            queue.push(ArrivalMsg { npc_index: i });
                        }
                        self.prev_arrivals[i] = arrived;
                    }
                }
            }

            // Build MultiMesh from cached positions + colors (no GPU readback!)
            // This eliminates the 480KB multimesh buffer readback
            if self.multimesh_rid.is_valid() {
                let packed = gpu.build_multimesh_from_cache(&gpu.colors.clone(), npc_count, MAX_NPC_COUNT);
                RenderingServer::singleton().multimesh_set_buffer(self.multimesh_rid, &packed);
            }
        }
    }
}

#[godot_api]
impl EcsNpcManager {
    /// Set up MultiMesh for batch rendering all NPCs with a single draw call.
    fn setup_multimesh(&mut self, max_count: i32) {
        let mut rs = RenderingServer::singleton();

        self.multimesh_rid = rs.multimesh_create();

        // Create a 16x16 quad mesh for each NPC sprite
        let mut mesh = QuadMesh::new_gd();
        mesh.set_size(Vector2::new(16.0, 16.0));
        let mesh_rid = mesh.get_rid();
        rs.multimesh_set_mesh(self.multimesh_rid, mesh_rid);

        // Allocate instance data: 2D transforms + colors
        rs.multimesh_allocate_data_ex(
            self.multimesh_rid,
            max_count,
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
        ).color_format(true).done();

        // Initialize all instances to identity transform (will be updated by GPU)
        let count = max_count as usize;
        let mut init_buffer = vec![0.0f32; count * FLOATS_PER_INSTANCE];
        for i in 0..count {
            let base = i * FLOATS_PER_INSTANCE;
            init_buffer[base + 0] = 1.0;   // Scale X
            init_buffer[base + 5] = 1.0;   // Scale Y
            init_buffer[base + 11] = 1.0;  // Alpha (visible)
        }
        let packed = PackedFloat32Array::from(init_buffer.as_slice());
        rs.multimesh_set_buffer(self.multimesh_rid, &packed);

        // Attach MultiMesh to scene tree for rendering
        self.canvas_item = rs.canvas_item_create();
        let parent_canvas = self.base().get_canvas_item();
        rs.canvas_item_set_parent(self.canvas_item, parent_canvas);
        rs.canvas_item_add_multimesh(self.canvas_item, self.multimesh_rid);

        self.mesh = Some(mesh);
    }

    /// Spawn a new NPC at position (x, y) with the given job type.
    ///
    /// Job types: 0 = Farmer (green), 1 = Guard (blue), 2 = Raider (red)
    #[func]
    fn spawn_npc(&mut self, x: f32, y: f32, job: i32) {
        // Increment GPU_NPC_COUNT immediately (before Bevy processes spawn)
        // This ensures the compute shader sees the new NPC this frame
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

        // Queue spawn message for Bevy ECS
        if let Ok(mut queue) = SPAWN_QUEUE.lock() {
            queue.push(SpawnNpcMsg { x, y, job });
        }

        // Upload initial data to GPU buffers immediately
        // (Can't wait for Bevy - GPU compute runs this frame)
        if let Some(gpu) = self.gpu.as_mut() {
            let (r, g, b, a) = Job::from_i32(job).color();

            // Position buffer: vec2 at index * 8 bytes
            let pos_bytes: Vec<u8> = [x, y].iter().flat_map(|f| f.to_le_bytes()).collect();
            let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &pos_packed);
            // Target = position initially (no movement)
            gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &pos_packed);

            // Color buffer: vec4 at index * 16 bytes
            let color_bytes: Vec<u8> = [r, g, b, a].iter().flat_map(|f| f.to_le_bytes()).collect();
            let color_packed = PackedByteArray::from(color_bytes.as_slice());
            gpu.rd.buffer_update(gpu.color_buffer, (idx * 16) as u32, 16, &color_packed);

            // Cache colors for CPU-side multimesh building
            gpu.colors[idx * 4] = r;
            gpu.colors[idx * 4 + 1] = g;
            gpu.colors[idx * 4 + 2] = b;
            gpu.colors[idx * 4 + 3] = a;

            // Speed buffer: float at index * 4 bytes
            let speed_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let speed_packed = PackedByteArray::from(speed_bytes.as_slice());
            gpu.rd.buffer_update(gpu.speed_buffer, (idx * 4) as u32, 4, &speed_packed);

            // Initialize arrival flag to 0 (not arrived)
            let arrival_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let arrival_packed = PackedByteArray::from(arrival_bytes.as_slice());
            gpu.rd.buffer_update(gpu.arrival_buffer, (idx * 4) as u32, 4, &arrival_packed);

            // Initialize backoff counter to 0 (not blocked)
            let backoff_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let backoff_packed = PackedByteArray::from(backoff_bytes.as_slice());
            gpu.rd.buffer_update(gpu.backoff_buffer, (idx * 4) as u32, 4, &backoff_packed);
        }
    }

    /// Spawn a guard NPC with home position and town assignment.
    /// Guards start in Patrolling state and will cycle through patrol posts.
    #[func]
    fn spawn_guard(&mut self, x: f32, y: f32, town_idx: i32, home_x: f32, home_y: f32) {
        // Increment GPU_NPC_COUNT immediately
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

        // Queue guard spawn message for Bevy ECS
        if let Ok(mut queue) = GUARD_QUEUE.lock() {
            queue.push(SpawnGuardMsg {
                x, y,
                town_idx: town_idx as u32,
                home_x, home_y,
                starting_post: 0,
            });
        }

        // Upload initial data to GPU buffers immediately
        if let Some(gpu) = self.gpu.as_mut() {
            let (r, g, b, a) = Job::Guard.color();

            let pos_bytes: Vec<u8> = [x, y].iter().flat_map(|f| f.to_le_bytes()).collect();
            let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &pos_packed);
            gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &pos_packed);

            let color_bytes: Vec<u8> = [r, g, b, a].iter().flat_map(|f| f.to_le_bytes()).collect();
            let color_packed = PackedByteArray::from(color_bytes.as_slice());
            gpu.rd.buffer_update(gpu.color_buffer, (idx * 16) as u32, 16, &color_packed);

            // Cache colors for CPU-side multimesh building
            gpu.colors[idx * 4] = r;
            gpu.colors[idx * 4 + 1] = g;
            gpu.colors[idx * 4 + 2] = b;
            gpu.colors[idx * 4 + 3] = a;

            let speed_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let speed_packed = PackedByteArray::from(speed_bytes.as_slice());
            gpu.rd.buffer_update(gpu.speed_buffer, (idx * 4) as u32, 4, &speed_packed);

            let arrival_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let arrival_packed = PackedByteArray::from(arrival_bytes.as_slice());
            gpu.rd.buffer_update(gpu.arrival_buffer, (idx * 4) as u32, 4, &arrival_packed);

            let backoff_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let backoff_packed = PackedByteArray::from(backoff_bytes.as_slice());
            gpu.rd.buffer_update(gpu.backoff_buffer, (idx * 4) as u32, 4, &backoff_packed);
        }

        // Set initial target to first patrol post
        if let Ok(world) = WORLD_DATA.lock() {
            if let Some(post) = world.guard_posts.iter()
                .find(|p| p.town_idx == town_idx as u32 && p.patrol_order == 0)
            {
                self.set_target(idx as i32, post.position.x, post.position.y);
            }
        }
    }

    /// Spawn a guard at a specific patrol post.
    /// Guard starts OnDuty at that post and will patrol clockwise from there.
    #[func]
    fn spawn_guard_at_post(&mut self, x: f32, y: f32, town_idx: i32, home_x: f32, home_y: f32, starting_post: i32) {
        // Increment GPU_NPC_COUNT immediately
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

        // Queue guard spawn message for Bevy ECS
        if let Ok(mut queue) = GUARD_QUEUE.lock() {
            queue.push(SpawnGuardMsg {
                x, y,
                town_idx: town_idx as u32,
                home_x, home_y,
                starting_post: starting_post as u32,
            });
        }

        // Upload initial data to GPU buffers immediately
        if let Some(gpu) = self.gpu.as_mut() {
            let (r, g, b, a) = Job::Guard.color();

            let pos_bytes: Vec<u8> = [x, y].iter().flat_map(|f| f.to_le_bytes()).collect();
            let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &pos_packed);
            // Target = position (guard starts at post, doesn't need to move)
            gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &pos_packed);

            let color_bytes: Vec<u8> = [r, g, b, a].iter().flat_map(|f| f.to_le_bytes()).collect();
            let color_packed = PackedByteArray::from(color_bytes.as_slice());
            gpu.rd.buffer_update(gpu.color_buffer, (idx * 16) as u32, 16, &color_packed);

            // Cache colors for CPU-side multimesh building
            gpu.colors[idx * 4] = r;
            gpu.colors[idx * 4 + 1] = g;
            gpu.colors[idx * 4 + 2] = b;
            gpu.colors[idx * 4 + 3] = a;

            let speed_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let speed_packed = PackedByteArray::from(speed_bytes.as_slice());
            gpu.rd.buffer_update(gpu.speed_buffer, (idx * 4) as u32, 4, &speed_packed);

            // Start as arrived (at post)
            let arrival_bytes: Vec<u8> = 1i32.to_le_bytes().to_vec();
            let arrival_packed = PackedByteArray::from(arrival_bytes.as_slice());
            gpu.rd.buffer_update(gpu.arrival_buffer, (idx * 4) as u32, 4, &arrival_packed);

            let backoff_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let backoff_packed = PackedByteArray::from(backoff_bytes.as_slice());
            gpu.rd.buffer_update(gpu.backoff_buffer, (idx * 4) as u32, 4, &backoff_packed);
        }

        // Mark as already arrived so we don't get a spurious arrival event
        self.prev_arrivals[idx] = true;
    }

    /// Spawn a farmer NPC with home and work positions.
    /// Farmer starts in GoingToWork state and will cycle between work and rest.
    #[func]
    fn spawn_farmer(&mut self, x: f32, y: f32, town_idx: i32, home_x: f32, home_y: f32, work_x: f32, work_y: f32) {
        // Increment GPU_NPC_COUNT immediately
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

        // Queue farmer spawn message for Bevy ECS
        if let Ok(mut queue) = FARMER_QUEUE.lock() {
            queue.push(SpawnFarmerMsg {
                x, y,
                town_idx: town_idx as u32,
                home_x, home_y,
                work_x, work_y,
            });
        }

        // Upload initial data to GPU buffers immediately
        if let Some(gpu) = self.gpu.as_mut() {
            let (r, g, b, a) = Job::Farmer.color();

            let pos_bytes: Vec<u8> = [x, y].iter().flat_map(|f| f.to_le_bytes()).collect();
            let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
            gpu.rd.buffer_update(gpu.position_buffer, (idx * 8) as u32, 8, &pos_packed);
            // Target = work position (farmer starts walking to work)
            let work_bytes: Vec<u8> = [work_x, work_y].iter().flat_map(|f| f.to_le_bytes()).collect();
            let work_packed = PackedByteArray::from(work_bytes.as_slice());
            gpu.rd.buffer_update(gpu.target_buffer, (idx * 8) as u32, 8, &work_packed);

            let color_bytes: Vec<u8> = [r, g, b, a].iter().flat_map(|f| f.to_le_bytes()).collect();
            let color_packed = PackedByteArray::from(color_bytes.as_slice());
            gpu.rd.buffer_update(gpu.color_buffer, (idx * 16) as u32, 16, &color_packed);

            // Cache colors for CPU-side multimesh building
            gpu.colors[idx * 4] = r;
            gpu.colors[idx * 4 + 1] = g;
            gpu.colors[idx * 4 + 2] = b;
            gpu.colors[idx * 4 + 3] = a;

            let speed_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let speed_packed = PackedByteArray::from(speed_bytes.as_slice());
            gpu.rd.buffer_update(gpu.speed_buffer, (idx * 4) as u32, 4, &speed_packed);

            // Not arrived yet - walking to work
            let arrival_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let arrival_packed = PackedByteArray::from(arrival_bytes.as_slice());
            gpu.rd.buffer_update(gpu.arrival_buffer, (idx * 4) as u32, 4, &arrival_packed);

            let backoff_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let backoff_packed = PackedByteArray::from(backoff_bytes.as_slice());
            gpu.rd.buffer_update(gpu.backoff_buffer, (idx * 4) as u32, 4, &backoff_packed);
        }
    }

    /// Set the movement target for an NPC.
    /// The NPC will move toward (x, y) until arrival or blocked.
    #[func]
    fn set_target(&mut self, npc_index: i32, x: f32, y: f32) {
        // Queue target message for Bevy ECS (adds HasTarget component)
        if let Ok(mut queue) = TARGET_QUEUE.lock() {
            queue.push(SetTargetMsg { npc_index: npc_index as usize, x, y });
        }

        // Upload target to GPU immediately (can't wait for Bevy)
        if let Some(gpu) = self.gpu.as_mut() {
            let idx = npc_index as usize;
            let npc_count = GPU_NPC_COUNT.lock().map(|c| *c).unwrap_or(0);
            if idx < npc_count {
                // Update target position
                let target_bytes: Vec<u8> = [x, y].iter()
                    .flat_map(|f| f.to_le_bytes()).collect();
                let target_packed = PackedByteArray::from(target_bytes.as_slice());
                gpu.rd.buffer_update(
                    gpu.target_buffer,
                    (idx * 8) as u32,
                    target_packed.len() as u32,
                    &target_packed
                );

                // Reset arrival flag so NPC moves toward new target
                let arrival_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
                let arrival_packed = PackedByteArray::from(arrival_bytes.as_slice());
                gpu.rd.buffer_update(
                    gpu.arrival_buffer,
                    (idx * 4) as u32,
                    4,
                    &arrival_packed
                );

                // Reset backoff counter for new target (fresh start)
                let backoff_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
                let backoff_packed = PackedByteArray::from(backoff_bytes.as_slice());
                gpu.rd.buffer_update(
                    gpu.backoff_buffer,
                    (idx * 4) as u32,
                    4,
                    &backoff_packed
                );
            }
        }
    }

    /// Get the current number of active NPCs.
    #[func]
    fn get_npc_count(&self) -> i32 {
        GPU_NPC_COUNT.lock().map(|c| *c as i32).unwrap_or(0)
    }

    /// Get build info for verifying correct DLL is loaded.
    #[func]
    fn get_build_info(&self) -> GString {
        // These are set at compile time by build.rs
        let timestamp = option_env!("BUILD_TIMESTAMP").unwrap_or("unknown");
        let commit = option_env!("BUILD_COMMIT").unwrap_or("unknown");
        GString::from(&format!("BUILD: {} ({})", timestamp, commit))
    }

    /// Get debug statistics from GPU buffers.
    /// Returns: { npc_count, arrived_count, avg_backoff, max_backoff }
    ///
    /// - arrived_count: NPCs that reached target or gave up
    /// - max_backoff: Highest backoff counter (indicates most blocked NPC)
    #[func]
    fn get_debug_stats(&mut self) -> Dictionary {
        let mut dict = Dictionary::new();
        if let Some(gpu) = &mut self.gpu {
            let npc_count = GPU_NPC_COUNT.lock().map(|c| *c).unwrap_or(0);

            // Read arrival buffer from GPU
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

            // Read backoff buffer from GPU
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

            // Debug: count how many cells have NPCs in them
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

    /// Get the current position of an NPC.
    /// Reads from cached positions (updated each frame during grid build).
    #[func]
    fn get_npc_position(&self, npc_index: i32) -> Vector2 {
        if let Some(gpu) = &self.gpu {
            let idx = npc_index as usize;
            let npc_count = GPU_NPC_COUNT.lock().map(|c| *c).unwrap_or(0);
            if idx < npc_count {
                // Read from cached positions (updated each frame during grid build)
                let x = gpu.positions.get(idx * 2).copied().unwrap_or(0.0);
                let y = gpu.positions.get(idx * 2 + 1).copied().unwrap_or(0.0);
                return Vector2::new(x, y);
            }
        }
        Vector2::ZERO
    }

    /// Reset the NPC system (clears all NPCs).
    /// Call this when reloading a scene to start fresh.
    #[func]
    fn reset(&mut self) {
        // Reset NPC count
        if let Ok(mut count) = GPU_NPC_COUNT.lock() {
            *count = 0;
        }

        // Clear queues
        if let Ok(mut queue) = SPAWN_QUEUE.lock() {
            queue.clear();
        }
        if let Ok(mut queue) = TARGET_QUEUE.lock() {
            queue.clear();
        }
        if let Ok(mut queue) = GUARD_QUEUE.lock() {
            queue.clear();
        }
        if let Ok(mut queue) = FARMER_QUEUE.lock() {
            queue.clear();
        }
        if let Ok(mut queue) = ARRIVAL_QUEUE.lock() {
            queue.clear();
        }
        if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
            queue.clear();
        }

        // Clear world data
        if let Ok(mut world) = WORLD_DATA.lock() {
            world.towns.clear();
            world.farms.clear();
            world.beds.clear();
            world.guard_posts.clear();
        }
        if let Ok(mut beds) = BED_OCCUPANCY.lock() {
            beds.occupant_npc.clear();
        }
        if let Ok(mut farms) = FARM_OCCUPANCY.lock() {
            farms.occupant_count.clear();
        }

        // Clear prev_arrivals so guards can detect arrival on new tests
        self.prev_arrivals.fill(false);

        // Signal Bevy to despawn all entities on next frame
        if let Ok(mut flag) = RESET_BEVY.lock() {
            *flag = true;
        }

    }

    // ========================================================================
    // WORLD DATA API - Initialize world layout from GDScript
    // ========================================================================

    /// Initialize world data storage. Call once after world generation.
    #[func]
    fn init_world(&mut self, town_count: i32) {
        if let Ok(mut world) = WORLD_DATA.lock() {
            world.towns = Vec::with_capacity(town_count as usize);
            world.farms = Vec::new();
            world.beds = Vec::new();
            world.guard_posts = Vec::new();
        }
        if let Ok(mut beds) = BED_OCCUPANCY.lock() {
            beds.occupant_npc = Vec::new();
        }
        if let Ok(mut farms) = FARM_OCCUPANCY.lock() {
            farms.occupant_count = Vec::new();
        }
    }

    /// Add a town to the world.
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

    /// Add a farm to the world.
    #[func]
    fn add_farm(&mut self, x: f32, y: f32, town_idx: i32) {
        if let Ok(mut world) = WORLD_DATA.lock() {
            world.farms.push(Farm {
                position: Vector2::new(x, y),
                town_idx: town_idx as u32,
            });
        }
        if let Ok(mut farms) = FARM_OCCUPANCY.lock() {
            farms.occupant_count.push(0);
        }
    }

    /// Add a bed to the world.
    #[func]
    fn add_bed(&mut self, x: f32, y: f32, town_idx: i32) {
        if let Ok(mut world) = WORLD_DATA.lock() {
            world.beds.push(Bed {
                position: Vector2::new(x, y),
                town_idx: town_idx as u32,
            });
        }
        if let Ok(mut beds) = BED_OCCUPANCY.lock() {
            beds.occupant_npc.push(-1); // -1 = free
        }
    }

    /// Add a guard post to the world.
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
    // WORLD QUERY API - Query world data from GDScript
    // ========================================================================

    /// Get the center position of a town.
    #[func]
    fn get_town_center(&self, town_idx: i32) -> Vector2 {
        if let Ok(world) = WORLD_DATA.lock() {
            if let Some(town) = world.towns.get(town_idx as usize) {
                return town.center;
            }
        }
        Vector2::ZERO
    }

    /// Get the camp position for a town's raiders.
    #[func]
    fn get_camp_position(&self, town_idx: i32) -> Vector2 {
        if let Ok(world) = WORLD_DATA.lock() {
            if let Some(town) = world.towns.get(town_idx as usize) {
                return town.camp_position;
            }
        }
        Vector2::ZERO
    }

    /// Get the position of a guard post by town and patrol order.
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

    /// Get the nearest free bed in a town. Returns bed index or -1 if none free.
    #[func]
    fn get_nearest_free_bed(&self, town_idx: i32, x: f32, y: f32) -> i32 {
        let pos = Vector2::new(x, y);
        let mut best_idx: i32 = -1;
        let mut best_dist = f32::MAX;

        if let (Ok(world), Ok(beds)) = (WORLD_DATA.lock(), BED_OCCUPANCY.lock()) {
            for (i, bed) in world.beds.iter().enumerate() {
                if bed.town_idx != town_idx as u32 {
                    continue;
                }
                if i >= beds.occupant_npc.len() {
                    continue;
                }
                if beds.occupant_npc[i] >= 0 {
                    continue; // occupied
                }
                let dist = pos.distance_to(bed.position);
                if dist < best_dist {
                    best_dist = dist;
                    best_idx = i as i32;
                }
            }
        }
        best_idx
    }

    /// Get the nearest free farm in a town. Returns farm index or -1 if none free.
    /// A farm is "free" if occupant_count < 1 (1 farmer per farm max).
    #[func]
    fn get_nearest_free_farm(&self, town_idx: i32, x: f32, y: f32) -> i32 {
        let pos = Vector2::new(x, y);
        let mut best_idx: i32 = -1;
        let mut best_dist = f32::MAX;

        if let (Ok(world), Ok(farms)) = (WORLD_DATA.lock(), FARM_OCCUPANCY.lock()) {
            for (i, farm) in world.farms.iter().enumerate() {
                if farm.town_idx != town_idx as u32 {
                    continue;
                }
                if i >= farms.occupant_count.len() {
                    continue;
                }
                if farms.occupant_count[i] >= 1 {
                    continue; // occupied
                }
                let dist = pos.distance_to(farm.position);
                if dist < best_dist {
                    best_dist = dist;
                    best_idx = i as i32;
                }
            }
        }
        best_idx
    }

    /// Reserve a bed for an NPC. Returns true if successful.
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

    /// Release a bed reservation.
    #[func]
    fn release_bed(&mut self, bed_idx: i32) {
        if let Ok(mut beds) = BED_OCCUPANCY.lock() {
            let idx = bed_idx as usize;
            if idx < beds.occupant_npc.len() {
                beds.occupant_npc[idx] = -1;
            }
        }
    }

    /// Reserve a farm slot. Returns true if successful.
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

    /// Release a farm reservation.
    #[func]
    fn release_farm(&mut self, farm_idx: i32) {
        if let Ok(mut farms) = FARM_OCCUPANCY.lock() {
            let idx = farm_idx as usize;
            if idx < farms.occupant_count.len() && farms.occupant_count[idx] > 0 {
                farms.occupant_count[idx] -= 1;
            }
        }
    }

    /// Get world data stats for debugging.
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

    /// Get guard debug info for testing.
    /// Returns: { arrived_flags, prev_arrivals, arrival_queue_len }
    #[func]
    fn get_guard_debug(&mut self) -> Dictionary {
        let mut dict = Dictionary::new();
        if let Some(gpu) = &mut self.gpu {
            let npc_count = GPU_NPC_COUNT.lock().map(|c| *c).unwrap_or(0);

            // Read arrival buffer
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

            // Count prev_arrivals that are true
            let prev_true = self.prev_arrivals.iter().take(npc_count).filter(|&&x| x).count();

            // Get queue length
            let queue_len = ARRIVAL_QUEUE.lock().map(|q| q.len()).unwrap_or(0);

            dict.set("arrived_flags", arrived_flags as i32);
            dict.set("prev_arrivals_true", prev_true as i32);
            dict.set("arrival_queue_len", queue_len as i32);
        }
        dict
    }
}
