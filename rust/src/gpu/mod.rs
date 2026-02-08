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
        render_phase::TrackedRenderPass,
        render_resource::{
            binding_types::{
                sampler, storage_buffer, storage_buffer_read_only, texture_2d, uniform_buffer,
            },
            *,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        view::ViewTarget,
        Render, RenderApp, RenderStartup, RenderSystems,
    },
    shader::PipelineCacheError,
};
use std::borrow::Cow;

use bevy::render::texture::GpuImage;
use bevy::render::render_asset::RenderAssets;

use crate::messages::{GpuUpdate, GPU_UPDATE_QUEUE};

// =============================================================================
// CONSTANTS
// =============================================================================

const SHADER_ASSET_PATH: &str = "shaders/npc_compute.wgsl";
const RENDER_SHADER_PATH: &str = "shaders/npc_render.wgsl";
const WORKGROUP_SIZE: u32 = 64;
const MAX_NPCS: u32 = 16384;
const GRID_WIDTH: u32 = 128;
const GRID_HEIGHT: u32 = 128;
const MAX_PER_CELL: u32 = 48;

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
    pub _pad1: f32,
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
            _pad1: 0.0,
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
    /// Sprite indices: [col, row, 0, 0] per NPC (vec4 for alignment)
    pub sprite_indices: Vec<f32>,
    /// Colors: [r, g, b, a] per NPC
    pub colors: Vec<f32>,
    /// Whether any data changed this frame (skip upload if false)
    pub dirty: bool,
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
            sprite_indices: vec![0.0; max * 4], // vec4 per NPC
            colors: vec![1.0; max * 4],          // RGBA, default white
            dirty: false,
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
                }
            }
            GpuUpdate::SetTarget { idx, x, y } => {
                let i = *idx * 2;
                if i + 1 < self.targets.len() {
                    self.targets[i] = *x;
                    self.targets[i + 1] = *y;
                    self.dirty = true;
                }
            }
            GpuUpdate::SetSpeed { idx, speed } => {
                if *idx < self.speeds.len() {
                    self.speeds[*idx] = *speed;
                    self.dirty = true;
                }
            }
            GpuUpdate::SetFaction { idx, faction } => {
                if *idx < self.factions.len() {
                    self.factions[*idx] = *faction;
                    self.dirty = true;
                }
            }
            GpuUpdate::SetHealth { idx, health } => {
                if *idx < self.healths.len() {
                    self.healths[*idx] = *health;
                    self.dirty = true;
                }
            }
            GpuUpdate::ApplyDamage { idx, amount } => {
                if *idx < self.healths.len() {
                    self.healths[*idx] = (self.healths[*idx] - amount).max(0.0);
                    self.dirty = true;
                }
            }
            GpuUpdate::HideNpc { idx } => {
                // Move to offscreen position
                let i = *idx * 2;
                if i + 1 < self.positions.len() {
                    self.positions[i] = -9999.0;
                    self.positions[i + 1] = -9999.0;
                    self.dirty = true;
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
            // These don't affect GPU buffers (visual effects handled separately)
            GpuUpdate::SetHealing { .. } |
            GpuUpdate::SetCarriedItem { .. } => {}
        }
    }
}

/// Drain GPU_UPDATE_QUEUE and apply updates to NpcBufferWrites.
/// Runs in main world each frame before extraction.
pub fn populate_buffer_writes(mut buffer_writes: ResMut<NpcBufferWrites>) {
    // Reset dirty flag - will be set if any updates applied
    buffer_writes.dirty = false;

    if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
        for update in queue.drain(..) {
            buffer_writes.apply(&update);
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
        // Initialize resources in main world
        app.init_resource::<NpcGpuData>()
            .init_resource::<NpcComputeParams>()
            .init_resource::<NpcBufferWrites>()
            .init_resource::<NpcSpriteTexture>()
            .add_systems(Update, update_gpu_data)
            .add_systems(PostUpdate, populate_buffer_writes);

        // Extract resources to render world
        app.add_plugins((
            ExtractResourcePlugin::<NpcGpuData>::default(),
            ExtractResourcePlugin::<NpcComputeParams>::default(),
            ExtractResourcePlugin::<NpcBufferWrites>::default(),
            ExtractResourcePlugin::<NpcSpriteTexture>::default(),
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
            .add_systems(RenderStartup, init_npc_compute_pipeline)
            .add_systems(
                Render,
                (
                    write_npc_buffers.in_set(RenderSystems::PrepareResources),
                    (
                        prepare_npc_bind_groups,
                        prepare_npc_texture_bind_group,
                        prepare_npc_render_bind_groups,
                    ).chain().in_set(RenderSystems::PrepareBindGroups),
                ),
            );

        // Add compute node to render graph
        {
            let world = render_app.world_mut();
            let render_node = NpcRenderNode::from_world(world);
            let mut render_graph = world.resource_mut::<RenderGraph>();
            render_graph.add_node(NpcComputeLabel, NpcComputeNode::default());
            render_graph.add_node(NpcRenderLabel, render_node);

            // Chain: Compute → Camera Driver → NPC Render (overlay on top)
            // NPC render must come AFTER camera driver to not be overwritten by Bevy's clear
            render_graph.add_node_edge(NpcComputeLabel, bevy::render::graph::CameraDriverLabel);
            render_graph.add_node_edge(bevy::render::graph::CameraDriverLabel, NpcRenderLabel);
        }

        info!("GPU compute plugin initialized");
    }
}

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
    // Render buffers
    pub sprite_indices: Buffer,  // vec4<f32> per NPC: xy=col/row
    pub colors: Buffer,          // vec4<f32> per NPC: RGBA
}

/// Bind groups for compute passes.
#[derive(Resource)]
struct NpcBindGroups {
    /// Bind group for all three modes
    bind_group: BindGroup,
}

/// Pipeline resources for compute.
#[derive(Resource)]
struct NpcComputePipeline {
    bind_group_layout: BindGroupLayoutDescriptor,
    pipeline_id: CachedComputePipelineId,
}

/// Mesh resources for instanced NPC rendering.
#[derive(Resource)]
pub struct NpcRenderMesh {
    /// Quad vertex buffer: 4 vertices with position (vec2) and uv (vec2)
    pub vertex_buffer: Buffer,
    /// Index buffer: 6 indices for 2 triangles
    pub index_buffer: Buffer,
}

/// Render pipeline for instanced NPCs.
#[derive(Resource)]
pub struct NpcRenderPipeline {
    pub pipeline_id: CachedRenderPipelineId,
    pub instance_bind_group_layout: BindGroupLayout,
    pub texture_bind_group_layout: BindGroupLayout,
}

/// Bind groups for NPC rendering.
#[derive(Resource)]
pub struct NpcRenderBindGroups {
    pub instance_bind_group: BindGroup,
    pub texture_bind_group: Option<BindGroup>,
}

/// Sprite texture bind group (created when texture is extracted).
#[derive(Resource, Clone)]
pub struct NpcSpriteTextureBindGroup {
    pub bind_group: BindGroup,
}

/// Handle to the NPC sprite texture (main world).
/// Set by the render module after loading sprite sheets.
#[derive(Resource, Clone, ExtractResource, Default)]
pub struct NpcSpriteTexture {
    pub handle: Option<Handle<Image>>,
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
        // Render buffers
        sprite_indices: render_device.create_buffer(&BufferDescriptor {
            label: Some("sprite_indices"),
            size: (MAX_NPCS as usize * std::mem::size_of::<[f32; 4]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        colors: render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_colors"),
            size: (MAX_NPCS as usize * std::mem::size_of::<[f32; 4]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
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

    // Create quad mesh for instanced rendering
    // Vertex layout: [x, y, u, v] per vertex
    #[rustfmt::skip]
    let quad_vertices: [[f32; 4]; 4] = [
        [-0.5, -0.5, 0.0, 1.0], // bottom-left
        [ 0.5, -0.5, 1.0, 1.0], // bottom-right
        [ 0.5,  0.5, 1.0, 0.0], // top-right
        [-0.5,  0.5, 0.0, 0.0], // top-left
    ];
    let vertex_data: Vec<u8> = quad_vertices
        .iter()
        .flat_map(|v| v.iter().flat_map(|f| f.to_le_bytes()))
        .collect();

    let vertex_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("npc_quad_vertices"),
        contents: &vertex_data,
        usage: BufferUsages::VERTEX,
    });

    // Two triangles: 0-1-2, 0-2-3
    let quad_indices: [u16; 6] = [0, 1, 2, 0, 2, 3];
    let index_data: Vec<u8> = quad_indices
        .iter()
        .flat_map(|i| i.to_le_bytes())
        .collect();

    let index_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("npc_quad_indices"),
        contents: &index_data,
        usage: BufferUsages::INDEX,
    });

    commands.insert_resource(NpcRenderMesh {
        vertex_buffer,
        index_buffer,
    });

    // Create render pipeline for instanced NPCs
    init_npc_render_pipeline(&mut commands, &render_device, &asset_server, &pipeline_cache);

    info!("NPC compute pipeline queued");
}

/// Initialize the render pipeline for instanced NPC sprites.
fn init_npc_render_pipeline(
    commands: &mut Commands,
    _render_device: &RenderDevice,
    asset_server: &AssetServer,
    pipeline_cache: &PipelineCache,
) {
    // Instance data bind group layout (group 0 for our shader)
    // Matches shader: positions, sprite_indices, colors
    let instance_layout_desc = BindGroupLayoutDescriptor::new(
        "NpcRenderInstanceLayout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX | ShaderStages::FRAGMENT,
            (
                // 0: positions - vec2<f32> per NPC
                storage_buffer_read_only::<Vec<[f32; 2]>>(false),
                // 1: sprite_indices - vec4<f32> per NPC
                storage_buffer_read_only::<Vec<[f32; 4]>>(false),
                // 2: colors - vec4<f32> per NPC
                storage_buffer_read_only::<Vec<[f32; 4]>>(false),
            ),
        ),
    );

    // Texture bind group layout (group 1 for our shader)
    let texture_layout_desc = BindGroupLayoutDescriptor::new(
        "NpcRenderTextureLayout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                // 0: sprite texture
                texture_2d(TextureSampleType::Float { filterable: true }),
                // 1: sampler
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );

    // Also create actual bind group layouts for later use
    let instance_bind_group_layout = pipeline_cache.get_bind_group_layout(&instance_layout_desc);
    let texture_bind_group_layout = pipeline_cache.get_bind_group_layout(&texture_layout_desc);

    // Load shader
    let shader = asset_server.load(RENDER_SHADER_PATH);

    // Create render pipeline
    let pipeline_id = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some(Cow::from("npc_render_pipeline")),
        layout: vec![
            // Group 0: Instance data (positions, sprites, colors)
            instance_layout_desc,
            // Group 1: Texture
            texture_layout_desc,
        ],
        vertex: VertexState {
            shader: shader.clone(),
            entry_point: Some(Cow::from("vertex")),
            shader_defs: vec![],
            buffers: vec![bevy::mesh::VertexBufferLayout::from_vertex_formats(
                VertexStepMode::Vertex,
                vec![
                    VertexFormat::Float32x2, // position
                    VertexFormat::Float32x2, // uv
                ],
            )],
        },
        fragment: Some(FragmentState {
            shader,
            entry_point: Some(Cow::from("fragment")),
            shader_defs: vec![],
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba8UnormSrgb,  // Match ViewTarget format
                blend: Some(BlendState::ALPHA_BLENDING),
                write_mask: ColorWrites::ALL,
            })],
        }),
        primitive: PrimitiveState {
            topology: PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: FrontFace::Ccw,
            cull_mode: None, // No culling for 2D sprites
            unclipped_depth: false,
            polygon_mode: PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: MultisampleState::default(),
        push_constant_ranges: vec![],
        zero_initialize_workgroup_memory: false,
    });

    commands.insert_resource(NpcRenderPipeline {
        pipeline_id,
        instance_bind_group_layout,
        texture_bind_group_layout,
    });

    info!("NPC render pipeline queued");
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

    // Write params to uniform buffer
    let mut uniform_buffer = UniformBuffer::from(params.clone());
    uniform_buffer.write_buffer(&render_device, &render_queue);

    let bind_group = render_device.create_bind_group(
        Some("npc_compute_bind_group"),
        &pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout),
        &BindGroupEntries::sequential((
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
            &uniform_buffer,
        )),
    );

    commands.insert_resource(NpcBindGroups { bind_group });
}

/// Create texture bind group from extracted sprite texture.
fn prepare_npc_texture_bind_group(
    mut commands: Commands,
    render_pipeline: Option<Res<NpcRenderPipeline>>,
    sprite_texture: Option<Res<NpcSpriteTexture>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_device: Res<RenderDevice>,
) {
    let Some(pipeline) = render_pipeline else { return };
    let Some(sprite_tex) = sprite_texture else { return };
    let Some(handle) = &sprite_tex.handle else { return };

    // Try to get the GPU image
    let Some(gpu_image) = gpu_images.get(handle) else { return };

    // Create texture bind group
    let bind_group = render_device.create_bind_group(
        Some("npc_sprite_texture_bind_group"),
        &pipeline.texture_bind_group_layout,
        &BindGroupEntries::sequential((
            &gpu_image.texture_view,
            &gpu_image.sampler,
        )),
    );

    commands.insert_resource(NpcSpriteTextureBindGroup { bind_group });
}

/// Prepare bind groups for NPC instanced rendering.
fn prepare_npc_render_bind_groups(
    mut commands: Commands,
    render_pipeline: Option<Res<NpcRenderPipeline>>,
    buffers: Option<Res<NpcGpuBuffers>>,
    render_device: Res<RenderDevice>,
    sprite_texture: Option<Res<NpcSpriteTextureBindGroup>>,
) {
    let Some(pipeline) = render_pipeline else { return };
    let Some(buffers) = buffers else { return };

    // Create instance data bind group
    let instance_bind_group = render_device.create_bind_group(
        Some("npc_render_instance_bind_group"),
        &pipeline.instance_bind_group_layout,
        &BindGroupEntries::sequential((
            buffers.positions.as_entire_buffer_binding(),
            buffers.sprite_indices.as_entire_buffer_binding(),
            buffers.colors.as_entire_buffer_binding(),
        )),
    );

    // Get texture bind group if available
    let texture_bind_group = sprite_texture.map(|t| t.bind_group.clone());

    commands.insert_resource(NpcRenderBindGroups {
        instance_bind_group,
        texture_bind_group,
    });
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

    // Write position buffer (Vec<f32> → &[u8])
    render_queue.write_buffer(
        &buffers.positions,
        0,
        bytemuck::cast_slice(&writes.positions),
    );

    // Write target buffer
    render_queue.write_buffer(
        &buffers.targets,
        0,
        bytemuck::cast_slice(&writes.targets),
    );

    // Write speed buffer
    render_queue.write_buffer(
        &buffers.speeds,
        0,
        bytemuck::cast_slice(&writes.speeds),
    );

    // Write faction buffer (i32 → &[u8])
    render_queue.write_buffer(
        &buffers.factions,
        0,
        bytemuck::cast_slice(&writes.factions),
    );

    // Write health buffer
    render_queue.write_buffer(
        &buffers.healths,
        0,
        bytemuck::cast_slice(&writes.healths),
    );

    // Write sprite indices buffer
    render_queue.write_buffer(
        &buffers.sprite_indices,
        0,
        bytemuck::cast_slice(&writes.sprite_indices),
    );

    // Write colors buffer
    render_queue.write_buffer(
        &buffers.colors,
        0,
        bytemuck::cast_slice(&writes.colors),
    );
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

        let _grid_cells = GRID_WIDTH * GRID_HEIGHT;

        // TODO: For now, just dispatch mode 2 (main logic)
        // Full implementation needs separate dispatches for mode 0, 1, 2
        // with uniform buffer updates between them
        let mut pass = render_context
            .command_encoder()
            .begin_compute_pass(&ComputePassDescriptor::default());

        pass.set_bind_group(0, &bind_groups.bind_group, &[]);
        pass.set_pipeline(compute_pipeline);
        pass.dispatch_workgroups(
            (npc_count + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE,
            1,
            1,
        );

        Ok(())
    }
}

// =============================================================================
// RENDER NODE (Instanced Sprite Rendering)
// =============================================================================

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct NpcRenderLabel;

enum NpcRenderState {
    Loading,
    Ready,
}

struct NpcRenderNode {
    state: NpcRenderState,
    view_query: QueryState<(Entity, &'static ViewTarget)>,
}

impl FromWorld for NpcRenderNode {
    fn from_world(world: &mut World) -> Self {
        Self {
            state: NpcRenderState::Loading,
            view_query: QueryState::new(world),
        }
    }
}

impl render_graph::Node for NpcRenderNode {
    fn update(&mut self, world: &mut World) {
        // Update query to see newly spawned ViewTarget entities
        self.view_query.update_archetypes(world);

        let Some(pipeline) = world.get_resource::<NpcRenderPipeline>() else {
            return;
        };
        let pipeline_cache = world.resource::<PipelineCache>();

        match self.state {
            NpcRenderState::Loading => {
                match pipeline_cache.get_render_pipeline_state(pipeline.pipeline_id) {
                    CachedPipelineState::Ok(_) => {
                        self.state = NpcRenderState::Ready;
                        info!("NPC render pipeline ready");
                    }
                    CachedPipelineState::Err(PipelineCacheError::ShaderNotLoaded(_)) => {}
                    CachedPipelineState::Err(err) => {
                        warn!("NPC render shader error: {err}");
                        // Don't panic - rendering is optional, compute still works
                    }
                    _ => {}
                }
            }
            NpcRenderState::Ready => {}
        }
    }

    fn run(
        &self,
        _graph: &mut render_graph::RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), render_graph::NodeRunError> {
        // Only run if ready
        if !matches!(self.state, NpcRenderState::Ready) {
            return Ok(());
        }

        let Some(render_bind_groups) = world.get_resource::<NpcRenderBindGroups>() else {
            return Ok(());
        };
        let Some(gpu_data) = world.get_resource::<NpcGpuData>() else {
            return Ok(());
        };
        let Some(mesh) = world.get_resource::<NpcRenderMesh>() else {
            return Ok(());
        };
        let pipeline_cache = world.resource::<PipelineCache>();
        let pipeline = world.resource::<NpcRenderPipeline>();

        let npc_count = gpu_data.npc_count;
        if npc_count == 0 {
            return Ok(());
        }

        let Some(render_pipeline) = pipeline_cache.get_render_pipeline(pipeline.pipeline_id)
        else {
            return Ok(());
        };

        // Skip if texture bind group not ready
        let Some(texture_bind_group) = &render_bind_groups.texture_bind_group else {
            return Ok(());
        };

        let render_device = world.resource::<RenderDevice>();

        // Iterate over extracted camera views using stored query
        for (_entity, view_target) in self.view_query.iter_manual(world) {
            // Get color attachment for this view
            let color_attachment = RenderPassColorAttachment {
                view: view_target.main_texture_view(),
                resolve_target: None,
                ops: Operations {
                    // Load existing content (don't clear - Bevy's camera already cleared)
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
                depth_slice: None,
            };

            let raw_render_pass = render_context
                .command_encoder()
                .begin_render_pass(&RenderPassDescriptor {
                    label: Some("npc_instanced_render_pass"),
                    color_attachments: &[Some(color_attachment)],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

            // Wrap in TrackedRenderPass to use Bevy's Buffer types
            let mut render_pass = TrackedRenderPass::new(&render_device, raw_render_pass);

            // Set pipeline and bind groups
            render_pass.set_render_pipeline(render_pipeline);
            render_pass.set_bind_group(0, &render_bind_groups.instance_bind_group, &[]);
            render_pass.set_bind_group(1, texture_bind_group, &[]);

            // Set vertex and index buffers
            render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            render_pass.set_index_buffer(mesh.index_buffer.slice(..), IndexFormat::Uint16);

            // Draw instanced: 6 indices (2 triangles), npc_count instances
            render_pass.draw_indexed(0..6, 0, 0..npc_count);
        }

        // Log once when first frame renders
        static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let view_count = self.view_query.iter_manual(world).count();
            info!("NPC instanced render: {} NPCs drawn to {} views", npc_count, view_count);
        }

        Ok(())
    }
}
