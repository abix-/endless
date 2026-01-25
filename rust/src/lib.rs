// Endless ECS - GPU compute + Bevy state machine
// GPU separation: 10,000 NPCs @ 140fps
// State machine: Bevy ECS via godot-bevy

use bevy::prelude::*;
use godot::prelude::*;
use godot::classes::{
    INode2D, QuadMesh, Label,
    RenderingServer, RenderingDevice, RdUniform,
    rendering_device::UniformType,
};
use godot_bevy::prelude::*;

// ============================================================
// ECS COMPONENTS
// ============================================================

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(i32)]
pub enum NpcState {
    #[default]
    Idle = 0,
    Resting = 1,
    Fighting = 2,
    Fleeing = 3,
    Walking = 4,
    Farming = 5,
    OffDuty = 6,
    OnDuty = 7,
    Patrolling = 8,
    Raiding = 9,
    Returning = 10,
    Wandering = 11,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
#[repr(i32)]
pub enum Job {
    Farmer = 0,
    Guard = 1,
    Raider = 2,
}

#[derive(Component, Default)]
pub struct Energy(pub f32);

#[derive(Component)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

#[derive(Component, Default)]
pub struct NpcPosition(pub Vec2);

#[derive(Component, Default)]
pub struct Target(pub Vec2);

#[derive(Component)]
pub struct TownIndex(pub i32);

#[derive(Component)]
pub struct NpcIndex(pub usize);  // Maps back to GDScript array index

#[derive(Component)]
pub struct Alive;

#[derive(Component)]
pub struct Recovering;

// ============================================================
// SHARED STATE (GDScript ↔ Bevy bridge)
// ============================================================

use std::sync::Mutex;

/// NPC data pushed from GDScript each frame
#[derive(Default)]
pub struct NpcInput {
    pub count: usize,
    pub jobs: Vec<i32>,
    pub states: Vec<i32>,
    pub energies: Vec<f32>,
    pub healths: Vec<f32>,
    pub is_daytime: bool,
    pub energy_hungry: f32,
}

/// State changes computed by Bevy, pulled by GDScript
#[derive(Default)]
pub struct NpcOutput {
    pub changes: Vec<(usize, i32)>,  // (npc_index, new_state)
}

static NPC_INPUT: Mutex<NpcInput> = Mutex::new(NpcInput {
    count: 0,
    jobs: Vec::new(),
    states: Vec::new(),
    energies: Vec::new(),
    healths: Vec::new(),
    is_daytime: true,
    energy_hungry: 50.0,
});

static NPC_OUTPUT: Mutex<NpcOutput> = Mutex::new(NpcOutput {
    changes: Vec::new(),
});

// ============================================================
// ECS RESOURCES
// ============================================================

#[derive(Resource, Default)]
pub struct StateChanges {
    pub changes: Vec<(usize, i32)>,  // (npc_index, new_state)
}

#[derive(Resource)]
pub struct GameConfig {
    pub energy_hungry: f32,
    pub is_daytime: bool,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            energy_hungry: 50.0,
            is_daytime: true,
        }
    }
}

// ============================================================
// ECS SYSTEMS
// ============================================================

/// Reads NPC_INPUT and updates GameConfig + spawns decisions
fn sync_input_system(
    mut config: ResMut<GameConfig>,
    mut changes: ResMut<StateChanges>,
) {
    // Clear previous changes
    changes.changes.clear();

    // Read input from GDScript
    let input = NPC_INPUT.lock().unwrap();
    config.energy_hungry = input.energy_hungry;
    config.is_daytime = input.is_daytime;

    // Run guard decisions inline (no entity overhead for now)
    for i in 0..input.count {
        let job = input.jobs.get(i).copied().unwrap_or(0);
        if job != Job::Guard as i32 {
            continue;
        }

        let state = input.states.get(i).copied().unwrap_or(0);
        let energy = input.energies.get(i).copied().unwrap_or(100.0);

        // Priority 1: Low energy → go rest
        if energy < config.energy_hungry {
            if state != NpcState::Resting as i32 && state != NpcState::Walking as i32 {
                changes.changes.push((i, NpcState::Walking as i32));
            }
            continue;
        }

        // Priority 2: Patrol
        if state != NpcState::OnDuty as i32 && state != NpcState::Patrolling as i32 {
            changes.changes.push((i, NpcState::Patrolling as i32));
        }
    }
}

/// Copies StateChanges to NPC_OUTPUT for GDScript to read
fn sync_output_system(
    changes: Res<StateChanges>,
) {
    let mut output = NPC_OUTPUT.lock().unwrap();
    output.changes.clear();
    output.changes.extend(changes.changes.iter().cloned());
}

// ============================================================
// BEVY APP ENTRY POINT
// ============================================================

#[bevy_app]
fn build_app(app: &mut App) {
    app.init_resource::<StateChanges>()
       .init_resource::<GameConfig>()
       .add_systems(Update, (sync_input_system, sync_output_system).chain());

    godot_print!("[godot-bevy] State machine systems registered");
}

// ============================================================
// GDSCRIPT BRIDGE
// ============================================================

#[derive(GodotClass)]
#[class(base=Node)]
struct NpcStateMachine {
    base: Base<Node>,
}

#[godot_api]
impl INode for NpcStateMachine {
    fn init(base: Base<Node>) -> Self {
        Self { base }
    }

    fn ready(&mut self) {
        godot_print!("[NpcStateMachine] Bridge ready");
    }
}

#[godot_api]
impl NpcStateMachine {
    /// Push NPC data from GDScript to Bevy
    #[func]
    fn push_npc_data(
        &self,
        jobs: PackedInt32Array,
        states: PackedInt32Array,
        energies: PackedFloat32Array,
        healths: PackedFloat32Array,
        is_daytime: bool,
        energy_hungry: f32,
    ) {
        let mut input = NPC_INPUT.lock().unwrap();
        input.count = jobs.len();
        input.jobs = jobs.to_vec();
        input.states = states.to_vec();
        input.energies = energies.to_vec();
        input.healths = healths.to_vec();
        input.is_daytime = is_daytime;
        input.energy_hungry = energy_hungry;
    }

    /// Pull state changes from Bevy to GDScript
    /// Returns Dictionary { npc_index: new_state, ... }
    #[func]
    fn pull_state_changes(&self) -> Dictionary {
        let output = NPC_OUTPUT.lock().unwrap();
        let mut dict = Dictionary::new();
        for (idx, state) in output.changes.iter() {
            dict.set(*idx as i64, *state);
        }
        dict
    }
}

// ============================================================
// CONSTANTS
// ============================================================

const NPC_COUNT: usize = 10000;
const WORLD_SIZE: f32 = 8000.0;
const CELL_SIZE: f32 = 64.0;
const GRID_WIDTH: usize = 128;
const GRID_HEIGHT: usize = 128;
const MAX_PER_CELL: usize = 48;
const GRID_CELLS: usize = GRID_WIDTH * GRID_HEIGHT;
const SEPARATION_RADIUS: f32 = 16.0;
const SEPARATION_STRENGTH: f32 = 50.0;
const DELTA: f32 = 1.0 / 60.0;
const DAMPING: f32 = 0.4;
const FLOATS_PER_INSTANCE: usize = 12;
const PUSH_CONSTANTS_SIZE: usize = 48;

// ============================================================
// SPATIAL GRID
// ============================================================

struct SpatialGrid {
    counts: Vec<i32>,
    data: Vec<i32>,
}

impl SpatialGrid {
    fn new() -> Self {
        Self {
            counts: vec![0i32; GRID_CELLS],
            data: vec![0i32; GRID_CELLS * MAX_PER_CELL],
        }
    }

    fn clear(&mut self) {
        self.counts.fill(0);
    }

    fn insert(&mut self, x: f32, y: f32, npc_idx: i32) {
        let cx = (x / CELL_SIZE).clamp(0.0, (GRID_WIDTH - 1) as f32) as usize;
        let cy = (y / CELL_SIZE).clamp(0.0, (GRID_HEIGHT - 1) as f32) as usize;
        let cell_idx = cy * GRID_WIDTH + cx;
        if cell_idx >= GRID_CELLS { return; }
        let count = self.counts[cell_idx] as usize;
        if count >= MAX_PER_CELL { return; }
        self.data[cell_idx * MAX_PER_CELL + count] = npc_idx;
        self.counts[cell_idx] = (count + 1) as i32;
    }
}

// ============================================================
// GPU COMPUTE MANAGER
// ============================================================

struct GpuCompute {
    rd: Gd<RenderingDevice>,
    shader: Rid,
    pipeline: Rid,
    position_buffer: Rid,
    velocity_buffer: Rid,
    grid_counts_buffer: Rid,
    grid_data_buffer: Rid,
    uniform_set: Rid,
    output_buffer: Rid,
}

impl GpuCompute {
    fn new(_multimesh_rid: Rid) -> Option<Self> {
        let rs = RenderingServer::singleton();
        let mut rd = rs.create_local_rendering_device()?;

        let shader_file = godot::classes::ResourceLoader::singleton()
            .load("res://shaders/npc_compute.glsl")?;
        let shader_file = shader_file.cast::<godot::classes::RdShaderFile>();
        let spirv = shader_file.get_spirv()?;

        let shader = rd.shader_create_from_spirv(&spirv);
        if !shader.is_valid() {
            godot_error!("Failed to create shader from SPIRV");
            return None;
        }

        let pipeline = rd.compute_pipeline_create(shader);
        if !pipeline.is_valid() {
            godot_error!("Failed to create compute pipeline");
            return None;
        }

        let position_buffer = rd.storage_buffer_create((NPC_COUNT * 8) as u32);
        let velocity_buffer = rd.storage_buffer_create((NPC_COUNT * 8) as u32);
        let grid_counts_buffer = rd.storage_buffer_create((GRID_CELLS * 4) as u32);
        let grid_data_buffer = rd.storage_buffer_create((GRID_CELLS * MAX_PER_CELL * 4) as u32);
        let output_buffer = rd.storage_buffer_create((NPC_COUNT * 8) as u32);

        let uniform_set = Self::create_uniform_set(
            &mut rd, shader,
            position_buffer, velocity_buffer,
            grid_counts_buffer, grid_data_buffer,
            output_buffer,
        )?;

        Some(Self {
            rd,
            shader,
            pipeline,
            position_buffer,
            velocity_buffer,
            grid_counts_buffer,
            grid_data_buffer,
            uniform_set,
            output_buffer,
        })
    }

    fn create_uniform_set(
        rd: &mut Gd<RenderingDevice>,
        shader: Rid,
        position_buffer: Rid,
        velocity_buffer: Rid,
        grid_counts_buffer: Rid,
        grid_data_buffer: Rid,
        output_buffer: Rid,
    ) -> Option<Rid> {
        let mut uniforms = Array::new();

        let buffers = [
            (0, position_buffer),
            (1, velocity_buffer),
            (2, grid_counts_buffer),
            (3, grid_data_buffer),
            (4, output_buffer),
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

    fn upload_initial_data(&mut self, positions: &[f32], velocities: &[f32]) {
        let pos_bytes = PackedByteArray::from(
            positions.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>().as_slice()
        );
        let vel_bytes = PackedByteArray::from(
            velocities.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>().as_slice()
        );

        self.rd.buffer_update(self.position_buffer, 0, pos_bytes.len() as u32, &pos_bytes);
        self.rd.buffer_update(self.velocity_buffer, 0, vel_bytes.len() as u32, &vel_bytes);
    }

    fn upload_grid(&mut self, grid: &SpatialGrid) {
        let counts_bytes = PackedByteArray::from(
            grid.counts.iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<u8>>().as_slice()
        );
        let data_bytes = PackedByteArray::from(
            grid.data.iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<u8>>().as_slice()
        );

        self.rd.buffer_update(self.grid_counts_buffer, 0, counts_bytes.len() as u32, &counts_bytes);
        self.rd.buffer_update(self.grid_data_buffer, 0, data_bytes.len() as u32, &data_bytes);
    }

    fn dispatch(&mut self) {
        let mut push_data = vec![0u8; PUSH_CONSTANTS_SIZE];
        push_data[0..4].copy_from_slice(&(NPC_COUNT as u32).to_le_bytes());
        push_data[4..8].copy_from_slice(&SEPARATION_RADIUS.to_le_bytes());
        push_data[8..12].copy_from_slice(&SEPARATION_STRENGTH.to_le_bytes());
        push_data[12..16].copy_from_slice(&DELTA.to_le_bytes());
        push_data[16..20].copy_from_slice(&(GRID_WIDTH as u32).to_le_bytes());
        push_data[20..24].copy_from_slice(&(GRID_HEIGHT as u32).to_le_bytes());
        push_data[24..28].copy_from_slice(&CELL_SIZE.to_le_bytes());
        push_data[28..32].copy_from_slice(&(MAX_PER_CELL as u32).to_le_bytes());
        push_data[32..36].copy_from_slice(&WORLD_SIZE.to_le_bytes());
        push_data[36..40].copy_from_slice(&DAMPING.to_le_bytes());
        let push_constants = PackedByteArray::from(push_data.as_slice());

        let compute_list = self.rd.compute_list_begin();
        self.rd.compute_list_bind_compute_pipeline(compute_list, self.pipeline);
        self.rd.compute_list_bind_uniform_set(compute_list, self.uniform_set, 0);
        self.rd.compute_list_set_push_constant(compute_list, &push_constants, PUSH_CONSTANTS_SIZE as u32);

        let workgroups = ((NPC_COUNT + 63) / 64) as u32;
        self.rd.compute_list_dispatch(compute_list, workgroups, 1, 1);
        self.rd.compute_list_end();

        self.rd.submit();
        self.rd.sync();
    }

    fn read_positions(&mut self) -> Vec<f32> {
        let bytes = self.rd.buffer_get_data(self.position_buffer);
        let mut positions = vec![0.0f32; NPC_COUNT * 2];
        for i in 0..positions.len() {
            let offset = i * 4;
            if offset + 4 <= bytes.len() {
                positions[i] = f32::from_le_bytes([
                    bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]
                ]);
            }
        }
        positions
    }

    fn cleanup(&mut self) {
        if self.uniform_set.is_valid() { self.rd.free_rid(self.uniform_set); }
        if self.position_buffer.is_valid() { self.rd.free_rid(self.position_buffer); }
        if self.velocity_buffer.is_valid() { self.rd.free_rid(self.velocity_buffer); }
        if self.grid_counts_buffer.is_valid() { self.rd.free_rid(self.grid_counts_buffer); }
        if self.grid_data_buffer.is_valid() { self.rd.free_rid(self.grid_data_buffer); }
        if self.output_buffer.is_valid() { self.rd.free_rid(self.output_buffer); }
        if self.pipeline.is_valid() { self.rd.free_rid(self.pipeline); }
        if self.shader.is_valid() { self.rd.free_rid(self.shader); }
    }
}

// ============================================================
// GPU BENCHMARK NODE
// ============================================================

#[derive(GodotClass)]
#[class(base=Node2D)]
struct NpcBenchmark {
    base: Base<Node2D>,
    gpu: Option<GpuCompute>,
    grid: SpatialGrid,
    multimesh_rid: Rid,
    frame_count: u64,
    fps_timer: f64,
    use_gpu: bool,
}

#[godot_api]
impl INode2D for NpcBenchmark {
    fn init(base: Base<Node2D>) -> Self {
        Self {
            base,
            gpu: None,
            grid: SpatialGrid::new(),
            multimesh_rid: Rid::Invalid,
            frame_count: 0,
            fps_timer: 0.0,
            use_gpu: true,
        }
    }

    fn ready(&mut self) {
        let mut rs = RenderingServer::singleton();

        let multimesh_rid = rs.multimesh_create();

        let mut mesh = QuadMesh::new_gd();
        mesh.set_size(Vector2::new(16.0, 16.0));
        let mesh_rid = mesh.get_rid();
        rs.multimesh_set_mesh(multimesh_rid, mesh_rid);

        rs.multimesh_allocate_data_ex(
            multimesh_rid,
            NPC_COUNT as i32,
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
        ).color_format(true).use_indirect(true).done();

        let mut init_buffer = vec![0.0f32; NPC_COUNT * FLOATS_PER_INSTANCE];
        for i in 0..NPC_COUNT {
            let base = i * FLOATS_PER_INSTANCE;
            init_buffer[base + 0] = 1.0;
            init_buffer[base + 5] = 1.0;
            init_buffer[base + 11] = 1.0;
        }
        let packed = PackedFloat32Array::from(init_buffer.as_slice());
        rs.multimesh_set_buffer(multimesh_rid, &packed);

        let canvas_item = rs.canvas_item_create();
        let parent_canvas = self.base().get_canvas_item();
        rs.canvas_item_set_parent(canvas_item, parent_canvas);
        rs.canvas_item_add_multimesh(canvas_item, multimesh_rid);

        std::mem::forget(mesh);

        let mut positions = vec![0.0f32; NPC_COUNT * 2];
        let mut velocities = vec![0.0f32; NPC_COUNT * 2];
        let mut seed: u64 = 12345;

        for i in 0..NPC_COUNT {
            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            positions[i * 2] = (seed as f32 / u64::MAX as f32) * WORLD_SIZE;
            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            positions[i * 2 + 1] = (seed as f32 / u64::MAX as f32) * WORLD_SIZE;

            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            let speed = 20.0 + (seed as f32 / u64::MAX as f32) * 60.0;
            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            let angle = (seed as f32 / u64::MAX as f32) * std::f32::consts::TAU;
            velocities[i * 2] = angle.cos() * speed;
            velocities[i * 2 + 1] = angle.sin() * speed;
        }

        self.multimesh_rid = multimesh_rid;

        if let Some(mut gpu) = GpuCompute::new(multimesh_rid) {
            gpu.upload_initial_data(&positions, &velocities);
            self.gpu = Some(gpu);
            godot_print!("[GPU Compute] Initialized with {} NPCs (godot-bevy)", NPC_COUNT);
        } else {
            godot_print!("[GPU Compute] GPU unavailable");
            self.use_gpu = false;
        }
    }

    fn process(&mut self, delta: f64) {
        if !self.use_gpu { return; }

        if let Some(ref mut gpu) = self.gpu {
            let positions = gpu.read_positions();

            self.grid.clear();
            for i in 0..NPC_COUNT {
                self.grid.insert(positions[i * 2], positions[i * 2 + 1], i as i32);
            }

            gpu.upload_grid(&self.grid);
            gpu.dispatch();

            let mut buffer = vec![0.0f32; NPC_COUNT * FLOATS_PER_INSTANCE];
            for i in 0..NPC_COUNT {
                let base = i * FLOATS_PER_INSTANCE;
                buffer[base + 0] = 1.0;
                buffer[base + 3] = positions[i * 2];
                buffer[base + 5] = 1.0;
                buffer[base + 7] = positions[i * 2 + 1];
                buffer[base + 8] = 0.2;
                buffer[base + 9] = 0.8;
                buffer[base + 10] = 0.2;
                buffer[base + 11] = 1.0;
            }

            let packed = PackedFloat32Array::from(buffer.as_slice());
            RenderingServer::singleton().multimesh_set_buffer(self.multimesh_rid, &packed);
        }

        self.frame_count += 1;
        self.fps_timer += delta;
        if self.fps_timer >= 1.0 {
            let fps = self.frame_count as f64 / self.fps_timer;
            if let Some(mut label) = self.base().try_get_node_as::<Label>("../UI/FPSLabel") {
                label.set_text(&format!("FPS: {:.0} ({} NPCs, godot-bevy)", fps, NPC_COUNT));
            }
            self.frame_count = 0;
            self.fps_timer = 0.0;
        }
    }

    fn exit_tree(&mut self) {
        if let Some(ref mut gpu) = self.gpu {
            gpu.cleanup();
        }
    }
}
