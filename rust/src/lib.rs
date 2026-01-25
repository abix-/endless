// Endless ECS - Chunk 3: GPU Physics
// Architecture: Bevy owns logical state, EcsNpcManager owns GPU compute

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;
use godot_bevy::prelude::*;
use godot::classes::{RenderingServer, RenderingDevice, RdUniform, QuadMesh, INode2D};
use godot::classes::rendering_device::UniformType;
use godot::classes::ResourceLoader;
use std::sync::Mutex;

// ============================================================
// CONSTANTS
// ============================================================

const MAX_NPC_COUNT: usize = 10000;
const GRID_WIDTH: usize = 128;
const GRID_HEIGHT: usize = 128;
const GRID_CELLS: usize = GRID_WIDTH * GRID_HEIGHT;
const MAX_PER_CELL: usize = 48;
const CELL_SIZE: f32 = 64.0;
const SEPARATION_RADIUS: f32 = 20.0;
const SEPARATION_STRENGTH: f32 = 200.0;
const ARRIVAL_THRESHOLD: f32 = 8.0;
const FLOATS_PER_INSTANCE: usize = 12;
const PUSH_CONSTANTS_SIZE: usize = 48;  // 12 fields * 4 bytes (with alignment padding)

// ============================================================
// ECS COMPONENTS (Bevy owns logical state)
// ============================================================

#[derive(Component, Clone, Copy)]
pub struct NpcIndex(pub usize);

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Job {
    Farmer,
    Guard,
    Raider,
}

impl Job {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Job::Guard,
            2 => Job::Raider,
            _ => Job::Farmer,
        }
    }

    pub fn color(&self) -> (f32, f32, f32, f32) {
        match self {
            Job::Farmer => (0.2, 0.8, 0.2, 1.0),
            Job::Guard => (0.2, 0.4, 0.9, 1.0),
            Job::Raider => (0.9, 0.2, 0.2, 1.0),
        }
    }
}

#[derive(Component)]
pub struct HasTarget;

#[derive(Component, Clone, Copy)]
pub struct Speed(pub f32);

impl Default for Speed {
    fn default() -> Self {
        Self(100.0)
    }
}

// ============================================================
// ECS MESSAGES
// ============================================================

#[derive(Message, Clone)]
pub struct SpawnNpcMsg {
    pub x: f32,
    pub y: f32,
    pub job: i32,
}

#[derive(Message, Clone)]
pub struct SetTargetMsg {
    pub npc_index: usize,
    pub x: f32,
    pub y: f32,
}

// ============================================================
// ECS RESOURCES
// ============================================================

#[derive(Resource, Default)]
pub struct NpcCount(pub usize);

#[derive(Resource)]
pub struct GpuData {
    pub positions: Vec<f32>,
    pub targets: Vec<f32>,
    pub colors: Vec<f32>,
    pub speeds: Vec<f32>,
    pub npc_count: usize,
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

// ============================================================
// STATIC QUEUES (GDScript -> Bevy communication)
// ============================================================

static SPAWN_QUEUE: Mutex<Vec<SpawnNpcMsg>> = Mutex::new(Vec::new());
static TARGET_QUEUE: Mutex<Vec<SetTargetMsg>> = Mutex::new(Vec::new());
static GPU_NPC_COUNT: Mutex<usize> = Mutex::new(0);

// ============================================================
// SPATIAL GRID (built on CPU, uploaded to GPU)
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
        let cx = ((x / CELL_SIZE) as usize).min(GRID_WIDTH - 1);
        let cy = ((y / CELL_SIZE) as usize).min(GRID_HEIGHT - 1);
        let cell_idx = cy * GRID_WIDTH + cx;

        let count = self.counts[cell_idx] as usize;
        if count < MAX_PER_CELL {
            self.data[cell_idx * MAX_PER_CELL + count] = npc_idx;
            self.counts[cell_idx] += 1;
        }
    }
}

// ============================================================
// GPU COMPUTE (owned by EcsNpcManager, not accessible from Bevy)
// ============================================================

struct GpuCompute {
    rd: Gd<RenderingDevice>,
    #[allow(dead_code)]
    shader: Rid,
    pipeline: Rid,
    position_buffer: Rid,
    target_buffer: Rid,
    color_buffer: Rid,
    speed_buffer: Rid,
    grid_counts_buffer: Rid,
    grid_data_buffer: Rid,
    multimesh_buffer: Rid,
    arrival_buffer: Rid,
    uniform_set: Rid,
    grid: SpatialGrid,
    positions: Vec<f32>,
}

impl GpuCompute {
    fn new() -> Option<Self> {
        let rs = RenderingServer::singleton();
        let mut rd = rs.create_local_rendering_device()?;

        let shader_file = ResourceLoader::singleton()
            .load("res://shaders/npc_compute.glsl")?;
        let shader_file = shader_file.cast::<godot::classes::RdShaderFile>();
        let spirv = shader_file.get_spirv()?;

        let shader = rd.shader_create_from_spirv(&spirv);
        if !shader.is_valid() {
            godot_error!("[GPU] Failed to create shader");
            return None;
        }

        let pipeline = rd.compute_pipeline_create(shader);
        if !pipeline.is_valid() {
            godot_error!("[GPU] Failed to create pipeline");
            return None;
        }

        let position_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 8) as u32);
        let target_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 8) as u32);
        let color_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 16) as u32);
        let speed_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 4) as u32);
        let grid_counts_buffer = rd.storage_buffer_create((GRID_CELLS * 4) as u32);
        let grid_data_buffer = rd.storage_buffer_create((GRID_CELLS * MAX_PER_CELL * 4) as u32);
        let multimesh_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * FLOATS_PER_INSTANCE * 4) as u32);
        let arrival_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 4) as u32);

        let uniform_set = Self::create_uniform_set(
            &mut rd, shader,
            position_buffer, target_buffer, color_buffer, speed_buffer,
            grid_counts_buffer, grid_data_buffer, multimesh_buffer, arrival_buffer,
        )?;

        godot_print!("[GPU] Compute shader initialized");

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
            uniform_set,
            grid: SpatialGrid::new(),
            positions: vec![0.0; MAX_NPC_COUNT * 2],
        })
    }

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
    ) -> Option<Rid> {
        let mut uniforms = Array::new();

        let buffers = [
            (0, position_buffer),
            (1, target_buffer),
            (2, color_buffer),
            (3, speed_buffer),
            (4, grid_counts_buffer),
            (5, grid_data_buffer),
            (6, multimesh_buffer),
            (7, arrival_buffer),
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

    fn build_and_upload_grid(&mut self, npc_count: usize) {
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

        self.grid.clear();
        for i in 0..npc_count {
            let x = self.positions[i * 2];
            let y = self.positions[i * 2 + 1];
            self.grid.insert(x, y, i as i32);
        }

        let counts_bytes: Vec<u8> = self.grid.counts.iter()
            .flat_map(|i| i.to_le_bytes()).collect();
        let counts_packed = PackedByteArray::from(counts_bytes.as_slice());
        self.rd.buffer_update(self.grid_counts_buffer, 0, counts_packed.len() as u32, &counts_packed);

        let data_bytes: Vec<u8> = self.grid.data.iter()
            .flat_map(|i| i.to_le_bytes()).collect();
        let data_packed = PackedByteArray::from(data_bytes.as_slice());
        self.rd.buffer_update(self.grid_data_buffer, 0, data_packed.len() as u32, &data_packed);
    }

    fn dispatch(&mut self, npc_count: usize, delta: f32) {
        if npc_count == 0 {
            return;
        }

        self.build_and_upload_grid(npc_count);

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
        push_data[36..40].copy_from_slice(&0.0f32.to_le_bytes());  // _pad1
        push_data[40..44].copy_from_slice(&0.0f32.to_le_bytes());  // _pad2
        push_data[44..48].copy_from_slice(&0.0f32.to_le_bytes());  // _pad3
        let push_constants = PackedByteArray::from(push_data.as_slice());

        let compute_list = self.rd.compute_list_begin();
        self.rd.compute_list_bind_compute_pipeline(compute_list, self.pipeline);
        self.rd.compute_list_bind_uniform_set(compute_list, self.uniform_set, 0);
        self.rd.compute_list_set_push_constant(compute_list, &push_constants, PUSH_CONSTANTS_SIZE as u32);

        let workgroups = ((npc_count + 63) / 64) as u32;
        self.rd.compute_list_dispatch(compute_list, workgroups, 1, 1);
        self.rd.compute_list_end();

        self.rd.submit();
        self.rd.sync();
    }

    fn read_multimesh_buffer(&mut self, npc_count: usize, max_count: usize) -> PackedFloat32Array {
        let bytes = self.rd.buffer_get_data(self.multimesh_buffer);
        let byte_slice = bytes.as_slice();

        let float_count = max_count * FLOATS_PER_INSTANCE;
        let mut floats = vec![0.0f32; float_count];

        for i in 0..max_count {
            let base = i * FLOATS_PER_INSTANCE;
            floats[base + 0] = 1.0;
            floats[base + 5] = 1.0;
            floats[base + 3] = -9999.0;
            floats[base + 7] = -9999.0;
        }

        let active_floats = npc_count * FLOATS_PER_INSTANCE;
        for i in 0..active_floats {
            let offset = i * 4;
            if offset + 4 <= byte_slice.len() {
                floats[i] = f32::from_le_bytes([
                    byte_slice[offset],
                    byte_slice[offset + 1],
                    byte_slice[offset + 2],
                    byte_slice[offset + 3],
                ]);
            }
        }
        PackedFloat32Array::from(floats.as_slice())
    }
}

// ============================================================
// ECS SYSTEMS
// ============================================================

fn drain_spawn_queue(mut messages: MessageWriter<SpawnNpcMsg>) {
    if let Ok(mut queue) = SPAWN_QUEUE.lock() {
        for msg in queue.drain(..) {
            messages.write(msg);
        }
    }
}

fn drain_target_queue(mut messages: MessageWriter<SetTargetMsg>) {
    if let Ok(mut queue) = TARGET_QUEUE.lock() {
        for msg in queue.drain(..) {
            messages.write(msg);
        }
    }
}

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

        commands.spawn((
            NpcIndex(idx),
            job,
            Speed::default(),
        ));
        count.0 += 1;
    }
}

fn apply_targets_system(
    mut commands: Commands,
    mut events: MessageReader<SetTargetMsg>,
    mut gpu_data: ResMut<GpuData>,
    query: Query<(Entity, &NpcIndex), Without<HasTarget>>,
) {
    for event in events.read() {
        if event.npc_index < gpu_data.npc_count {
            gpu_data.targets[event.npc_index * 2] = event.x;
            gpu_data.targets[event.npc_index * 2 + 1] = event.y;
            gpu_data.dirty = true;

            for (entity, npc_idx) in query.iter() {
                if npc_idx.0 == event.npc_index {
                    commands.entity(entity).insert(HasTarget);
                    break;
                }
            }
        }
    }
}

// ============================================================
// BEVY APP
// ============================================================

#[bevy_app]
fn build_app(app: &mut bevy::prelude::App) {
    app.add_message::<SpawnNpcMsg>()
       .add_message::<SetTargetMsg>()
       .init_resource::<NpcCount>()
       .init_resource::<GpuData>()
       .add_systems(bevy::prelude::Update, (
           drain_spawn_queue,
           drain_target_queue,
           spawn_npc_system,
           apply_targets_system,
       ).chain());

    godot_print!("[ECS] Bevy app initialized");
}

// ============================================================
// GODOT CLASS
// ============================================================

#[derive(GodotClass)]
#[class(base=Node2D)]
pub struct EcsNpcManager {
    base: Base<Node2D>,
    gpu: Option<GpuCompute>,
    multimesh_rid: Rid,
    canvas_item: Rid,
    #[allow(dead_code)]
    mesh: Option<Gd<QuadMesh>>,
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
        }
    }

    fn ready(&mut self) {
        self.gpu = GpuCompute::new();
        if self.gpu.is_none() {
            godot_error!("[EcsNpcManager] Failed to initialize GPU compute");
            return;
        }

        self.setup_multimesh(MAX_NPC_COUNT as i32);
        godot_print!("[EcsNpcManager] Ready - Bevy ECS + GPU compute");
    }

    fn process(&mut self, delta: f64) {
        let gpu = match self.gpu.as_mut() {
            Some(g) => g,
            None => return,
        };

        let npc_count = GPU_NPC_COUNT.lock().map(|c| *c).unwrap_or(0);

        if npc_count > 0 {
            gpu.dispatch(npc_count, delta as f32);

            if self.multimesh_rid.is_valid() {
                let packed = gpu.read_multimesh_buffer(npc_count, MAX_NPC_COUNT);
                RenderingServer::singleton().multimesh_set_buffer(self.multimesh_rid, &packed);
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
        godot_print!("[EcsNpcManager] MultiMesh allocated: {}", max_count);
    }

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

            let speed_bytes: Vec<u8> = 100.0f32.to_le_bytes().to_vec();
            let speed_packed = PackedByteArray::from(speed_bytes.as_slice());
            gpu.rd.buffer_update(gpu.speed_buffer, (idx * 4) as u32, 4, &speed_packed);

            // Initialize arrival flag to 0 (not arrived)
            let arrival_bytes: Vec<u8> = 0i32.to_le_bytes().to_vec();
            let arrival_packed = PackedByteArray::from(arrival_bytes.as_slice());
            gpu.rd.buffer_update(gpu.arrival_buffer, (idx * 4) as u32, 4, &arrival_packed);
        }
    }

    #[func]
    fn set_target(&mut self, npc_index: i32, x: f32, y: f32) {
        if let Ok(mut queue) = TARGET_QUEUE.lock() {
            queue.push(SetTargetMsg { npc_index: npc_index as usize, x, y });
        }

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
            }
        }
    }

    #[func]
    fn get_npc_count(&self) -> i32 {
        GPU_NPC_COUNT.lock().map(|c| *c as i32).unwrap_or(0)
    }

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

        godot_print!("[EcsNpcManager] Reset - NPC count cleared");
    }
}
