//! GPU Compute Module - wgpu-based NPC physics via Bevy's render graph.
//!
//! Follows Bevy 0.18's compute_shader_game_of_life.rs pattern.
//! Three-phase dispatch per frame: clear grid → insert NPCs → main logic.
//!
//! Data flow:
//! - Main world: Systems write GpuUpdateMsg → GPU_UPDATE_QUEUE
//! - Main world: populate_buffer_writes drains queue → NpcBufferWrites
//! - Extract: NpcBufferWrites copied to render world
//! - Render: write_npc_buffers uploads data to GPU

use bevy::{
    prelude::*,
    render::{
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_graph::{self, RenderGraph, RenderLabel},
        render_resource::{
            binding_types::{storage_buffer, storage_buffer_read_only, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        Render, RenderApp, RenderStartup, RenderSystems,
    },
    shader::PipelineCacheError,
};
use std::borrow::Cow;

use crate::messages::{GpuUpdate, GPU_UPDATE_QUEUE, GPU_READ_STATE, ProjGpuUpdate, PROJ_GPU_UPDATE_QUEUE, PROJ_HIT_STATE};

// =============================================================================
// CONSTANTS
// =============================================================================

const SHADER_ASSET_PATH: &str = "shaders/npc_compute.wgsl";
const PROJ_SHADER_ASSET_PATH: &str = "shaders/projectile_compute.wgsl";
const WORKGROUP_SIZE: u32 = 64;
const MAX_NPCS: u32 = 16384;
const MAX_PROJECTILES: u32 = 50_000;
const GRID_WIDTH: u32 = 128;
const GRID_HEIGHT: u32 = 128;
const MAX_PER_CELL: u32 = 48;
const HIT_RADIUS: f32 = 10.0;

// =============================================================================
// RESOURCES (Main World)
// =============================================================================

/// GPU buffer data extracted to render world each frame.
#[derive(Resource, Clone, ExtractResource)]
pub struct NpcGpuData {
    /// Number of active NPCs
    pub npc_count: u32,
    /// Frame delta time
    pub delta: f32,
}

impl Default for NpcGpuData {
    fn default() -> Self {
        Self {
            npc_count: 0,
            delta: 0.016,
        }
    }
}

/// GPU compute parameters (uniform buffer).
#[derive(Resource, Clone, ExtractResource, ShaderType)]
pub struct NpcComputeParams {
    pub count: u32,
    pub separation_radius: f32,
    pub separation_strength: f32,
    pub delta: f32,
    pub grid_width: u32,
    pub grid_height: u32,
    pub cell_size: f32,
    pub max_per_cell: u32,
    pub arrival_threshold: f32,
    pub mode: u32,
    pub combat_range: f32,
    pub _pad2: f32,
}

impl Default for NpcComputeParams {
    fn default() -> Self {
        Self {
            count: 0,
            separation_radius: 20.0,
            separation_strength: 100.0,
            delta: 0.016,
            grid_width: GRID_WIDTH,
            grid_height: GRID_HEIGHT,
            cell_size: 64.0,
            max_per_cell: MAX_PER_CELL,
            arrival_threshold: 8.0,
            mode: 0,
            combat_range: 300.0,
            _pad2: 0.0,
        }
    }
}

/// Buffer data to upload to GPU each frame.
/// Populated in main world, extracted to render world.
#[derive(Resource, Clone, ExtractResource)]
pub struct NpcBufferWrites {
    /// Position buffer: [x0, y0, x1, y1, ...] flattened
    pub positions: Vec<f32>,
    /// Target buffer: [x0, y0, x1, y1, ...] flattened
    pub targets: Vec<f32>,
    /// Speed buffer: one f32 per NPC
    pub speeds: Vec<f32>,
    /// Faction buffer: one i32 per NPC
    pub factions: Vec<i32>,
    /// Health buffer: one f32 per NPC
    pub healths: Vec<f32>,
    /// Arrival flags: one i32 per NPC (0 = moving, 1 = settled)
    pub arrivals: Vec<i32>,
    /// Sprite indices: [col, row, 0, 0] per NPC (vec4 for alignment)
    pub sprite_indices: Vec<f32>,
    /// Colors: [r, g, b, a] per NPC
    pub colors: Vec<f32>,
    /// Damage flash intensity: 0.0-1.0 per NPC (decays each frame)
    pub flash_values: Vec<f32>,
    /// Whether any data changed this frame (skip upload if false)
    pub dirty: bool,
    /// Per-field dirty flags to avoid overwriting GPU-computed positions
    pub positions_dirty: bool,
    pub targets_dirty: bool,
    pub speeds_dirty: bool,
    pub factions_dirty: bool,
    pub healths_dirty: bool,
    pub arrivals_dirty: bool,
}

impl Default for NpcBufferWrites {
    fn default() -> Self {
        // Pre-allocate for MAX_NPCS
        let max = MAX_NPCS as usize;
        Self {
            positions: vec![0.0; max * 2],
            targets: vec![0.0; max * 2],
            speeds: vec![100.0; max],
            factions: vec![0; max],
            healths: vec![100.0; max],
            arrivals: vec![0; max],  // 0 = wants to move
            sprite_indices: vec![0.0; max * 4], // vec4 per NPC
            colors: vec![1.0; max * 4],          // RGBA, default white
            flash_values: vec![0.0; max],
            dirty: false,
            positions_dirty: false,
            targets_dirty: false,
            speeds_dirty: false,
            factions_dirty: false,
            healths_dirty: false,
            arrivals_dirty: false,
        }
    }
}

impl NpcBufferWrites {
    /// Apply a GPU update to the buffer data.
    pub fn apply(&mut self, update: &GpuUpdate) {
        match update {
            GpuUpdate::SetPosition { idx, x, y } => {
                let i = *idx * 2;
                if i + 1 < self.positions.len() {
                    self.positions[i] = *x;
                    self.positions[i + 1] = *y;
                    self.dirty = true;
                    self.positions_dirty = true;
                }
            }
            GpuUpdate::SetTarget { idx, x, y } => {
                let i = *idx * 2;
                if i + 1 < self.targets.len() {
                    self.targets[i] = *x;
                    self.targets[i + 1] = *y;
                    self.dirty = true;
                    self.targets_dirty = true;
                }
                // Reset arrival flag so GPU resumes movement toward new target
                if *idx < self.arrivals.len() {
                    self.arrivals[*idx] = 0;
                    self.arrivals_dirty = true;
                }
            }
            GpuUpdate::SetSpeed { idx, speed } => {
                if *idx < self.speeds.len() {
                    self.speeds[*idx] = *speed;
                    self.dirty = true;
                    self.speeds_dirty = true;
                }
            }
            GpuUpdate::SetFaction { idx, faction } => {
                if *idx < self.factions.len() {
                    self.factions[*idx] = *faction;
                    self.dirty = true;
                    self.factions_dirty = true;
                }
            }
            GpuUpdate::SetHealth { idx, health } => {
                if *idx < self.healths.len() {
                    self.healths[*idx] = *health;
                    self.dirty = true;
                    self.healths_dirty = true;
                }
            }
            GpuUpdate::ApplyDamage { idx, amount } => {
                if *idx < self.healths.len() {
                    self.healths[*idx] = (self.healths[*idx] - amount).max(0.0);
                    self.dirty = true;
                    self.healths_dirty = true;
                }
            }
            GpuUpdate::HideNpc { idx } => {
                // Move to offscreen position
                let i = *idx * 2;
                if i + 1 < self.positions.len() {
                    self.positions[i] = -9999.0;
                    self.positions[i + 1] = -9999.0;
                    self.dirty = true;
                    self.positions_dirty = true;
                }
            }
            GpuUpdate::SetColor { idx, r, g, b, a } => {
                let i = *idx * 4;
                if i + 3 < self.colors.len() {
                    self.colors[i] = *r;
                    self.colors[i + 1] = *g;
                    self.colors[i + 2] = *b;
                    self.colors[i + 3] = *a;
                    self.dirty = true;
                }
            }
            GpuUpdate::SetSpriteFrame { idx, col, row } => {
                let i = *idx * 4;
                if i + 3 < self.sprite_indices.len() {
                    self.sprite_indices[i] = *col;
                    self.sprite_indices[i + 1] = *row;
                    // zw unused, leave as 0
                    self.dirty = true;
                }
            }
            GpuUpdate::SetDamageFlash { idx, intensity } => {
                if *idx < self.flash_values.len() {
                    self.flash_values[*idx] = *intensity;
                    self.dirty = true;
                }
            }
            // These don't affect GPU buffers (visual effects handled separately)
            GpuUpdate::SetHealing { .. } |
            GpuUpdate::SetCarriedItem { .. } => {}
        }
    }
}

/// Drain GPU_UPDATE_QUEUE and apply updates to NpcBufferWrites.
/// Runs in main world each frame before extraction.
pub fn populate_buffer_writes(mut buffer_writes: ResMut<NpcBufferWrites>, time: Res<Time>) {
    // Reset dirty flags - will be set if any updates applied
    buffer_writes.dirty = false;
    buffer_writes.positions_dirty = false;
    buffer_writes.targets_dirty = false;
    buffer_writes.speeds_dirty = false;
    buffer_writes.factions_dirty = false;
    buffer_writes.healths_dirty = false;
    buffer_writes.arrivals_dirty = false;

    if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
        for update in queue.drain(..) {
            buffer_writes.apply(&update);
        }
    }

    // Decay damage flash values (1.0 → 0.0 in ~0.2s)
    let dt = time.delta_secs();
    const FLASH_DECAY_RATE: f32 = 5.0;
    let mut any_flash = false;
    for flash in buffer_writes.flash_values.iter_mut() {
        if *flash > 0.0 {
            *flash = (*flash - dt * FLASH_DECAY_RATE).max(0.0);
            any_flash = true;
        }
    }
    if any_flash {
        buffer_writes.dirty = true;
    }
}

// =============================================================================
// PROJECTILE RESOURCES (Main World)
// =============================================================================

/// Projectile GPU data extracted to render world each frame.
#[derive(Resource, Clone, ExtractResource)]
pub struct ProjGpuData {
    pub proj_count: u32,
    pub npc_count: u32,
    pub delta: f32,
}

impl Default for ProjGpuData {
    fn default() -> Self {
        Self { proj_count: 0, npc_count: 0, delta: 0.016 }
    }
}

/// Projectile compute parameters (uniform buffer).
#[derive(Resource, Clone, ExtractResource, ShaderType)]
pub struct ProjComputeParams {
    pub proj_count: u32,
    pub npc_count: u32,
    pub delta: f32,
    pub hit_radius: f32,
    pub grid_width: u32,
    pub grid_height: u32,
    pub cell_size: f32,
    pub max_per_cell: u32,
}

impl Default for ProjComputeParams {
    fn default() -> Self {
        Self {
            proj_count: 0,
            npc_count: 0,
            delta: 0.016,
            hit_radius: HIT_RADIUS,
            grid_width: GRID_WIDTH,
            grid_height: GRID_HEIGHT,
            cell_size: 64.0,
            max_per_cell: MAX_PER_CELL,
        }
    }
}

/// Projectile buffer data to upload to GPU each frame.
#[derive(Resource, Clone, ExtractResource)]
pub struct ProjBufferWrites {
    pub positions: Vec<f32>,   // [x, y] per proj
    pub velocities: Vec<f32>,  // [vx, vy] per proj
    pub damages: Vec<f32>,
    pub factions: Vec<i32>,
    pub shooters: Vec<i32>,
    pub lifetimes: Vec<f32>,
    pub active: Vec<i32>,
    pub hits: Vec<i32>,        // [npc_idx, processed] per proj
    pub dirty: bool,
}

impl Default for ProjBufferWrites {
    fn default() -> Self {
        let max = MAX_PROJECTILES as usize;
        Self {
            positions: vec![0.0; max * 2],
            velocities: vec![0.0; max * 2],
            damages: vec![0.0; max],
            factions: vec![0; max],
            shooters: vec![-1; max],
            lifetimes: vec![0.0; max],
            active: vec![0; max],
            hits: vec![-1; max * 2],   // -1 = no hit
            dirty: true,  // Force first-frame upload so GPU gets -1 hit initialization
        }
    }
}

impl ProjBufferWrites {
    pub fn apply(&mut self, update: &ProjGpuUpdate) {
        match update {
            ProjGpuUpdate::Spawn { idx, x, y, vx, vy, damage, faction, shooter, lifetime } => {
                let i2 = *idx * 2;
                if i2 + 1 < self.positions.len() {
                    self.positions[i2] = *x;
                    self.positions[i2 + 1] = *y;
                    self.velocities[i2] = *vx;
                    self.velocities[i2 + 1] = *vy;
                    self.damages[*idx] = *damage;
                    self.factions[*idx] = *faction;
                    self.shooters[*idx] = *shooter;
                    self.lifetimes[*idx] = *lifetime;
                    self.active[*idx] = 1;
                    self.hits[i2] = -1;
                    self.hits[i2 + 1] = 0;
                    self.dirty = true;
                }
            }
            ProjGpuUpdate::Deactivate { idx } => {
                if *idx < self.active.len() {
                    self.active[*idx] = 0;
                    // Reset hit record so GPU doesn't re-trigger
                    let i2 = *idx * 2;
                    if i2 + 1 < self.hits.len() {
                        self.hits[i2] = -1;
                        self.hits[i2 + 1] = 0;
                    }
                    self.dirty = true;
                }
            }
        }
    }
}

/// Drain PROJ_GPU_UPDATE_QUEUE and apply updates to ProjBufferWrites.
pub fn populate_proj_buffer_writes(mut writes: ResMut<ProjBufferWrites>) {
    writes.dirty = false;
    if let Ok(mut queue) = PROJ_GPU_UPDATE_QUEUE.lock() {
        for update in queue.drain(..) {
            writes.apply(&update);
        }
    }
}

// =============================================================================
// PLUGIN
// =============================================================================

pub struct GpuComputePlugin;

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct NpcComputeLabel;

impl Plugin for GpuComputePlugin {
    fn build(&self, app: &mut App) {
        // Initialize NPC resources in main world
        app.init_resource::<NpcGpuData>()
            .init_resource::<NpcComputeParams>()
            .init_resource::<NpcBufferWrites>()
            .init_resource::<NpcSpriteTexture>()
            .add_systems(Update, update_gpu_data)
            .add_systems(PostUpdate, populate_buffer_writes);

        // Initialize projectile resources in main world
        app.init_resource::<ProjGpuData>()
            .init_resource::<ProjComputeParams>()
            .init_resource::<ProjBufferWrites>()
            .add_systems(Update, update_proj_gpu_data)
            .add_systems(PostUpdate, populate_proj_buffer_writes);

        // Extract resources to render world
        app.add_plugins((
            ExtractResourcePlugin::<NpcGpuData>::default(),
            ExtractResourcePlugin::<NpcComputeParams>::default(),
            ExtractResourcePlugin::<NpcBufferWrites>::default(),
            ExtractResourcePlugin::<NpcSpriteTexture>::default(),
            ExtractResourcePlugin::<ProjGpuData>::default(),
            ExtractResourcePlugin::<ProjComputeParams>::default(),
            ExtractResourcePlugin::<ProjBufferWrites>::default(),
        ));

        // Set up render world systems
        let render_app = match app.get_sub_app_mut(RenderApp) {
            Some(ra) => ra,
            None => {
                warn!("RenderApp not available - GPU compute disabled");
                return;
            }
        };

        render_app
            .add_systems(RenderStartup, (init_npc_compute_pipeline, init_proj_compute_pipeline))
            .add_systems(
                Render,
                (
                    (write_npc_buffers, write_proj_buffers).in_set(RenderSystems::PrepareResources),
                    (prepare_npc_bind_groups, prepare_proj_bind_groups).in_set(RenderSystems::PrepareBindGroups),
                    (readback_npc_positions, readback_proj_data).in_set(RenderSystems::Cleanup),
                ),
            );

        // Add compute nodes to render graph
        // NPC compute → Projectile compute → Camera driver
        {
            let mut render_graph = render_app.world_mut().resource_mut::<RenderGraph>();
            render_graph.add_node(NpcComputeLabel, NpcComputeNode::default());
            render_graph.add_node(ProjectileComputeLabel, ProjectileComputeNode::default());
            render_graph.add_node_edge(NpcComputeLabel, ProjectileComputeLabel);
            render_graph.add_node_edge(ProjectileComputeLabel, bevy::render::graph::CameraDriverLabel);
        }

        info!("GPU compute plugin initialized");
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct ProjectileComputeLabel;

/// Update GPU data from ECS each frame.
fn update_gpu_data(
    mut gpu_data: ResMut<NpcGpuData>,
    mut params: ResMut<NpcComputeParams>,
    npc_count: Res<crate::resources::NpcCount>,
    time: Res<Time>,
) {
    gpu_data.npc_count = npc_count.0 as u32;
    gpu_data.delta = time.delta_secs();
    params.delta = time.delta_secs();
    params.count = npc_count.0 as u32;
}

// =============================================================================
// RENDER WORLD RESOURCES
// =============================================================================

/// GPU buffers for NPC compute and rendering.
#[derive(Resource)]
pub struct NpcGpuBuffers {
    // Compute buffers
    pub positions: Buffer,
    pub targets: Buffer,
    pub speeds: Buffer,
    pub grid_counts: Buffer,
    pub grid_data: Buffer,
    pub arrivals: Buffer,
    pub backoff: Buffer,
    pub factions: Buffer,
    pub healths: Buffer,
    pub combat_targets: Buffer,
    /// Staging buffer for CPU readback of positions (MAP_READ | COPY_DST)
    pub position_staging: Buffer,
    /// Staging buffer for CPU readback of combat targets (MAP_READ | COPY_DST)
    pub combat_target_staging: Buffer,
}

/// Bind groups for compute passes (one per mode, different uniform buffer).
#[derive(Resource)]
struct NpcBindGroups {
    mode0: BindGroup,  // Clear grid
    mode1: BindGroup,  // Build grid
    mode2: BindGroup,  // Movement + targeting
}

/// Pipeline resources for compute.
#[derive(Resource)]
struct NpcComputePipeline {
    bind_group_layout: BindGroupLayoutDescriptor,
    pipeline_id: CachedComputePipelineId,
}

/// Handle to the NPC sprite texture (main world).
/// Set by the render module after loading sprite sheets.
#[derive(Resource, Clone, ExtractResource, Default)]
pub struct NpcSpriteTexture {
    pub handle: Option<Handle<Image>>,
}

/// GPU buffers for projectile compute.
#[derive(Resource)]
pub struct ProjGpuBuffers {
    pub positions: Buffer,
    pub velocities: Buffer,
    pub damages: Buffer,
    pub factions: Buffer,
    pub shooters: Buffer,
    pub lifetimes: Buffer,
    pub active: Buffer,
    pub hits: Buffer,
    /// Staging buffer for CPU readback of hit results (MAP_READ | COPY_DST)
    pub hit_staging: Buffer,
    /// Staging buffer for CPU readback of projectile positions (MAP_READ | COPY_DST)
    pub position_staging: Buffer,
}

/// Bind groups for projectile compute pass.
#[derive(Resource)]
struct ProjBindGroups {
    bind_group: BindGroup,
}

/// Pipeline resources for projectile compute.
#[derive(Resource)]
struct ProjComputePipeline {
    bind_group_layout: BindGroupLayoutDescriptor,
    pipeline_id: CachedComputePipelineId,
}

// =============================================================================
// PIPELINE INITIALIZATION
// =============================================================================

fn init_npc_compute_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let grid_cells = (GRID_WIDTH * GRID_HEIGHT) as usize;
    let grid_data_size = grid_cells * MAX_PER_CELL as usize;

    // Create GPU buffers
    let buffers = NpcGpuBuffers {
        positions: render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_positions"),
            size: (MAX_NPCS as usize * std::mem::size_of::<[f32; 2]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        targets: render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_targets"),
            size: (MAX_NPCS as usize * std::mem::size_of::<[f32; 2]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        speeds: render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_speeds"),
            size: (MAX_NPCS as usize * std::mem::size_of::<f32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        grid_counts: render_device.create_buffer(&BufferDescriptor {
            label: Some("grid_counts"),
            size: (grid_cells * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE,
            mapped_at_creation: false,
        }),
        grid_data: render_device.create_buffer(&BufferDescriptor {
            label: Some("grid_data"),
            size: (grid_data_size * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE,
            mapped_at_creation: false,
        }),
        arrivals: render_device.create_buffer(&BufferDescriptor {
            label: Some("arrivals"),
            size: (MAX_NPCS as usize * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        backoff: render_device.create_buffer(&BufferDescriptor {
            label: Some("backoff"),
            size: (MAX_NPCS as usize * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE,
            mapped_at_creation: false,
        }),
        factions: render_device.create_buffer(&BufferDescriptor {
            label: Some("factions"),
            size: (MAX_NPCS as usize * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        healths: render_device.create_buffer(&BufferDescriptor {
            label: Some("healths"),
            size: (MAX_NPCS as usize * std::mem::size_of::<f32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        combat_targets: render_device.create_buffer(&BufferDescriptor {
            label: Some("combat_targets"),
            size: (MAX_NPCS as usize * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        position_staging: render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_position_staging"),
            size: (MAX_NPCS as usize * std::mem::size_of::<[f32; 2]>()) as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        combat_target_staging: render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_combat_target_staging"),
            size: (MAX_NPCS as usize * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
    };

    commands.insert_resource(buffers);

    // Define bind group layout (all storage buffers are read_write for simplicity)
    let bind_group_layout = BindGroupLayoutDescriptor::new(
        "NpcComputeLayout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                // 0: positions
                storage_buffer::<Vec<[f32; 2]>>(false),
                // 1: goals (targets)
                storage_buffer::<Vec<[f32; 2]>>(false),
                // 2: speeds
                storage_buffer::<Vec<f32>>(false),
                // 3: grid_counts
                storage_buffer::<Vec<i32>>(false),
                // 4: grid_data
                storage_buffer::<Vec<i32>>(false),
                // 5: arrivals
                storage_buffer::<Vec<i32>>(false),
                // 6: backoff
                storage_buffer::<Vec<i32>>(false),
                // 7: factions
                storage_buffer::<Vec<i32>>(false),
                // 8: healths
                storage_buffer::<Vec<f32>>(false),
                // 9: combat_targets
                storage_buffer::<Vec<i32>>(false),
                // 10: params (uniform)
                uniform_buffer::<NpcComputeParams>(false),
            ),
        ),
    );

    // Queue compute pipeline
    let shader = asset_server.load(SHADER_ASSET_PATH);
    let pipeline_id = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some(Cow::from("npc_compute_pipeline")),
        layout: vec![bind_group_layout.clone()],
        shader,
        entry_point: Some(Cow::from("main")),
        ..default()
    });

    commands.insert_resource(NpcComputePipeline {
        bind_group_layout,
        pipeline_id,
    });

    info!("NPC compute pipeline queued");
}


// =============================================================================
// BIND GROUP PREPARATION
// =============================================================================

fn prepare_npc_bind_groups(
    mut commands: Commands,
    pipeline: Option<Res<NpcComputePipeline>>,
    buffers: Option<Res<NpcGpuBuffers>>,
    params: Option<Res<NpcComputeParams>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline_cache: Res<PipelineCache>,
) {
    let Some(pipeline) = pipeline else { return };
    let Some(buffers) = buffers else { return };
    let Some(params) = params else { return };

    // Create 3 uniform buffers (one per mode) for multi-dispatch
    let layout = &pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout);
    let storage_bindings = (
        buffers.positions.as_entire_buffer_binding(),
        buffers.targets.as_entire_buffer_binding(),
        buffers.speeds.as_entire_buffer_binding(),
        buffers.grid_counts.as_entire_buffer_binding(),
        buffers.grid_data.as_entire_buffer_binding(),
        buffers.arrivals.as_entire_buffer_binding(),
        buffers.backoff.as_entire_buffer_binding(),
        buffers.factions.as_entire_buffer_binding(),
        buffers.healths.as_entire_buffer_binding(),
        buffers.combat_targets.as_entire_buffer_binding(),
    );

    let mut p0 = params.clone(); p0.mode = 0;
    let mut p1 = params.clone(); p1.mode = 1;
    let mut p2 = params.clone(); p2.mode = 2;

    let mut ub0 = UniformBuffer::from(p0);
    let mut ub1 = UniformBuffer::from(p1);
    let mut ub2 = UniformBuffer::from(p2);
    ub0.write_buffer(&render_device, &render_queue);
    ub1.write_buffer(&render_device, &render_queue);
    ub2.write_buffer(&render_device, &render_queue);

    let mode0 = render_device.create_bind_group(
        Some("npc_compute_bg_mode0"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(), storage_bindings.1.clone(),
            storage_bindings.2.clone(), storage_bindings.3.clone(),
            storage_bindings.4.clone(), storage_bindings.5.clone(),
            storage_bindings.6.clone(), storage_bindings.7.clone(),
            storage_bindings.8.clone(), storage_bindings.9.clone(),
            &ub0,
        )),
    );
    let mode1 = render_device.create_bind_group(
        Some("npc_compute_bg_mode1"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(), storage_bindings.1.clone(),
            storage_bindings.2.clone(), storage_bindings.3.clone(),
            storage_bindings.4.clone(), storage_bindings.5.clone(),
            storage_bindings.6.clone(), storage_bindings.7.clone(),
            storage_bindings.8.clone(), storage_bindings.9.clone(),
            &ub1,
        )),
    );
    let mode2 = render_device.create_bind_group(
        Some("npc_compute_bg_mode2"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(), storage_bindings.1.clone(),
            storage_bindings.2.clone(), storage_bindings.3.clone(),
            storage_bindings.4.clone(), storage_bindings.5.clone(),
            storage_bindings.6.clone(), storage_bindings.7.clone(),
            storage_bindings.8.clone(), storage_bindings.9.clone(),
            &ub2,
        )),
    );

    commands.insert_resource(NpcBindGroups { mode0, mode1, mode2 });
}


// =============================================================================
// BUFFER WRITING
// =============================================================================

/// Write NPC data from extracted resource to GPU buffers.
/// Runs in render world before bind group preparation.
fn write_npc_buffers(
    buffers: Option<Res<NpcGpuBuffers>>,
    buffer_writes: Option<Res<NpcBufferWrites>>,
    render_queue: Res<RenderQueue>,
) {
    let Some(buffers) = buffers else { return };
    let Some(writes) = buffer_writes else { return };

    // Skip if no changes this frame
    if !writes.dirty {
        return;
    }

    // Debug: log first 5 NPCs data once
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        for i in 0..5 {
            let px = writes.positions.get(i * 2).copied().unwrap_or(-1.0);
            let py = writes.positions.get(i * 2 + 1).copied().unwrap_or(-1.0);
            let sc = writes.sprite_indices.get(i * 4).copied().unwrap_or(-1.0);
            let sr = writes.sprite_indices.get(i * 4 + 1).copied().unwrap_or(-1.0);
            let cr = writes.colors.get(i * 4).copied().unwrap_or(-1.0);
            let cg = writes.colors.get(i * 4 + 1).copied().unwrap_or(-1.0);
            let cb = writes.colors.get(i * 4 + 2).copied().unwrap_or(-1.0);
            info!("NPC[{}] pos=({:.0},{:.0}) sprite=({:.0},{:.0}) color=({:.2},{:.2},{:.2})",
                  i, px, py, sc, sr, cr, cg, cb);
        }
    }

    // Only upload fields that actually changed — avoids overwriting GPU-computed positions
    if writes.positions_dirty {
        render_queue.write_buffer(
            &buffers.positions,
            0,
            bytemuck::cast_slice(&writes.positions),
        );
    }

    if writes.targets_dirty {
        render_queue.write_buffer(
            &buffers.targets,
            0,
            bytemuck::cast_slice(&writes.targets),
        );
    }

    if writes.speeds_dirty {
        render_queue.write_buffer(
            &buffers.speeds,
            0,
            bytemuck::cast_slice(&writes.speeds),
        );
    }

    if writes.factions_dirty {
        render_queue.write_buffer(
            &buffers.factions,
            0,
            bytemuck::cast_slice(&writes.factions),
        );
    }

    if writes.healths_dirty {
        render_queue.write_buffer(
            &buffers.healths,
            0,
            bytemuck::cast_slice(&writes.healths),
        );
    }

    if writes.arrivals_dirty {
        render_queue.write_buffer(
            &buffers.arrivals,
            0,
            bytemuck::cast_slice(&writes.arrivals),
        );
    }

}

/// Read back NPC positions from GPU staging buffer to CPU.
/// Runs in render world after command submission (Cleanup phase).
fn readback_npc_positions(
    buffers: Option<Res<NpcGpuBuffers>>,
    gpu_data: Option<Res<NpcGpuData>>,
    render_device: Res<RenderDevice>,
) {
    let Some(buffers) = buffers else { return };
    let Some(gpu_data) = gpu_data else { return };

    let npc_count = gpu_data.npc_count as usize;
    if npc_count == 0 {
        return;
    }

    // Map both staging buffers (positions + combat targets)
    let pos_size = npc_count * std::mem::size_of::<[f32; 2]>();
    let ct_size = npc_count * std::mem::size_of::<i32>();

    let pos_slice = buffers.position_staging.slice(..pos_size as u64);
    let ct_slice = buffers.combat_target_staging.slice(..ct_size as u64);

    let (tx1, rx1) = std::sync::mpsc::sync_channel(1);
    let (tx2, rx2) = std::sync::mpsc::sync_channel(1);

    pos_slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx1.send(r); });
    ct_slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx2.send(r); });

    // Single poll flushes both maps
    let _ = render_device.poll(wgpu::PollType::wait_indefinitely());

    let pos_ok = rx1.recv().map_or(false, |r| r.is_ok());
    let ct_ok = rx2.recv().map_or(false, |r| r.is_ok());

    if pos_ok && ct_ok {
        let pos_data = pos_slice.get_mapped_range();
        let ct_data = ct_slice.get_mapped_range();
        let positions: &[f32] = bytemuck::cast_slice(&pos_data);
        let combat_targets: &[i32] = bytemuck::cast_slice(&ct_data);

        if let Ok(mut state) = GPU_READ_STATE.lock() {
            state.positions.clear();
            state.positions.extend_from_slice(&positions[..npc_count * 2]);
            state.combat_targets.clear();
            state.combat_targets.extend_from_slice(&combat_targets[..npc_count]);
            state.npc_count = npc_count;
        }

        drop(pos_data);
        drop(ct_data);
    }

    // Always unmap what was successfully mapped
    if pos_ok { buffers.position_staging.unmap(); }
    if ct_ok { buffers.combat_target_staging.unmap(); }
}

/// Read back projectile hit results from GPU → PROJ_HIT_STATE static.
/// Read back projectile hits AND positions from GPU staging buffers.
/// Single poll for both maps (same pattern as NPC readback).
fn readback_proj_data(
    proj_buffers: Option<Res<ProjGpuBuffers>>,
    proj_data: Option<Res<ProjGpuData>>,
    render_device: Res<RenderDevice>,
) {
    let Some(buffers) = proj_buffers else { return };
    let Some(data) = proj_data else { return };

    let proj_count = data.proj_count as usize;
    if proj_count == 0 { return; }

    let hit_size = proj_count * std::mem::size_of::<[i32; 2]>();
    let pos_size = proj_count * std::mem::size_of::<[f32; 2]>();

    let hit_slice = buffers.hit_staging.slice(..hit_size as u64);
    let pos_slice = buffers.position_staging.slice(..pos_size as u64);

    let (tx1, rx1) = std::sync::mpsc::sync_channel(1);
    let (tx2, rx2) = std::sync::mpsc::sync_channel(1);
    hit_slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx1.send(r); });
    pos_slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx2.send(r); });

    // Single poll flushes both maps
    let _ = render_device.poll(wgpu::PollType::wait_indefinitely());

    if let Ok(Ok(())) = rx1.recv() {
        let mapped = hit_slice.get_mapped_range();
        let hits: &[[i32; 2]] = bytemuck::cast_slice(&mapped);
        if let Ok(mut state) = PROJ_HIT_STATE.lock() {
            state.clear();
            state.extend_from_slice(&hits[..proj_count]);
        }
        drop(mapped);
        buffers.hit_staging.unmap();
    }

    if let Ok(Ok(())) = rx2.recv() {
        let mapped = pos_slice.get_mapped_range();
        let positions: &[f32] = bytemuck::cast_slice(&mapped);
        if let Ok(mut state) = crate::messages::PROJ_POSITION_STATE.lock() {
            state.clear();
            state.extend_from_slice(&positions[..proj_count * 2]);
        }
        drop(mapped);
        buffers.position_staging.unmap();
    }
}

// =============================================================================
// RENDER GRAPH NODE
// =============================================================================

enum NpcComputeState {
    Loading,
    Ready,
}

struct NpcComputeNode {
    state: NpcComputeState,
}

impl Default for NpcComputeNode {
    fn default() -> Self {
        Self {
            state: NpcComputeState::Loading,
        }
    }
}

impl render_graph::Node for NpcComputeNode {
    fn update(&mut self, world: &mut World) {
        let Some(pipeline) = world.get_resource::<NpcComputePipeline>() else {
            return;
        };
        let pipeline_cache = world.resource::<PipelineCache>();

        match self.state {
            NpcComputeState::Loading => {
                match pipeline_cache.get_compute_pipeline_state(pipeline.pipeline_id) {
                    CachedPipelineState::Ok(_) => {
                        self.state = NpcComputeState::Ready;
                        info!("NPC compute pipeline ready");
                    }
                    CachedPipelineState::Err(PipelineCacheError::ShaderNotLoaded(_)) => {}
                    CachedPipelineState::Err(err) => {
                        panic!("NPC compute shader error: {err}")
                    }
                    _ => {}
                }
            }
            NpcComputeState::Ready => {}
        }
    }

    fn run(
        &self,
        _graph: &mut render_graph::RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), render_graph::NodeRunError> {
        // Only run if ready
        if !matches!(self.state, NpcComputeState::Ready) {
            return Ok(());
        }

        let Some(bind_groups) = world.get_resource::<NpcBindGroups>() else {
            return Ok(());
        };
        let Some(gpu_data) = world.get_resource::<NpcGpuData>() else {
            return Ok(());
        };
        let pipeline_cache = world.resource::<PipelineCache>();
        let pipeline = world.resource::<NpcComputePipeline>();

        let npc_count = gpu_data.npc_count;
        if npc_count == 0 {
            return Ok(());
        }

        let Some(compute_pipeline) = pipeline_cache.get_compute_pipeline(pipeline.pipeline_id)
        else {
            return Ok(());
        };

        let grid_cells = GRID_WIDTH * GRID_HEIGHT;
        let grid_wg = (grid_cells + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
        let npc_wg = (npc_count + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;

        // Pass 0: Clear spatial grid
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_bind_group(0, &bind_groups.mode0, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(grid_wg, 1, 1);
        }

        // Pass 1: Build spatial grid (insert NPCs into cells)
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_bind_group(0, &bind_groups.mode1, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(npc_wg, 1, 1);
        }

        // Pass 2: Movement + combat targeting
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_bind_group(0, &bind_groups.mode2, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(npc_wg, 1, 1);
        }

        // Copy positions + combat_targets → staging buffers for CPU readback
        let buffers = world.resource::<NpcGpuBuffers>();
        let pos_copy_size = (npc_count as u64) * std::mem::size_of::<[f32; 2]>() as u64;
        let ct_copy_size = (npc_count as u64) * std::mem::size_of::<i32>() as u64;

        render_context.command_encoder().copy_buffer_to_buffer(
            &buffers.positions, 0, &buffers.position_staging, 0, pos_copy_size,
        );
        render_context.command_encoder().copy_buffer_to_buffer(
            &buffers.combat_targets, 0, &buffers.combat_target_staging, 0, ct_copy_size,
        );

        Ok(())
    }
}

// =============================================================================
// PROJECTILE COMPUTE
// =============================================================================

/// Update projectile GPU data from ECS each frame.
fn update_proj_gpu_data(
    mut proj_data: ResMut<ProjGpuData>,
    mut proj_params: ResMut<ProjComputeParams>,
    npc_count: Res<crate::resources::NpcCount>,
    proj_alloc: Res<crate::resources::ProjSlotAllocator>,
    time: Res<Time>,
) {
    let pc = proj_alloc.next as u32;
    let nc = npc_count.0 as u32;
    let dt = time.delta_secs();
    proj_data.proj_count = pc;
    proj_data.npc_count = nc;
    proj_data.delta = dt;
    proj_params.proj_count = pc;
    proj_params.npc_count = nc;
    proj_params.delta = dt;
}

fn init_proj_compute_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let max = MAX_PROJECTILES as usize;

    let buffers = ProjGpuBuffers {
        positions: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_positions"),
            size: (max * std::mem::size_of::<[f32; 2]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        velocities: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_velocities"),
            size: (max * std::mem::size_of::<[f32; 2]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        damages: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_damages"),
            size: (max * std::mem::size_of::<f32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        factions: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_factions"),
            size: (max * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        shooters: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_shooters"),
            size: (max * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        lifetimes: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_lifetimes"),
            size: (max * std::mem::size_of::<f32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        active: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_active"),
            size: (max * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        hits: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_hits"),
            size: (max * std::mem::size_of::<[i32; 2]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        hit_staging: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_hit_staging"),
            size: (max * std::mem::size_of::<[i32; 2]>()) as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        position_staging: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_position_staging"),
            size: (max * std::mem::size_of::<[f32; 2]>()) as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
    };

    commands.insert_resource(buffers);

    // 14 bindings: 8 proj (rw) + 3 NPC (ro) + 2 grid (ro) + 1 uniform
    let bind_group_layout = BindGroupLayoutDescriptor::new(
        "ProjComputeLayout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                // 0-7: projectile buffers (read_write)
                storage_buffer::<Vec<[f32; 2]>>(false),  // positions
                storage_buffer::<Vec<[f32; 2]>>(false),  // velocities
                storage_buffer::<Vec<f32>>(false),        // damages
                storage_buffer::<Vec<i32>>(false),        // factions
                storage_buffer::<Vec<i32>>(false),        // shooters
                storage_buffer::<Vec<f32>>(false),        // lifetimes
                storage_buffer::<Vec<i32>>(false),        // active
                storage_buffer::<Vec<[i32; 2]>>(false),   // hits
                // 8-10: NPC buffers (read only)
                storage_buffer_read_only::<Vec<[f32; 2]>>(false), // npc_positions
                storage_buffer_read_only::<Vec<i32>>(false),      // npc_factions
                storage_buffer_read_only::<Vec<f32>>(false),      // npc_healths
                // 11-12: spatial grid (read only)
                storage_buffer_read_only::<Vec<i32>>(false),      // grid_counts
                storage_buffer_read_only::<Vec<i32>>(false),      // grid_data
                // 13: uniform params
                uniform_buffer::<ProjComputeParams>(false),
            ),
        ),
    );

    let shader = asset_server.load(PROJ_SHADER_ASSET_PATH);
    let pipeline_id = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some(Cow::from("proj_compute_pipeline")),
        layout: vec![bind_group_layout.clone()],
        shader,
        entry_point: Some(Cow::from("main")),
        ..default()
    });

    commands.insert_resource(ProjComputePipeline {
        bind_group_layout,
        pipeline_id,
    });

    info!("Projectile compute pipeline queued");
}

fn prepare_proj_bind_groups(
    mut commands: Commands,
    pipeline: Option<Res<ProjComputePipeline>>,
    proj_buffers: Option<Res<ProjGpuBuffers>>,
    npc_buffers: Option<Res<NpcGpuBuffers>>,
    params: Option<Res<ProjComputeParams>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline_cache: Res<PipelineCache>,
) {
    let Some(pipeline) = pipeline else { return };
    let Some(proj) = proj_buffers else { return };
    let Some(npc) = npc_buffers else { return };
    let Some(params) = params else { return };

    let mut uniform_buffer = UniformBuffer::from(params.clone());
    uniform_buffer.write_buffer(&render_device, &render_queue);

    let bind_group = render_device.create_bind_group(
        Some("proj_compute_bind_group"),
        &pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout),
        &BindGroupEntries::sequential((
            // 0-7: projectile buffers
            proj.positions.as_entire_buffer_binding(),
            proj.velocities.as_entire_buffer_binding(),
            proj.damages.as_entire_buffer_binding(),
            proj.factions.as_entire_buffer_binding(),
            proj.shooters.as_entire_buffer_binding(),
            proj.lifetimes.as_entire_buffer_binding(),
            proj.active.as_entire_buffer_binding(),
            proj.hits.as_entire_buffer_binding(),
            // 8-10: NPC buffers (shared, read only)
            npc.positions.as_entire_buffer_binding(),
            npc.factions.as_entire_buffer_binding(),
            npc.healths.as_entire_buffer_binding(),
            // 11-12: spatial grid (shared, read only)
            npc.grid_counts.as_entire_buffer_binding(),
            npc.grid_data.as_entire_buffer_binding(),
            // 13: uniform
            &uniform_buffer,
        )),
    );

    commands.insert_resource(ProjBindGroups { bind_group });
}

/// Write projectile data from extracted resource to GPU buffers.
fn write_proj_buffers(
    buffers: Option<Res<ProjGpuBuffers>>,
    writes: Option<Res<ProjBufferWrites>>,
    render_queue: Res<RenderQueue>,
) {
    let Some(buffers) = buffers else { return };
    let Some(writes) = writes else { return };

    if !writes.dirty {
        return;
    }

    render_queue.write_buffer(&buffers.positions, 0, bytemuck::cast_slice(&writes.positions));
    render_queue.write_buffer(&buffers.velocities, 0, bytemuck::cast_slice(&writes.velocities));
    render_queue.write_buffer(&buffers.damages, 0, bytemuck::cast_slice(&writes.damages));
    render_queue.write_buffer(&buffers.factions, 0, bytemuck::cast_slice(&writes.factions));
    render_queue.write_buffer(&buffers.shooters, 0, bytemuck::cast_slice(&writes.shooters));
    render_queue.write_buffer(&buffers.lifetimes, 0, bytemuck::cast_slice(&writes.lifetimes));
    render_queue.write_buffer(&buffers.active, 0, bytemuck::cast_slice(&writes.active));
    render_queue.write_buffer(&buffers.hits, 0, bytemuck::cast_slice(&writes.hits));
}

enum ProjComputeState {
    Loading,
    Ready,
}

struct ProjectileComputeNode {
    state: ProjComputeState,
}

impl Default for ProjectileComputeNode {
    fn default() -> Self {
        Self { state: ProjComputeState::Loading }
    }
}

impl render_graph::Node for ProjectileComputeNode {
    fn update(&mut self, world: &mut World) {
        let Some(pipeline) = world.get_resource::<ProjComputePipeline>() else {
            return;
        };
        let pipeline_cache = world.resource::<PipelineCache>();

        match self.state {
            ProjComputeState::Loading => {
                match pipeline_cache.get_compute_pipeline_state(pipeline.pipeline_id) {
                    CachedPipelineState::Ok(_) => {
                        self.state = ProjComputeState::Ready;
                        info!("Projectile compute pipeline ready");
                    }
                    CachedPipelineState::Err(PipelineCacheError::ShaderNotLoaded(_)) => {}
                    CachedPipelineState::Err(err) => {
                        panic!("Projectile compute shader error: {err}")
                    }
                    _ => {}
                }
            }
            ProjComputeState::Ready => {}
        }
    }

    fn run(
        &self,
        _graph: &mut render_graph::RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), render_graph::NodeRunError> {
        if !matches!(self.state, ProjComputeState::Ready) {
            return Ok(());
        }

        let Some(bind_groups) = world.get_resource::<ProjBindGroups>() else {
            return Ok(());
        };
        let Some(proj_data) = world.get_resource::<ProjGpuData>() else {
            return Ok(());
        };
        let pipeline_cache = world.resource::<PipelineCache>();
        let pipeline = world.resource::<ProjComputePipeline>();

        let proj_count = proj_data.proj_count;
        if proj_count == 0 {
            return Ok(());
        }

        let Some(compute_pipeline) = pipeline_cache.get_compute_pipeline(pipeline.pipeline_id)
        else {
            return Ok(());
        };

        // Scope compute pass so it drops before we use the encoder for copy
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());

            pass.set_bind_group(0, &bind_groups.bind_group, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(
                (proj_count + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE,
                1,
                1,
            );
        }

        // Copy hits + positions → staging buffers for CPU readback
        let proj_buffers = world.resource::<ProjGpuBuffers>();
        let hit_copy_size = (proj_count as u64) * std::mem::size_of::<[i32; 2]>() as u64;
        render_context.command_encoder().copy_buffer_to_buffer(
            &proj_buffers.hits, 0, &proj_buffers.hit_staging, 0, hit_copy_size,
        );
        let pos_copy_size = (proj_count as u64) * std::mem::size_of::<[f32; 2]>() as u64;
        render_context.command_encoder().copy_buffer_to_buffer(
            &proj_buffers.positions, 0, &proj_buffers.position_staging, 0, pos_copy_size,
        );

        Ok(())
    }
}
