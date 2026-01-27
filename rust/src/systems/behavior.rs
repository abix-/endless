//! Behavior systems - State transitions based on energy and arrivals

use godot_bevy::prelude::bevy_ecs_prelude::*;

use crate::components::*;
use crate::messages::*;
use crate::constants::*;

/// Tired system: anyone with Home + Energy below threshold goes to rest.
/// Skip NPCs in combat - they fight until the enemy is dead or they flee.
pub fn tired_system(
    mut commands: Commands,
    query: Query<(Entity, &Energy, &NpcIndex, &Home),
                 (Without<GoingToRest>, Without<Resting>, Without<InCombat>)>,
) {
    for (entity, energy, npc_idx, home) in query.iter() {
        if energy.0 < ENERGY_HUNGRY {
            // Low energy - go rest
            commands.entity(entity)
                .remove::<OnDuty>()
                .remove::<Working>()
                .insert(GoingToRest);

            // Set target to home position (push to GPU queue)
            if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
                queue.push(SetTargetMsg {
                    npc_index: npc_idx.0,
                    x: home.0.x,
                    y: home.0.y,
                });
            }
        }
    }
}

/// Resume patrol when energy recovered (anyone with PatrolRoute + Resting).
/// Skip NPCs in combat.
pub fn resume_patrol_system(
    mut commands: Commands,
    query: Query<(Entity, &PatrolRoute, &Energy, &NpcIndex), (With<Resting>, Without<InCombat>)>,
) {
    for (entity, patrol, energy, npc_idx) in query.iter() {
        if energy.0 >= ENERGY_RESTED {
            // Rested enough - go patrol
            commands.entity(entity)
                .remove::<Resting>()
                .insert(Patrolling);

            // Get current patrol post and set target
            if let Some(pos) = patrol.posts.get(patrol.current) {
                if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
                    queue.push(SetTargetMsg {
                        npc_index: npc_idx.0,
                        x: pos.x,
                        y: pos.y,
                    });
                }
            }
        }
    }
}

/// Resume work when energy recovered (anyone with WorkPosition + Resting).
/// Skip NPCs in combat.
pub fn resume_work_system(
    mut commands: Commands,
    query: Query<(Entity, &WorkPosition, &Energy, &NpcIndex), (With<Resting>, Without<InCombat>)>,
) {
    for (entity, work_pos, energy, npc_idx) in query.iter() {
        if energy.0 >= ENERGY_RESTED {
            // Rested enough - go to work
            commands.entity(entity)
                .remove::<Resting>()
                .insert(GoingToWork);

            // Set target to work position
            if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
                queue.push(SetTargetMsg {
                    npc_index: npc_idx.0,
                    x: work_pos.0.x,
                    y: work_pos.0.y,
                });
            }
        }
    }
}

/// Patrol system: count ticks at post and move to next (anyone with PatrolRoute + OnDuty).
/// Skip NPCs in combat - they chase enemies instead.
pub fn patrol_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut PatrolRoute, &mut OnDuty, &NpcIndex), Without<InCombat>>,
) {
    for (entity, mut patrol, mut on_duty, npc_idx) in query.iter_mut() {
        on_duty.ticks_waiting += 1;

        if on_duty.ticks_waiting >= GUARD_PATROL_WAIT {
            // Time to move to next post
            if !patrol.posts.is_empty() {
                patrol.current = (patrol.current + 1) % patrol.posts.len();
            }

            commands.entity(entity)
                .remove::<OnDuty>()
                .insert(Patrolling);

            // Set target to next patrol post
            if let Some(pos) = patrol.posts.get(patrol.current) {
                if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
                    queue.push(SetTargetMsg {
                        npc_index: npc_idx.0,
                        x: pos.x,
                        y: pos.y,
                    });
                }
            }
        }
    }
}

/// Handle arrivals: transition states based on what the NPC was doing.
/// - Patrolling → OnDuty (arrived at patrol post)
/// - GoingToRest → Resting (arrived at home)
/// - GoingToWork → Working (arrived at work position)
pub fn handle_arrival_system(
    mut commands: Commands,
    mut events: MessageReader<ArrivalMsg>,
    patrolling_query: Query<(Entity, &NpcIndex), With<Patrolling>>,
    going_to_rest_query: Query<(Entity, &NpcIndex), With<GoingToRest>>,
    going_to_work_query: Query<(Entity, &NpcIndex), With<GoingToWork>>,
) {
    for event in events.read() {
        // Check if a patrolling NPC arrived at post
        for (entity, npc_idx) in patrolling_query.iter() {
            if npc_idx.0 == event.npc_index {
                commands.entity(entity)
                    .remove::<Patrolling>()
                    .insert(OnDuty { ticks_waiting: 0 });
                break;
            }
        }

        // Check if an NPC going to rest arrived at home
        for (entity, npc_idx) in going_to_rest_query.iter() {
            if npc_idx.0 == event.npc_index {
                commands.entity(entity)
                    .remove::<GoingToRest>()
                    .insert(Resting);
                break;
            }
        }

        // Check if an NPC going to work arrived
        for (entity, npc_idx) in going_to_work_query.iter() {
            if npc_idx.0 == event.npc_index {
                commands.entity(entity)
                    .remove::<GoingToWork>()
                    .insert(Working);
                break;
            }
        }
    }
}
