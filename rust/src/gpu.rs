//! GPU Compute - Runs physics on thousands of NPCs in parallel.
//! See docs/gpu-compute.md for architecture.

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

    // === Combat Buffers ===
    pub faction_buffer: Rid,
    pub health_buffer: Rid,
    pub combat_target_buffer: Rid,

    // === Sprite Buffer ===
    pub sprite_frame_buffer: Rid,

    // === Projectile Buffers ===
    pub proj_position_buffer: Rid,
    pub proj_velocity_buffer: Rid,
    pub proj_damage_buffer: Rid,
    pub proj_faction_buffer: Rid,
    pub proj_shooter_buffer: Rid,
    pub proj_lifetime_buffer: Rid,
    pub proj_active_buffer: Rid,
    pub proj_hit_buffer: Rid,      // ivec2: (hit_npc_idx, processed)

    /// Uniform set for NPC shader
    uniform_set: Rid,

    /// Projectile shader and pipeline
    #[allow(dead_code)]
    proj_shader: Rid,
    pub proj_pipeline: Rid,
    proj_uniform_set: Rid,

    /// CPU-side spatial grid
    pub grid: SpatialGrid,

    /// Cached positions
    pub positions: Vec<f32>,

    /// Cached targets (movement destination)
    pub targets: Vec<f32>,

    /// Cached colors
    pub colors: Vec<f32>,

    /// Cached factions (0=Villager, 1=Raider)
    pub factions: Vec<i32>,

    /// Cached healths
    pub healths: Vec<f32>,

    /// Combat targets read from GPU (-1 = no target)
    pub combat_targets: Vec<i32>,

    /// Sprite frames (column, row) per NPC - set at spawn
    pub sprite_frames: Vec<f32>,

    /// Healing aura flags per NPC - true if in healing zone
    pub healing_flags: Vec<bool>,

    /// Carried item per NPC (0 = none, 1 = food, etc.)
    pub carried_items: Vec<u8>,

    // === Projectile CPU Caches ===
    pub proj_positions: Vec<f32>,
    pub proj_velocities: Vec<f32>,
    pub proj_damages: Vec<f32>,
    pub proj_factions: Vec<i32>,
    pub proj_active: Vec<i32>,
    pub proj_count: usize,
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

        // Combat buffers
        let faction_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 4) as u32);
        let health_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 4) as u32);
        let combat_target_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 4) as u32);

        // Sprite buffer (vec2 per NPC = 8 bytes)
        let sprite_frame_buffer = rd.storage_buffer_create((MAX_NPC_COUNT * 8) as u32);

        // Projectile buffers
        let proj_position_buffer = rd.storage_buffer_create((MAX_PROJECTILES * 8) as u32);
        let proj_velocity_buffer = rd.storage_buffer_create((MAX_PROJECTILES * 8) as u32);
        let proj_damage_buffer = rd.storage_buffer_create((MAX_PROJECTILES * 4) as u32);
        let proj_faction_buffer = rd.storage_buffer_create((MAX_PROJECTILES * 4) as u32);
        let proj_shooter_buffer = rd.storage_buffer_create((MAX_PROJECTILES * 4) as u32);
        let proj_lifetime_buffer = rd.storage_buffer_create((MAX_PROJECTILES * 4) as u32);
        let proj_active_buffer = rd.storage_buffer_create((MAX_PROJECTILES * 4) as u32);
        let proj_hit_buffer = rd.storage_buffer_create((MAX_PROJECTILES * 8) as u32); // ivec2

        // Initialize hit buffer to all -1 (no hits) - GPU zeros by default which
        // the shader misinterprets as "hit NPC index 0"
        let hit_init: Vec<u8> = (0..MAX_PROJECTILES)
            .flat_map(|_| {
                let mut bytes = Vec::with_capacity(8);
                bytes.extend_from_slice(&(-1i32).to_le_bytes());
                bytes.extend_from_slice(&0i32.to_le_bytes());
                bytes
            })
            .collect();
        let hit_init_packed = PackedByteArray::from(hit_init.as_slice());
        rd.buffer_update(proj_hit_buffer, 0, hit_init_packed.len() as u32, &hit_init_packed);

        let uniform_set = Self::create_uniform_set(
            &mut rd, shader,
            position_buffer, target_buffer, color_buffer, speed_buffer,
            grid_counts_buffer, grid_data_buffer, multimesh_buffer, arrival_buffer,
            backoff_buffer, faction_buffer, health_buffer, combat_target_buffer,
            sprite_frame_buffer,
        )?;

        // Load projectile shader
        let proj_shader_file = ResourceLoader::singleton()
            .load("res://shaders/projectile_compute.glsl");
        let (proj_shader, proj_pipeline, proj_uniform_set) = if let Some(file) = proj_shader_file {
            let file = file.cast::<godot::classes::RdShaderFile>();
            if let Some(spirv) = file.get_spirv() {
                let shader = rd.shader_create_from_spirv(&spirv);
                if shader.is_valid() {
                    let pipeline = rd.compute_pipeline_create(shader);
                    if pipeline.is_valid() {
                        let uniform_set = Self::create_projectile_uniform_set(
                            &mut rd, shader,
                            proj_position_buffer, proj_velocity_buffer, proj_damage_buffer,
                            proj_faction_buffer, proj_shooter_buffer, proj_lifetime_buffer,
                            proj_active_buffer, proj_hit_buffer,
                            position_buffer, faction_buffer, health_buffer,
                            grid_counts_buffer, grid_data_buffer,
                        );
                        if let Some(us) = uniform_set {
                            (shader, pipeline, us)
                        } else {
                            godot_warn!("[GPU] Failed to create projectile uniform set");
                            (Rid::Invalid, Rid::Invalid, Rid::Invalid)
                        }
                    } else {
                        godot_warn!("[GPU] Failed to create projectile pipeline");
                        (Rid::Invalid, Rid::Invalid, Rid::Invalid)
                    }
                } else {
                    godot_warn!("[GPU] Failed to create projectile shader");
                    (Rid::Invalid, Rid::Invalid, Rid::Invalid)
                }
            } else {
                godot_warn!("[GPU] No SPIRV in projectile shader");
                (Rid::Invalid, Rid::Invalid, Rid::Invalid)
            }
        } else {
            godot_warn!("[GPU] Projectile shader not found - projectiles disabled");
            (Rid::Invalid, Rid::Invalid, Rid::Invalid)
        };

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
            faction_buffer,
            health_buffer,
            combat_target_buffer,
            sprite_frame_buffer,
            proj_position_buffer,
            proj_velocity_buffer,
            proj_damage_buffer,
            proj_faction_buffer,
            proj_shooter_buffer,
            proj_lifetime_buffer,
            proj_active_buffer,
            proj_hit_buffer,
            uniform_set,
            proj_shader,
            proj_pipeline,
            proj_uniform_set,
            grid: SpatialGrid::new(),
            positions: vec![0.0; MAX_NPC_COUNT * 2],
            targets: vec![0.0; MAX_NPC_COUNT * 2],
            colors: vec![0.0; MAX_NPC_COUNT * 4],
            factions: vec![0; MAX_NPC_COUNT],
            healths: vec![0.0; MAX_NPC_COUNT],
            combat_targets: vec![-1; MAX_NPC_COUNT],
            sprite_frames: vec![0.0; MAX_NPC_COUNT * 2],
            healing_flags: vec![false; MAX_NPC_COUNT],
            carried_items: vec![0; MAX_NPC_COUNT],
            proj_positions: vec![0.0; MAX_PROJECTILES * 2],
            proj_velocities: vec![0.0; MAX_PROJECTILES * 2],
            proj_damages: vec![0.0; MAX_PROJECTILES],
            proj_factions: vec![0; MAX_PROJECTILES],
            proj_active: vec![0; MAX_PROJECTILES],
            proj_count: 0,
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
        faction_buffer: Rid,
        health_buffer: Rid,
        combat_target_buffer: Rid,
        sprite_frame_buffer: Rid,
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
            (9, faction_buffer),
            (10, health_buffer),
            (11, combat_target_buffer),
            (12, sprite_frame_buffer),
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

    fn create_projectile_uniform_set(
        rd: &mut Gd<RenderingDevice>,
        shader: Rid,
        proj_position_buffer: Rid,
        proj_velocity_buffer: Rid,
        proj_damage_buffer: Rid,
        proj_faction_buffer: Rid,
        proj_shooter_buffer: Rid,
        proj_lifetime_buffer: Rid,
        proj_active_buffer: Rid,
        proj_hit_buffer: Rid,
        npc_position_buffer: Rid,
        npc_faction_buffer: Rid,
        npc_health_buffer: Rid,
        grid_counts_buffer: Rid,
        grid_data_buffer: Rid,
    ) -> Option<Rid> {
        let mut uniforms = Array::new();

        // Projectile buffers (0-7)
        let buffers = [
            (0, proj_position_buffer),
            (1, proj_velocity_buffer),
            (2, proj_damage_buffer),
            (3, proj_faction_buffer),
            (4, proj_shooter_buffer),
            (5, proj_lifetime_buffer),
            (6, proj_active_buffer),
            (7, proj_hit_buffer),
            // NPC data for collision (8-10)
            (8, npc_position_buffer),
            (9, npc_faction_buffer),
            (10, npc_health_buffer),
            // Grid data for spatial queries (11-12)
            (11, grid_counts_buffer),
            (12, grid_data_buffer),
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
            // Transform2D (8 floats)
            floats[base + 0] = 1.0;
            floats[base + 1] = 0.0;
            floats[base + 2] = 0.0;
            floats[base + 3] = self.positions[i * 2];
            floats[base + 4] = 0.0;
            floats[base + 5] = 1.0;
            floats[base + 6] = 0.0;
            floats[base + 7] = self.positions[i * 2 + 1];
            // Color (4 floats)
            floats[base + 8] = colors[i * 4];
            floats[base + 9] = colors[i * 4 + 1];
            floats[base + 10] = colors[i * 4 + 2];
            floats[base + 11] = colors[i * 4 + 3];
            // CustomData (4 floats): health_pct, healing+flash, sprite_x/255, sprite_y/255
            // Encoding: 0-1 = flash only, 2-3 = healing + flash (subtract 2 for flash intensity)
            let health_pct = (self.healths[i] / 100.0).clamp(0.0, 1.0);
            let sprite_col = self.sprite_frames[i * 2];
            let sprite_row = self.sprite_frames[i * 2 + 1];
            let healing_offset = if self.healing_flags[i] { 2.0 } else { 0.0 };
            floats[base + 12] = health_pct;
            floats[base + 13] = healing_offset;  // TODO: add flash intensity when implemented
            floats[base + 14] = sprite_col / 255.0;
            floats[base + 15] = sprite_row / 255.0;
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

    /// Upload faction data to GPU
    pub fn upload_factions(&mut self, npc_count: usize) {
        let bytes: Vec<u8> = self.factions[..npc_count].iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let packed = PackedByteArray::from(bytes.as_slice());
        self.rd.buffer_update(self.faction_buffer, 0, packed.len() as u32, &packed);
    }

    /// Upload health data to GPU
    pub fn upload_healths(&mut self, npc_count: usize) {
        let bytes: Vec<u8> = self.healths[..npc_count].iter()
            .flat_map(|h| h.to_le_bytes())
            .collect();
        let packed = PackedByteArray::from(bytes.as_slice());
        self.rd.buffer_update(self.health_buffer, 0, packed.len() as u32, &packed);
    }

    /// Read combat targets from GPU
    pub fn read_combat_targets(&mut self, npc_count: usize) {
        let bytes = self.rd.buffer_get_data(self.combat_target_buffer);
        let byte_slice = bytes.as_slice();
        for i in 0..npc_count {
            let offset = i * 4;
            if offset + 4 <= byte_slice.len() {
                self.combat_targets[i] = i32::from_le_bytes([
                    byte_slice[offset],
                    byte_slice[offset + 1],
                    byte_slice[offset + 2],
                    byte_slice[offset + 3],
                ]);
            }
        }
    }

    // ========================================================================
    // PROJECTILE METHODS
    // ========================================================================

    /// Upload a single projectile to GPU buffers
    pub fn upload_projectile(
        &mut self,
        idx: usize,
        x: f32, y: f32,
        vx: f32, vy: f32,
        damage: f32,
        faction: i32,
        shooter: i32,
        lifetime: f32,
    ) {
        // Position
        let pos_bytes: Vec<u8> = [x, y].iter().flat_map(|f| f.to_le_bytes()).collect();
        let pos_packed = PackedByteArray::from(pos_bytes.as_slice());
        self.rd.buffer_update(self.proj_position_buffer, (idx * 8) as u32, 8, &pos_packed);
        self.proj_positions[idx * 2] = x;
        self.proj_positions[idx * 2 + 1] = y;

        // Velocity
        let vel_bytes: Vec<u8> = [vx, vy].iter().flat_map(|f| f.to_le_bytes()).collect();
        let vel_packed = PackedByteArray::from(vel_bytes.as_slice());
        self.rd.buffer_update(self.proj_velocity_buffer, (idx * 8) as u32, 8, &vel_packed);
        self.proj_velocities[idx * 2] = vx;
        self.proj_velocities[idx * 2 + 1] = vy;

        // Damage
        let dmg_bytes: Vec<u8> = damage.to_le_bytes().to_vec();
        let dmg_packed = PackedByteArray::from(dmg_bytes.as_slice());
        self.rd.buffer_update(self.proj_damage_buffer, (idx * 4) as u32, 4, &dmg_packed);
        self.proj_damages[idx] = damage;

        // Faction
        let fac_bytes: Vec<u8> = faction.to_le_bytes().to_vec();
        let fac_packed = PackedByteArray::from(fac_bytes.as_slice());
        self.rd.buffer_update(self.proj_faction_buffer, (idx * 4) as u32, 4, &fac_packed);
        self.proj_factions[idx] = faction;

        // Shooter
        let shooter_bytes: Vec<u8> = shooter.to_le_bytes().to_vec();
        let shooter_packed = PackedByteArray::from(shooter_bytes.as_slice());
        self.rd.buffer_update(self.proj_shooter_buffer, (idx * 4) as u32, 4, &shooter_packed);

        // Lifetime
        let lifetime_bytes: Vec<u8> = lifetime.to_le_bytes().to_vec();
        let lifetime_packed = PackedByteArray::from(lifetime_bytes.as_slice());
        self.rd.buffer_update(self.proj_lifetime_buffer, (idx * 4) as u32, 4, &lifetime_packed);

        // Active
        let active_bytes: Vec<u8> = 1i32.to_le_bytes().to_vec();
        let active_packed = PackedByteArray::from(active_bytes.as_slice());
        self.rd.buffer_update(self.proj_active_buffer, (idx * 4) as u32, 4, &active_packed);
        self.proj_active[idx] = 1;

        // Clear hit status (-1 = no hit)
        let hit_bytes: Vec<u8> = [-1i32, 0i32].iter().flat_map(|i| i.to_le_bytes()).collect();
        let hit_packed = PackedByteArray::from(hit_bytes.as_slice());
        self.rd.buffer_update(self.proj_hit_buffer, (idx * 8) as u32, 8, &hit_packed);

        // Update count
        if idx >= self.proj_count {
            self.proj_count = idx + 1;
        }
    }

    /// Dispatch projectile compute shader
    pub fn dispatch_projectiles(&mut self, proj_count: usize, npc_count: usize, delta: f32) {
        if proj_count == 0 || !self.proj_pipeline.is_valid() {
            return;
        }

        // Pack push constants
        let mut push_data = vec![0u8; PROJ_PUSH_CONSTANTS_SIZE];
        push_data[0..4].copy_from_slice(&(proj_count as u32).to_le_bytes());
        push_data[4..8].copy_from_slice(&(npc_count as u32).to_le_bytes());
        push_data[8..12].copy_from_slice(&delta.to_le_bytes());
        push_data[12..16].copy_from_slice(&PROJECTILE_HIT_RADIUS.to_le_bytes());
        push_data[16..20].copy_from_slice(&(GRID_WIDTH as u32).to_le_bytes());
        push_data[20..24].copy_from_slice(&(GRID_HEIGHT as u32).to_le_bytes());
        push_data[24..28].copy_from_slice(&CELL_SIZE.to_le_bytes());
        push_data[28..32].copy_from_slice(&(MAX_PER_CELL as u32).to_le_bytes());
        let push_constants = PackedByteArray::from(push_data.as_slice());

        let compute_list = self.rd.compute_list_begin();
        self.rd.compute_list_bind_compute_pipeline(compute_list, self.proj_pipeline);
        self.rd.compute_list_bind_uniform_set(compute_list, self.proj_uniform_set, 0);
        self.rd.compute_list_set_push_constant(compute_list, &push_constants, PROJ_PUSH_CONSTANTS_SIZE as u32);

        let workgroups = ((proj_count + 63) / 64) as u32;
        self.rd.compute_list_dispatch(compute_list, workgroups, 1, 1);
        self.rd.compute_list_end();

        self.rd.submit();
        self.rd.sync();
    }

    /// Read projectile hits from GPU. Returns vec of (proj_idx, npc_idx, damage).
    pub fn read_projectile_hits(&mut self) -> Vec<(usize, usize, f32)> {
        let mut hits = Vec::new();
        let bytes = self.rd.buffer_get_data(self.proj_hit_buffer);
        let byte_slice = bytes.as_slice();

        for i in 0..self.proj_count {
            let offset = i * 8;
            if offset + 8 > byte_slice.len() {
                continue;
            }

            let npc_idx = i32::from_le_bytes([
                byte_slice[offset],
                byte_slice[offset + 1],
                byte_slice[offset + 2],
                byte_slice[offset + 3],
            ]);
            let processed = i32::from_le_bytes([
                byte_slice[offset + 4],
                byte_slice[offset + 5],
                byte_slice[offset + 6],
                byte_slice[offset + 7],
            ]);

            if npc_idx >= 0 && processed == 0 {
                let damage = self.proj_damages[i];
                hits.push((i, npc_idx as usize, damage));

                // Mark as processed
                let hit_bytes: Vec<u8> = [npc_idx, 1i32].iter().flat_map(|i| i.to_le_bytes()).collect();
                let hit_packed = PackedByteArray::from(hit_bytes.as_slice());
                self.rd.buffer_update(self.proj_hit_buffer, (i * 8) as u32, 8, &hit_packed);

                // Mark projectile as inactive
                self.proj_active[i] = 0;
            }
        }

        hits
    }

    /// Read projectile positions from GPU for rendering
    pub fn read_projectile_positions(&mut self) {
        let bytes = self.rd.buffer_get_data(self.proj_position_buffer);
        let byte_slice = bytes.as_slice();
        for i in 0..(self.proj_count * 2) {
            let offset = i * 4;
            if offset + 4 <= byte_slice.len() {
                self.proj_positions[i] = f32::from_le_bytes([
                    byte_slice[offset],
                    byte_slice[offset + 1],
                    byte_slice[offset + 2],
                    byte_slice[offset + 3],
                ]);
            }
        }
    }

    /// Read projectile active flags from GPU
    pub fn read_projectile_active(&mut self) {
        let bytes = self.rd.buffer_get_data(self.proj_active_buffer);
        let byte_slice = bytes.as_slice();
        for i in 0..self.proj_count {
            let offset = i * 4;
            if offset + 4 <= byte_slice.len() {
                self.proj_active[i] = i32::from_le_bytes([
                    byte_slice[offset],
                    byte_slice[offset + 1],
                    byte_slice[offset + 2],
                    byte_slice[offset + 3],
                ]);
            }
        }
    }

    /// Read raw GPU projectile state for debugging.
    /// Returns lifetime, active, position, hit for first N projectiles.
    /// Read grid cell count for a given position (debug)
    pub fn trace_grid_cell(&mut self, x: f32, y: f32) -> (usize, usize, i32) {
        let cx = (x / CELL_SIZE) as usize;
        let cy = (y / CELL_SIZE) as usize;
        if cx >= GRID_WIDTH || cy >= GRID_HEIGHT {
            return (cx, cy, -1);
        }
        let cell_idx = cy * GRID_WIDTH + cx;
        // Read from CPU cache (already uploaded this frame)
        let count = self.grid.counts.get(cell_idx).copied().unwrap_or(-1);
        (cx, cy, count)
    }

    pub fn trace_projectile_gpu_state(&mut self, count: usize) -> Vec<(f32, i32, f32, f32, i32, i32)> {
        let mut result = Vec::new();
        let lifetime_bytes = self.rd.buffer_get_data(self.proj_lifetime_buffer);
        let active_bytes = self.rd.buffer_get_data(self.proj_active_buffer);
        let pos_bytes = self.rd.buffer_get_data(self.proj_position_buffer);
        let hit_bytes = self.rd.buffer_get_data(self.proj_hit_buffer);

        let lt = lifetime_bytes.as_slice();
        let act = active_bytes.as_slice();
        let pos = pos_bytes.as_slice();
        let hit = hit_bytes.as_slice();

        for i in 0..count.min(self.proj_count) {
            let lifetime = if i * 4 + 4 <= lt.len() {
                f32::from_le_bytes([lt[i*4], lt[i*4+1], lt[i*4+2], lt[i*4+3]])
            } else { -999.0 };
            let active = if i * 4 + 4 <= act.len() {
                i32::from_le_bytes([act[i*4], act[i*4+1], act[i*4+2], act[i*4+3]])
            } else { -999 };
            let px = if i * 8 + 4 <= pos.len() {
                f32::from_le_bytes([pos[i*8], pos[i*8+1], pos[i*8+2], pos[i*8+3]])
            } else { -999.0 };
            let py = if i * 8 + 8 <= pos.len() {
                f32::from_le_bytes([pos[i*8+4], pos[i*8+5], pos[i*8+6], pos[i*8+7]])
            } else { -999.0 };
            let hit_npc = if i * 8 + 4 <= hit.len() {
                i32::from_le_bytes([hit[i*8], hit[i*8+1], hit[i*8+2], hit[i*8+3]])
            } else { -999 };
            let hit_proc = if i * 8 + 8 <= hit.len() {
                i32::from_le_bytes([hit[i*8+4], hit[i*8+5], hit[i*8+6], hit[i*8+7]])
            } else { -999 };

            result.push((lifetime, active, px, py, hit_npc, hit_proc));
        }
        result
    }

    /// Build carried item MultiMesh buffer.
    /// Items render above NPC heads at same X, Y-12 offset.
    /// Item colors: 1=food(yellow), 2=wood(brown), 3=stone(gray), 4=weapon(red)
    pub fn build_item_multimesh(&self, npc_count: usize, max_count: usize) -> PackedFloat32Array {
        const FLOATS_PER_ITEM: usize = 12; // Transform2D(8) + Color(4)
        const Y_OFFSET: f32 = -12.0; // Above NPC head

        // Item colors by ID (index = item_id - 1)
        const ITEM_COLORS: [(f32, f32, f32); 4] = [
            (1.0, 0.9, 0.3),  // 1: Food (yellow/gold)
            (0.6, 0.4, 0.2),  // 2: Wood (brown)
            (0.5, 0.5, 0.6),  // 3: Stone (gray)
            (0.8, 0.3, 0.3),  // 4: Weapon (red)
        ];

        let float_count = max_count * FLOATS_PER_ITEM;
        let mut floats = vec![0.0f32; float_count];

        // Initialize all as hidden
        for i in 0..max_count {
            let base = i * FLOATS_PER_ITEM;
            floats[base + 0] = 1.0;      // scale x
            floats[base + 5] = 1.0;      // scale y
            floats[base + 3] = -9999.0;  // pos x (hidden)
            floats[base + 7] = -9999.0;  // pos y (hidden)
            floats[base + 11] = 1.0;     // alpha
        }

        // Set items for NPCs that are carrying something
        for i in 0..npc_count {
            let item_id = self.carried_items[i];
            if item_id == 0 {
                continue; // Not carrying anything
            }

            let base = i * FLOATS_PER_ITEM;
            let x = self.positions[i * 2];
            let y = self.positions[i * 2 + 1] + Y_OFFSET;

            // Transform2D (identity scale, positioned above NPC)
            floats[base + 0] = 1.0;  // scale x
            floats[base + 3] = x;    // pos x
            floats[base + 5] = 1.0;  // scale y
            floats[base + 7] = y;    // pos y (above head)

            // Color by item type
            let color_idx = ((item_id - 1) as usize).min(ITEM_COLORS.len() - 1);
            let (r, g, b) = ITEM_COLORS[color_idx];
            floats[base + 8] = r;
            floats[base + 9] = g;
            floats[base + 10] = b;
            floats[base + 11] = 1.0;
        }

        PackedFloat32Array::from(floats.as_slice())
    }

    /// Build projectile MultiMesh buffer
    pub fn build_proj_multimesh(&self, max_count: usize) -> PackedFloat32Array {
        let float_count = max_count * PROJ_FLOATS_PER_INSTANCE;
        let mut floats = vec![0.0f32; float_count];

        // Initialize all as hidden
        for i in 0..max_count {
            let base = i * PROJ_FLOATS_PER_INSTANCE;
            floats[base + 0] = 1.0;  // scale x
            floats[base + 5] = 1.0;  // scale y
            floats[base + 3] = -9999.0;  // pos x (hidden)
            floats[base + 7] = -9999.0;  // pos y (hidden)
        }

        // Set active projectiles
        for i in 0..self.proj_count {
            if self.proj_active[i] == 0 {
                continue;
            }

            let base = i * PROJ_FLOATS_PER_INSTANCE;
            let x = self.proj_positions[i * 2];
            let y = self.proj_positions[i * 2 + 1];
            let vx = self.proj_velocities[i * 2];
            let vy = self.proj_velocities[i * 2 + 1];

            // Calculate rotation from velocity
            let angle = vy.atan2(vx);
            let cos_a = angle.cos();
            let sin_a = angle.sin();

            // Transform2D with rotation
            floats[base + 0] = cos_a;   // a (scale_x * cos)
            floats[base + 1] = sin_a;   // b (scale_x * sin)
            floats[base + 2] = 0.0;
            floats[base + 3] = x;
            floats[base + 4] = -sin_a;  // c (-scale_y * sin)
            floats[base + 5] = cos_a;   // d (scale_y * cos)
            floats[base + 6] = 0.0;
            floats[base + 7] = y;

            // Faction color: blue for villager (0), red for raider (1)
            let faction = self.proj_factions.get(i).copied().unwrap_or(0);
            if faction == 1 {
                floats[base + 8] = 1.0;   // r
                floats[base + 9] = 0.2;   // g
                floats[base + 10] = 0.2;  // b
            } else {
                floats[base + 8] = 0.3;   // r
                floats[base + 9] = 0.5;   // g
                floats[base + 10] = 1.0;  // b
            }
            floats[base + 11] = 1.0;  // a
        }

        PackedFloat32Array::from(floats.as_slice())
    }
}
