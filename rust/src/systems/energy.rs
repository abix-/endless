//! Energy systems - Drain and recovery

use godot_bevy::prelude::bevy_ecs_prelude::*;

use crate::components::*;
use crate::constants::*;
use crate::world::FarmOccupancy;

// Energy thresholds are now in constants.rs

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
            // Wake-up is now handled in decision_system to avoid command sync race
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
