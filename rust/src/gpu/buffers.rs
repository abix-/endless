//! GPU Buffer Management
//!
//! Allocates and manages all GPU buffers for NPC compute shader.
//! Currently stubbed - will use bevy_render when implementing compute pipeline.

/// All GPU buffers for NPC compute.
/// Stubbed until wgpu compute pipeline is implemented.
#[derive(Default)]
pub struct GpuBuffers {
    /// Whether buffers are allocated
    pub allocated: bool,
    /// Maximum NPC capacity
    pub max_npcs: u32,
}

impl GpuBuffers {
    /// Create placeholder (no actual GPU allocation yet).
    pub fn new(max_npcs: u32) -> Self {
        Self {
            allocated: false,
            max_npcs,
        }
    }
}
