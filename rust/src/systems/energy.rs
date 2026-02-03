//! Energy systems - Drain and recovery

use godot_bevy::prelude::bevy_ecs_prelude::*;

use crate::components::*;
use crate::constants::*;
use crate::world::FarmOccupancy;

/// Energy threshold to wake up from resting (90%).
const ENERGY_WAKE_THRESHOLD: f32 = 90.0;

/// Energy threshold to stop working and seek rest (30%).
const ENERGY_TIRED_THRESHOLD: f32 = 30.0;

/// Energy system: drain while active, recover while resting.
/// Removes Resting when energy is full enough.
/// Removes Working when energy is low (so NPC can decide to rest).
pub fn energy_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut Energy, Option<&Resting>, Option<&Working>, Option<&AssignedFarm>)>,
    mut farm_occupancy: ResMut<FarmOccupancy>,
) {
    for (entity, mut energy, resting, working, assigned_farm) in query.iter_mut() {
        if resting.is_some() {
            energy.0 = (energy.0 + ENERGY_RECOVER_RATE).min(100.0);
            // Wake up when energy is restored
            if energy.0 >= ENERGY_WAKE_THRESHOLD {
                commands.entity(entity).remove::<Resting>();
            }
        } else {
            energy.0 = (energy.0 - ENERGY_DRAIN_RATE).max(0.0);

            // Working NPCs stop working when tired (energy below threshold)
            if working.is_some() && energy.0 < ENERGY_TIRED_THRESHOLD {
                commands.entity(entity).remove::<Working>();

                // Release assigned farm if any
                if let Some(assigned) = assigned_farm {
                    if assigned.0 < farm_occupancy.occupant_count.len() {
                        farm_occupancy.occupant_count[assigned.0] = (farm_occupancy.occupant_count[assigned.0] - 1).max(0);
                    }
                    commands.entity(entity).remove::<AssignedFarm>();
                }
            }
        }
    }
}
