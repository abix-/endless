//! Energy systems - Drain and recovery

use bevy::prelude::*;

use crate::components::{Activity, Building, Dead, Energy, GpuSlot};
use crate::resources::{GameTime};

/// Energy recovery/drain rates (per game hour)
const ENERGY_RECOVER_PER_HOUR: f32 = 100.0 / 6.0;  // 6 hours to full (resting)
const ENERGY_DRAIN_PER_HOUR: f32 = 100.0 / 24.0;   // 24 hours to empty (active)

/// Energy system: drain while active, recover while resting or healing at fountain.
/// Uses game time so it respects time_scale.
/// State transitions (wake-up, stop working) are handled in decision_system.
pub fn energy_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut energy_q: Query<(&GpuSlot, &mut Energy, &Activity), (Without<Building>, Without<Dead>)>,
) {
    if game_time.is_paused() {
        return;
    }

    // Convert delta to game hours
    let hours_elapsed = game_time.delta(&time) / game_time.seconds_per_hour;

    for (_es, mut energy, activity) in energy_q.iter_mut() {
        if matches!(*activity, Activity::Resting | Activity::HealingAtFountain { .. }) {
            energy.0 = (energy.0 + ENERGY_RECOVER_PER_HOUR * hours_elapsed).min(100.0);
        } else {
            energy.0 = (energy.0 - ENERGY_DRAIN_PER_HOUR * hours_elapsed).max(0.0);
        }
    }
}
