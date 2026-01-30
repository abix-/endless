//! Behavior systems - State transitions based on energy and arrivals

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;

use crate::components::*;
use crate::messages::{ArrivalMsg, GpuUpdate, GpuUpdateMsg};
use crate::constants::*;
use crate::resources::{FoodEvents, FoodDelivered, PopulationStats, GpuReadState, FoodStorage};
use crate::systems::economy::*;
use crate::world::{WorldData, LocationKind, find_nearest_location};

/// Patrol system: count ticks at post and move to next (anyone with PatrolRoute + OnDuty).
/// Skip NPCs in combat - they chase enemies instead.
pub fn patrol_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut PatrolRoute, &mut OnDuty, &NpcIndex), Without<InCombat>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
) {
    for (entity, mut patrol, mut on_duty, npc_idx) in query.iter_mut() {
        on_duty.ticks_waiting += 1;

        if on_duty.ticks_waiting >= GUARD_PATROL_WAIT {
            if !patrol.posts.is_empty() {
                patrol.current = (patrol.current + 1) % patrol.posts.len();
            }

            commands.entity(entity)
                .remove::<OnDuty>()
                .insert(Patrolling);

            if let Some(pos) = patrol.posts.get(patrol.current) {
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                    idx: npc_idx.0,
                    x: pos.x,
                    y: pos.y,
                }));
            }
        }
    }
}

/// Arrival system: transition states based on current marker.
/// - Patrolling → OnDuty
/// - GoingToRest → Resting
/// - GoingToWork → Working
/// - Raiding → CarryingFood + Returning
/// - Returning → deliver food, clear state
/// Also checks WoundedThreshold for recovery mode.
pub fn arrival_system(
    mut commands: Commands,
    mut events: MessageReader<ArrivalMsg>,
    query: Query<(
        Entity, &NpcIndex, &Job, &TownId, &Home, &Health,
        Option<&Patrolling>, Option<&GoingToRest>, Option<&GoingToWork>,
        Option<&Raiding>, Option<&Returning>, Option<&CarryingFood>,
        Option<&WoundedThreshold>,
    ), Without<Recovering>>,
    mut pop_stats: ResMut<PopulationStats>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut food_storage: ResMut<FoodStorage>,
    mut food_events: ResMut<FoodEvents>,
    world_data: Res<WorldData>,
    gpu_state: Res<GpuReadState>,
) {
    let positions = &gpu_state.positions;
    let farms: Vec<Vector2> = world_data.farms.iter().map(|f| f.position).collect();
    const FARM_ARRIVAL_RADIUS: f32 = 40.0;  // Grid spacing is 34px, keep tight to avoid false positives

    for event in events.read() {
        for (entity, npc_idx, job, town, home, health,
             patrolling, going_rest, going_work,
             raiding, returning, carrying, wounded) in query.iter()
        {
            if npc_idx.0 != event.npc_index { continue; }
            let idx = npc_idx.0;

            if patrolling.is_some() {
                commands.entity(entity)
                    .remove::<Patrolling>()
                    .insert(OnDuty { ticks_waiting: 0 });
            } else if going_rest.is_some() {
                commands.entity(entity)
                    .remove::<GoingToRest>()
                    .insert(Resting);
            } else if going_work.is_some() {
                commands.entity(entity)
                    .remove::<GoingToWork>()
                    .insert(Working);
                pop_inc_working(&mut pop_stats, *job, town.0);
            } else if raiding.is_some() {
                if idx * 2 + 1 < positions.len() {
                    let pos = Vector2::new(positions[idx * 2], positions[idx * 2 + 1]);
                    let near_farm = farms.iter().any(|farm| {
                        let dx = pos.x - farm.x;
                        let dy = pos.y - farm.y;
                        (dx * dx + dy * dy).sqrt() < FARM_ARRIVAL_RADIUS
                    });
                    if near_farm {
                        commands.entity(entity)
                            .remove::<Raiding>()
                            .insert(CarryingFood)
                            .insert(Returning);
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetColor {
                            idx: npc_idx.0,
                            r: 1.0, g: 0.9, b: 0.2, a: 1.0,
                        }));
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                            idx: npc_idx.0,
                            x: home.0.x,
                            y: home.0.y,
                        }));
                    }
                }
            } else if returning.is_some() {
                let mut cmds = commands.entity(entity);
                cmds.remove::<Returning>();

                if carrying.is_some() {
                    cmds.remove::<CarryingFood>();
                    let (r, g, b, a) = Job::Raider.color();
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetColor {
                        idx: npc_idx.0, r, g, b, a,
                    }));
                    if !food_storage.food.is_empty() {
                        let last_idx = food_storage.food.len() - 1;
                        food_storage.food[last_idx] += 1;
                    }
                    food_events.delivered.push(FoodDelivered { camp_idx: 0 });
                }
            }

            if let Some(w) = wounded {
                let health_pct = health.0 / 100.0;
                if health_pct < w.pct {
                    commands.entity(entity)
                        .insert(Recovering { threshold: 0.75 })
                        .insert(Resting);
                }
            }

            break;
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
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
) {
    for (entity, npc_idx, health, flee, home, carrying) in query.iter() {
        let health_pct = health.0 / 100.0;
        if health_pct < flee.pct {
            let mut cmds = commands.entity(entity);
            cmds.remove::<InCombat>();
            cmds.remove::<CombatOrigin>();
            cmds.remove::<Raiding>();
            cmds.insert(Returning);

            if carrying.is_some() {
                cmds.remove::<CarryingFood>();
                let (r, g, b, a) = Job::Raider.color();
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetColor {
                    idx: npc_idx.0, r, g, b, a,
                }));
            }

            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                idx: npc_idx.0,
                x: home.0.x,
                y: home.0.y,
            }));
        }
    }
}

/// Disengage combat when too far from where combat started.
pub fn leash_system(
    mut commands: Commands,
    query: Query<(Entity, &NpcIndex, &LeashRange, &Home, &CombatOrigin), With<InCombat>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    gpu_state: Res<GpuReadState>,
) {
    let positions = &gpu_state.positions;

    for (entity, npc_idx, leash, home, origin) in query.iter() {
        let i = npc_idx.0;
        if i * 2 + 1 >= positions.len() {
            continue;
        }

        let x = positions[i * 2];
        let y = positions[i * 2 + 1];
        let dx = x - origin.x;
        let dy = y - origin.y;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist > leash.distance {
            commands.entity(entity)
                .remove::<InCombat>()
                .remove::<CombatOrigin>()
                .remove::<Raiding>()
                .insert(Returning);

            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                idx: npc_idx.0,
                x: home.0.x,
                y: home.0.y,
            }));
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
    Fight,
    Flee,
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
/// Runs on NPCs without an active state, OR raiders continuing their mission after combat.
pub fn decision_system(
    mut commands: Commands,
    query: Query<
        (Entity, &NpcIndex, &Job, &Energy, &Health, &Home, &Personality,
         Option<&WorkPosition>, Option<&PatrolRoute>, Option<&Stealer>, Option<&Raiding>),
        (Without<Patrolling>, Without<OnDuty>, Without<Working>, Without<GoingToWork>,
         Without<Resting>, Without<GoingToRest>, Without<Returning>,
         Without<InCombat>, Without<Recovering>, Without<Dead>)
    >,
    _pop_stats: ResMut<PopulationStats>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    world_data: Res<WorldData>,
    gpu_state: Res<GpuReadState>,
) {
    let frame = DECISION_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    for (entity, npc_idx, job, energy, _health, home, personality, work_pos, patrol, _stealer, raiding) in query.iter() {
        let idx = npc_idx.0;

        // Raiders continuing mission after combat - re-target nearest farm
        if raiding.is_some() {
            let pos = if idx * 2 + 1 < gpu_state.positions.len() {
                Vector2::new(gpu_state.positions[idx * 2], gpu_state.positions[idx * 2 + 1])
            } else {
                home.0
            };
            if let Some(farm_pos) = find_nearest_location(pos, &world_data, LocationKind::Farm) {
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                    idx, x: farm_pos.x, y: farm_pos.y,
                }));
            }
            continue;
        }

        let en = energy.0;
        let (_fight_m, _flee_m, rest_m, eat_m, work_m, wander_m) = personality.get_multipliers();

        let food_available = home.is_valid();
        let mut scores: Vec<(Action, f32)> = Vec::with_capacity(6);

        if food_available {
            let eat_score = (100.0 - en) * SCORE_EAT_MULT * eat_m;
            if eat_score > 0.0 {
                scores.push((Action::Eat, eat_score));
            }
        }

        let rest_score = (100.0 - en) * SCORE_REST_MULT * rest_m;
        if rest_score > 0.0 && home.is_valid() {
            scores.push((Action::Rest, rest_score));
        }

        let can_work = match job {
            Job::Farmer => work_pos.is_some(),
            Job::Guard => patrol.is_some(),
            Job::Raider => true,
            Job::Fighter => false,
        };
        if can_work {
            let work_score = SCORE_WORK_BASE * work_m;
            scores.push((Action::Work, work_score));
        }

        let wander_score = SCORE_WANDER_BASE * wander_m;
        scores.push((Action::Wander, wander_score));

        let action = weighted_random(&scores, idx, frame);

        match action {
            Action::Eat | Action::Rest => {
                if home.is_valid() {
                    commands.entity(entity).insert(GoingToRest);
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                        idx, x: home.0.x, y: home.0.y,
                    }));
                }
            }
            Action::Work => {
                match job {
                    Job::Farmer => {
                        if let Some(wp) = work_pos {
                            commands.entity(entity).insert(GoingToWork);
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                                idx, x: wp.0.x, y: wp.0.y,
                            }));
                        }
                    }
                    Job::Guard => {
                        if let Some(p) = patrol {
                            commands.entity(entity).insert(Patrolling);
                            if let Some(pos) = p.posts.get(p.current) {
                                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                                    idx, x: pos.x, y: pos.y,
                                }));
                            }
                        }
                    }
                    Job::Raider => {
                        let pos = if idx * 2 + 1 < gpu_state.positions.len() {
                            Vector2::new(gpu_state.positions[idx * 2], gpu_state.positions[idx * 2 + 1])
                        } else {
                            home.0
                        };
                        if let Some(farm_pos) = find_nearest_location(pos, &world_data, LocationKind::Farm) {
                            commands.entity(entity).insert(Raiding);
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                                idx, x: farm_pos.x, y: farm_pos.y,
                            }));
                        }
                    }
                    Job::Fighter => {}
                }
            }
            Action::Wander => {
                if idx * 2 + 1 < gpu_state.positions.len() {
                    let x = gpu_state.positions[idx * 2];
                    let y = gpu_state.positions[idx * 2 + 1];
                    let offset_x = (pseudo_random(idx, frame + 1) - 0.5) * 200.0;
                    let offset_y = (pseudo_random(idx, frame + 2) - 0.5) * 200.0;
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                        idx, x: x + offset_x, y: y + offset_y,
                    }));
                }
            }
            Action::Fight | Action::Flee => {}
        }
    }
}
