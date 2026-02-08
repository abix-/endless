//! GPU Compute Module - wgpu-based NPC physics via Bevy's render graph.
//!
//! Follows Bevy 0.18's compute_shader_game_of_life.rs pattern.
//! Three-phase dispatch per frame: clear grid → insert NPCs → main logic.

use bevy::{
    prelude::*,
    render::{
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_graph::{self, RenderGraph, RenderLabel},
        render_resource::{binding_types::{storage_buffer, uniform_buffer}, *},
        renderer::{RenderContext, RenderDevice, RenderQueue},
        Render, RenderApp, RenderStartup, RenderSystems,
    },
    shader::PipelineCacheError,
};
use std::borrow::Cow;

// =============================================================================
// CONSTANTS
// =============================================================================

const SHADER_ASSET_PATH: &str = "shaders/npc_compute.wgsl";
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
            .add_systems(Update, update_gpu_data);

        // Extract resources to render world
        app.add_plugins((
            ExtractResourcePlugin::<NpcGpuData>::default(),
            ExtractResourcePlugin::<NpcComputeParams>::default(),
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
                prepare_npc_bind_groups.in_set(RenderSystems::PrepareBindGroups),
            );

        // Add compute node to render graph
        let mut render_graph = render_app.world_mut().resource_mut::<RenderGraph>();
        render_graph.add_node(NpcComputeLabel, NpcComputeNode::default());
        render_graph.add_node_edge(NpcComputeLabel, bevy::render::graph::CameraDriverLabel);

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

/// GPU buffers for NPC compute.
#[derive(Resource)]
struct NpcGpuBuffers {
    positions: Buffer,
    targets: Buffer,
    speeds: Buffer,
    grid_counts: Buffer,
    grid_data: Buffer,
    arrivals: Buffer,
    backoff: Buffer,
    factions: Buffer,
    healths: Buffer,
    combat_targets: Buffer,
}

/// Bind groups for compute passes.
#[derive(Resource)]
struct NpcBindGroups {
    /// Bind group for all three modes
    bind_group: BindGroup,
}

/// Pipeline resources.
#[derive(Resource)]
struct NpcComputePipeline {
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
