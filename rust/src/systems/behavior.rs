//! Behavior systems - State transitions based on energy and arrivals

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;

use crate::components::*;
use crate::messages::*;
use crate::constants::*;
use crate::resources::*;
use crate::systems::economy::*;
use crate::world::WORLD_DATA;

/// Tired system: anyone with Home + Energy below threshold goes to rest.
/// Skip NPCs in combat - they fight until the enemy is dead or they flee.
pub fn tired_system(
    mut commands: Commands,
    query: Query<(Entity, &Energy, &NpcIndex, &Home, &Job, &TownId, Option<&Working>),
                 (Without<GoingToRest>, Without<Resting>, Without<InCombat>)>,
    mut pop_stats: ResMut<PopulationStats>,
) {
    for (entity, energy, npc_idx, home, job, clan, working) in query.iter() {
        if energy.0 < ENERGY_HUNGRY && home.is_valid() {
            if working.is_some() {
                pop_dec_working(&mut pop_stats, *job, clan.0);
            }
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
    going_to_work_query: Query<(Entity, &NpcIndex, &Job, &TownId), With<GoingToWork>>,
    mut pop_stats: ResMut<PopulationStats>,
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
        for (entity, npc_idx, job, clan) in going_to_work_query.iter() {
            if npc_idx.0 == event.npc_index {
                commands.entity(entity)
                    .remove::<GoingToWork>()
                    .insert(Working);
                pop_inc_working(&mut pop_stats, *job, clan.0);
                break;
            }
        }
    }
}

// ============================================================================
// STEALING SYSTEMS (generic — any NPC with Stealer component)
// ============================================================================

/// Handle arrivals for raiders (Raiding → pickup, Returning → deliver).
pub fn raider_arrival_system(
    mut commands: Commands,
    mut events: MessageReader<ArrivalMsg>,
    raiding_query: Query<(Entity, &NpcIndex, &Home, &Health, Option<&WoundedThreshold>), With<Raiding>>,
    returning_query: Query<(Entity, &NpcIndex, Option<&CarryingFood>), With<Returning>>,
) {
    // Get current positions and farm locations
    let positions = match GPU_READ_STATE.lock() {
        Ok(state) => state.positions.clone(),
        Err(_) => return,
    };
    let farms: Vec<Vector2> = match WORLD_DATA.lock() {
        Ok(world) => world.farms.iter().map(|f| f.position).collect(),
        Err(_) => Vec::new(),
    };
    const FARM_ARRIVAL_RADIUS: f32 = 100.0;

    for event in events.read() {
        // Raiding NPC arrived at farm → pick up food
        for (entity, npc_idx, home, _health, _wounded) in raiding_query.iter() {
            if npc_idx.0 == event.npc_index {
                // Verify raider is actually near a farm (not a stale arrival event)
                let idx = npc_idx.0;
                if idx * 2 + 1 >= positions.len() {
                    break;
                }
                let pos = Vector2::new(positions[idx * 2], positions[idx * 2 + 1]);
                let near_farm = farms.iter().any(|farm| {
                    let dx = pos.x - farm.x;
                    let dy = pos.y - farm.y;
                    (dx * dx + dy * dy).sqrt() < FARM_ARRIVAL_RADIUS
                });
                if !near_farm {
                    // Stale arrival event - ignore
                    break;
                }

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

                    // Deliver food to raider town
                    // TODO: use NPC's TownId component to determine which town
                    if let Ok(mut food) = FOOD_STORAGE.lock() {
                        if !food.food.is_empty() {
                            // Raider towns are at the end of the food array
                            let last_idx = food.food.len() - 1;
                            food.food[last_idx] += 1;
                        }
                    }
                    if let Ok(mut queue) = FOOD_DELIVERED_QUEUE.lock() {
                        queue.push(FoodDelivered { camp_idx: 0 });
                    }
                }

                // Fall through to npc_decision_system next tick
                // (entity has no active state markers)
                break;
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
            cmds.remove::<CombatOrigin>();
            cmds.remove::<Raiding>();  // Clear raiding state when fleeing
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

/// Disengage combat when too far from where combat started.
pub fn leash_system(
    mut commands: Commands,
    query: Query<(Entity, &NpcIndex, &LeashRange, &Home, &CombatOrigin), With<InCombat>>,
) {
    let positions = match GPU_READ_STATE.lock() {
        Ok(state) => state.positions.clone(),
        Err(_) => return,
    };

    for (entity, npc_idx, leash, home, origin) in query.iter() {
        let i = npc_idx.0;
        if i * 2 + 1 >= positions.len() {
            continue;
        }

        let x = positions[i * 2];
        let y = positions[i * 2 + 1];
        // Check distance from combat origin, not home
        let dx = x - origin.x;
        let dy = y - origin.y;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist > leash.distance {
            commands.entity(entity)
                .remove::<InCombat>()
                .remove::<CombatOrigin>()
                .remove::<Raiding>()  // Clear raiding state when leashing
                .insert(Returning);

            // Return home after disengaging
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
            // Falls through to npc_decision_system next tick
        }
    }
}

// ============================================================================
// UTILITY AI DECISION SYSTEM
// ============================================================================

/// Actions an NPC can take.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)]
enum Action {
    Fight,  // Reserved for combat decisions
    Flee,   // Reserved for combat decisions
    Eat,
    Rest,
    Work,
    Wander,
}

/// Simple deterministic "random" for weighted selection.
fn pseudo_random(seed: usize, frame: usize) -> f32 {
    let x = ((seed.wrapping_mul(1103515245).wrapping_add(frame)) >> 16) & 0x7fff;
    (x as f32) / 32767.0
}

/// Weighted random selection from scored actions.
fn weighted_random(scores: &[(Action, f32)], seed: usize, frame: usize) -> Action {
    let total: f32 = scores.iter().map(|(_, s)| *s).sum();
    if total <= 0.0 {
        return Action::Wander;
    }

    let roll = pseudo_random(seed, frame) * total;
    let mut acc = 0.0;
    for (action, score) in scores {
        acc += score;
        if roll < acc {
            return *action;
        }
    }
    scores.last().map(|(a, _)| *a).unwrap_or(Action::Wander)
}

/// Frame counter for pseudo-random seeding.
static DECISION_FRAME: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Unified decision system: score actions, weighted random, execute.
/// Runs on NPCs without an active state.
pub fn npc_decision_system(
    mut commands: Commands,
    query: Query<
        (Entity, &NpcIndex, &Job, &Energy, &Health, &Home, &Personality,
         Option<&WorkPosition>, Option<&PatrolRoute>, Option<&Stealer>),
        (Without<Patrolling>, Without<OnDuty>, Without<Working>, Without<GoingToWork>,
         Without<Resting>, Without<GoingToRest>, Without<Raiding>, Without<Returning>,
         Without<InCombat>, Without<Recovering>, Without<Dead>)
    >,
    _pop_stats: ResMut<PopulationStats>,
) {
    let frame = DECISION_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    for (entity, npc_idx, job, energy, _health, home, personality, work_pos, patrol, _stealer) in query.iter() {
        let en = energy.0;
        let (_fight_m, _flee_m, rest_m, eat_m, work_m, wander_m) = personality.get_multipliers();

        // Check if food is available at home (simplified: assume yes if home is valid)
        let food_available = home.is_valid();

        // Score all possible actions
        let mut scores: Vec<(Action, f32)> = Vec::with_capacity(6);

        // Eat: based on low energy, higher multiplier than rest
        if food_available {
            let eat_score = (100.0 - en) * SCORE_EAT_MULT * eat_m;
            if eat_score > 0.0 {
                scores.push((Action::Eat, eat_score));
            }
        }

        // Rest: based on low energy
        let rest_score = (100.0 - en) * SCORE_REST_MULT * rest_m;
        if rest_score > 0.0 && home.is_valid() {
            scores.push((Action::Rest, rest_score));
        }

        // Work: job-specific
        let can_work = match job {
            Job::Farmer => work_pos.is_some(),
            Job::Guard => patrol.is_some(),
            Job::Raider => true, // Raiders "work" by raiding
            Job::Fighter => false,
        };
        if can_work {
            let work_score = SCORE_WORK_BASE * work_m;
            scores.push((Action::Work, work_score));
        }

        // Wander: always available baseline
        let wander_score = SCORE_WANDER_BASE * wander_m;
        scores.push((Action::Wander, wander_score));

        // Choose action via weighted random
        let action = weighted_random(&scores, npc_idx.0, frame);

        // Execute chosen action
        match action {
            Action::Eat | Action::Rest => {
                // Go home to eat or rest
                if home.is_valid() {
                    commands.entity(entity).insert(GoingToRest);
                    if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                        queue.push(GpuUpdate::SetTarget {
                            idx: npc_idx.0,
                            x: home.0.x,
                            y: home.0.y,
                        });
                    }
                }
            }
            Action::Work => {
                match job {
                    Job::Farmer => {
                        if let Some(wp) = work_pos {
                            commands.entity(entity).insert(GoingToWork);
                            if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                                queue.push(GpuUpdate::SetTarget {
                                    idx: npc_idx.0,
                                    x: wp.0.x,
                                    y: wp.0.y,
                                });
                            }
                        }
                    }
                    Job::Guard => {
                        if let Some(p) = patrol {
                            commands.entity(entity).insert(Patrolling);
                            if let Some(pos) = p.posts.get(p.current) {
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
                    Job::Raider => {
                        // Find nearest farm and raid it
                        let nearest_farm = if let Ok(world) = WORLD_DATA.lock() {
                            let pos = if let Ok(state) = GPU_READ_STATE.lock() {
                                let i = npc_idx.0;
                                if i * 2 + 1 < state.positions.len() {
                                    Vector2::new(state.positions[i * 2], state.positions[i * 2 + 1])
                                } else {
                                    home.0
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
                    Job::Fighter => {}
                }
            }
            Action::Wander => {
                // Random wander near current position
                if let Ok(state) = GPU_READ_STATE.lock() {
                    let i = npc_idx.0;
                    if i * 2 + 1 < state.positions.len() {
                        let x = state.positions[i * 2];
                        let y = state.positions[i * 2 + 1];
                        // Wander within 100px
                        let offset_x = (pseudo_random(npc_idx.0, frame + 1) - 0.5) * 200.0;
                        let offset_y = (pseudo_random(npc_idx.0, frame + 2) - 0.5) * 200.0;
                        if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                            queue.push(GpuUpdate::SetTarget {
                                idx: npc_idx.0,
                                x: x + offset_x,
                                y: y + offset_y,
                            });
                        }
                    }
                }
            }
            Action::Fight | Action::Flee => {
                // These are handled by combat systems, not here
            }
        }
    }
}
