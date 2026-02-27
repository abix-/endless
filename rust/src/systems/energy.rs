//! Energy systems - Drain and recovery

use bevy::prelude::*;

use crate::components::{Activity, Energy, EntitySlot};
use crate::resources::{EntityMap, GameTime, SystemTimings};

/// Energy recovery/drain rates (per game hour)
const ENERGY_RECOVER_PER_HOUR: f32 = 100.0 / 6.0;  // 6 hours to full (resting)
const ENERGY_DRAIN_PER_HOUR: f32 = 100.0 / 24.0;   // 24 hours to empty (active)

/// Energy system: drain while active, recover while resting or healing at fountain.
/// Uses game time so it respects time_scale.
/// State transitions (wake-up, stop working) are handled in decision_system.
pub fn energy_system(
    entity_map: Res<EntityMap>,
    time: Res<Time>,
    game_time: Res<GameTime>,
    timings: Res<SystemTimings>,
    activity_q: Query<&Activity>,
    mut energy_q: Query<(&EntitySlot, &mut Energy)>,
) {
    let _t = timings.scope("energy");
    if game_time.paused {
        return;
    }

    // Convert delta to game hours
    let hours_elapsed = game_time.delta(&time) / game_time.seconds_per_hour;

    for (es, mut energy) in energy_q.iter_mut() {
        let slot = es.0;
        let Some(npc) = entity_map.get_npc(slot) else { continue };
        if npc.dead { continue; }
        if activity_q.get(npc.entity).is_ok_and(|a| matches!(*a, Activity::Resting | Activity::HealingAtFountain { .. })) {
            energy.0 = (energy.0 + ENERGY_RECOVER_PER_HOUR * hours_elapsed).min(100.0);
        } else {
            energy.0 = (energy.0 - ENERGY_DRAIN_PER_HOUR * hours_elapsed).max(0.0);
        }
    }
}
