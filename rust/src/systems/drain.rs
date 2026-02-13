//! Queue drain systems - Move messages from static queues to Bevy events

use bevy::prelude::*;

use crate::messages::*;
use crate::resources::{self, ResetFlag, SystemTimings};

/// Drain game config staging into Bevy Resource (one-shot).
pub fn drain_game_config(mut config: ResMut<crate::resources::GameConfig>, timings: Res<SystemTimings>) {
    let _t = timings.scope("drain_game_config");
    if let Ok(mut staging) = GAME_CONFIG_STAGING.lock() {
        if let Some(new_config) = staging.take() {
            *config = new_config;
        }
    }
}

/// Collect GPU update events from all systems into the static queue.
/// Runs at end of Behavior phase - single lock point for all GPU writes.
pub fn collect_gpu_updates(mut events: MessageReader<GpuUpdateMsg>, timings: Res<SystemTimings>) {
    let _t = timings.scope("collect_gpu_updates");
    if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
        for msg in events.read() {
            queue.push(msg.0.clone());
        }
    }
}

/// Reset Bevy resources when reset flag is set.
pub fn reset_bevy_system(
    mut reset_flag: ResMut<ResetFlag>,
    mut npc_map: ResMut<resources::NpcEntityMap>,
    mut pop_stats: ResMut<resources::PopulationStats>,
    mut slot_alloc: ResMut<resources::SlotAllocator>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("reset_bevy");
    if !reset_flag.0 {
        return;
    }
    reset_flag.0 = false;

    npc_map.0.clear();
    pop_stats.0.clear();
    slot_alloc.reset();
}
