//! Behavior systems - State transitions based on energy and arrivals

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;

use crate::components::*;
use crate::messages::*;
use crate::constants::*;
use crate::world::WORLD_DATA;

/// Tired system: anyone with Home + Energy below threshold goes to rest.
/// Skip NPCs in combat - they fight until the enemy is dead or they flee.
pub fn tired_system(
    mut commands: Commands,
    query: Query<(Entity, &Energy, &NpcIndex, &Home),
                 (Without<GoingToRest>, Without<Resting>, Without<InCombat>)>,
) {
    for (entity, energy, npc_idx, home) in query.iter() {
        if energy.0 < ENERGY_HUNGRY && home.is_valid() {
            // Low energy - go rest
            commands.entity(entity)
                .remove::<OnDuty>()
                .remove::<Working>()
                .remove::<Raiding>()
                .remove::<Returning>()
                .insert(GoingToRest);

            // GPU-FIRST: Push to GPU_UPDATE_QUEUE
            if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                queue.push(GpuUpdate::SetTarget {
                    idx: npc_idx.0,
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
                // GPU-FIRST: Push to GPU_UPDATE_QUEUE
                if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                    queue.push(GpuUpdate::SetTarget {
                        idx: npc_idx.0,
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

            // GPU-FIRST: Push to GPU_UPDATE_QUEUE
            if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                queue.push(GpuUpdate::SetTarget {
                    idx: npc_idx.0,
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

            // GPU-FIRST: Push to GPU_UPDATE_QUEUE
            if let Some(pos) = patrol.posts.get(patrol.current) {
                if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                    queue.push(GpuUpdate::SetTarget {
                        idx: npc_idx.0,
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

// ============================================================================
// STEALING SYSTEMS (generic — any NPC with Stealer component)
// ============================================================================

/// Handle arrivals for stealing NPCs (Raiding → pickup, Returning → deliver).
pub fn steal_arrival_system(
    mut commands: Commands,
    mut events: MessageReader<ArrivalMsg>,
    raiding_query: Query<(Entity, &NpcIndex, &Home, &Health, Option<&WoundedThreshold>), With<Raiding>>,
    returning_query: Query<(Entity, &NpcIndex, Option<&CarryingFood>), With<Returning>>,
) {
    for event in events.read() {
        // Raiding NPC arrived at farm → pick up food
        for (entity, npc_idx, home, _health, _wounded) in raiding_query.iter() {
            if npc_idx.0 == event.npc_index {
                // Arrived at farm: pick up food, head home
                commands.entity(entity)
                    .remove::<Raiding>()
                    .insert(CarryingFood)
                    .insert(Returning);

                // Change color to yellow (carrying food)
                if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                    queue.push(GpuUpdate::SetColor {
                        idx: npc_idx.0,
                        r: 1.0, g: 0.9, b: 0.2, a: 1.0,
                    });
                    queue.push(GpuUpdate::SetTarget {
                        idx: npc_idx.0,
                        x: home.0.x,
                        y: home.0.y,
                    });
                }
                break;
            }
        }

        // Returning NPC arrived at camp → deliver food, re-enter decision
        for (entity, npc_idx, carrying) in returning_query.iter() {
            if npc_idx.0 == event.npc_index {
                let mut cmds = commands.entity(entity);
                cmds.remove::<Returning>();

                if carrying.is_some() {
                    cmds.remove::<CarryingFood>();

                    // Reset color to raider red
                    if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                        let (r, g, b, a) = Job::Raider.color();
                        queue.push(GpuUpdate::SetColor {
                            idx: npc_idx.0, r, g, b, a,
                        });
                    }

                    // Deliver food to camp
                    if let Ok(mut food) = FOOD_STORAGE.lock() {
                        // Camp idx 0 for now — TODO: multi-camp support via component
                        if !food.camp_food.is_empty() {
                            food.camp_food[0] += 1;
                        }
                    }
                    if let Ok(mut queue) = FOOD_DELIVERED_QUEUE.lock() {
                        queue.push(FoodDelivered { camp_idx: 0 });
                    }
                }

                // Fall through to steal_decision_system next tick
                // (entity has no active state markers)
                break;
            }
        }
    }
}

/// Decision system for idle stealers: pick next action.
/// Runs on Stealer NPCs without any active state.
pub fn steal_decision_system(
    mut commands: Commands,
    query: Query<
        (Entity, &NpcIndex, &Home, &Health, Option<&CarryingFood>, Option<&Energy>, Option<&WoundedThreshold>),
        (With<Stealer>,
         Without<Raiding>, Without<Returning>, Without<Resting>,
         Without<InCombat>, Without<Recovering>, Without<GoingToRest>,
         Without<Dead>)
    >,
) {
    for (entity, npc_idx, home, health, carrying, energy, wounded) in query.iter() {
        let health_pct = health.0 / 100.0;

        // Priority 1: Wounded — drop food, go home, rest
        if let Some(w) = wounded {
            if health_pct < w.pct && home.is_valid() {
                let mut cmds = commands.entity(entity);
                cmds.remove::<CarryingFood>();
                cmds.insert(Returning);

                if carrying.is_some() {
                    // Reset color when dropping food
                    if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                        let (r, g, b, a) = Job::Raider.color();
                        queue.push(GpuUpdate::SetColor {
                            idx: npc_idx.0, r, g, b, a,
                        });
                    }
                }

                if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                    queue.push(GpuUpdate::SetTarget {
                        idx: npc_idx.0,
                        x: home.0.x,
                        y: home.0.y,
                    });
                }
                continue;
            }
        }

        // Priority 2: Carrying food — deliver it
        if carrying.is_some() && home.is_valid() {
            commands.entity(entity).insert(Returning);
            if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                queue.push(GpuUpdate::SetTarget {
                    idx: npc_idx.0,
                    x: home.0.x,
                    y: home.0.y,
                });
            }
            continue;
        }

        // Priority 3: Low energy — go home
        if let Some(e) = energy {
            if e.0 < ENERGY_HUNGRY && home.is_valid() {
                commands.entity(entity).insert(Returning);
                if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                    queue.push(GpuUpdate::SetTarget {
                        idx: npc_idx.0,
                        x: home.0.x,
                        y: home.0.y,
                    });
                }
                continue;
            }
        }

        // Priority 4: Go raid nearest farm
        let nearest_farm = if let Ok(world) = WORLD_DATA.lock() {
            // Read NPC position from GPU state
            let pos = if let Ok(state) = GPU_READ_STATE.lock() {
                let i = npc_idx.0;
                if i * 2 + 1 < state.positions.len() {
                    Vector2::new(state.positions[i * 2], state.positions[i * 2 + 1])
                } else {
                    home.0 // fallback
                }
            } else {
                home.0
            };

            let mut best: Option<(f32, Vector2)> = None;
            for farm in &world.farms {
                let dx = farm.position.x - pos.x;
                let dy = farm.position.y - pos.y;
                let dist_sq = dx * dx + dy * dy;
                if best.is_none() || dist_sq < best.unwrap().0 {
                    best = Some((dist_sq, farm.position));
                }
            }
            best.map(|(_, p)| p)
        } else {
            None
        };

        if let Some(farm_pos) = nearest_farm {
            commands.entity(entity).insert(Raiding);
            if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                queue.push(GpuUpdate::SetTarget {
                    idx: npc_idx.0,
                    x: farm_pos.x,
                    y: farm_pos.y,
                });
            }
        }
    }
}

// ============================================================================
// FLEE / LEASH / RECOVERY SYSTEMS (generic)
// ============================================================================

/// Flee combat when HP drops below FleeThreshold.
pub fn flee_system(
    mut commands: Commands,
    query: Query<(Entity, &NpcIndex, &Health, &FleeThreshold, &Home, Option<&CarryingFood>), With<InCombat>>,
) {
    for (entity, npc_idx, health, flee, home, carrying) in query.iter() {
        let health_pct = health.0 / 100.0;
        if health_pct < flee.pct {
            let mut cmds = commands.entity(entity);
            cmds.remove::<InCombat>();
            cmds.insert(Returning);

            if carrying.is_some() {
                cmds.remove::<CarryingFood>();
                // Reset color
                if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                    let (r, g, b, a) = Job::Raider.color();
                    queue.push(GpuUpdate::SetColor {
                        idx: npc_idx.0, r, g, b, a,
                    });
                }
            }

            if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                queue.push(GpuUpdate::SetTarget {
                    idx: npc_idx.0,
                    x: home.0.x,
                    y: home.0.y,
                });
            }
        }
    }
}

/// Disengage combat when too far from home.
pub fn leash_system(
    mut commands: Commands,
    query: Query<(Entity, &NpcIndex, &LeashRange, &Home), With<InCombat>>,
) {
    let positions = match GPU_READ_STATE.lock() {
        Ok(state) => state.positions.clone(),
        Err(_) => return,
    };

    for (entity, npc_idx, leash, home) in query.iter() {
        let i = npc_idx.0;
        if i * 2 + 1 >= positions.len() {
            continue;
        }

        let x = positions[i * 2];
        let y = positions[i * 2 + 1];
        let dx = x - home.0.x;
        let dy = y - home.0.y;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist > leash.distance {
            commands.entity(entity)
                .remove::<InCombat>()
                .insert(Returning);

            if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                queue.push(GpuUpdate::SetTarget {
                    idx: npc_idx.0,
                    x: home.0.x,
                    y: home.0.y,
                });
            }
        }
    }
}

/// Wounded NPCs arriving home enter recovery mode.
pub fn wounded_rest_system(
    mut commands: Commands,
    mut events: MessageReader<ArrivalMsg>,
    query: Query<(Entity, &NpcIndex, &Health, &WoundedThreshold), Without<Recovering>>,
) {
    for event in events.read() {
        for (entity, npc_idx, health, wounded) in query.iter() {
            if npc_idx.0 == event.npc_index {
                let health_pct = health.0 / 100.0;
                if health_pct < wounded.pct {
                    commands.entity(entity)
                        .insert(Recovering { threshold: 0.75 })
                        .insert(Resting);
                }
                break;
            }
        }
    }
}

/// Recovery system: resting NPCs with Recovering resume activity when healed.
pub fn recovery_system(
    mut commands: Commands,
    query: Query<(Entity, &Health, &Recovering), With<Resting>>,
) {
    for (entity, health, recovering) in query.iter() {
        let health_pct = health.0 / 100.0;
        if health_pct >= recovering.threshold {
            commands.entity(entity)
                .remove::<Recovering>()
                .remove::<Resting>();
            // Falls through to steal_decision_system or resume_patrol next tick
        }
    }
}
