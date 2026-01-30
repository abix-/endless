//! Sync systems - GPU state to Bevy and Bevy to Godot (Phase 11)
//!
//! Handles GPU readback sync and Changed<T> queries.

use godot_bevy::prelude::bevy_ecs_prelude::*;

use crate::channels::{BevyToGodot, BevyToGodotMsg};
use crate::components::{NpcIndex, Health};
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

/// Write changed NPC state to BevyToGodot outbox.
/// Runs once per frame, only sends messages for components that actually changed.
pub fn bevy_to_godot_write(
    healths: Query<(&NpcIndex, &Health), Changed<Health>>,
    outbox: Option<Res<BevyToGodot>>,
) {
    let outbox = match outbox {
        Some(o) => o,
        None => return,
    };

    // Sync changed health
    for (npc_idx, health) in healths.iter() {
        let _ = outbox.0.send(BevyToGodotMsg::SyncHealth {
            slot: npc_idx.0,
            hp: health.0,
            max_hp: 100.0,
        });
    }
}
