//! Queue drain systems - Move messages from static queues to Bevy events

use bevy::prelude::*;

use crate::messages::*;
use crate::resources::{SystemTimings, CombatLog};

/// Drain game config staging into Bevy Resource (one-shot).
pub fn drain_game_config(mut config: ResMut<crate::resources::GameConfig>, timings: Res<SystemTimings>) {
    let _t = timings.scope("drain_game_config");
    if let Ok(mut staging) = GAME_CONFIG_STAGING.lock() {
        if let Some(new_config) = staging.take() {
            *config = new_config;
        }
    }
}

/// Drain CombatLogMsg messages into the CombatLog resource for UI display.
pub fn drain_combat_log(mut msgs: MessageReader<CombatLogMsg>, mut log: ResMut<CombatLog>) {
    for msg in msgs.read() {
        log.push_at(msg.kind, msg.faction, msg.day, msg.hour, msg.minute, msg.message.clone(), msg.location);
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
