//! Sync systems - GPU state to Bevy (Phase 11)
//!
//! Handles GPU readback sync.

use bevy::prelude::*;

use crate::messages::GPU_READ_STATE;
use crate::resources::GpuReadState;

/// Sync GPU_READ_STATE static to Bevy GpuReadState resource.
/// Must run before behavior systems that read positions.
pub fn sync_gpu_state_to_bevy(mut gpu_state: ResMut<GpuReadState>) {
    if let Ok(state) = GPU_READ_STATE.lock() {
        gpu_state.positions = state.positions.clone();
        gpu_state.combat_targets = state.combat_targets.clone();
        gpu_state.health = state.health.clone();
        gpu_state.factions = state.factions.clone();
        gpu_state.npc_count = state.npc_count;
    }
}
