//! Energy systems - Drain and recovery

use godot_bevy::prelude::bevy_ecs_prelude::*;

use crate::components::*;
use crate::constants::*;

/// Energy threshold to wake up from resting (90%).
const ENERGY_WAKE_THRESHOLD: f32 = 90.0;

/// Energy system: drain while active, recover while resting.
/// Removes Resting when energy is full enough.
pub fn energy_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut Energy, Option<&Resting>)>,
) {
    for (entity, mut energy, resting) in query.iter_mut() {
        if resting.is_some() {
            energy.0 = (energy.0 + ENERGY_RECOVER_RATE).min(100.0);
            // Wake up when energy is restored
            if energy.0 >= ENERGY_WAKE_THRESHOLD {
                commands.entity(entity).remove::<Resting>();
            }
        } else {
            energy.0 = (energy.0 - ENERGY_DRAIN_RATE).max(0.0);
        }
    }
}
