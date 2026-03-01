//! Queue drain systems - Move messages from static queues to Bevy events

use bevy::prelude::*;

use crate::messages::*;
use crate::resources::CombatLog;

/// Drain game config staging into Bevy Resource (one-shot).
pub fn drain_game_config(mut config: ResMut<crate::resources::GameConfig>) {
    if let Ok(mut staging) = GAME_CONFIG_STAGING.lock() {
        if let Some(new_config) = staging.take() {
            *config = new_config;
        }
    }
}

/// Drain CombatLogMsg messages into the CombatLog resource for UI display.
pub fn drain_combat_log(mut msgs: MessageReader<CombatLogMsg>, mut log: ResMut<CombatLog>) {
    for msg in msgs.read() {
        log.push_at(
            msg.kind,
            msg.faction,
            msg.day,
            msg.hour,
            msg.minute,
            msg.message.clone(),
            msg.location,
        );
    }
}
