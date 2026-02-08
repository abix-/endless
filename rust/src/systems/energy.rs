//! Energy systems - Drain and recovery

use bevy::prelude::*;

use crate::components::*;
use crate::resources::GameTime;

/// Energy recovery/drain rates (per game hour)
const ENERGY_RECOVER_PER_HOUR: f32 = 100.0 / 6.0;  // 6 hours to full (resting)
const ENERGY_DRAIN_PER_HOUR: f32 = 100.0 / 24.0;   // 24 hours to empty (active)

/// Energy system: drain while active, recover while resting.
/// Uses game time so it respects time_scale.
/// State transitions (wake-up, stop working) are handled in decision_system.
pub fn energy_system(
    mut query: Query<(&mut Energy, Option<&Resting>)>,
    time: Res<Time>,
    game_time: Res<GameTime>,
) {
    if game_time.paused {
        return;
    }

    // Convert delta to game hours
    let hours_elapsed = (time.delta_secs() * game_time.time_scale) / game_time.seconds_per_hour;

    for (mut energy, resting) in query.iter_mut() {
        if resting.is_some() {
            energy.0 = (energy.0 + ENERGY_RECOVER_PER_HOUR * hours_elapsed).min(100.0);
        } else {
            energy.0 = (energy.0 - ENERGY_DRAIN_PER_HOUR * hours_elapsed).max(0.0);
        }
    }
}
