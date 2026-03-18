//! NPC + Projectile Instanced Rendering via Bevy's RenderCommand pattern.
//!
//! Two render paths:
//! - Storage buffer path (NPCs): vertex shader reads positions/health from compute
//!   shader output directly, visual/equip data from CPU-uploaded storage buffers.
//! - Instance buffer path (farms, building HP bars, projectiles): classic per-instance
//!   vertex attributes via InstanceData.

use std::borrow::Cow;

use bevy::{
    core_pipeline::core_2d::Transparent2d,
    ecs::{
        query::ROQueryItem,
        system::{SystemParamItem, lifetimeless::SRes},
    },
    math::FloatOrd,
    mesh::VertexBufferLayout,
    prelude::*,
    render::{
        Extract, Render, RenderApp, RenderStartup, RenderSystems,
        render_asset::RenderAssets,
        render_phase::{
            AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand,
            RenderCommandResult, SetItemPipeline, SortedRenderPhase, TrackedRenderPass,
            ViewSortedRenderPhases,
        },
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            BlendState, Buffer, BufferDescriptor, BufferInitDescriptor, BufferUsages,
            ColorTargetState, ColorWrites, CompareFunction, DepthBiasState, DepthStencilState,
            FragmentState, IndexFormat, MultisampleState, PipelineCache, PrimitiveState,
            RawBufferVec, RenderPipelineDescriptor, SamplerBindingType, ShaderStages, ShaderType,
            SpecializedRenderPipeline, SpecializedRenderPipelines, StencilState, TextureFormat,
            TextureSampleType, UniformBuffer, VertexAttribute, VertexState, VertexStepMode,
            binding_types::{sampler, storage_buffer_read_only, texture_2d, uniform_buffer},
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::MainEntity,
        texture::GpuImage,
        view::ExtractedView,
    },
};
use bytemuck::{Pod, Zeroable};

use crate::constants::MAX_ENTITIES;
use crate::gpu::{
    EntityGpuBuffers, EntityGpuState, NpcVisualUpload, ProjBufferWrites, ProjGpuBuffers,
    RenderFrameConfig,
};
use crate::render::{CameraState, MainCamera};
use crate::resources::GpuSlotPool;

// =============================================================================
// MARKER COMPONENT
// =============================================================================

/// Layer count: body + 4 equipment layers (armor/helm/weapon/shield) + item + status + healing.
pub const LAYER_COUNT: usize = 8;

// =============================================================================
// RENDER ORDER CONTRACT
// =============================================================================
// Sort keys for Transparent2d phase items. Sole ordering mechanism (depth_compare = Always).
//   (tilemap)          Terrain               Bevy internal
//   0.2                Building bodies       StorageDrawMode::BuildingBody
//   0.3                Building overlays     Instance buffer (HP bars, farm/mine progress)
//   0.5                NPC bodies            StorageDrawMode::NpcBody
//   0.6                NPC overlays          StorageDrawMode::NpcOverlay (equipment layers 1-6)
//   1.0                Projectiles           Instance buffer
//   1.5                Selection brackets    StorageDrawMode::SelectionBracket
pub const ORDER_BUILDING_BODY: f32 = 0.2;
pub const ORDER_BUILDING_OVERLAY: f32 = 0.3;
pub const ORDER_NPC_BODY: f32 = 0.5;
pub const ORDER_NPC_OVERLAY: f32 = 0.6;
pub const ORDER_PROJECTILES: f32 = 1.0;
pub const ORDER_SELECTION_OVERLAY: f32 = 1.5;

/// Which storage-buffer draw pass to specialize. Maps to shader_defs in vertex_npc.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageDrawMode {
    BuildingBody,
    NpcBody,
    NpcOverlay,
    SelectionBracket,
}

/// Marker component for the NPC batch entity.
#[derive(Component, Clone)]
pub struct NpcBatch;

/// Marker component for the single projectile batch entity.
#[derive(Component, Clone)]
pub struct ProjBatch;

// =============================================================================
// VERTEX DATA
// =============================================================================

/// Instance data for a single sprite (used by farms, building HP bars, projectiles).
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct InstanceData {
    pub position: [f32; 2],
    pub sprite: [f32; 2],
    pub color: [f32; 4],
    pub health: f32,
    pub flash: f32,
    pub scale: f32,
    pub atlas_id: f32,
    pub rotation: f32,
}

/// Per-bracket instance data for GPU selection overlay.
/// Position is read from npc_positions[slot] in the vertex shader.
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct SelectionInstance {
    pub slot: u32,
    pub color: [f32; 4],
    pub scale: f32,
    pub y_offset: f32,
    pub _pad: f32,
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
    QuadVertex {
        position: [-0.5, -0.5],
        uv: [0.0, 1.0],
    }, // bottom-left
    QuadVertex {
        position: [0.5, -0.5],
        uv: [1.0, 1.0],
    }, // bottom-right
    QuadVertex {
        position: [0.5, 0.5],
        uv: [1.0, 0.0],
    }, // top-right
    QuadVertex {
        position: [-0.5, 0.5],
        uv: [0.0, 0.0],
    }, // top-left
];

/// Two triangles forming a quad
static QUAD_INDICES: [u16; 6] = [0, 1, 2, 0, 2, 3];

// =============================================================================
// RENDER RESOURCES
// =============================================================================

/// Shared quad geometry (slot 0) for all render paths.
#[derive(Resource)]
pub struct NpcRenderBuffers {
    pub vertex_buffer: Buffer,
    pub index_buffer: Buffer,
}

/// GPU storage buffers for NPC visual data (CPU-uploaded, read by vertex_npc shader).
#[derive(Resource)]
pub struct NpcVisualBuffers {
    /// [f32; 8] per slot: [sprite_col, sprite_row, body_atlas, flash, r, g, b, a]
    pub visual: Buffer,
    /// [f32; 24] per slot: 6 equipment layers × [col, row, atlas, _pad]
    pub equip: Buffer,
    /// Bind group 2 for NPC storage buffer pipeline
    pub bind_group: Option<BindGroup>,
}

/// Main-world overlay instances, rebuilt each frame. Zero-clone extracted to render world.
/// Building body instances, built each frame from BuildingGpuState.
/// Replaces the old storage-buffer path for building body rendering.
#[derive(Resource, Default)]
pub struct BuildingBodyInstances(pub Vec<InstanceData>);

/// Dirty flag for BuildingBodyInstances. Set by mark_building_body_dirty, cleared after rebuild.
/// Avoids O(68K) rebuild every frame when nothing visual has changed for buildings.
#[derive(Resource)]
pub struct BuildingBodyDirty {
    /// Rebuild needed this frame.
    pub dirty: bool,
    /// Any building slot had active flash last frame (need one extra rebuild to clear flash=0).
    pub had_building_flash: bool,
    /// Building count at last rebuild (detects placements/removals).
    pub last_building_count: usize,
}

impl Default for BuildingBodyDirty {
    fn default() -> Self {
        Self {
            dirty: true, // start dirty so first frame always builds
            had_building_flash: false,
            last_building_count: usize::MAX, // force rebuild on first frame
        }
    }
}

/// Any system that needs to render building/farm/mine overlays pushes InstanceData here.
#[derive(Resource, Default)]
pub struct OverlayInstances(pub Vec<InstanceData>);

/// Selection bracket instances, rebuilt each frame from SelectedNpc/SelectedBuilding/DC state.
#[derive(Resource, Default)]
pub struct SelectionOverlayInstances(pub Vec<SelectionInstance>);

/// Instance buffer for building overlays (farms, building HP bars, mine progress).
#[derive(Resource)]
pub struct BuildingOverlayBuffers {
    pub instances: RawBufferVec<InstanceData>,
    pub count: u32,
}

/// Instance buffer for building body rendering (separate from NPC storage buffer path).
#[derive(Resource)]
pub struct BuildingBodyRenderBuffers {
    pub instances: RawBufferVec<InstanceData>,
    pub count: u32,
}

/// GPU buffers for selection bracket rendering (render world).
#[derive(Resource)]
pub struct SelectionRenderBuffers {
    pub instances: RawBufferVec<SelectionInstance>,
    pub count: u32,
}

/// GPU buffers for projectile rendering (shares quad/index from NpcRenderBuffers).
#[derive(Resource)]
pub struct ProjRenderBuffers {
    pub instance_buffer: RawBufferVec<InstanceData>,
    pub instance_count: u32,
}

/// The specialized render pipeline — supports both instance and storage buffer modes.
#[derive(Resource)]
pub struct NpcPipeline {
    pub shader: Handle<Shader>,
    pub texture_bind_group_layout: BindGroupLayoutDescriptor,
    pub camera_bind_group_layout: BindGroupLayoutDescriptor,
    pub npc_data_bind_group_layout: BindGroupLayoutDescriptor,
}

/// Bind group for NPC sprite texture.
#[derive(Resource)]
pub struct NpcTextureBindGroup {
    pub bind_group: BindGroup,
}

/// Camera uniform data uploaded to GPU each frame.
/// Field order matches WGSL Camera struct layout (entity_count fills alignment padding).
#[derive(Clone, ShaderType)]
pub struct CameraUniform {
    pub camera_pos: Vec2,
    pub zoom: f32,
    pub entity_count: u32,
    pub viewport: Vec2,
    pub bldg_layers: f32,
    pub extras_cols: f32,
    pub lod_zoom: f32,
    /// HP bar display mode: 0=Off, 1=WhenDamaged, 2=Always.
    pub hp_bar_mode: u32,
}

/// Bind group for camera uniform.
#[derive(Resource)]
pub struct NpcCameraBindGroup {
    pub buffer: UniformBuffer<CameraUniform>,
    pub bind_group: Option<BindGroup>,
}

// =============================================================================
// RENDER COMMANDS
// =============================================================================

/// Generic storage buffer draw command. BODY_ONLY selects layer range:
/// - true: layer 0 only (1 draw call). Shader #ifdef gates building vs NPC.
/// - false: layers 1..LAYER_COUNT (6 draw calls, equipment/status overlays).
pub struct DrawStoragePass<const BODY_ONLY: bool>;

impl<P: PhaseItem, const BODY_ONLY: bool> RenderCommand<P> for DrawStoragePass<BODY_ONLY> {
    type Param = (
        SRes<NpcRenderBuffers>,
        SRes<NpcVisualBuffers>,
        SRes<RenderFrameConfig>,
    );
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, 'w, Self::ViewQuery>,
        _entity: Option<ROQueryItem<'w, 'w, Self::ItemQuery>>,
        params: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (npc_buffers, visual_buffers, config) = params;
        let npc_buffers = npc_buffers.into_inner();
        let visual_buffers = visual_buffers.into_inner();
        let entity_count = config.into_inner().npc.count;
        if entity_count == 0 {
            return RenderCommandResult::Skip;
        }

        let Some(ref bind_group) = visual_buffers.bind_group else {
            return RenderCommandResult::Skip;
        };

        pass.set_bind_group(2, bind_group, &[]);
        pass.set_vertex_buffer(0, npc_buffers.vertex_buffer.slice(..));
        pass.set_index_buffer(npc_buffers.index_buffer.slice(..), IndexFormat::Uint16);

        if BODY_ONLY {
            pass.draw_indexed(0..6, 0, 0..entity_count);
        } else {
            for layer in 1..LAYER_COUNT as u32 {
                pass.draw_indexed(
                    0..6,
                    0,
                    (layer * entity_count)..((layer + 1) * entity_count),
                );
            }
        }

        RenderCommandResult::Success
    }
}

/// Draw command for building bodies using instance buffer (decoupled from NPC compute).
pub struct DrawBuildingBody;

impl<P: PhaseItem> RenderCommand<P> for DrawBuildingBody {
    type Param = (SRes<NpcRenderBuffers>, SRes<BuildingBodyRenderBuffers>);
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, 'w, Self::ViewQuery>,
        _entity: Option<ROQueryItem<'w, 'w, Self::ItemQuery>>,
        params: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (npc_buffers, body_buffers) = params;
        let npc_buffers = npc_buffers.into_inner();
        let body_buffers = body_buffers.into_inner();

        if body_buffers.count == 0 {
            return RenderCommandResult::Skip;
        }
        let Some(instance_buffer) = body_buffers.instances.buffer() else {
            return RenderCommandResult::Skip;
        };

        pass.set_vertex_buffer(0, npc_buffers.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, instance_buffer.slice(..));
        pass.set_index_buffer(npc_buffers.index_buffer.slice(..), IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..body_buffers.count);

        RenderCommandResult::Success
    }
}

/// Draw command for building overlay instance buffer path (farms, building HP bars, mine progress).
pub struct DrawBuildingOverlay;

impl<P: PhaseItem> RenderCommand<P> for DrawBuildingOverlay {
    type Param = (SRes<NpcRenderBuffers>, SRes<BuildingOverlayBuffers>);
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, 'w, Self::ViewQuery>,
        _entity: Option<ROQueryItem<'w, 'w, Self::ItemQuery>>,
        params: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (npc_buffers, overlay_buffers) = params;
        let npc_buffers = npc_buffers.into_inner();
        let overlay_buffers = overlay_buffers.into_inner();

        if overlay_buffers.count == 0 {
            return RenderCommandResult::Skip;
        }
        let Some(instance_buffer) = overlay_buffers.instances.buffer() else {
            return RenderCommandResult::Skip;
        };

        pass.set_vertex_buffer(0, npc_buffers.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, instance_buffer.slice(..));
        pass.set_index_buffer(npc_buffers.index_buffer.slice(..), IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..overlay_buffers.count);

        RenderCommandResult::Success
    }
}

/// Draw command for selection brackets (storage buffer path + selection instance buffer).
pub struct DrawSelectionBrackets;

impl<P: PhaseItem> RenderCommand<P> for DrawSelectionBrackets {
    type Param = (
        SRes<NpcRenderBuffers>,
        SRes<NpcVisualBuffers>,
        SRes<SelectionRenderBuffers>,
    );
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, 'w, Self::ViewQuery>,
        _entity: Option<ROQueryItem<'w, 'w, Self::ItemQuery>>,
        params: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (npc_buffers, visual_buffers, sel_buffers) = params;
        let npc_buffers = npc_buffers.into_inner();
        let visual_buffers = visual_buffers.into_inner();
        let sel_buffers = sel_buffers.into_inner();

        if sel_buffers.count == 0 {
            return RenderCommandResult::Skip;
        }
        let Some(ref bind_group) = visual_buffers.bind_group else {
            return RenderCommandResult::Skip;
        };
        let Some(instance_buffer) = sel_buffers.instances.buffer() else {
            return RenderCommandResult::Skip;
        };

        pass.set_bind_group(2, bind_group, &[]);
        pass.set_vertex_buffer(0, npc_buffers.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, instance_buffer.slice(..));
        pass.set_index_buffer(npc_buffers.index_buffer.slice(..), IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..sel_buffers.count);

        RenderCommandResult::Success
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

/// Building body draw commands (instance buffer, decoupled from NPC compute).
type DrawBuildingBodyCommands = (
    SetItemPipeline,
    SetNpcTextureBindGroup<0>,
    SetNpcCameraBindGroup<1>,
    DrawBuildingBody,
);

/// NPC body draw commands (storage buffer, layer 0, non-building only).
type DrawNpcBodyCommands = (
    SetItemPipeline,
    SetNpcTextureBindGroup<0>,
    SetNpcCameraBindGroup<1>,
    DrawStoragePass<true>,
);

/// NPC overlay draw commands (storage buffer, layers 1-6, non-building only).
type DrawNpcOverlayCommands = (
    SetItemPipeline,
    SetNpcTextureBindGroup<0>,
    SetNpcCameraBindGroup<1>,
    DrawStoragePass<false>,
);

/// Building overlay instance draw commands (farms, building HP bars, mine progress).
type DrawBuildingOverlayCommands = (
    SetItemPipeline,
    SetNpcTextureBindGroup<0>,
    SetNpcCameraBindGroup<1>,
    DrawBuildingOverlay,
);

/// Selection bracket draw commands (storage buffer path + selection instance buffer).
type DrawSelectionBracketCommands = (
    SetItemPipeline,
    SetNpcTextureBindGroup<0>,
    SetNpcCameraBindGroup<1>,
    DrawSelectionBrackets,
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

        pass.set_vertex_buffer(0, npc_buffers.vertex_buffer.slice(..));

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
        app.init_resource::<OverlayInstances>()
            .init_resource::<BuildingBodyInstances>()
            .init_resource::<BuildingBodyDirty>()
            .init_resource::<SelectionOverlayInstances>()
            .init_resource::<crate::resources::DirectControlSet>()
            .add_systems(Startup, (spawn_npc_batch, spawn_proj_batch))
            .add_systems(
                PostUpdate,
                (
                    sync_direct_control_set,
                    mark_building_body_dirty.after(crate::gpu::populate_gpu_state),
                    build_building_body_instances.after(mark_building_body_dirty),
                    build_overlay_instances,
                    build_selection_overlay.after(sync_direct_control_set),
                ),
            );

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .add_render_command::<Transparent2d, DrawBuildingBodyCommands>()
            .add_render_command::<Transparent2d, DrawBuildingOverlayCommands>()
            .add_render_command::<Transparent2d, DrawNpcBodyCommands>()
            .add_render_command::<Transparent2d, DrawNpcOverlayCommands>()
            .add_render_command::<Transparent2d, DrawProjCommands>()
            .add_render_command::<Transparent2d, DrawSelectionBracketCommands>()
            .init_resource::<SpecializedRenderPipelines<NpcPipeline>>()
            .add_systems(RenderStartup, init_npc_render_pipeline)
            .add_systems(
                ExtractSchedule,
                (
                    extract_npc_batch,
                    extract_proj_batch,
                    extract_camera_state,
                    extract_npc_data,
                    extract_proj_data,
                    extract_overlay_instances,
                    extract_building_body_instances,
                    extract_selection_overlay,
                ),
            )
            .add_systems(
                Render,
                (
                    prepare_npc_buffers.in_set(RenderSystems::PrepareResources),
                    prepare_npc_texture_bind_group.in_set(RenderSystems::PrepareBindGroups),
                    prepare_npc_camera_bind_group.in_set(RenderSystems::PrepareBindGroups),
                    queue_npcs.in_set(RenderSystems::Queue),
                    queue_projs.in_set(RenderSystems::Queue),
                ),
            );
    }
}

/// Spawn the single NPC batch entity (represents all NPC layers for rendering).
fn spawn_npc_batch(mut commands: Commands) {
    commands.spawn((NpcBatch, Transform::default(), Visibility::default()));
    info!("NPC batch entity spawned ({LAYER_COUNT} layers)");
}

// =============================================================================
// EXTRACT
// =============================================================================

/// Extract NPC batch entity to render world. Despawns stale copies first to prevent leak.
fn extract_npc_batch(
    mut commands: Commands,
    query: Extract<Query<Entity, With<NpcBatch>>>,
    stale: Query<Entity, With<NpcBatch>>,
) {
    for entity in &stale {
        commands.entity(entity).despawn();
    }
    for entity in &query {
        commands.spawn((NpcBatch, MainEntity::from(entity)));
    }
}

/// Extract camera state from Bevy camera into render world resource.
fn extract_camera_state(
    mut commands: Commands,
    query: Extract<Query<(&Transform, &Projection), With<MainCamera>>>,
    windows: Extract<Query<&Window>>,
    user_settings: Extract<Res<crate::settings::UserSettings>>,
) {
    let Ok((transform, projection)) = query.single() else {
        return;
    };
    let Ok(window) = windows.single() else { return };

    let zoom = match projection {
        Projection::Orthographic(ortho) => 1.0 / ortho.scale,
        _ => 1.0,
    };

    commands.insert_resource(CameraState {
        position: transform.translation.truncate(),
        zoom,
        viewport: Vec2::new(window.width(), window.height()),
        lod_zoom: user_settings.lod_transition,
        hp_bar_mode: user_settings.hp_bar_mode.as_u32(),
    });
}

// =============================================================================
// EXTRACT: Direct GPU Upload (zero clone)
// =============================================================================

// =============================================================================
// OVERLAY INSTANCES (main world → render world, zero-clone)
// =============================================================================

/// Mark BuildingBodyInstances dirty when any building visual state may have changed.
/// Runs in PostUpdate after populate_gpu_state so flash_only_indices is current.
fn mark_building_body_dirty(
    mut dirty: ResMut<BuildingBodyDirty>,
    construction_changed: Query<(), Changed<crate::components::ConstructionProgress>>,
    entity_map: Res<crate::resources::EntityMap>,
    gpu_state: Res<crate::gpu::EntityGpuState>,
) {
    // Building placed or removed: count changed.
    let count = entity_map.building_count();
    if count != dirty.last_building_count {
        dirty.last_building_count = count;
        dirty.dirty = true;
        dirty.had_building_flash = false;
        return;
    }
    // Any building under construction ticked (ConstructionProgress changed).
    if !construction_changed.is_empty() {
        dirty.dirty = true;
        return;
    }
    // Any building slot has active damage flash (check flash_only_indices, not all 68K slots).
    let mut has_building_flash = false;
    for &slot in &gpu_state.flash_only_indices {
        if entity_map.get_instance(slot).is_some() {
            has_building_flash = true;
            break;
        }
    }
    // Dirty if flash active now OR was active last frame (one extra rebuild to clear flash=0).
    if has_building_flash || dirty.had_building_flash {
        dirty.dirty = true;
    }
    dirty.had_building_flash = has_building_flash;
}

/// Build building body instances from EntityGpuState for instance-buffer rendering.
/// Skips rebuild when BuildingBodyDirty is false (nothing changed since last frame).
fn build_building_body_instances(
    gpu_state: Res<crate::gpu::EntityGpuState>,
    entity_map: Res<crate::resources::EntityMap>,
    mut instances: ResMut<BuildingBodyInstances>,
    mut dirty: ResMut<BuildingBodyDirty>,
    construction_q: Query<&crate::components::ConstructionProgress>,
) {
    if !dirty.dirty {
        return;
    }
    dirty.dirty = false;
    instances.0.clear();
    for inst in entity_map.iter_instances() {
        let idx = inst.slot;
        let i2 = idx * 2;
        let x = gpu_state.positions.get(i2).copied().unwrap_or(-9999.0);
        let y = gpu_state.positions.get(i2 + 1).copied().unwrap_or(-9999.0);
        if x < -9000.0 {
            continue;
        } // hidden/dead

        let col = gpu_state
            .sprite_indices
            .get(idx * 4)
            .copied()
            .unwrap_or(-1.0);
        if col < 0.0 {
            continue;
        } // no sprite assigned

        let row = gpu_state
            .sprite_indices
            .get(idx * 4 + 1)
            .copied()
            .unwrap_or(0.0);
        let atlas = gpu_state
            .sprite_indices
            .get(idx * 4 + 2)
            .copied()
            .unwrap_or(1.0);
        let flash = gpu_state.flash_values.get(idx).copied().unwrap_or(0.0);
        let faction = gpu_state.factions.get(idx).copied().unwrap_or(0);
        // During construction, pass progress fraction (0→0.999) so shader clips sprite.
        // Fully-built buildings pass real HP (always >> 1.0), so shader skips the clip.
        let under_construction = entity_map
            .entities
            .get(&idx)
            .and_then(|&e| construction_q.get(e).ok())
            .map_or(0.0, |c| c.0);
        let health = if under_construction > 0.0 {
            let total = crate::constants::BUILDING_CONSTRUCT_SECS;
            ((total - under_construction) / total).clamp(0.0, 0.999)
        } else {
            gpu_state.healths.get(idx).copied().unwrap_or(0.0)
        };

        let (r, g, b, a) = if faction <= crate::constants::FACTION_PLAYER {
            // Neutral (resource nodes) and player buildings both use lod_color.
            let lc = crate::constants::building_def(inst.kind).lod_color;
            (lc[0], lc[1], lc[2], lc[3])
        } else {
            // Buildings use a subtle faction tint (not full recolor like NPCs/projectiles).
            let (fr, fg, fb, _fa) = crate::constants::raider_faction_color(faction);
            let tint = 0.30_f32;
            (
                1.0 + (fr - 1.0) * tint,
                1.0 + (fg - 1.0) * tint,
                1.0 + (fb - 1.0) * tint,
                1.0,
            )
        };

        instances.0.push(InstanceData {
            position: [x, y],
            sprite: [col, row],
            color: [r, g, b, a],
            health,
            flash,
            scale: 64.0,
            atlas_id: atlas,
            rotation: 0.0,
        });
    }
}

/// Build overlay instances from EntityMap (farm/mine growth) + BuildingHpRender each frame.
/// Runs in main world PostUpdate. Future visual features push here instead of adding new resources.
fn build_overlay_instances(
    mut overlay: ResMut<OverlayInstances>,
    building_hp: Res<crate::resources::BuildingHpRender>,
    production_q: Query<(
        &crate::components::Position,
        &crate::components::Building,
        &crate::components::ProductionState,
        &crate::components::ConstructionProgress,
        Option<&crate::components::FarmModeComp>,
    )>,
) {
    overlay.0.clear();

    for (pos, building, production, construction, farm_mode) in &production_q {
        if pos.x < -9000.0 || construction.0 > 0.0 {
            continue;
        }

        match building.kind {
            crate::world::BuildingKind::Farm => {
                let is_cow = farm_mode.is_some_and(|m| m.0 == crate::components::FarmMode::Cows);
                let color = if is_cow {
                    // Cows: brown tint (growing) / orange-gold (ready)
                    if production.ready {
                        [1.0, 0.65, 0.2, 1.0]
                    } else {
                        [0.65, 0.4, 0.2, 1.0]
                    }
                } else {
                    // Crops: green (growing) / gold (ready)
                    if production.ready {
                        [1.0, 0.85, 0.0, 1.0]
                    } else {
                        [0.4, 0.8, 0.2, 1.0]
                    }
                };
                overlay.0.push(InstanceData {
                    position: [pos.x, pos.y],
                    sprite: [24.0, 9.0],
                    color,
                    health: production.progress.clamp(0.0, 1.0),
                    flash: 0.0,
                    scale: 32.0,
                    atlas_id: 1.0,
                    rotation: 0.0,
                });
            }
            crate::world::BuildingKind::GoldMine => {
                overlay.0.push(InstanceData {
                    position: [pos.x, pos.y + 24.0],
                    sprite: [0.0, 0.0],
                    color: [1.0, 0.85, 0.0, 1.0],
                    health: production.progress.clamp(0.0, 1.0),
                    flash: 0.0,
                    scale: 24.0,
                    atlas_id: 6.0,
                    rotation: 0.0,
                });
            }
            crate::world::BuildingKind::TreeNode => {
                if production.progress > 0.0 {
                    overlay.0.push(InstanceData {
                        position: [pos.x, pos.y + 24.0],
                        sprite: [0.0, 0.0],
                        color: [0.3, 0.75, 0.2, 1.0],
                        health: production.progress.clamp(0.0, 1.0),
                        flash: 0.0,
                        scale: 24.0,
                        atlas_id: 6.0,
                        rotation: 0.0,
                    });
                }
            }
            crate::world::BuildingKind::RockNode => {
                if production.progress > 0.0 {
                    overlay.0.push(InstanceData {
                        position: [pos.x, pos.y + 24.0],
                        sprite: [0.0, 0.0],
                        color: [0.6, 0.6, 0.6, 1.0],
                        health: production.progress.clamp(0.0, 1.0),
                        flash: 0.0,
                        scale: 24.0,
                        atlas_id: 6.0,
                        rotation: 0.0,
                    });
                }
            }
            _ => {}
        }
    }

    let bhp_count = building_hp
        .positions
        .len()
        .min(building_hp.health_pcts.len());
    for i in 0..bhp_count {
        overlay.0.push(InstanceData {
            position: [building_hp.positions[i].x, building_hp.positions[i].y],
            sprite: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
            health: building_hp.health_pcts[i],
            flash: 0.0,
            scale: 64.0,
            atlas_id: 5.0,
            rotation: 0.0,
        });
    }
}

/// Incrementally maintain `DirectControlSet` from `Changed<NpcFlags>`.
/// O(changed) per frame instead of O(all_npcs).
fn sync_direct_control_set(
    mut dc_set: ResMut<crate::resources::DirectControlSet>,
    changed_q: Query<
        (Entity, &crate::components::NpcFlags),
        (
            Changed<crate::components::NpcFlags>,
            Without<crate::components::Building>,
        ),
    >,
) {
    for (entity, flags) in &changed_q {
        if flags.direct_control {
            dc_set.0.insert(entity);
        } else {
            dc_set.0.remove(&entity);
        }
    }
}

/// Build selection bracket instances from SelectedNpc, SelectedBuilding, and DirectControl state.
fn build_selection_overlay(
    mut instances: ResMut<SelectionOverlayInstances>,
    selected_npc: Res<crate::resources::SelectedNpc>,
    selected_building: Res<crate::resources::SelectedBuilding>,
    mut dc_set: ResMut<crate::resources::DirectControlSet>,
    npc_q: Query<
        (&crate::components::GpuSlot, &crate::components::NpcFlags),
        (
            Without<crate::components::Building>,
            Without<crate::components::Dead>,
        ),
    >,
) {
    instances.0.clear();
    let sel_slot = selected_npc.0;

    // Single NPC selection (cyan)
    if sel_slot >= 0 {
        instances.0.push(SelectionInstance {
            slot: sel_slot as u32,
            color: [0.39, 0.78, 1.0, 0.86],
            scale: 40.0,
            y_offset: 0.0,
            _pad: 0.0,
        });
    }

    // Single building selection (gold, slightly offset Y)
    if selected_building.active {
        if let Some(slot) = selected_building.slot {
            instances.0.push(SelectionInstance {
                slot: slot as u32,
                color: [1.0, 0.86, 0.35, 0.90],
                scale: 72.0,
                y_offset: 4.0,
                _pad: 0.0,
            });
        }
    }

    // DirectControl multi-select (green), skip selected NPC, cap at 200
    let mut dc_count = 0usize;
    let tracked_entities: Vec<Entity> = dc_set.0.drain().collect();
    for entity in tracked_entities {
        let Ok((gpu_slot, flags)) = npc_q.get(entity) else {
            continue;
        };
        if !flags.direct_control {
            continue;
        }
        let slot = gpu_slot.0;
        dc_set.0.insert(entity);
        if sel_slot >= 0 && slot == sel_slot as usize {
            continue;
        }
        if dc_count >= 200 {
            continue;
        }
        instances.0.push(SelectionInstance {
            slot: slot as u32,
            color: [0.31, 0.86, 0.31, 0.70],
            scale: 40.0,
            y_offset: 0.0,
            _pad: 0.0,
        });
        dc_count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Faction, GpuSlot, Job, NpcFlags};

    fn setup_selection_overlay_app() -> App {
        let mut app = App::new();
        app.init_resource::<SelectionOverlayInstances>()
            .init_resource::<crate::resources::DirectControlSet>()
            .insert_resource(crate::resources::SelectedNpc::default())
            .insert_resource(crate::resources::SelectedBuilding::default())
            .add_systems(
                Update,
                (
                    sync_direct_control_set,
                    build_selection_overlay.after(sync_direct_control_set),
                ),
            );
        app
    }

    #[test]
    fn selection_overlay_prunes_despawned_direct_control_entities() {
        let mut app = setup_selection_overlay_app();
        let direct_control_npc = app
            .world_mut()
            .spawn((
                GpuSlot(7),
                Job::Archer,
                Faction(crate::constants::FACTION_PLAYER),
                NpcFlags {
                    direct_control: true,
                    ..Default::default()
                },
            ))
            .id();

        app.update();

        let overlays = &app.world().resource::<SelectionOverlayInstances>().0;
        assert_eq!(
            overlays.len(),
            1,
            "direct-control NPC should render one bracket"
        );
        assert_eq!(overlays[0].slot, 7);

        app.world_mut().entity_mut(direct_control_npc).despawn();
        app.update();

        assert!(
            app.world()
                .resource::<crate::resources::DirectControlSet>()
                .0
                .is_empty(),
            "despawned direct-control entities should be pruned from the tracked set"
        );
        assert!(
            app.world()
                .resource::<SelectionOverlayInstances>()
                .0
                .is_empty(),
            "selection overlay should not keep brackets for despawned direct-control entities"
        );
    }

    #[test]
    fn selection_overlay_retains_dc_entities_beyond_render_cap() {
        let mut app = setup_selection_overlay_app();
        // Spawn 210 DC entities -- exceeds the 200 render cap
        for i in 0..210 {
            app.world_mut().spawn((
                GpuSlot(i),
                Job::Archer,
                Faction(crate::constants::FACTION_PLAYER),
                NpcFlags {
                    direct_control: true,
                    ..Default::default()
                },
            ));
        }
        app.update();

        let dc_set = &app
            .world()
            .resource::<crate::resources::DirectControlSet>()
            .0;
        assert_eq!(
            dc_set.len(),
            210,
            "all 210 DC entities must remain in the set, not just the 200 rendered"
        );
        let overlays = &app.world().resource::<SelectionOverlayInstances>().0;
        assert_eq!(
            overlays.len(),
            200,
            "only 200 selection brackets should be rendered"
        );
    }

    /// Regression test for issue #163: visual/equip buffers must cover MAX_ENTITIES (unified slot
    /// pool namespace = NPCs + buildings), not just MAX_NPC_COUNT.
    ///
    /// The equip buffer stride is 7 layers x [col, row, atlas, pad] = 28 floats per slot.
    /// If either assertion fails the buffer creation in `setup_npc_visual_buffers` is undersized
    /// and will overflow when slot indices exceed the old MAX_NPC_COUNT cap.
    #[test]
    fn visual_equip_buffer_sizes_cover_max_entities() {
        use crate::constants::{MAX_ENTITIES, MAX_NPC_COUNT};

        // Visual buffer: 8 floats per slot, must cover MAX_ENTITIES
        let visual_bytes = MAX_ENTITIES * std::mem::size_of::<[f32; 8]>();
        let visual_bytes_old = MAX_NPC_COUNT * std::mem::size_of::<[f32; 8]>();
        assert!(
            visual_bytes > visual_bytes_old,
            "visual buffer sized to MAX_NPC_COUNT; must use MAX_ENTITIES"
        );

        // Equip buffer: 7 layers × 4 floats per slot, must cover MAX_ENTITIES
        let equip_floats_per_slot = 7 * 4; // 28
        let equip_bytes = MAX_ENTITIES * equip_floats_per_slot * std::mem::size_of::<f32>();
        let equip_bytes_old_wrong = MAX_NPC_COUNT * 6 * std::mem::size_of::<[f32; 4]>();
        assert!(
            equip_bytes > equip_bytes_old_wrong,
            "equip buffer still uses old MAX_NPC_COUNT x 6-layer formula"
        );

        // Sentinel vectors must match buffer sizes exactly
        let sentinel_visual_len = MAX_ENTITIES * 8;
        let sentinel_equip_len = MAX_ENTITIES * 7 * 4;
        assert_eq!(
            sentinel_visual_len * std::mem::size_of::<f32>(),
            visual_bytes,
            "sentinel_visual length must match visual buffer byte size"
        );
        assert_eq!(
            sentinel_equip_len * std::mem::size_of::<f32>(),
            equip_bytes,
            "sentinel_equip length must match equip buffer byte size"
        );
    }

    /// Regression test for issue #187: build_building_body_instances must skip rebuild when dirty=false.
    /// If the guard is removed, the system would clear instances every frame regardless of dirty flag.
    #[test]
    fn building_body_instances_skips_rebuild_when_not_dirty() {
        let mut app = App::new();

        // Pre-populate instances with a sentinel value.
        let mut instances = BuildingBodyInstances::default();
        instances.0.push(InstanceData {
            position: [1.0, 2.0],
            sprite: [3.0, 4.0],
            color: [1.0, 1.0, 1.0, 1.0],
            health: 1.0,
            flash: 0.0,
            scale: 64.0,
            atlas_id: 1.0,
            rotation: 0.0,
        });

        app.insert_resource(instances)
            .insert_resource(BuildingBodyDirty {
                dirty: false,
                had_building_flash: false,
                last_building_count: 0,
            })
            .insert_resource(crate::resources::EntityMap::default())
            .insert_resource(crate::gpu::EntityGpuState::default())
            .add_systems(Update, build_building_body_instances);

        app.update();

        let result = app.world().resource::<BuildingBodyInstances>();
        assert_eq!(
            result.0.len(),
            1,
            "build_building_body_instances must not clear instances when dirty=false"
        );
    }

    /// Regression test for issue #187: mark_building_body_dirty must set dirty=true when building count changes.
    /// If the count check is removed, newly placed buildings would not trigger a rebuild.
    #[test]
    fn building_body_dirty_triggers_on_building_count_change() {
        use crate::entity_map::BuildingInstance;
        use crate::world::BuildingKind;

        let mut app = App::new();

        app.insert_resource(BuildingBodyDirty {
            dirty: false,
            had_building_flash: false,
            last_building_count: 0,
        })
        .insert_resource(crate::resources::EntityMap::default())
        .insert_resource(crate::gpu::EntityGpuState::default())
        .add_systems(Update, mark_building_body_dirty);

        // First update: no buildings -> count stays 0 -> dirty stays false.
        app.update();
        assert!(
            !app.world().resource::<BuildingBodyDirty>().dirty,
            "dirty should remain false when no buildings exist"
        );

        // Add a building to EntityMap to simulate a placement.
        app.world_mut()
            .resource_mut::<crate::resources::EntityMap>()
            .add_instance(BuildingInstance {
                kind: BuildingKind::Farm,
                position: bevy::math::Vec2::new(64.0, 64.0),
                town_idx: 1,
                slot: 100,
                faction: 1,
            });

        // Second update: count is now 1, last_building_count was 0 -> dirty=true.
        app.update();
        let dirty = app.world().resource::<BuildingBodyDirty>();
        assert!(
            dirty.dirty,
            "mark_building_body_dirty must set dirty=true when building count changes"
        );
        assert_eq!(
            dirty.last_building_count, 1,
            "last_building_count must be updated to new count"
        );
    }
}

/// Zero-clone extract: reads OverlayInstances from main world, writes to BuildingOverlayBuffers.
fn extract_overlay_instances(
    mut commands: Commands,
    overlay: Extract<Res<OverlayInstances>>,
    existing: Option<ResMut<BuildingOverlayBuffers>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    if let Some(mut buf) = existing {
        // Reuse existing RawBufferVec allocation
        buf.instances.clear();
        for inst in overlay.0.iter() {
            buf.instances.push(*inst);
        }
        buf.count = buf.instances.len() as u32;
        buf.instances.write_buffer(&render_device, &render_queue);
    } else {
        let mut instances = RawBufferVec::new(BufferUsages::VERTEX);
        for inst in overlay.0.iter() {
            instances.push(*inst);
        }
        let count = instances.len() as u32;
        instances.write_buffer(&render_device, &render_queue);
        commands.insert_resource(BuildingOverlayBuffers { instances, count });
    }
}

/// Zero-clone extract: reads BuildingBodyInstances from main world, writes to BuildingBodyRenderBuffers.
fn extract_building_body_instances(
    mut commands: Commands,
    body: Extract<Res<BuildingBodyInstances>>,
    existing: Option<ResMut<BuildingBodyRenderBuffers>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    if let Some(mut buf) = existing {
        buf.instances.clear();
        for inst in body.0.iter() {
            buf.instances.push(*inst);
        }
        buf.count = buf.instances.len() as u32;
        buf.instances.write_buffer(&render_device, &render_queue);
    } else {
        let mut instances = RawBufferVec::new(BufferUsages::VERTEX);
        for inst in body.0.iter() {
            instances.push(*inst);
        }
        let count = instances.len() as u32;
        instances.write_buffer(&render_device, &render_queue);
        commands.insert_resource(BuildingBodyRenderBuffers { instances, count });
    }
}

/// Zero-clone extract: reads SelectionOverlayInstances from main world, writes to SelectionRenderBuffers.
fn extract_selection_overlay(
    mut commands: Commands,
    sel: Extract<Res<SelectionOverlayInstances>>,
    existing: Option<ResMut<SelectionRenderBuffers>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    if let Some(mut buf) = existing {
        buf.instances.clear();
        for inst in sel.0.iter() {
            buf.instances.push(*inst);
        }
        buf.count = buf.instances.len() as u32;
        buf.instances.write_buffer(&render_device, &render_queue);
    } else {
        let mut instances = RawBufferVec::new(BufferUsages::VERTEX);
        for inst in sel.0.iter() {
            instances.push(*inst);
        }
        let count = instances.len() as u32;
        instances.write_buffer(&render_device, &render_queue);
        commands.insert_resource(SelectionRenderBuffers { instances, count });
    }
}

// =============================================================================
// EXTRACT: NPC + PROJECTILE DATA
// =============================================================================

// Upload NPC compute + visual data directly to GPU buffers during Extract phase.
// --- Shared dirty-write helpers (used by both NPC and projectile extract) ---

// Coalescing gap thresholds for CPU-AUTHORITATIVE buffers (slots).
// Tuned for DX12 backend: balances per-call overhead (~4μs) vs wasted bytes (~3KB/gap).
// Wider gaps merge more but upload non-dirty data; narrower gaps have more write_buffer calls.
const GAP_STRIDE_1: usize = 750; // speeds, factions, healths, flags (750 × 1 × 4 = 3KB/gap)
const GAP_STRIDE_2: usize = 375; // targets, half_sizes (375 × 2 × 4 = 3KB/gap)
const GAP_VISUAL: usize = 93; // visual_data (93 × 8 × 4 = 3KB/gap)
const GAP_EQUIP: usize = 27; // equip_data (27 × 28 × 4 = 3KB/gap)

/// Bulk-write the first `count` elements of `data` to `buf` in a single write_buffer call.
fn write_bulk<T: bytemuck::NoUninit>(queue: &RenderQueue, buf: &Buffer, data: &[T], count: usize) {
    let count = count.min(data.len());
    if count > 0 {
        queue.write_buffer(buf, 0, bytemuck::cast_slice(&data[..count]));
    }
}

/// Coalesce pre-sorted dirty indices into contiguous ranges, one write_buffer per range.
/// Falls back to offset bulk write when dirty coverage > 40% of the dirty window.
fn write_coalesced_f32(
    queue: &RenderQueue,
    buf: &Buffer,
    data: &[f32],
    dirty: &[usize],
    stride: usize,
    gap: usize,
) {
    if dirty.is_empty() {
        return;
    }
    let first = dirty[0];
    let last = dirty[dirty.len() - 1];
    let window = last - first + 1;
    if dirty.len() > window * 2 / 5 {
        let start = first * stride;
        let end = ((last + 1) * stride).min(data.len());
        if start < end {
            queue.write_buffer(
                buf,
                (start * 4) as u64,
                bytemuck::cast_slice(&data[start..end]),
            );
        }
        return;
    }
    let mut range_start = first;
    let mut range_end = first;
    for &idx in &dirty[1..] {
        if idx <= range_end.saturating_add(gap + 1) {
            range_end = idx;
        } else {
            flush_range_f32(queue, buf, data, range_start, range_end, stride);
            range_start = idx;
            range_end = idx;
        }
    }
    flush_range_f32(queue, buf, data, range_start, range_end, stride);
}

fn flush_range_f32(
    queue: &RenderQueue,
    buf: &Buffer,
    data: &[f32],
    start_idx: usize,
    end_idx: usize,
    stride: usize,
) {
    let start = start_idx * stride;
    let end = (end_idx + 1) * stride;
    debug_assert!(
        end <= data.len(),
        "coalesced range overflows: {}..{} len={}",
        start,
        end,
        data.len()
    );
    if end > data.len() {
        return;
    }
    queue.write_buffer(
        buf,
        (start * 4) as u64,
        bytemuck::cast_slice(&data[start..end]),
    );
}

fn write_coalesced_i32(
    queue: &RenderQueue,
    buf: &Buffer,
    data: &[i32],
    dirty: &[usize],
    stride: usize,
    gap: usize,
) {
    if dirty.is_empty() {
        return;
    }
    let first = dirty[0];
    let last = dirty[dirty.len() - 1];
    let window = last - first + 1;
    if dirty.len() > window * 2 / 5 {
        let start = first * stride;
        let end = ((last + 1) * stride).min(data.len());
        if start < end {
            queue.write_buffer(
                buf,
                (start * 4) as u64,
                bytemuck::cast_slice(&data[start..end]),
            );
        }
        return;
    }
    let mut range_start = first;
    let mut range_end = first;
    for &idx in &dirty[1..] {
        if idx <= range_end.saturating_add(gap + 1) {
            range_end = idx;
        } else {
            let s = range_start * stride;
            let e = ((range_end + 1) * stride).min(data.len());
            if s < e {
                queue.write_buffer(buf, (s * 4) as u64, bytemuck::cast_slice(&data[s..e]));
            }
            range_start = idx;
            range_end = idx;
        }
    }
    let s = range_start * stride;
    let e = ((range_end + 1) * stride).min(data.len());
    if s < e {
        queue.write_buffer(buf, (s * 4) as u64, bytemuck::cast_slice(&data[s..e]));
    }
}

fn write_coalesced_u32(
    queue: &RenderQueue,
    buf: &Buffer,
    data: &[u32],
    dirty: &[usize],
    stride: usize,
    gap: usize,
) {
    if dirty.is_empty() {
        return;
    }
    let first = dirty[0];
    let last = dirty[dirty.len() - 1];
    let window = last - first + 1;
    if dirty.len() > window * 2 / 5 {
        let start = first * stride;
        let end = ((last + 1) * stride).min(data.len());
        if start < end {
            queue.write_buffer(
                buf,
                (start * 4) as u64,
                bytemuck::cast_slice(&data[start..end]),
            );
        }
        return;
    }
    let mut range_start = first;
    let mut range_end = first;
    for &idx in &dirty[1..] {
        if idx <= range_end.saturating_add(gap + 1) {
            range_end = idx;
        } else {
            let s = range_start * stride;
            let e = ((range_end + 1) * stride).min(data.len());
            if s < e {
                queue.write_buffer(buf, (s * 4) as u64, bytemuck::cast_slice(&data[s..e]));
            }
            range_start = idx;
            range_end = idx;
        }
    }
    let s = range_start * stride;
    let e = ((range_end + 1) * stride).min(data.len());
    if s < e {
        queue.write_buffer(buf, (s * 4) as u64, bytemuck::cast_slice(&data[s..e]));
    }
}

/// Strict coalesce for GPU-authoritative buffers. Merges only exactly-adjacent
/// dirty indices (idx == prev + 1). No gap merging, no dense bulk fallback.
/// Dirty indices MUST be sorted+deduped (debug-asserted).
fn write_coalesced_exact_f32(
    queue: &RenderQueue,
    buf: &Buffer,
    data: &[f32],
    dirty: &[usize],
    stride: usize,
) {
    if dirty.is_empty() {
        return;
    }
    debug_assert!(
        dirty.array_windows().all(|[a, b]| a < b),
        "dirty indices not sorted+deduped"
    );
    debug_assert!(
        dirty[0] * stride + stride <= data.len(),
        "first dirty index {} out of bounds (len={})",
        dirty[0],
        data.len()
    );
    let mut range_start = dirty[0];
    let mut range_end = dirty[0];
    for &idx in &dirty[1..] {
        debug_assert!(
            idx * stride + stride <= data.len(),
            "dirty index {idx} out of bounds (len={})",
            data.len()
        );
        if idx == range_end.saturating_add(1) {
            range_end = idx;
        } else {
            flush_range_f32(queue, buf, data, range_start, range_end, stride);
            range_start = idx;
            range_end = idx;
        }
    }
    flush_range_f32(queue, buf, data, range_start, range_end, stride);
}

/// Zero-clone NPC extract: reads main world via Extract<Res<T>>, writes directly to GPU.
fn extract_npc_data(
    gpu_state: Extract<Res<EntityGpuState>>,
    config: Extract<Res<RenderFrameConfig>>,
    visual_upload: Extract<Res<NpcVisualUpload>>,
    slots: Extract<Res<GpuSlotPool>>,
    gpu_buffers: Option<Res<EntityGpuBuffers>>,
    visual_buffers: Option<Res<NpcVisualBuffers>>,
    render_queue: Res<RenderQueue>,
    mut prev_target_size: Local<usize>,
) {
    use crate::messages::{
        EXTRACT_DIRTY_COUNTS, RENDER_PROFILING, RENDER_TIMINGS, RT_EXTRACT_COMPUTE, RT_EXTRACT_NPC,
        RT_EXTRACT_VISUAL,
    };
    use std::sync::atomic::Ordering;
    let profiling = RENDER_PROFILING.load(Ordering::Relaxed);
    let start = if profiling {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // --- Sub-timing: compute buffers ---
    let t0 = std::time::Instant::now();
    if let Some(gpu_bufs) = gpu_buffers {
        // Read live count from authoritative GpuSlotPool — not stale RenderFrameConfig copy
        let n = slots.count();
        // Positions: strict coalescing — GPU writes positions[i] every frame, stale CPU values teleport NPCs
        write_coalesced_exact_f32(
            &render_queue,
            &gpu_bufs.positions,
            &gpu_state.positions,
            &gpu_state.position_dirty_indices,
            2,
        );
        // Arrivals: gap-safe — CPU array is always 0, GPU re-computes settled=1 same frame
        write_coalesced_i32(
            &render_queue,
            &gpu_bufs.arrivals,
            &gpu_state.arrivals,
            &gpu_state.arrival_dirty_indices,
            1,
            GAP_STRIDE_1,
        );
        // CPU-authoritative: gap-based coalescing safe (EntityGpuState is ground truth)
        if gpu_state.dirty_targets {
            if *prev_target_size != n {
                write_bulk(&render_queue, &gpu_bufs.targets, &gpu_state.targets, n * 2);
                *prev_target_size = n;
            } else {
                write_coalesced_f32(
                    &render_queue,
                    &gpu_bufs.targets,
                    &gpu_state.targets,
                    &gpu_state.target_dirty_indices,
                    2,
                    GAP_STRIDE_2,
                );
            }
        }
        write_coalesced_f32(
            &render_queue,
            &gpu_bufs.speeds,
            &gpu_state.speeds,
            &gpu_state.speed_dirty_indices,
            1,
            GAP_STRIDE_1,
        );
        write_coalesced_i32(
            &render_queue,
            &gpu_bufs.factions,
            &gpu_state.factions,
            &gpu_state.faction_dirty_indices,
            1,
            GAP_STRIDE_1,
        );
        write_coalesced_f32(
            &render_queue,
            &gpu_bufs.healths,
            &gpu_state.healths,
            &gpu_state.health_dirty_indices,
            1,
            GAP_STRIDE_1,
        );
        write_coalesced_u32(
            &render_queue,
            &gpu_bufs.entity_flags,
            &gpu_state.entity_flags,
            &gpu_state.flags_dirty_indices,
            1,
            GAP_STRIDE_1,
        );
        write_coalesced_f32(
            &render_queue,
            &gpu_bufs.half_sizes,
            &gpu_state.half_sizes,
            &gpu_state.half_size_dirty_indices,
            2,
            GAP_STRIDE_2,
        );
        // Road flags: upload when present (rebuilt when roads change)
        if !config.tile_flags.is_empty() {
            render_queue.write_buffer(
                &gpu_bufs.tile_flags,
                0,
                bytemuck::cast_slice(&config.tile_flags),
            );
        }
    }
    let t1 = std::time::Instant::now();

    // --- Sub-timing: visual data ---
    if let Some(vis_bufs) = visual_buffers {
        if visual_upload.visual_full_upload {
            if visual_upload.entity_count > 0 {
                render_queue.write_buffer(
                    &vis_bufs.visual,
                    0,
                    bytemuck::cast_slice(&visual_upload.visual_data),
                );
                render_queue.write_buffer(
                    &vis_bufs.equip,
                    0,
                    bytemuck::cast_slice(&visual_upload.equip_data),
                );
            }
        } else if !visual_upload.visual_uploaded_indices.is_empty() {
            write_coalesced_f32(
                &render_queue,
                &vis_bufs.visual,
                &visual_upload.visual_data,
                &visual_upload.visual_uploaded_indices,
                8,
                GAP_VISUAL,
            );
            // Equip uses separate indices — excludes flash-only slots (equipment didn't change)
            if !visual_upload.equip_uploaded_indices.is_empty() {
                write_coalesced_f32(
                    &render_queue,
                    &vis_bufs.equip,
                    &visual_upload.equip_data,
                    &visual_upload.equip_uploaded_indices,
                    28,
                    GAP_EQUIP,
                );
            }
        }
    }
    let t2 = std::time::Instant::now();

    if profiling {
        let compute_ms = (t1 - t0).as_secs_f64() as f32 * 1000.0;
        let visual_ms = (t2 - t1).as_secs_f64() as f32 * 1000.0;
        RENDER_TIMINGS[RT_EXTRACT_COMPUTE].store(compute_ms.to_bits(), Ordering::Relaxed);
        RENDER_TIMINGS[RT_EXTRACT_VISUAL].store(visual_ms.to_bits(), Ordering::Relaxed);
    }

    // Store dirty counts for profiler tab
    EXTRACT_DIRTY_COUNTS[0].store(
        gpu_state.position_dirty_indices.len() as u32,
        Ordering::Relaxed,
    );
    EXTRACT_DIRTY_COUNTS[1].store(
        gpu_state.arrival_dirty_indices.len() as u32,
        Ordering::Relaxed,
    );
    EXTRACT_DIRTY_COUNTS[2].store(
        gpu_state.target_dirty_indices.len() as u32,
        Ordering::Relaxed,
    );
    EXTRACT_DIRTY_COUNTS[3].store(
        gpu_state.speed_dirty_indices.len() as u32,
        Ordering::Relaxed,
    );
    EXTRACT_DIRTY_COUNTS[4].store(
        gpu_state.faction_dirty_indices.len() as u32,
        Ordering::Relaxed,
    );
    EXTRACT_DIRTY_COUNTS[5].store(
        gpu_state.health_dirty_indices.len() as u32,
        Ordering::Relaxed,
    );
    EXTRACT_DIRTY_COUNTS[6].store(
        gpu_state.flags_dirty_indices.len() as u32,
        Ordering::Relaxed,
    );
    EXTRACT_DIRTY_COUNTS[7].store(
        gpu_state.half_size_dirty_indices.len() as u32,
        Ordering::Relaxed,
    );
    EXTRACT_DIRTY_COUNTS[8].store(
        visual_upload.visual_uploaded_indices.len() as u32,
        Ordering::Relaxed,
    );
    EXTRACT_DIRTY_COUNTS[9].store(
        visual_upload.equip_uploaded_indices.len() as u32,
        Ordering::Relaxed,
    );

    if let Some(s) = start {
        RENDER_TIMINGS[RT_EXTRACT_NPC].store(
            (s.elapsed().as_secs_f64() as f32 * 1000.0).to_bits(),
            Ordering::Relaxed,
        );
    }
}

/// Zero-clone projectile extract: compute dirty writes + instance buffer building.
/// Replaces both write_proj_buffers (gpu.rs) and prepare_proj_buffers.
fn extract_proj_data(
    mut commands: Commands,
    writes: Extract<Res<ProjBufferWrites>>,
    proj_pos_state: Extract<Res<crate::resources::ProjPositionState>>,
    gpu_buffers: Option<Res<ProjGpuBuffers>>,
    existing_buffers: Option<ResMut<ProjRenderBuffers>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    use crate::messages::{RENDER_PROFILING, RENDER_TIMINGS, RT_EXTRACT_PROJ};
    use std::sync::atomic::Ordering;
    let profiling = RENDER_PROFILING.load(Ordering::Relaxed);
    let start = if profiling {
        Some(std::time::Instant::now())
    } else {
        None
    };
    // --- Compute data: coalesced dirty writes (same pattern as NPC uploads) ---
    if let Some(gpu_bufs) = gpu_buffers {
        if writes.dirty {
            const GAP: usize = 8;
            // Spawn: coalesce all fields
            let mut spawn = writes.spawn_dirty_indices.clone();
            spawn.sort_unstable();
            write_coalesced_f32(
                &render_queue,
                &gpu_bufs.positions,
                &writes.positions,
                &spawn,
                2,
                GAP,
            );
            write_coalesced_f32(
                &render_queue,
                &gpu_bufs.velocities,
                &writes.velocities,
                &spawn,
                2,
                GAP,
            );
            write_coalesced_f32(
                &render_queue,
                &gpu_bufs.damages,
                &writes.damages,
                &spawn,
                1,
                GAP,
            );
            write_coalesced_i32(
                &render_queue,
                &gpu_bufs.factions,
                &writes.factions,
                &spawn,
                1,
                GAP,
            );
            write_coalesced_i32(
                &render_queue,
                &gpu_bufs.shooters,
                &writes.shooters,
                &spawn,
                1,
                GAP,
            );
            write_coalesced_f32(
                &render_queue,
                &gpu_bufs.lifetimes,
                &writes.lifetimes,
                &spawn,
                1,
                GAP,
            );
            write_coalesced_i32(
                &render_queue,
                &gpu_bufs.homing_targets,
                &writes.homing_targets,
                &spawn,
                1,
                GAP,
            );
            write_coalesced_i32(
                &render_queue,
                &gpu_bufs.active,
                &writes.active,
                &spawn,
                1,
                GAP,
            );
            write_coalesced_i32(&render_queue, &gpu_bufs.hits, &writes.hits, &spawn, 2, GAP);
            // Deactivate: coalesce active + hits + homing_targets
            let mut deact = writes.deactivate_dirty_indices.clone();
            deact.sort_unstable();
            write_coalesced_i32(
                &render_queue,
                &gpu_bufs.homing_targets,
                &writes.homing_targets,
                &deact,
                1,
                GAP,
            );
            write_coalesced_i32(
                &render_queue,
                &gpu_bufs.active,
                &writes.active,
                &deact,
                1,
                GAP,
            );
            write_coalesced_i32(&render_queue, &gpu_bufs.hits, &writes.hits, &deact, 2, GAP);
        }
    }

    // --- Build projectile instance buffer for rendering ---
    let readback_positions = &proj_pos_state.0;

    let mut instances = RawBufferVec::new(BufferUsages::VERTEX);
    instances.reserve(writes.active_set.len(), &render_device);
    for &i in &writes.active_set {
        let i2 = i * 2;

        let (px, py) = if i2 + 1 < readback_positions.len() {
            (readback_positions[i2], readback_positions[i2 + 1])
        } else {
            (
                writes.positions.get(i2).copied().unwrap_or(0.0),
                writes.positions.get(i2 + 1).copied().unwrap_or(0.0),
            )
        };
        if px < -9000.0 {
            continue;
        }

        let faction = writes.factions[i];
        let (cr, cg, cb) = if faction == crate::constants::FACTION_PLAYER {
            (0.0, 0.0, 1.0)
        } else {
            let (r, g, b, _) = crate::constants::raider_faction_color(faction);
            (r, g, b)
        };

        let vx = writes.velocities[i2];
        let vy = writes.velocities[i2 + 1];
        let angle = vy.atan2(vx) - std::f32::consts::FRAC_PI_2;

        instances.push(InstanceData {
            position: [px, py],
            sprite: [0.0, 0.0],
            color: [cr, cg, cb, 1.0],
            health: 1.0,
            flash: 0.0,
            scale: 32.0,
            atlas_id: 4.0,
            rotation: angle,
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

    if let Some(s) = start {
        RENDER_TIMINGS[RT_EXTRACT_PROJ].store(
            (s.elapsed().as_secs_f64() as f32 * 1000.0).to_bits(),
            Ordering::Relaxed,
        );
    }
}

// =============================================================================
// PREPARE
// =============================================================================

/// Prepare misc instance buffer (farms, building HP bars) + NPC visual buffer creation/bind groups.
fn prepare_npc_buffers(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline: Option<Res<NpcPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    gpu_buffers: Option<Res<EntityGpuBuffers>>,
    existing_render: Option<ResMut<NpcRenderBuffers>>,
    existing_visual: Option<ResMut<NpcVisualBuffers>>,
) {
    use crate::messages::{RENDER_PROFILING, RENDER_TIMINGS, RT_PREPARE_NPC};
    use std::sync::atomic::Ordering;
    let profiling = RENDER_PROFILING.load(Ordering::Relaxed);
    let start = if profiling {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // --- NPC visual storage buffers ---
    // Visual data is uploaded by extract_npc_data in Extract phase (zero clone).
    // Here we only handle: first-frame buffer creation, bind group recreation, quad geometry.
    if let Some(mut visual_buffers) = existing_visual {
        // Recreate bind group each frame (gpu_buffers may not exist on first frame)
        if let (Some(gpu_bufs), Some(pipeline)) = (gpu_buffers.as_ref(), pipeline.as_ref()) {
            let layout = pipeline_cache.get_bind_group_layout(&pipeline.npc_data_bind_group_layout);
            visual_buffers.bind_group = Some(render_device.create_bind_group(
                Some("npc_storage_bind_group"),
                &layout,
                &BindGroupEntries::sequential((
                    gpu_bufs.positions.as_entire_binding(),
                    gpu_bufs.healths.as_entire_binding(),
                    visual_buffers.visual.as_entire_binding(),
                    visual_buffers.equip.as_entire_binding(),
                )),
            ));
        }
    } else {
        // First run: create storage buffers with sentinel data (all hidden)
        // Buffers cover the full unified slot namespace (NPCs + buildings = MAX_ENTITIES).
        // Equip uses 7 layers x [col, row, atlas, pad] = 28 floats per slot.
        let visual_buffer = render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_visual_data"),
            size: (MAX_ENTITIES * std::mem::size_of::<[f32; 8]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let equip_buffer = render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_equip_data"),
            size: (MAX_ENTITIES * 7 * std::mem::size_of::<[f32; 4]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Write sentinel -1.0 so all sprites are hidden until extract_npc_data writes real data
        let sentinel_visual = vec![-1.0f32; MAX_ENTITIES * 8];
        let sentinel_equip = vec![-1.0f32; MAX_ENTITIES * 7 * 4];
        render_queue.write_buffer(&visual_buffer, 0, bytemuck::cast_slice(&sentinel_visual));
        render_queue.write_buffer(&equip_buffer, 0, bytemuck::cast_slice(&sentinel_equip));

        // Create bind group if gpu_buffers available
        let bind_group = if let (Some(gpu_bufs), Some(pipeline)) =
            (gpu_buffers.as_ref(), pipeline.as_ref())
        {
            let layout = pipeline_cache.get_bind_group_layout(&pipeline.npc_data_bind_group_layout);
            Some(render_device.create_bind_group(
                Some("npc_storage_bind_group"),
                &layout,
                &BindGroupEntries::sequential((
                    gpu_bufs.positions.as_entire_binding(),
                    gpu_bufs.healths.as_entire_binding(),
                    visual_buffer.as_entire_binding(),
                    equip_buffer.as_entire_binding(),
                )),
            ))
        } else {
            None
        };

        commands.insert_resource(NpcVisualBuffers {
            visual: visual_buffer,
            equip: equip_buffer,
            bind_group,
        });
    }

    // --- Shared quad geometry (created once) ---
    if existing_render.is_none() {
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
        });
    }

    if let Some(s) = start {
        RENDER_TIMINGS[RT_PREPARE_NPC].store(
            (s.elapsed().as_secs_f64() as f32 * 1000.0).to_bits(),
            Ordering::Relaxed,
        );
    }
}

/// Prepare texture bind group (4 atlases: char, world, building, extras).
fn prepare_npc_texture_bind_group(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline: Option<Res<NpcPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    config: Option<Res<RenderFrameConfig>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    mut atlases_ready: Local<bool>,
) {
    let Some(pipeline) = pipeline else { return };
    let Some(config) = config else { return };
    let textures = &config.textures;
    let Some(char_handle) = &textures.handle else {
        return;
    };
    let Some(world_handle) = &textures.world_handle else {
        return;
    };
    let Some(char_image) = gpu_images.get(char_handle) else {
        return;
    };
    let Some(world_image) = gpu_images.get(world_handle) else {
        return;
    };

    // Building + extras atlases: fallback to char_image until spawn_world_tilemap composites them
    let building_image = textures
        .building_handle
        .as_ref()
        .and_then(|h| gpu_images.get(h));
    let extras_image = textures
        .extras_handle
        .as_ref()
        .and_then(|h| gpu_images.get(h));
    if !*atlases_ready {
        if building_image.is_some() && extras_image.is_some() {
            info!("Building + extras atlases bound");
            *atlases_ready = true;
        }
    }
    let building_image = building_image.unwrap_or(char_image);
    let extras_image = extras_image.unwrap_or(char_image);

    let layout = pipeline_cache.get_bind_group_layout(&pipeline.texture_bind_group_layout);

    let bind_group = render_device.create_bind_group(
        Some("npc_texture_bind_group"),
        &layout,
        &BindGroupEntries::sequential((
            &char_image.texture_view,
            &char_image.sampler,
            &world_image.texture_view,
            &world_image.sampler,
            &building_image.texture_view,
            &building_image.sampler,
            &extras_image.texture_view,
            &extras_image.sampler,
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
    config: Option<Res<RenderFrameConfig>>,
) {
    let Some(pipeline) = pipeline else { return };
    let Some(camera_state) = camera_state else {
        return;
    };

    let uniform = CameraUniform {
        camera_pos: camera_state.position,
        zoom: camera_state.zoom,
        entity_count: config.map(|c| c.npc.count).unwrap_or(0),
        viewport: camera_state.viewport,
        bldg_layers: (crate::constants::BUILDING_REGISTRY.len()
            + crate::constants::autotile_total_extra_layers()) as f32,
        extras_cols: 4.0,
        lod_zoom: camera_state.lod_zoom,
        hp_bar_mode: camera_state.hp_bar_mode,
    };

    let mut buffer = UniformBuffer::from(uniform);
    buffer.write_buffer(&render_device, &render_queue);

    let Some(binding) = buffer.binding() else {
        return;
    };

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

/// Add a single Transparent2d phase item with the given draw function, pipeline, and sort key.
fn queue_phase_item(
    phase: &mut SortedRenderPhase<Transparent2d>,
    draw_function: bevy::render::render_phase::DrawFunctionId,
    pipeline: bevy::render::render_resource::CachedRenderPipelineId,
    sort_key: f32,
    view_entity: Entity,
    batch_entity: Entity,
) {
    phase.add(Transparent2d {
        entity: (view_entity, MainEntity::from(batch_entity)),
        draw_function,
        pipeline,
        sort_key: FloatOrd(sort_key),
        batch_range: 0..1,
        extra_index: PhaseItemExtraIndex::None,
        extracted_index: usize::MAX,
        indexed: true,
    });
}

/// Queue building/NPC storage + building overlay instance draws into Transparent2d phase.
fn queue_npcs(
    draw_functions: Res<DrawFunctions<Transparent2d>>,
    pipeline: Res<NpcPipeline>,
    mut pipelines: ResMut<SpecializedRenderPipelines<NpcPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    render_buffers: Option<Res<NpcRenderBuffers>>,
    visual_buffers: Option<Res<NpcVisualBuffers>>,
    overlay_buffers: Option<Res<BuildingOverlayBuffers>>,
    body_buffers: Option<Res<BuildingBodyRenderBuffers>>,
    selection_buffers: Option<Res<SelectionRenderBuffers>>,
    config: Option<Res<RenderFrameConfig>>,
    mut transparent_phases: ResMut<ViewSortedRenderPhases<Transparent2d>>,
    views: Query<(Entity, &ExtractedView, &Msaa)>,
    npc_batch: Query<Entity, With<NpcBatch>>,
) {
    use crate::messages::{RENDER_PROFILING, RENDER_TIMINGS, RT_QUEUE_NPC};
    use std::sync::atomic::Ordering;
    let profiling = RENDER_PROFILING.load(Ordering::Relaxed);
    let start = if profiling {
        Some(std::time::Instant::now())
    } else {
        None
    };
    let Some(_render_buffers) = render_buffers else {
        return;
    };

    let has_npcs = visual_buffers
        .as_ref()
        .is_some_and(|vb| vb.bind_group.is_some())
        && config.as_ref().is_some_and(|c| c.npc.count > 0);
    let has_building_overlays = overlay_buffers.as_ref().is_some_and(|m| m.count > 0);
    let has_building_bodies = body_buffers.as_ref().is_some_and(|b| b.count > 0);
    let has_selection = selection_buffers.as_ref().is_some_and(|s| s.count > 0);

    if !has_npcs && !has_building_overlays && !has_building_bodies && !has_selection {
        return;
    }

    let building_body_draw = draw_functions.read().id::<DrawBuildingBodyCommands>();
    let building_overlay_draw = draw_functions.read().id::<DrawBuildingOverlayCommands>();
    let npc_body_draw = draw_functions.read().id::<DrawNpcBodyCommands>();
    let npc_overlay_draw = draw_functions.read().id::<DrawNpcOverlayCommands>();

    for (view_entity, view, msaa) in &views {
        let Some(transparent_phase) = transparent_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };

        for batch_entity in &npc_batch {
            // Building bodies: instance buffer path (decoupled from NPC compute)
            if has_building_bodies {
                let building_body_pid = pipelines.specialize(
                    &pipeline_cache,
                    &pipeline,
                    (view.hdr, msaa.samples(), None),
                );
                queue_phase_item(
                    transparent_phase,
                    building_body_draw,
                    building_body_pid,
                    ORDER_BUILDING_BODY,
                    view_entity,
                    batch_entity,
                );
            }

            if has_building_overlays {
                let overlay_pid = pipelines.specialize(
                    &pipeline_cache,
                    &pipeline,
                    (view.hdr, msaa.samples(), None),
                );
                queue_phase_item(
                    transparent_phase,
                    building_overlay_draw,
                    overlay_pid,
                    ORDER_BUILDING_OVERLAY,
                    view_entity,
                    batch_entity,
                );
            }

            if has_npcs {
                let npc_body_pid = pipelines.specialize(
                    &pipeline_cache,
                    &pipeline,
                    (view.hdr, msaa.samples(), Some(StorageDrawMode::NpcBody)),
                );
                queue_phase_item(
                    transparent_phase,
                    npc_body_draw,
                    npc_body_pid,
                    ORDER_NPC_BODY,
                    view_entity,
                    batch_entity,
                );

                let npc_overlay_pid = pipelines.specialize(
                    &pipeline_cache,
                    &pipeline,
                    (view.hdr, msaa.samples(), Some(StorageDrawMode::NpcOverlay)),
                );
                queue_phase_item(
                    transparent_phase,
                    npc_overlay_draw,
                    npc_overlay_pid,
                    ORDER_NPC_OVERLAY,
                    view_entity,
                    batch_entity,
                );
            }

            if has_selection {
                let sel_draw = draw_functions.read().id::<DrawSelectionBracketCommands>();
                let sel_pid = pipelines.specialize(
                    &pipeline_cache,
                    &pipeline,
                    (
                        view.hdr,
                        msaa.samples(),
                        Some(StorageDrawMode::SelectionBracket),
                    ),
                );
                queue_phase_item(
                    transparent_phase,
                    sel_draw,
                    sel_pid,
                    ORDER_SELECTION_OVERLAY,
                    view_entity,
                    batch_entity,
                );
            }
        }
    }

    if let Some(s) = start {
        RENDER_TIMINGS[RT_QUEUE_NPC].store(
            (s.elapsed().as_secs_f64() as f32 * 1000.0).to_bits(),
            Ordering::Relaxed,
        );
    }
}

// =============================================================================
// PIPELINE SPECIALIZATION
// =============================================================================

/// Quad vertex buffer layout (slot 0) — shared by both paths.
fn quad_vertex_layout() -> VertexBufferLayout {
    VertexBufferLayout {
        array_stride: std::mem::size_of::<QuadVertex>() as u64,
        step_mode: VertexStepMode::Vertex,
        attributes: vec![
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            },
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Float32x2,
                offset: 8,
                shader_location: 1,
            },
        ],
    }
}

/// Instance data vertex buffer layout (slot 1) — used by instance path only.
fn instance_vertex_layout() -> VertexBufferLayout {
    VertexBufferLayout {
        array_stride: std::mem::size_of::<InstanceData>() as u64,
        step_mode: VertexStepMode::Instance,
        attributes: vec![
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 2,
            },
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Float32x2,
                offset: 8,
                shader_location: 3,
            },
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Float32x4,
                offset: 16,
                shader_location: 4,
            },
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Float32,
                offset: 32,
                shader_location: 5,
            },
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Float32,
                offset: 36,
                shader_location: 6,
            },
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Float32,
                offset: 40,
                shader_location: 7,
            },
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Float32,
                offset: 44,
                shader_location: 8,
            },
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Float32,
                offset: 48,
                shader_location: 9,
            },
        ],
    }
}

/// Selection bracket instance layout (slot 1): slot(u32) + color(vec4) + scale(f32) + y_offset(f32).
fn selection_instance_layout() -> VertexBufferLayout {
    VertexBufferLayout {
        array_stride: std::mem::size_of::<SelectionInstance>() as u64,
        step_mode: VertexStepMode::Instance,
        attributes: vec![
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Uint32,
                offset: 0,
                shader_location: 2, // slot
            },
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Float32x4,
                offset: 4,
                shader_location: 3, // color
            },
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Float32,
                offset: 20,
                shader_location: 4, // scale
            },
            VertexAttribute {
                format: bevy::render::render_resource::VertexFormat::Float32,
                offset: 24,
                shader_location: 5, // y_offset
            },
        ],
    }
}

impl SpecializedRenderPipeline for NpcPipeline {
    type Key = (bool, u32, Option<StorageDrawMode>); // (HDR, MSAA, storage mode or instance)

    fn specialize(&self, (hdr, sample_count, mode): Self::Key) -> RenderPipelineDescriptor {
        let format = if hdr {
            TextureFormat::Rgba16Float
        } else {
            TextureFormat::Rgba8UnormSrgb
        };

        let storage_layout = vec![
            self.texture_bind_group_layout.clone(),
            self.camera_bind_group_layout.clone(),
            self.npc_data_bind_group_layout.clone(),
        ];
        let instance_layout = vec![
            self.texture_bind_group_layout.clone(),
            self.camera_bind_group_layout.clone(),
        ];

        let (label, layout, entry_point, buffers, vertex_shader_defs) = match mode {
            Some(StorageDrawMode::BuildingBody) => (
                "building_body_pipeline",
                storage_layout,
                "vertex_npc",
                vec![quad_vertex_layout()],
                vec!["MODE_BUILDING_BODY".into()],
            ),
            Some(StorageDrawMode::NpcBody) => (
                "npc_body_pipeline",
                storage_layout,
                "vertex_npc",
                vec![quad_vertex_layout()],
                vec!["MODE_NPC_BODY".into()],
            ),
            Some(StorageDrawMode::NpcOverlay) => (
                "npc_overlay_pipeline",
                storage_layout,
                "vertex_npc",
                vec![quad_vertex_layout()],
                vec!["MODE_NPC_OVERLAY".into()],
            ),
            Some(StorageDrawMode::SelectionBracket) => (
                "selection_bracket_pipeline",
                storage_layout,
                "vertex_selection",
                vec![quad_vertex_layout(), selection_instance_layout()],
                vec!["MODE_SELECTION_BRACKET".into()],
            ),
            None => (
                "npc_instance_pipeline",
                instance_layout,
                "vertex",
                vec![quad_vertex_layout(), instance_vertex_layout()],
                vec![],
            ),
        };

        RenderPipelineDescriptor {
            label: Some(label.into()),
            layout,
            vertex: VertexState {
                shader: self.shader.clone(),
                shader_defs: vertex_shader_defs,
                entry_point: Some(Cow::from(entry_point)),
                buffers,
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
            // Depth policy: these passes are strictly 2D at z=0 with no occluders.
            // CompareFunction::Always makes sort-key the sole ordering guarantee.
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: CompareFunction::Always,
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

fn init_npc_render_pipeline(mut commands: Commands, asset_server: Res<AssetServer>) {
    // 4 texture+sampler pairs: char, world, building, extras
    let texture_bind_group_layout = BindGroupLayoutDescriptor::new(
        "npc_texture_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );

    let camera_bind_group_layout = BindGroupLayoutDescriptor::new(
        "npc_camera_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (uniform_buffer::<CameraUniform>(false),),
        ),
    );

    let npc_data_bind_group_layout = BindGroupLayoutDescriptor::new(
        "npc_data_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX,
            (
                storage_buffer_read_only::<Vec<[f32; 2]>>(false),
                storage_buffer_read_only::<Vec<f32>>(false),
                storage_buffer_read_only::<Vec<[f32; 8]>>(false),
                storage_buffer_read_only::<Vec<[f32; 4]>>(false),
            ),
        ),
    );

    commands.insert_resource(NpcPipeline {
        shader: asset_server.load("shaders/npc_render.wgsl"),
        texture_bind_group_layout,
        camera_bind_group_layout,
        npc_data_bind_group_layout,
    });
}

// =============================================================================
// PROJECTILE RENDERING
// =============================================================================

fn spawn_proj_batch(mut commands: Commands) {
    commands.spawn((ProjBatch, Transform::default(), Visibility::default()));
    info!("Projectile batch entity spawned");
}

/// Extract projectile batch entity to render world. Despawns stale copies first.
fn extract_proj_batch(
    mut commands: Commands,
    query: Extract<Query<Entity, With<ProjBatch>>>,
    stale: Query<Entity, With<ProjBatch>>,
) {
    for entity in &stale {
        commands.entity(entity).despawn();
    }
    for entity in &query {
        commands.spawn((ProjBatch, MainEntity::from(entity)));
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
    let Some(_npc_buffers) = npc_buffers else {
        return;
    };
    let Some(proj_buffers) = proj_buffers else {
        return;
    };
    if proj_buffers.instance_count == 0 {
        return;
    }

    let draw_function = draw_functions.read().id::<DrawProjCommands>();

    for (view_entity, view, msaa) in &views {
        let Some(transparent_phase) = transparent_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };

        let pipeline_id =
            pipelines.specialize(&pipeline_cache, &pipeline, (view.hdr, msaa.samples(), None));

        for batch_entity in &proj_batch {
            queue_phase_item(
                transparent_phase,
                draw_function,
                pipeline_id,
                ORDER_PROJECTILES,
                view_entity,
                batch_entity,
            );
        }
    }
}
