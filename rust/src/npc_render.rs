//! Universal Instanced Rendering via Bevy's RenderCommand pattern.
//!
//! Renders terrain, buildings, NPCs, equipment, and projectiles with instanced draw calls.
//! Based on Bevy's custom_phase_item.rs example pattern.

use std::borrow::Cow;

use bevy::{
    core_pipeline::core_2d::Transparent2d,
    ecs::{
        query::ROQueryItem,
        system::{lifetimeless::SRes, SystemParamItem},
    },
    math::FloatOrd,
    mesh::VertexBufferLayout,
    prelude::*,
    render::{
        render_asset::RenderAssets,
        render_phase::{
            AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand,
            RenderCommandResult, SetItemPipeline, TrackedRenderPass, ViewSortedRenderPhases,
        },
        render_resource::{
            binding_types::{sampler, texture_2d, uniform_buffer},
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            BlendState, Buffer, BufferInitDescriptor, BufferUsages, ColorTargetState,
            ColorWrites, CompareFunction, DepthBiasState, DepthStencilState, FragmentState,
            IndexFormat, MultisampleState, PipelineCache, PrimitiveState, RawBufferVec,
            RenderPipelineDescriptor, SamplerBindingType, ShaderStages, ShaderType,
            SpecializedRenderPipeline, SpecializedRenderPipelines, StencilState, TextureFormat,
            TextureSampleType, UniformBuffer, VertexAttribute, VertexState, VertexStepMode,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::MainEntity,
        texture::GpuImage,
        view::ExtractedView,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        Extract, Render, RenderApp, RenderSystems,
    },
};
use bytemuck::{Pod, Zeroable};

use crate::gpu::{NpcBufferWrites, NpcGpuData, NpcSpriteTexture, ProjBufferWrites, ProjGpuData};
use crate::render::CameraState;
use crate::world::{WorldData, WorldGrid};

// =============================================================================
// MARKER COMPONENT
// =============================================================================

/// Layer count: terrain + buildings + body + 4 equipment layers.
pub const LAYER_COUNT: usize = 7;

/// Marker component for the NPC batch entity.
#[derive(Component, Clone)]
pub struct NpcBatch;

/// Marker component for the single projectile batch entity.
#[derive(Component, Clone)]
pub struct ProjBatch;

// =============================================================================
// VERTEX DATA
// =============================================================================

/// Instance data for a single NPC (sent to GPU per-instance).
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct InstanceData {
    /// World position (x, y)
    pub position: [f32; 2],
    /// Sprite atlas cell (col, row)
    pub sprite: [f32; 2],
    /// Color tint (r, g, b, a)
    pub color: [f32; 4],
    /// Health percentage (0.0-1.0), used for health bar rendering
    pub health: f32,
    /// Damage flash intensity (0.0-1.0), white overlay that fades out
    pub flash: f32,
    /// World-space quad size (16.0 for NPCs, 32.0 for terrain tiles)
    pub scale: f32,
    /// Which texture atlas to sample (0.0 = character, 1.0 = world)
    pub atlas_id: f32,
}

/// Static quad vertex: position and UV
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
struct QuadVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

/// Unit quad vertices (centered at origin, size 1x1)
static QUAD_VERTICES: [QuadVertex; 4] = [
    QuadVertex { position: [-0.5, -0.5], uv: [0.0, 1.0] }, // bottom-left
    QuadVertex { position: [ 0.5, -0.5], uv: [1.0, 1.0] }, // bottom-right
    QuadVertex { position: [ 0.5,  0.5], uv: [1.0, 0.0] }, // top-right
    QuadVertex { position: [-0.5,  0.5], uv: [0.0, 0.0] }, // top-left
];

/// Two triangles forming a quad
static QUAD_INDICES: [u16; 6] = [0, 1, 2, 0, 2, 3];

// =============================================================================
// RENDER RESOURCES
// =============================================================================

/// Per-layer instance buffer and count.
pub struct LayerBuffer {
    instances: RawBufferVec<InstanceData>,
    count: u32,
}

/// GPU buffers for NPC rendering — one layer buffer per rendering layer.
#[derive(Resource)]
pub struct NpcRenderBuffers {
    /// Static quad geometry (slot 0)
    pub vertex_buffer: Buffer,
    pub index_buffer: Buffer,
    /// Per-layer instance data (body + 4 equipment layers)
    pub layers: Vec<LayerBuffer>,
}

/// GPU buffers for projectile rendering (shares quad/index from NpcRenderBuffers).
#[derive(Resource)]
pub struct ProjRenderBuffers {
    pub instance_buffer: RawBufferVec<InstanceData>,
    pub instance_count: u32,
}

/// The specialized render pipeline for NPCs.
#[derive(Resource)]
pub struct NpcPipeline {
    pub shader: Handle<Shader>,
    pub texture_bind_group_layout: BindGroupLayoutDescriptor,
    pub camera_bind_group_layout: BindGroupLayoutDescriptor,
}

/// Bind group for NPC sprite texture.
#[derive(Resource)]
pub struct NpcTextureBindGroup {
    pub bind_group: BindGroup,
}

/// Camera uniform data uploaded to GPU each frame.
#[derive(Clone, ShaderType)]
pub struct CameraUniform {
    pub camera_pos: Vec2,
    pub zoom: f32,
    pub viewport: Vec2,
}

/// Bind group for camera uniform.
#[derive(Resource)]
pub struct NpcCameraBindGroup {
    pub buffer: UniformBuffer<CameraUniform>,
    pub bind_group: Option<BindGroup>,
}

/// Pre-computed terrain + building instances for rendering. Computed once from WorldGrid,
/// then extracted to render world each frame. Static data — only rebuilt when world changes.
#[derive(Resource, Clone, ExtractResource, Default)]
pub struct WorldRenderInstances {
    pub terrain: Vec<InstanceData>,
    pub buildings: Vec<InstanceData>,
}

/// Compute terrain + building instances from WorldGrid (runs once when grid is populated).
fn compute_world_render_instances(
    mut commands: Commands,
    grid: Res<WorldGrid>,
    world_data: Res<WorldData>,
    existing: Option<Res<WorldRenderInstances>>,
    mut computed: Local<bool>,
) {
    // Only compute once, and only when grid is populated
    if *computed || grid.width == 0 { return; }
    // Don't overwrite if already exists (e.g. from a previous run)
    if existing.is_some() && existing.as_ref().unwrap().terrain.len() > 0 { *computed = true; return; }

    let mut terrain = Vec::with_capacity(grid.width * grid.height);
    for row in 0..grid.height {
        for col in 0..grid.width {
            let cell_index = row * grid.width + col;
            let cell = &grid.cells[cell_index];
            let world_pos = grid.grid_to_world(col, row);
            let (sprite_col, sprite_row) = cell.terrain.sprite(cell_index);

            terrain.push(InstanceData {
                position: [world_pos.x, world_pos.y],
                sprite: [sprite_col, sprite_row],
                color: [1.0, 1.0, 1.0, 1.0],
                health: 1.0,
                flash: 0.0,
                scale: grid.cell_size,
                atlas_id: 1.0,
            });
        }
    }

    let mut buildings = Vec::new();
    for sprite in world_data.get_all_sprites() {
        buildings.push(InstanceData {
            position: [sprite.pos.x, sprite.pos.y],
            sprite: [sprite.uv.0 as f32, sprite.uv.1 as f32],
            color: [1.0, 1.0, 1.0, 1.0],
            health: 1.0,
            flash: 0.0,
            scale: sprite.scale * 16.0, // SpriteDef.scale * base sprite size
            atlas_id: 1.0,
        });
    }

    info!("World render instances: {} terrain, {} buildings", terrain.len(), buildings.len());
    commands.insert_resource(WorldRenderInstances { terrain, buildings });
    *computed = true;
}

// =============================================================================
// RENDER COMMAND
// =============================================================================

/// Custom draw command for NPCs — draws all layers sequentially (body, then equipment).
pub struct DrawNpcs;

impl<P: PhaseItem> RenderCommand<P> for DrawNpcs {
    type Param = SRes<NpcRenderBuffers>;
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, 'w, Self::ViewQuery>,
        _entity: Option<ROQueryItem<'w, 'w, Self::ItemQuery>>,
        buffers: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let buffers = buffers.into_inner();

        // Shared geometry
        pass.set_vertex_buffer(0, buffers.vertex_buffer.slice(..));
        pass.set_index_buffer(buffers.index_buffer.slice(..), IndexFormat::Uint16);

        // Draw each non-empty layer: terrain → buildings → body → armor → helmet → weapon → item
        let mut drew_any = false;
        for layer in &buffers.layers {
            if layer.count == 0 { continue; }
            if let Some(instance_buffer) = layer.instances.buffer() {
                pass.set_vertex_buffer(1, instance_buffer.slice(..));
                pass.draw_indexed(0..6, 0, 0..layer.count);
                drew_any = true;
            }
        }

        if drew_any { RenderCommandResult::Success } else { RenderCommandResult::Skip }
    }
}

/// Bind group setter for NPC texture.
pub struct SetNpcTextureBindGroup<const I: usize>;

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetNpcTextureBindGroup<I> {
    type Param = SRes<NpcTextureBindGroup>;
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, 'w, Self::ViewQuery>,
        _entity: Option<ROQueryItem<'w, 'w, Self::ItemQuery>>,
        texture_bind_group: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        pass.set_bind_group(I, &texture_bind_group.into_inner().bind_group, &[]);
        RenderCommandResult::Success
    }
}

/// Bind group setter for camera uniform.
pub struct SetNpcCameraBindGroup<const I: usize>;

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetNpcCameraBindGroup<I> {
    type Param = SRes<NpcCameraBindGroup>;
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, 'w, Self::ViewQuery>,
        _entity: Option<ROQueryItem<'w, 'w, Self::ItemQuery>>,
        camera_bg: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        if let Some(ref bind_group) = camera_bg.into_inner().bind_group {
            pass.set_bind_group(I, bind_group, &[]);
            RenderCommandResult::Success
        } else {
            RenderCommandResult::Skip
        }
    }
}

/// Complete draw commands for NPCs.
type DrawNpcCommands = (
    SetItemPipeline,
    SetNpcTextureBindGroup<0>,
    SetNpcCameraBindGroup<1>,
    DrawNpcs,
);

/// Draw command for projectiles. Shares NPC quad geometry, uses proj instance buffer.
pub struct DrawProjs;

impl<P: PhaseItem> RenderCommand<P> for DrawProjs {
    type Param = (SRes<NpcRenderBuffers>, SRes<ProjRenderBuffers>);
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, 'w, Self::ViewQuery>,
        _entity: Option<ROQueryItem<'w, 'w, Self::ItemQuery>>,
        params: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (npc_buffers, proj_buffers) = params;
        let npc_buffers = npc_buffers.into_inner();
        let proj_buffers = proj_buffers.into_inner();

        if proj_buffers.instance_count == 0 {
            return RenderCommandResult::Skip;
        }

        // Slot 0: static quad vertices (shared with NPCs)
        pass.set_vertex_buffer(0, npc_buffers.vertex_buffer.slice(..));

        // Slot 1: per-instance projectile data
        let Some(instance_buffer) = proj_buffers.instance_buffer.buffer() else {
            return RenderCommandResult::Skip;
        };
        pass.set_vertex_buffer(1, instance_buffer.slice(..));

        pass.set_index_buffer(npc_buffers.index_buffer.slice(..), IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..proj_buffers.instance_count);

        RenderCommandResult::Success
    }
}

/// Complete draw commands for projectiles (reuses NPC pipeline + bind groups).
type DrawProjCommands = (
    SetItemPipeline,
    SetNpcTextureBindGroup<0>,
    SetNpcCameraBindGroup<1>,
    DrawProjs,
);

// =============================================================================
// PLUGIN
// =============================================================================

pub struct NpcRenderPlugin;

impl Plugin for NpcRenderPlugin {
    fn build(&self, app: &mut App) {
        // World render instances: computed in main world, extracted to render world
        app.init_resource::<WorldRenderInstances>();
        app.add_systems(Update, compute_world_render_instances);
        app.add_plugins(ExtractResourcePlugin::<WorldRenderInstances>::default());

        // Spawn batch entities in main world
        app.add_systems(Startup, (spawn_npc_batch, spawn_proj_batch));

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .add_render_command::<Transparent2d, DrawNpcCommands>()
            .add_render_command::<Transparent2d, DrawProjCommands>()
            .init_resource::<SpecializedRenderPipelines<NpcPipeline>>()
            .add_systems(
                ExtractSchedule,
                (extract_npc_batch, extract_proj_batch),
            )
            .add_systems(
                Render,
                (
                    prepare_npc_buffers.in_set(RenderSystems::PrepareResources),
                    prepare_proj_buffers.in_set(RenderSystems::PrepareResources),
                    prepare_npc_texture_bind_group.in_set(RenderSystems::PrepareBindGroups),
                    prepare_npc_camera_bind_group.in_set(RenderSystems::PrepareBindGroups),
                    queue_npcs.in_set(RenderSystems::Queue),
                    queue_projs.in_set(RenderSystems::Queue),
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.init_resource::<NpcPipeline>();
    }
}

/// Spawn the single NPC batch entity (represents all NPC layers for rendering).
fn spawn_npc_batch(mut commands: Commands) {
    commands.spawn((
        NpcBatch,
        Transform::default(),
        Visibility::default(),
    ));
    info!("NPC batch entity spawned ({LAYER_COUNT} layers)");
}

// =============================================================================
// EXTRACT
// =============================================================================

/// Extract NPC batch entity to render world.
fn extract_npc_batch(
    mut commands: Commands,
    query: Extract<Query<Entity, With<NpcBatch>>>,
) {
    for entity in &query {
        commands.spawn((NpcBatch, MainEntity::from(entity)));
    }
}

// =============================================================================
// PREPARE
// =============================================================================

/// Equipment layer sprite sources (matches EquipLayer enum order).
const EQUIP_LAYER_FIELDS: [fn(&NpcBufferWrites, usize) -> (f32, f32); 4] = [
    |w, i| { let j = i * 2; (w.armor_sprites.get(j).copied().unwrap_or(-1.0), w.armor_sprites.get(j+1).copied().unwrap_or(0.0)) },
    |w, i| { let j = i * 2; (w.helmet_sprites.get(j).copied().unwrap_or(-1.0), w.helmet_sprites.get(j+1).copied().unwrap_or(0.0)) },
    |w, i| { let j = i * 2; (w.weapon_sprites.get(j).copied().unwrap_or(-1.0), w.weapon_sprites.get(j+1).copied().unwrap_or(0.0)) },
    |w, i| { let j = i * 2; (w.item_sprites.get(j).copied().unwrap_or(-1.0), w.item_sprites.get(j+1).copied().unwrap_or(0.0)) },
];

/// Prepare all instance buffers — terrain, buildings, NPCs, equipment (7 layers).
fn prepare_npc_buffers(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    buffer_writes: Option<Res<NpcBufferWrites>>,
    gpu_data: Option<Res<NpcGpuData>>,
    world_instances: Option<Res<WorldRenderInstances>>,
    existing_buffers: Option<ResMut<NpcRenderBuffers>>,
) {
    let Some(writes) = buffer_writes else { return };

    // Use actual NPC count, not buffer length (buffer is pre-allocated for MAX_NPCS)
    let npc_count = gpu_data.map(|d| d.npc_count).unwrap_or(0) as usize;

    // Use GPU readback positions if available (compute shader moves NPCs),
    // fall back to NpcBufferWrites for first frame before readback starts
    let readback_positions = crate::messages::GPU_READ_STATE
        .lock()
        .ok()
        .filter(|s| s.positions.len() >= npc_count * 2)
        .map(|s| s.positions.clone());

    // Build all layer buffers: [0]=terrain, [1]=buildings, [2]=body, [3-6]=equipment
    let mut layer_instances: Vec<RawBufferVec<InstanceData>> = (0..LAYER_COUNT)
        .map(|_| RawBufferVec::new(BufferUsages::VERTEX))
        .collect();

    // Layers 0-1: Terrain + buildings from pre-computed world instances
    if let Some(world) = world_instances {
        for inst in &world.terrain {
            layer_instances[0].push(*inst);
        }
        for inst in &world.buildings {
            layer_instances[1].push(*inst);
        }
    }

    for i in 0..npc_count {
        let (px, py) = if let Some(ref pos) = readback_positions {
            (pos[i * 2], pos[i * 2 + 1])
        } else {
            (
                writes.positions.get(i * 2).copied().unwrap_or(0.0),
                writes.positions.get(i * 2 + 1).copied().unwrap_or(0.0),
            )
        };

        // Skip hidden NPCs
        if px < -9000.0 {
            continue;
        }

        // Layer 0: Body (existing logic)
        let sc = writes.sprite_indices.get(i * 4).copied().unwrap_or(0.0);
        let sr = writes.sprite_indices.get(i * 4 + 1).copied().unwrap_or(0.0);
        let cr = writes.colors.get(i * 4).copied().unwrap_or(1.0);
        let cg = writes.colors.get(i * 4 + 1).copied().unwrap_or(1.0);
        let cb = writes.colors.get(i * 4 + 2).copied().unwrap_or(1.0);
        let ca = writes.colors.get(i * 4 + 3).copied().unwrap_or(1.0);
        let health = (writes.healths.get(i).copied().unwrap_or(100.0) / 100.0).clamp(0.0, 1.0);
        let flash = writes.flash_values.get(i).copied().unwrap_or(0.0);

        layer_instances[2].push(InstanceData {
            position: [px, py],
            sprite: [sc, sr],
            color: [cr, cg, cb, ca],
            health,
            flash,
            scale: 16.0,
            atlas_id: 0.0,
        });

        // Layers 3-6: Equipment (only if sprite col >= 0, i.e. equipped)
        for (layer_idx, get_sprite) in EQUIP_LAYER_FIELDS.iter().enumerate() {
            let (ecol, erow) = get_sprite(&writes, i);
            if ecol >= 0.0 {
                layer_instances[layer_idx + 3].push(InstanceData {
                    position: [px, py],
                    sprite: [ecol, erow],
                    color: [1.0, 1.0, 1.0, 1.0],
                    health: 1.0,
                    flash,
                    scale: 16.0,
                    atlas_id: 0.0,
                });
            }
        }
    }

    // Write all layer buffers to GPU and collect counts
    let layers: Vec<LayerBuffer> = layer_instances
        .into_iter()
        .map(|mut inst| {
            let count = inst.len() as u32;
            inst.write_buffer(&render_device, &render_queue);
            LayerBuffer { instances: inst, count }
        })
        .collect();

    if let Some(mut buffers) = existing_buffers {
        buffers.layers = layers;
    } else {
        // Create static quad buffers on first run
        let vertex_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("npc_quad_vertices"),
            contents: bytemuck::cast_slice(&QUAD_VERTICES),
            usage: BufferUsages::VERTEX,
        });

        let index_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("npc_quad_indices"),
            contents: bytemuck::cast_slice(&QUAD_INDICES),
            usage: BufferUsages::INDEX,
        });

        commands.insert_resource(NpcRenderBuffers {
            vertex_buffer,
            index_buffer,
            layers,
        });
    }
}

/// Prepare texture bind group (dual atlas: character + world).
fn prepare_npc_texture_bind_group(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline: Option<Res<NpcPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    sprite_texture: Option<Res<NpcSpriteTexture>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
) {
    let Some(pipeline) = pipeline else { return };
    let Some(sprite_texture) = sprite_texture else { return };
    let Some(char_handle) = &sprite_texture.handle else { return };
    let Some(world_handle) = &sprite_texture.world_handle else { return };
    let Some(char_image) = gpu_images.get(char_handle) else { return };
    let Some(world_image) = gpu_images.get(world_handle) else { return };

    let layout = pipeline_cache.get_bind_group_layout(&pipeline.texture_bind_group_layout);

    let bind_group = render_device.create_bind_group(
        Some("npc_texture_bind_group"),
        &layout,
        &BindGroupEntries::sequential((
            &char_image.texture_view,
            &char_image.sampler,
            &world_image.texture_view,
            &world_image.sampler,
        )),
    );

    commands.insert_resource(NpcTextureBindGroup { bind_group });
}

/// Prepare camera uniform bind group — uploads CameraState to GPU each frame.
fn prepare_npc_camera_bind_group(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline: Option<Res<NpcPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    camera_state: Option<Res<CameraState>>,
) {
    let Some(pipeline) = pipeline else { return };
    let Some(camera_state) = camera_state else { return };

    let uniform = CameraUniform {
        camera_pos: camera_state.position,
        zoom: camera_state.zoom,
        viewport: camera_state.viewport,
    };

    let mut buffer = UniformBuffer::from(uniform);
    buffer.write_buffer(&render_device, &render_queue);

    let Some(binding) = buffer.binding() else { return };

    let layout = pipeline_cache.get_bind_group_layout(&pipeline.camera_bind_group_layout);

    let bind_group = render_device.create_bind_group(
        Some("npc_camera_bind_group"),
        &layout,
        &BindGroupEntries::sequential((binding,)),
    );

    commands.insert_resource(NpcCameraBindGroup {
        buffer,
        bind_group: Some(bind_group),
    });
}

// =============================================================================
// QUEUE
// =============================================================================

/// Queue NPC batch into Transparent2d phase — single entity draws all layers.
fn queue_npcs(
    draw_functions: Res<DrawFunctions<Transparent2d>>,
    pipeline: Res<NpcPipeline>,
    mut pipelines: ResMut<SpecializedRenderPipelines<NpcPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    buffers: Option<Res<NpcRenderBuffers>>,
    mut transparent_phases: ResMut<ViewSortedRenderPhases<Transparent2d>>,
    views: Query<(Entity, &ExtractedView, &Msaa)>,
    npc_batch: Query<Entity, With<NpcBatch>>,
) {
    let Some(buffers) = buffers else { return };
    // Check if any layer has instances
    if !buffers.layers.iter().any(|l| l.count > 0) { return; }

    let draw_function = draw_functions.read().id::<DrawNpcCommands>();

    for (view_entity, view, msaa) in &views {
        let Some(transparent_phase) = transparent_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };

        let pipeline_id = pipelines.specialize(&pipeline_cache, &pipeline, (view.hdr, msaa.samples()));

        for batch_entity in &npc_batch {
            transparent_phase.add(Transparent2d {
                entity: (view_entity, MainEntity::from(batch_entity)),
                draw_function,
                pipeline: pipeline_id,
                sort_key: FloatOrd(0.0),
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                extracted_index: usize::MAX,
                indexed: true,
            });
        }
    }
}

// =============================================================================
// PIPELINE SPECIALIZATION
// =============================================================================

impl SpecializedRenderPipeline for NpcPipeline {
    type Key = (bool, u32); // (HDR, MSAA sample count)

    fn specialize(&self, (hdr, sample_count): Self::Key) -> RenderPipelineDescriptor {
        let format = if hdr {
            TextureFormat::Rgba16Float
        } else {
            TextureFormat::Rgba8UnormSrgb
        };

        RenderPipelineDescriptor {
            label: Some("npc_render_pipeline".into()),
            layout: vec![
                self.texture_bind_group_layout.clone(),   // group 0: texture + sampler
                self.camera_bind_group_layout.clone(),    // group 1: camera uniform
            ],
            vertex: VertexState {
                shader: self.shader.clone(),
                shader_defs: vec![],
                entry_point: Some(Cow::from("vertex")),
                buffers: vec![
                    // Slot 0: Static quad vertices
                    VertexBufferLayout {
                        array_stride: std::mem::size_of::<QuadVertex>() as u64,
                        step_mode: VertexStepMode::Vertex,
                        attributes: vec![
                            VertexAttribute {
                                format: bevy::render::render_resource::VertexFormat::Float32x2,
                                offset: 0,
                                shader_location: 0, // position
                            },
                            VertexAttribute {
                                format: bevy::render::render_resource::VertexFormat::Float32x2,
                                offset: 8,
                                shader_location: 1, // uv
                            },
                        ],
                    },
                    // Slot 1: Per-instance NPC data
                    VertexBufferLayout {
                        array_stride: std::mem::size_of::<InstanceData>() as u64,
                        step_mode: VertexStepMode::Instance,
                        attributes: vec![
                            VertexAttribute {
                                format: bevy::render::render_resource::VertexFormat::Float32x2,
                                offset: 0,
                                shader_location: 2, // instance position
                            },
                            VertexAttribute {
                                format: bevy::render::render_resource::VertexFormat::Float32x2,
                                offset: 8,
                                shader_location: 3, // sprite col/row
                            },
                            VertexAttribute {
                                format: bevy::render::render_resource::VertexFormat::Float32x4,
                                offset: 16,
                                shader_location: 4, // color
                            },
                            VertexAttribute {
                                format: bevy::render::render_resource::VertexFormat::Float32,
                                offset: 32,
                                shader_location: 5, // health
                            },
                            VertexAttribute {
                                format: bevy::render::render_resource::VertexFormat::Float32,
                                offset: 36,
                                shader_location: 6, // flash
                            },
                            VertexAttribute {
                                format: bevy::render::render_resource::VertexFormat::Float32,
                                offset: 40,
                                shader_location: 7, // scale
                            },
                            VertexAttribute {
                                format: bevy::render::render_resource::VertexFormat::Float32,
                                offset: 44,
                                shader_location: 8, // atlas_id
                            },
                        ],
                    },
                ],
            },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs: vec![],
                entry_point: Some(Cow::from("fragment")),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: CompareFunction::GreaterEqual,
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState {
                count: sample_count,
                ..default()
            },
            push_constant_ranges: vec![],
            zero_initialize_workgroup_memory: false,
        }
    }
}

impl FromWorld for NpcPipeline {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();

        let texture_bind_group_layout = BindGroupLayoutDescriptor::new(
            "npc_texture_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    // Bindings 0-1: Character atlas
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    // Bindings 2-3: World atlas
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        );

        let camera_bind_group_layout = BindGroupLayoutDescriptor::new(
            "npc_camera_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX,
                (uniform_buffer::<CameraUniform>(false),),
            ),
        );

        NpcPipeline {
            shader: asset_server.load("shaders/npc_render.wgsl"),
            texture_bind_group_layout,
            camera_bind_group_layout,
        }
    }
}

// =============================================================================
// PROJECTILE RENDERING
// =============================================================================

fn spawn_proj_batch(mut commands: Commands) {
    commands.spawn((
        ProjBatch,
        Transform::default(),
        Visibility::default(),
    ));
    info!("Projectile batch entity spawned");
}

fn extract_proj_batch(
    mut commands: Commands,
    query: Extract<Query<Entity, With<ProjBatch>>>,
) {
    for entity in &query {
        commands.spawn((ProjBatch, MainEntity::from(entity)));
    }
}

/// Prepare projectile instance buffers from GPU readback positions.
fn prepare_proj_buffers(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    buffer_writes: Option<Res<ProjBufferWrites>>,
    proj_data: Option<Res<ProjGpuData>>,
    existing_buffers: Option<ResMut<ProjRenderBuffers>>,
) {
    let Some(writes) = buffer_writes else { return };
    let Some(data) = proj_data else { return };

    let proj_count = data.proj_count as usize;

    // Use GPU readback positions (compute shader moves projectiles each frame)
    let readback_positions = crate::messages::PROJ_POSITION_STATE
        .lock()
        .ok()
        .filter(|s| s.len() >= proj_count * 2)
        .map(|s| s.clone());

    let mut instances = RawBufferVec::new(BufferUsages::VERTEX);
    for i in 0..proj_count {
        // Only render active projectiles
        if writes.active.get(i).copied().unwrap_or(0) == 0 {
            continue;
        }

        let (px, py) = if let Some(ref pos) = readback_positions {
            (pos[i * 2], pos[i * 2 + 1])
        } else {
            (
                writes.positions.get(i * 2).copied().unwrap_or(0.0),
                writes.positions.get(i * 2 + 1).copied().unwrap_or(0.0),
            )
        };

        if px < -9000.0 { continue; }

        // Color by faction: 0 = villager (blue), 1+ = raider (red)
        let faction = writes.factions.get(i).copied().unwrap_or(0);
        let (cr, cg, cb) = if faction == 0 {
            (0.4, 0.6, 1.0)
        } else {
            (1.0, 0.3, 0.2)
        };

        // Projectile sprite (20, 7) — small arrow/bolt
        instances.push(InstanceData {
            position: [px, py],
            sprite: [20.0, 7.0],
            color: [cr, cg, cb, 1.0],
            health: 1.0,
            flash: 0.0,
            scale: 16.0,
            atlas_id: 0.0,
        });
    }

    let actual_count = instances.len() as u32;
    instances.write_buffer(&render_device, &render_queue);

    if let Some(mut buffers) = existing_buffers {
        buffers.instance_buffer = instances;
        buffers.instance_count = actual_count;
    } else {
        commands.insert_resource(ProjRenderBuffers {
            instance_buffer: instances,
            instance_count: actual_count,
        });
    }
}

/// Queue projectile batch into Transparent2d phase (above NPCs).
fn queue_projs(
    draw_functions: Res<DrawFunctions<Transparent2d>>,
    pipeline: Res<NpcPipeline>,
    mut pipelines: ResMut<SpecializedRenderPipelines<NpcPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    npc_buffers: Option<Res<NpcRenderBuffers>>,
    proj_buffers: Option<Res<ProjRenderBuffers>>,
    mut transparent_phases: ResMut<ViewSortedRenderPhases<Transparent2d>>,
    views: Query<(Entity, &ExtractedView, &Msaa)>,
    proj_batch: Query<Entity, With<ProjBatch>>,
) {
    let Some(_npc_buffers) = npc_buffers else { return };
    let Some(proj_buffers) = proj_buffers else { return };
    if proj_buffers.instance_count == 0 { return; }

    let draw_function = draw_functions.read().id::<DrawProjCommands>();

    for (view_entity, view, msaa) in &views {
        let Some(transparent_phase) = transparent_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };

        let pipeline_id = pipelines.specialize(&pipeline_cache, &pipeline, (view.hdr, msaa.samples()));

        for batch_entity in &proj_batch {
            transparent_phase.add(Transparent2d {
                entity: (view_entity, MainEntity::from(batch_entity)),
                draw_function,
                pipeline: pipeline_id,
                sort_key: FloatOrd(1.0), // Above NPCs (NPCs use 0.0)
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                extracted_index: usize::MAX,
                indexed: true,
            });
        }
    }
}
