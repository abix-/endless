//! Energy systems - Drain and recovery

use bevy::prelude::*;

use crate::components::Activity;
use crate::resources::{EntityMap, GameTime, SystemTimings};

/// Energy recovery/drain rates (per game hour)
const ENERGY_RECOVER_PER_HOUR: f32 = 100.0 / 6.0;  // 6 hours to full (resting)
const ENERGY_DRAIN_PER_HOUR: f32 = 100.0 / 24.0;   // 24 hours to empty (active)

/// Energy system: drain while active, recover while resting or healing at fountain.
/// Uses game time so it respects time_scale.
/// State transitions (wake-up, stop working) are handled in decision_system.
pub fn energy_system(
    mut entity_map: ResMut<EntityMap>,
    time: Res<Time>,
    game_time: Res<GameTime>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("energy");
    if game_time.paused {
        return;
    }

    // Convert delta to game hours
    let hours_elapsed = game_time.delta(&time) / game_time.seconds_per_hour;

    for npc in entity_map.iter_npcs_mut() {
        if npc.dead || !npc.has_energy { continue; }
        if matches!(npc.activity, Activity::Resting | Activity::HealingAtFountain { .. }) {
            npc.energy = (npc.energy + ENERGY_RECOVER_PER_HOUR * hours_elapsed).min(100.0);
        } else {
            npc.energy = (npc.energy - ENERGY_DRAIN_PER_HOUR * hours_elapsed).max(0.0);
        }
    }
}
