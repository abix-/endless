//! Energy systems - Drain and recovery

use godot_bevy::prelude::bevy_ecs_prelude::*;

use crate::components::*;
use crate::constants::*;

/// Energy system: drain while active, recover while resting.
/// State transitions (wake-up, stop working) are handled in decision_system.
pub fn energy_system(
    mut query: Query<(&mut Energy, Option<&Resting>)>,
) {
    for (mut energy, resting) in query.iter_mut() {
        if resting.is_some() {
            energy.0 = (energy.0 + ENERGY_RECOVER_RATE).min(100.0);
        } else {
            energy.0 = (energy.0 - ENERGY_DRAIN_RATE).max(0.0);
        }
    }
}
