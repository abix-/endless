//! GPU Compute - Runs physics on thousands of NPCs in parallel
//!
//! Why GPU compute instead of CPU?
//! - GPU has thousands of cores vs CPU's ~8-16
//! - Separation physics is "embarrassingly parallel" - each NPC is independent
//! - Memory bandwidth: GPU can read/write 10K positions in microseconds

use godot::prelude::*;
use godot::classes::{RenderingServer, RenderingDevice, RdUniform};
use godot::classes::rendering_device::UniformType;
use godot::classes::ResourceLoader;

use crate::constants::*;

// ============================================================================
// SPATIAL GRID
// ============================================================================

/// Spatial partitioning grid for efficient neighbor queries.
pub struct SpatialGrid {
    /// Number of NPCs in each cell: counts[cell_idx] = n
    pub counts: Vec<i32>,
    /// NPC indices in each cell: data[cell_idx * MAX_PER_CELL + n] = npc_index
    pub data: Vec<i32>,
}

impl SpatialGrid {
    pub fn new() -> Self {
        Self {
            counts: vec![0i32; GRID_CELLS],
            data: vec![0i32; GRID_CELLS * MAX_PER_CELL],
        }
    }

    /// Reset all cell counts to zero
    pub fn clear(&mut self) {
        self.counts.fill(0);
    }

    /// Add an NPC to the grid cell containing position (x, y)
    pub fn insert(&mut self, x: f32, y: f32, npc_idx: i32) {
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

// ============================================================================
// GPU COMPUTE
// ============================================================================

/// GPU compute context - owns RenderingDevice and all GPU buffers.
pub struct GpuCompute {
    /// Godot's GPU abstraction
    pub rd: Gd<RenderingDevice>,

    /// Compiled compute shader
    #[allow(dead_code)]
    shader: Rid,

    /// Compute pipeline
    pipeline: Rid,

    // === GPU Buffers ===
    pub position_buffer: Rid,
    pub target_buffer: Rid,
    pub color_buffer: Rid,
    pub speed_buffer: Rid,
    pub grid_counts_buffer: Rid,
    pub grid_data_buffer: Rid,
    #[allow(dead_code)]
    pub multimesh_buffer: Rid,
    pub arrival_buffer: Rid,
    pub backoff_buffer: Rid,

    /// Uniform set
    uniform_set: Rid,

    /// CPU-side spatial grid
    pub grid: SpatialGrid,

    /// Cached positions
    pub positions: Vec<f32>,

    /// Cached colors
    pub colors: Vec<f32>,
}

impl GpuCompute {
    /// Initialize GPU compute
    pub fn new() -> Option<Self> {
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

        // Allocate GPU buffers
        let position_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 8) as u32);
        let target_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 8) as u32);
        let color_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 16) as u32);
        let speed_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 4) as u32);
        let grid_counts_buffer = rd.storage_buffer_create((GRID_CELLS * 4) as u32);
        let grid_data_buffer = rd.storage_buffer_create((GRID_CELLS * MAX_PER_CELL * 4) as u32);
        let multimesh_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * FLOATS_PER_INSTANCE * 4) as u32);
        let arrival_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 4) as u32);
        let backoff_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 4) as u32);

        let uniform_set = Self::create_uniform_set(
            &mut rd, shader,
            position_buffer, target_buffer, color_buffer, speed_buffer,
            grid_counts_buffer, grid_data_buffer, multimesh_buffer, arrival_buffer,
            backoff_buffer,
        )?;

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

        let buffers = [
            (0, position_buffer),
            (1, target_buffer),
            (2, color_buffer),
            (3, speed_buffer),
            (4, grid_counts_buffer),
            (5, grid_data_buffer),
            (6, multimesh_buffer),
            (7, arrival_buffer),
            (8, backoff_buffer),
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

    /// Rebuild spatial grid and upload to GPU
    pub fn build_and_upload_grid(&mut self, npc_count: usize) {
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

    /// Dispatch the compute shader
    pub fn dispatch(&mut self, npc_count: usize, delta: f32) {
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
        push_data[36..40].copy_from_slice(&0.0f32.to_le_bytes());
        push_data[40..44].copy_from_slice(&0.0f32.to_le_bytes());
        push_data[44..48].copy_from_slice(&0.0f32.to_le_bytes());
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

    /// Build MultiMesh buffer from cached data
    pub fn build_multimesh_from_cache(&self, colors: &[f32], npc_count: usize, max_count: usize) -> PackedFloat32Array {
        let float_count = max_count * FLOATS_PER_INSTANCE;
        let mut floats = vec![0.0f32; float_count];

        for i in 0..max_count {
            let base = i * FLOATS_PER_INSTANCE;
            floats[base + 0] = 1.0;
            floats[base + 5] = 1.0;
            floats[base + 3] = -9999.0;
            floats[base + 7] = -9999.0;
        }

        for i in 0..npc_count {
            let base = i * FLOATS_PER_INSTANCE;
            floats[base + 0] = 1.0;
            floats[base + 1] = 0.0;
            floats[base + 2] = 0.0;
            floats[base + 3] = self.positions[i * 2];
            floats[base + 4] = 0.0;
            floats[base + 5] = 1.0;
            floats[base + 6] = 0.0;
            floats[base + 7] = self.positions[i * 2 + 1];
            floats[base + 8] = colors[i * 4];
            floats[base + 9] = colors[i * 4 + 1];
            floats[base + 10] = colors[i * 4 + 2];
            floats[base + 11] = colors[i * 4 + 3];
        }

        PackedFloat32Array::from(floats.as_slice())
    }

    /// Read positions from GPU
    pub fn read_positions_from_gpu(&mut self, npc_count: usize) {
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
