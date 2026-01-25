// Phase 3 POC: GPU compute shader writes directly to MultiMesh buffer
// Target: 10,000+ NPCs with zero-copy rendering

use godot::prelude::*;
use godot::classes::{
    INode2D, QuadMesh, Label,
    RenderingServer, RenderingDevice, RdUniform,
    rendering_device::UniformType,
};

// ============================================================
// CONSTANTS
// ============================================================

const NPC_COUNT: usize = 10000;  // GPU compute for CPU headroom
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

// MultiMesh buffer: 12 floats per instance (Transform2D + Color)
const FLOATS_PER_INSTANCE: usize = 12;

// Push constants size (must match shader + 16-byte alignment)
const PUSH_CONSTANTS_SIZE: usize = 48;  // 10 x 4 bytes + 8 padding for alignment

// ============================================================
// SPATIAL GRID (CPU - uploaded to GPU each frame)
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
    multimesh_buffer_rid: Rid,
}

impl GpuCompute {
    fn new(_multimesh_rid: Rid) -> Option<Self> {
        // Create LOCAL rendering device (can use submit/sync)
        let rs = RenderingServer::singleton();
        let mut rd = rs.create_local_rendering_device()?;

        // Load shader
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

        // Create GPU buffers (no MultiMesh buffer - local device can't access it)
        let position_buffer = rd.storage_buffer_create((NPC_COUNT * 8) as u32); // vec2 per NPC
        let velocity_buffer = rd.storage_buffer_create((NPC_COUNT * 8) as u32);
        let grid_counts_buffer = rd.storage_buffer_create((GRID_CELLS * 4) as u32);
        let grid_data_buffer = rd.storage_buffer_create((GRID_CELLS * MAX_PER_CELL * 4) as u32);
        // Output buffer for positions (read back to CPU)
        let output_buffer = rd.storage_buffer_create((NPC_COUNT * 8) as u32);

        // Create uniform set (without MultiMesh buffer)
        let uniform_set = Self::create_uniform_set_local(
            &mut rd, shader,
            position_buffer, velocity_buffer,
            grid_counts_buffer, grid_data_buffer,
            output_buffer,
        );
        godot_print!("[GPU] Local uniform set created: {}", uniform_set.is_some());
        let uniform_set = uniform_set?;

        Some(Self {
            rd,
            shader,
            pipeline,
            position_buffer,
            velocity_buffer,
            grid_counts_buffer,
            grid_data_buffer,
            uniform_set,
            multimesh_buffer_rid: output_buffer, // Reuse field for output buffer
        })
    }

    fn create_uniform_set_local(
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
            (4, output_buffer),  // Output buffer instead of MultiMesh
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
        // Push constants
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
        // Bytes 40-47 are padding (already zero)
        let push_constants = PackedByteArray::from(push_data.as_slice());

        // Dispatch compute shader
        let compute_list = self.rd.compute_list_begin();
        self.rd.compute_list_bind_compute_pipeline(compute_list, self.pipeline);
        self.rd.compute_list_bind_uniform_set(compute_list, self.uniform_set, 0);
        self.rd.compute_list_set_push_constant(compute_list, &push_constants, PUSH_CONSTANTS_SIZE as u32);

        let workgroups = ((NPC_COUNT + 63) / 64) as u32;
        self.rd.compute_list_dispatch(compute_list, workgroups, 1, 1);
        self.rd.compute_list_end();

        // Submit and sync (works on local device)
        self.rd.submit();
        self.rd.sync();
    }

    fn read_output_buffer(&mut self) -> Vec<f32> {
        // Read MultiMesh-format buffer (12 floats per instance)
        let bytes = self.rd.buffer_get_data(self.multimesh_buffer_rid);
        let mut data = vec![0.0f32; NPC_COUNT * FLOATS_PER_INSTANCE];
        for i in 0..data.len() {
            let offset = i * 4;
            if offset + 4 <= bytes.len() {
                data[i] = f32::from_le_bytes([
                    bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]
                ]);
            }
        }
        data
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
        if self.uniform_set.is_valid() {
            self.rd.free_rid(self.uniform_set);
        }
        if self.position_buffer.is_valid() {
            self.rd.free_rid(self.position_buffer);
        }
        if self.velocity_buffer.is_valid() {
            self.rd.free_rid(self.velocity_buffer);
        }
        if self.grid_counts_buffer.is_valid() {
            self.rd.free_rid(self.grid_counts_buffer);
        }
        if self.grid_data_buffer.is_valid() {
            self.rd.free_rid(self.grid_data_buffer);
        }
        if self.pipeline.is_valid() {
            self.rd.free_rid(self.pipeline);
        }
        if self.shader.is_valid() {
            self.rd.free_rid(self.shader);
        }
    }
}

// ============================================================
// GODOT NODE
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

        // Create MultiMesh via RenderingServer with use_indirect for GPU compute
        let multimesh_rid = rs.multimesh_create();

        // Create mesh
        let mut mesh = QuadMesh::new_gd();
        mesh.set_size(Vector2::new(16.0, 16.0));
        let mesh_rid = mesh.get_rid();
        rs.multimesh_set_mesh(multimesh_rid, mesh_rid);

        // Allocate with use_indirect=true for GPU compute writes
        rs.multimesh_allocate_data_ex(
            multimesh_rid,
            NPC_COUNT as i32,
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
        ).color_format(true).use_indirect(true).done();

        // Initialize buffer with identity transforms
        let mut init_buffer = vec![0.0f32; NPC_COUNT * FLOATS_PER_INSTANCE];
        for i in 0..NPC_COUNT {
            let base = i * FLOATS_PER_INSTANCE;
            init_buffer[base + 0] = 1.0;  // scale x
            init_buffer[base + 5] = 1.0;  // scale y
            init_buffer[base + 11] = 1.0; // alpha
        }
        let packed = PackedFloat32Array::from(init_buffer.as_slice());
        rs.multimesh_set_buffer(multimesh_rid, &packed);

        // Create canvas item for 2D rendering
        let canvas_item = rs.canvas_item_create();
        let parent_canvas = self.base().get_canvas_item();
        rs.canvas_item_set_parent(canvas_item, parent_canvas);
        rs.canvas_item_add_multimesh(canvas_item, multimesh_rid);

        // Keep mesh alive
        std::mem::forget(mesh);

        // Generate initial positions and velocities
        let mut positions = vec![0.0f32; NPC_COUNT * 2];
        let mut velocities = vec![0.0f32; NPC_COUNT * 2];
        let mut seed: u64 = 12345;

        for i in 0..NPC_COUNT {
            // Position
            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            positions[i * 2] = (seed as f32 / u64::MAX as f32) * WORLD_SIZE;
            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            positions[i * 2 + 1] = (seed as f32 / u64::MAX as f32) * WORLD_SIZE;

            // Velocity
            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            let speed = 20.0 + (seed as f32 / u64::MAX as f32) * 60.0;
            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            let angle = (seed as f32 / u64::MAX as f32) * std::f32::consts::TAU;
            velocities[i * 2] = angle.cos() * speed;
            velocities[i * 2 + 1] = angle.sin() * speed;
        }

        // Store multimesh RID for updates
        self.multimesh_rid = multimesh_rid;

        // Try to initialize GPU compute (local device)
        if let Some(mut gpu) = GpuCompute::new(multimesh_rid) {
            gpu.upload_initial_data(&positions, &velocities);
            self.gpu = Some(gpu);
            godot_print!("[GPU Compute POC] Local device initialized with {} NPCs", NPC_COUNT);
        } else {
            godot_print!("[GPU Compute POC] GPU compute unavailable");
            self.use_gpu = false;
        }
    }

    fn process(&mut self, delta: f64) {
        if !self.use_gpu {
            return;
        }

        if let Some(ref mut gpu) = self.gpu {
            // Read positions from GPU
            let positions = gpu.read_positions();

            // Build spatial grid on CPU
            self.grid.clear();
            for i in 0..NPC_COUNT {
                let x = positions[i * 2];
                let y = positions[i * 2 + 1];
                self.grid.insert(x, y, i as i32);
            }

            // Upload grid to GPU and dispatch compute
            gpu.upload_grid(&self.grid);
            gpu.dispatch();

            // Build MultiMesh buffer on CPU (fast)
            let mut buffer = vec![0.0f32; NPC_COUNT * FLOATS_PER_INSTANCE];
            for i in 0..NPC_COUNT {
                let base = i * FLOATS_PER_INSTANCE;
                buffer[base + 0] = 1.0;  // scale x
                buffer[base + 3] = positions[i * 2];      // pos.x
                buffer[base + 5] = 1.0;  // scale y
                buffer[base + 7] = positions[i * 2 + 1];  // pos.y
                buffer[base + 8] = 0.2;  // r
                buffer[base + 9] = 0.8;  // g
                buffer[base + 10] = 0.2; // b
                buffer[base + 11] = 1.0; // a
            }

            let packed = PackedFloat32Array::from(buffer.as_slice());
            RenderingServer::singleton().multimesh_set_buffer(self.multimesh_rid, &packed);
        }

        // FPS counter
        self.frame_count += 1;
        self.fps_timer += delta;
        if self.fps_timer >= 1.0 {
            let fps = self.frame_count as f64 / self.fps_timer;
            if let Some(mut label) = self.base().try_get_node_as::<Label>("../UI/FPSLabel") {
                label.set_text(&format!("FPS: {:.0} ({} NPCs, GPU)", fps, NPC_COUNT));
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

// ============================================================
// GDEXTENSION ENTRY
// ============================================================

struct EndlessEcs;

#[gdextension]
unsafe impl ExtensionLibrary for EndlessEcs {}
