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
            binding_types::{sampler, storage_buffer_read_only, texture_2d, uniform_buffer},
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            BlendState, Buffer, BufferDescriptor, BufferInitDescriptor, BufferUsages,
            ColorTargetState, ColorWrites, CompareFunction, DepthBiasState, DepthStencilState,
            FragmentState, IndexFormat, MultisampleState, PipelineCache, PrimitiveState,
            RawBufferVec, RenderPipelineDescriptor, SamplerBindingType, ShaderStages, ShaderType,
            SpecializedRenderPipeline, SpecializedRenderPipelines, StencilState, TextureFormat,
            TextureSampleType, UniformBuffer, VertexAttribute, VertexState, VertexStepMode,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::MainEntity,
        texture::GpuImage,
        view::ExtractedView,
        Extract, Render, RenderApp, RenderSystems,
    },
};
use bytemuck::{Pod, Zeroable};

use crate::constants::MAX_NPC_COUNT;
use crate::gpu::{NpcGpuState, NpcGpuBuffers, NpcGpuData, NpcVisualUpload, NpcSpriteTexture, ProjBufferWrites, ProjGpuData};
use crate::render::{CameraState, MainCamera};

// =============================================================================
// MARKER COMPONENT
// =============================================================================

/// Layer count: body + 4 equipment layers + status + healing.
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

/// Instance buffer for non-NPC sprites (farms, building HP bars).
#[derive(Resource)]
pub struct NpcMiscBuffers {
    pub instances: RawBufferVec<InstanceData>,
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
/// Field order matches WGSL Camera struct layout (npc_count fills alignment padding).
#[derive(Clone, ShaderType)]
pub struct CameraUniform {
    pub camera_pos: Vec2,
    pub zoom: f32,
    pub npc_count: u32,
    pub viewport: Vec2,
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

/// Draw command for NPC storage buffer path — 7 draw calls (body + 6 equipment layers).
/// Shader derives layer from instance_index / npc_count.
pub struct DrawNpcsStorage;

impl<P: PhaseItem> RenderCommand<P> for DrawNpcsStorage {
    type Param = (SRes<NpcRenderBuffers>, SRes<NpcVisualBuffers>, SRes<NpcGpuData>);
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, 'w, Self::ViewQuery>,
        _entity: Option<ROQueryItem<'w, 'w, Self::ItemQuery>>,
        params: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (npc_buffers, visual_buffers, gpu_data) = params;
        let npc_buffers = npc_buffers.into_inner();
        let visual_buffers = visual_buffers.into_inner();
        let npc_count = gpu_data.into_inner().npc_count;
        if npc_count == 0 { return RenderCommandResult::Skip; }

        let Some(ref bind_group) = visual_buffers.bind_group else {
            return RenderCommandResult::Skip;
        };

        pass.set_bind_group(2, bind_group, &[]);
        pass.set_vertex_buffer(0, npc_buffers.vertex_buffer.slice(..));
        pass.set_index_buffer(npc_buffers.index_buffer.slice(..), IndexFormat::Uint16);

        // 7 draw calls — shader derives layer = instance_index / npc_count
        for layer in 0..LAYER_COUNT as u32 {
            pass.draw_indexed(0..6, 0, (layer * npc_count)..((layer + 1) * npc_count));
        }

        RenderCommandResult::Success
    }
}

/// Draw command for misc instance buffer path (farms, building HP bars).
pub struct DrawMisc;

impl<P: PhaseItem> RenderCommand<P> for DrawMisc {
    type Param = (SRes<NpcRenderBuffers>, SRes<NpcMiscBuffers>);
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, 'w, Self::ViewQuery>,
        _entity: Option<ROQueryItem<'w, 'w, Self::ItemQuery>>,
        params: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (npc_buffers, misc_buffers) = params;
        let npc_buffers = npc_buffers.into_inner();
        let misc_buffers = misc_buffers.into_inner();

        if misc_buffers.count == 0 { return RenderCommandResult::Skip; }
        let Some(instance_buffer) = misc_buffers.instances.buffer() else {
            return RenderCommandResult::Skip;
        };

        pass.set_vertex_buffer(0, npc_buffers.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, instance_buffer.slice(..));
        pass.set_index_buffer(npc_buffers.index_buffer.slice(..), IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..misc_buffers.count);

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

/// NPC storage buffer draw commands (body + equipment via storage buffers).
type DrawNpcStorageCommands = (
    SetItemPipeline,
    SetNpcTextureBindGroup<0>,
    SetNpcCameraBindGroup<1>,
    DrawNpcsStorage,
);

/// Misc instance draw commands (farms, building HP bars).
type DrawMiscCommands = (
    SetItemPipeline,
    SetNpcTextureBindGroup<0>,
    SetNpcCameraBindGroup<1>,
    DrawMisc,
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
        app.add_systems(Startup, (spawn_npc_batch, spawn_proj_batch));

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .add_render_command::<Transparent2d, DrawNpcStorageCommands>()
            .add_render_command::<Transparent2d, DrawMiscCommands>()
            .add_render_command::<Transparent2d, DrawProjCommands>()
            .init_resource::<SpecializedRenderPipelines<NpcPipeline>>()
            .add_systems(
                ExtractSchedule,
                (extract_npc_batch, extract_proj_batch, extract_camera_state, extract_npc_data),
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
) {
    let Ok((transform, projection)) = query.single() else { return };
    let Ok(window) = windows.single() else { return };

    let zoom = match projection {
        Projection::Orthographic(ortho) => 1.0 / ortho.scale,
        _ => 1.0,
    };

    commands.insert_resource(CameraState {
        position: transform.translation.truncate(),
        zoom,
        viewport: Vec2::new(window.width(), window.height()),
    });
}

// =============================================================================
// EXTRACT: Direct GPU Upload (zero clone)
// =============================================================================

/// Upload NPC compute + visual data directly to GPU buffers during Extract phase.
/// Reads main world data via Extract<Res<T>> (immutable reference, zero clone).
/// Replaces both write_npc_buffers (compute) and prepare_npc_buffers visual repack.
fn extract_npc_data(
    gpu_state: Extract<Res<NpcGpuState>>,
    visual_upload: Extract<Res<NpcVisualUpload>>,
    gpu_buffers: Option<Res<NpcGpuBuffers>>,
    visual_buffers: Option<Res<NpcVisualBuffers>>,
    render_queue: Res<RenderQueue>,
) {
    // --- Compute data: per-dirty-index write_buffer ---
    if let Some(gpu_bufs) = gpu_buffers {
        if gpu_state.dirty {
            for &idx in &gpu_state.position_dirty_indices {
                let start = idx * 2;
                if start + 2 <= gpu_state.positions.len() {
                    let byte_offset = (start * std::mem::size_of::<f32>()) as u64;
                    render_queue.write_buffer(
                        &gpu_bufs.positions, byte_offset,
                        bytemuck::cast_slice(&gpu_state.positions[start..start + 2]),
                    );
                }
            }
            for &idx in &gpu_state.target_dirty_indices {
                let start = idx * 2;
                if start + 2 <= gpu_state.targets.len() {
                    let byte_offset = (start * std::mem::size_of::<f32>()) as u64;
                    render_queue.write_buffer(
                        &gpu_bufs.targets, byte_offset,
                        bytemuck::cast_slice(&gpu_state.targets[start..start + 2]),
                    );
                }
            }
            for &idx in &gpu_state.speed_dirty_indices {
                if idx < gpu_state.speeds.len() {
                    let byte_offset = (idx * std::mem::size_of::<f32>()) as u64;
                    render_queue.write_buffer(
                        &gpu_bufs.speeds, byte_offset,
                        bytemuck::cast_slice(&gpu_state.speeds[idx..idx + 1]),
                    );
                }
            }
            for &idx in &gpu_state.faction_dirty_indices {
                if idx < gpu_state.factions.len() {
                    let byte_offset = (idx * std::mem::size_of::<i32>()) as u64;
                    render_queue.write_buffer(
                        &gpu_bufs.factions, byte_offset,
                        bytemuck::cast_slice(&gpu_state.factions[idx..idx + 1]),
                    );
                }
            }
            for &idx in &gpu_state.health_dirty_indices {
                if idx < gpu_state.healths.len() {
                    let byte_offset = (idx * std::mem::size_of::<f32>()) as u64;
                    render_queue.write_buffer(
                        &gpu_bufs.healths, byte_offset,
                        bytemuck::cast_slice(&gpu_state.healths[idx..idx + 1]),
                    );
                }
            }
            for &idx in &gpu_state.arrival_dirty_indices {
                if idx < gpu_state.arrivals.len() {
                    let byte_offset = (idx * std::mem::size_of::<i32>()) as u64;
                    render_queue.write_buffer(
                        &gpu_bufs.arrivals, byte_offset,
                        bytemuck::cast_slice(&gpu_state.arrivals[idx..idx + 1]),
                    );
                }
            }
        }
    }

    // --- Visual data: bulk write_buffer ---
    if let Some(vis_bufs) = visual_buffers {
        if visual_upload.npc_count > 0 {
            render_queue.write_buffer(
                &vis_bufs.visual, 0,
                bytemuck::cast_slice(&visual_upload.visual_data),
            );
            render_queue.write_buffer(
                &vis_bufs.equip, 0,
                bytemuck::cast_slice(&visual_upload.equip_data),
            );
        }
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
    gpu_buffers: Option<Res<NpcGpuBuffers>>,
    existing_render: Option<ResMut<NpcRenderBuffers>>,
    existing_visual: Option<ResMut<NpcVisualBuffers>>,
    existing_misc: Option<ResMut<NpcMiscBuffers>>,
    farm_states: Option<Res<crate::resources::FarmStates>>,
    building_hp_render: Option<Res<crate::resources::BuildingHpRender>>,
) {
    // --- Misc instance buffer (farms + building HP bars) ---
    let mut misc_instances = RawBufferVec::new(BufferUsages::VERTEX);

    if let Some(farms) = farm_states {
        let count = farms.positions.len().min(farms.progress.len()).min(farms.states.len());
        for i in 0..count {
            let pos = farms.positions[i];
            if pos.x < -9000.0 { continue; }

            let ready = farms.states[i] == crate::resources::FarmGrowthState::Ready;
            let color = if ready {
                [1.0, 0.85, 0.0, 1.0]
            } else {
                [0.4, 0.8, 0.2, 1.0]
            };

            misc_instances.push(InstanceData {
                position: [pos.x, pos.y],
                sprite: [24.0, 9.0],
                color,
                health: farms.progress[i].clamp(0.0, 1.0),
                flash: 0.0,
                scale: 16.0,
                atlas_id: 1.0,
                rotation: 0.0,
            });
        }
    }

    if let Some(bhp) = building_hp_render {
        let count = bhp.positions.len().min(bhp.health_pcts.len());
        for i in 0..count {
            misc_instances.push(InstanceData {
                position: [bhp.positions[i].x, bhp.positions[i].y],
                sprite: [0.0, 0.0],
                color: [1.0, 1.0, 1.0, 1.0],
                health: bhp.health_pcts[i],
                flash: 0.0,
                scale: 32.0,
                atlas_id: 5.0,
                rotation: 0.0,
            });
        }
    }

    let misc_count = misc_instances.len() as u32;
    misc_instances.write_buffer(&render_device, &render_queue);

    if let Some(mut misc) = existing_misc {
        misc.instances = misc_instances;
        misc.count = misc_count;
    } else {
        commands.insert_resource(NpcMiscBuffers {
            instances: misc_instances,
            count: misc_count,
        });
    }

    // --- NPC visual storage buffers ---
    // Visual data is uploaded by extract_npc_data in Extract phase (zero clone).
    // Here we only handle: first-frame buffer creation, bind group recreation, quad geometry.
    if let Some(mut visual_buffers) = existing_visual {
        // Recreate bind group each frame (gpu_buffers may not exist on first frame)
        if let (Some(gpu_bufs), Some(ref pipeline)) = (gpu_buffers.as_ref(), pipeline.as_ref()) {
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
        let visual_buffer = render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_visual_data"),
            size: (MAX_NPC_COUNT * std::mem::size_of::<[f32; 8]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let equip_buffer = render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_equip_data"),
            size: (MAX_NPC_COUNT * 6 * std::mem::size_of::<[f32; 4]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Write sentinel -1.0 so all sprites are hidden until extract_npc_data writes real data
        let sentinel_visual = vec![-1.0f32; MAX_NPC_COUNT * 8];
        let sentinel_equip = vec![-1.0f32; MAX_NPC_COUNT * 6 * 4];
        render_queue.write_buffer(&visual_buffer, 0, bytemuck::cast_slice(&sentinel_visual));
        render_queue.write_buffer(&equip_buffer, 0, bytemuck::cast_slice(&sentinel_equip));

        // Create bind group if gpu_buffers available
        let bind_group = if let (Some(gpu_bufs), Some(ref pipeline)) = (gpu_buffers.as_ref(), pipeline.as_ref()) {
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
    let Some(heal_handle) = &sprite_texture.heal_handle else { return };
    let Some(sleep_handle) = &sprite_texture.sleep_handle else { return };
    let Some(arrow_handle) = &sprite_texture.arrow_handle else { return };
    let Some(char_image) = gpu_images.get(char_handle) else { return };
    let Some(world_image) = gpu_images.get(world_handle) else { return };
    let Some(heal_image) = gpu_images.get(heal_handle) else { return };
    let Some(sleep_image) = gpu_images.get(sleep_handle) else { return };
    let Some(arrow_image) = gpu_images.get(arrow_handle) else { return };

    let layout = pipeline_cache.get_bind_group_layout(&pipeline.texture_bind_group_layout);

    let bind_group = render_device.create_bind_group(
        Some("npc_texture_bind_group"),
        &layout,
        &BindGroupEntries::sequential((
            &char_image.texture_view,
            &char_image.sampler,
            &world_image.texture_view,
            &world_image.sampler,
            &heal_image.texture_view,
            &heal_image.sampler,
            &sleep_image.texture_view,
            &sleep_image.sampler,
            &arrow_image.texture_view,
            &arrow_image.sampler,
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
    gpu_data: Option<Res<NpcGpuData>>,
) {
    let Some(pipeline) = pipeline else { return };
    let Some(camera_state) = camera_state else { return };

    let uniform = CameraUniform {
        camera_pos: camera_state.position,
        zoom: camera_state.zoom,
        npc_count: gpu_data.map(|d| d.npc_count).unwrap_or(0),
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

/// Queue NPC storage buffer + misc instance draws into Transparent2d phase.
fn queue_npcs(
    draw_functions: Res<DrawFunctions<Transparent2d>>,
    pipeline: Res<NpcPipeline>,
    mut pipelines: ResMut<SpecializedRenderPipelines<NpcPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    render_buffers: Option<Res<NpcRenderBuffers>>,
    visual_buffers: Option<Res<NpcVisualBuffers>>,
    misc_buffers: Option<Res<NpcMiscBuffers>>,
    gpu_data: Option<Res<NpcGpuData>>,
    mut transparent_phases: ResMut<ViewSortedRenderPhases<Transparent2d>>,
    views: Query<(Entity, &ExtractedView, &Msaa)>,
    npc_batch: Query<Entity, With<NpcBatch>>,
) {
    let Some(_render_buffers) = render_buffers else { return };

    let has_npcs = visual_buffers.as_ref().is_some_and(|vb| vb.bind_group.is_some())
        && gpu_data.as_ref().is_some_and(|d| d.npc_count > 0);
    let has_misc = misc_buffers.as_ref().is_some_and(|m| m.count > 0);

    if !has_npcs && !has_misc { return; }

    let npc_draw = draw_functions.read().id::<DrawNpcStorageCommands>();
    let misc_draw = draw_functions.read().id::<DrawMiscCommands>();

    for (view_entity, view, msaa) in &views {
        let Some(transparent_phase) = transparent_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };

        for batch_entity in &npc_batch {
            let entity = (view_entity, MainEntity::from(batch_entity));

            // Misc (farms/BHP) below NPCs
            if has_misc {
                let pipeline_id = pipelines.specialize(
                    &pipeline_cache, &pipeline, (view.hdr, msaa.samples(), false),
                );
                transparent_phase.add(Transparent2d {
                    entity,
                    draw_function: misc_draw,
                    pipeline: pipeline_id,
                    sort_key: FloatOrd(0.4),
                    batch_range: 0..1,
                    extra_index: PhaseItemExtraIndex::None,
                    extracted_index: usize::MAX,
                    indexed: true,
                });
            }

            // NPCs via storage buffers
            if has_npcs {
                let pipeline_id = pipelines.specialize(
                    &pipeline_cache, &pipeline, (view.hdr, msaa.samples(), true),
                );
                transparent_phase.add(Transparent2d {
                    entity,
                    draw_function: npc_draw,
                    pipeline: pipeline_id,
                    sort_key: FloatOrd(0.5),
                    batch_range: 0..1,
                    extra_index: PhaseItemExtraIndex::None,
                    extracted_index: usize::MAX,
                    indexed: true,
                });
            }
        }
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

impl SpecializedRenderPipeline for NpcPipeline {
    type Key = (bool, u32, bool); // (HDR, MSAA sample count, storage_mode)

    fn specialize(&self, (hdr, sample_count, storage_mode): Self::Key) -> RenderPipelineDescriptor {
        let format = if hdr {
            TextureFormat::Rgba16Float
        } else {
            TextureFormat::Rgba8UnormSrgb
        };

        let (label, layout, entry_point, buffers) = if storage_mode {
            (
                "npc_storage_pipeline",
                vec![
                    self.texture_bind_group_layout.clone(),
                    self.camera_bind_group_layout.clone(),
                    self.npc_data_bind_group_layout.clone(),
                ],
                "vertex_npc",
                vec![quad_vertex_layout()],
            )
        } else {
            (
                "npc_instance_pipeline",
                vec![
                    self.texture_bind_group_layout.clone(),
                    self.camera_bind_group_layout.clone(),
                ],
                "vertex",
                vec![quad_vertex_layout(), instance_vertex_layout()],
            )
        };

        RenderPipelineDescriptor {
            label: Some(label.into()),
            layout,
            vertex: VertexState {
                shader: self.shader.clone(),
                shader_defs: vec![],
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
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
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
                ShaderStages::VERTEX,
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

        NpcPipeline {
            shader: asset_server.load("shaders/npc_render.wgsl"),
            texture_bind_group_layout,
            camera_bind_group_layout,
            npc_data_bind_group_layout,
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

/// Prepare projectile instance buffers from GPU readback positions.
fn prepare_proj_buffers(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    buffer_writes: Option<Res<ProjBufferWrites>>,
    proj_data: Option<Res<ProjGpuData>>,
    existing_buffers: Option<ResMut<ProjRenderBuffers>>,
    proj_pos_state: Option<Res<crate::resources::ProjPositionState>>,
) {
    let Some(writes) = buffer_writes else { return };
    let Some(data) = proj_data else { return };

    let proj_count = data.proj_count as usize;

    let readback_positions = proj_pos_state
        .filter(|s| s.0.len() >= proj_count * 2)
        .map(|s| s.0.clone());

    let mut instances = RawBufferVec::new(BufferUsages::VERTEX);
    for i in 0..proj_count {
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

        let faction = writes.factions.get(i).copied().unwrap_or(0);
        let (cr, cg, cb) = if faction == 0 {
            (0.0, 0.0, 1.0)
        } else {
            let (r, g, b, _) = crate::constants::raider_faction_color(faction);
            (r, g, b)
        };

        let vx = writes.velocities.get(i * 2).copied().unwrap_or(0.0);
        let vy = writes.velocities.get(i * 2 + 1).copied().unwrap_or(0.0);
        let angle = vy.atan2(vx) - std::f32::consts::FRAC_PI_2;

        instances.push(InstanceData {
            position: [px, py],
            sprite: [0.0, 0.0],
            color: [cr, cg, cb, 1.0],
            health: 1.0,
            flash: 0.0,
            scale: 16.0,
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

        let pipeline_id = pipelines.specialize(&pipeline_cache, &pipeline, (view.hdr, msaa.samples(), false));

        for batch_entity in &proj_batch {
            transparent_phase.add(Transparent2d {
                entity: (view_entity, MainEntity::from(batch_entity)),
                draw_function,
                pipeline: pipeline_id,
                sort_key: FloatOrd(1.0),
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                extracted_index: usize::MAX,
                indexed: true,
            });
        }
    }
}
