//! GPU Compute Module - wgpu-based NPC physics.
//!
//! TODO: Port compute shaders to WGSL. Currently stubbed.
//! Three-phase dispatch per frame: clear grid → insert NPCs → main logic.

mod buffers;

use bevy::prelude::*;

pub use buffers::*;

// =============================================================================
// CONSTANTS
// =============================================================================

/// Workgroup size (must match WGSL @workgroup_size)
pub const WORKGROUP_SIZE: u32 = 64;

/// Grid dimensions
pub const GRID_WIDTH: u32 = 128;
pub const GRID_HEIGHT: u32 = 128;
pub const MAX_PER_CELL: u32 = 48;

// =============================================================================
// GPU RESOURCES (stubbed)
// =============================================================================

/// GPU compute resources - initialized once, reused each frame.
/// Currently stubbed until wgpu compute pipeline is set up.
#[derive(Resource, Default)]
pub struct GpuCompute {
    /// Whether GPU compute is available
    pub available: bool,
    /// Maximum NPC count
    pub max_npcs: u32,
}

/// Parameters passed to shader each frame.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuParams {
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

// =============================================================================
// PLUGIN
// =============================================================================

pub struct GpuComputePlugin;

impl Plugin for GpuComputePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GpuCompute>();
        app.add_systems(Startup, setup_gpu_compute);
    }
}

/// Initialize GPU compute resources.
fn setup_gpu_compute(mut gpu: ResMut<GpuCompute>) {
    // TODO: Initialize wgpu compute pipeline
    // For now, just log that GPU compute is stubbed
    gpu.available = false;
    gpu.max_npcs = 16384;

    info!("GPU compute initialized (stubbed): {} max NPCs, {}x{} grid",
          gpu.max_npcs, GRID_WIDTH, GRID_HEIGHT);
    info!("Note: GPU physics disabled until wgpu pipeline is implemented");
}
