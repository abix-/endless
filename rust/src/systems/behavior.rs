//! Behavior systems - Unified decision-making and state transitions
//!
//! Key systems:
//! - `arrival_system`: Handles state transitions when NPCs arrive at destinations
//! - `on_duty_tick_system`: Increments guard wait counters
//! - `decision_system`: Central priority-based decision making for all NPCs
//!
//! The decision system uses a priority cascade (first match wins):
//! 1. InCombat + should_flee? → Flee
//! 2. InCombat + should_leash? → Leash
//! 3. InCombat → Skip (attack_system handles)
//! 4. Recovering + healed? → Resume
//! 5. Working + tired? → Stop work
//! 6. OnDuty + time_to_patrol? → Patrol
//! 7. Resting + rested? → Wake up
//! 8. Raiding (post-combat) → Re-target farm
//! 9. Idle → Score Eat/Rest/Work/Wander

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;

use crate::components::*;
use crate::messages::{ArrivalMsg, GpuUpdate, GpuUpdateMsg};
use crate::constants::*;
use crate::resources::{FoodEvents, FoodDelivered, FoodConsumed, PopulationStats, GpuReadState, FoodStorage, GameTime, NpcLogCache, FarmStates, FarmGrowthState};
use crate::systems::economy::*;
use crate::world::{WorldData, LocationKind, find_nearest_location, find_location_within_radius, FarmOccupancy};

// Distinct colors for raider factions (must match spawn.rs)
const RAIDER_COLORS: [(f32, f32, f32); 10] = [
    (0.9, 0.2, 0.2),   // Red
    (0.9, 0.5, 0.1),   // Orange
    (0.8, 0.2, 0.6),   // Magenta
    (0.6, 0.2, 0.8),   // Purple
    (0.9, 0.8, 0.1),   // Yellow
    (0.7, 0.3, 0.2),   // Brown
    (0.9, 0.3, 0.5),   // Pink
    (0.5, 0.1, 0.1),   // Dark red
    (0.8, 0.6, 0.2),   // Gold
    (0.6, 0.1, 0.4),   // Dark magenta
];

fn raider_faction_color(faction: &Faction) -> (f32, f32, f32, f32) {
    let idx = ((faction.0 - 1).max(0) as usize) % RAIDER_COLORS.len();
    let (r, g, b) = RAIDER_COLORS[idx];
    (r, g, b, 1.0)
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
        Entity, &NpcIndex, &Job, &TownId, &Home, &Health, &Faction,
        Option<&Patrolling>, Option<&GoingToRest>, Option<&GoingToWork>,
        Option<&Raiding>, Option<&Returning>, Option<&CarryingFood>,
        Option<&WoundedThreshold>, Option<&Wandering>,
    ), Without<Recovering>>,
    // Separate query for WorkPosition (Bevy limits tuples to 15 elements)
    work_positions: Query<&WorkPosition>,
    // Query for Working farmers with AssignedFarm (for drift check)
    working_farmers: Query<(&NpcIndex, &AssignedFarm), With<Working>>,
    mut pop_stats: ResMut<PopulationStats>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut food_storage: ResMut<FoodStorage>,
    mut food_events: ResMut<FoodEvents>,
    world_data: Res<WorldData>,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    mut npc_logs: ResMut<NpcLogCache>,
    mut farm_states: ResMut<FarmStates>,
    mut farm_occupancy: ResMut<FarmOccupancy>,
    mut frame_counter: Local<u32>,
) {
    let positions = &gpu_state.positions;
    const FARM_ARRIVAL_RADIUS: f32 = 20.0;  // Tight: farmer must be near farm center
    const DELIVERY_RADIUS: f32 = 150.0;     // Same as healing radius - deliver when near camp
    const MAX_DRIFT: f32 = 20.0;            // Keep farmers visually on the farm (3x3 = ~51px, so 20px from center)

    // Increment frame counter for throttled drift check
    *frame_counter = frame_counter.wrapping_add(1);
    let frame_slot = *frame_counter % 30;

    // Working farmer drift check (throttled: each farmer checked once per 30 frames)
    for (npc_idx, assigned) in working_farmers.iter() {
        // Stagger checks: only check if npc_idx % 30 == current frame slot
        if (npc_idx.0 as u32) % 30 != frame_slot { continue; }

        // Get farm position
        if assigned.0 >= world_data.farms.len() { continue; }
        let farm_pos = world_data.farms[assigned.0].position;

        // Get farmer's current position
        let idx = npc_idx.0;
        if idx * 2 + 1 >= positions.len() { continue; }
        let current = Vector2::new(positions[idx * 2], positions[idx * 2 + 1]);

        // If drifted too far, re-target to farm
        if current.distance_to(farm_pos) > MAX_DRIFT {
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                idx, x: farm_pos.x, y: farm_pos.y
            }));
        }
    }

    // Proximity-based arrival for Returning and GoingToRest (don't wait for exact arrival)
    for (entity, npc_idx, _job, town, home, _health, faction,
         _patrolling, going_rest, _going_work,
         _raiding, returning, carrying, _wounded, _wandering) in query.iter()
    {
        let is_returning = returning.is_some();
        let is_going_rest = going_rest.is_some();
        if !is_returning && !is_going_rest { continue; }

        let idx = npc_idx.0;
        if idx * 2 + 1 >= positions.len() { continue; }

        let x = positions[idx * 2];
        let y = positions[idx * 2 + 1];
        let dx = x - home.0.x;
        let dy = y - home.0.y;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist <= DELIVERY_RADIUS {
            let mut cmds = commands.entity(entity);

            if is_returning {
                cmds.remove::<Returning>();
                if carrying.is_some() {
                    cmds.remove::<CarryingFood>();
                    cmds.remove::<CarriedItem>();
                    let (r, g, b, a) = raider_faction_color(faction);
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetColor { idx, r, g, b, a }));
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetCarriedItem { idx, item_id: CarriedItem::NONE }));
                    let camp_idx = town.0 as usize;
                    if camp_idx < food_storage.food.len() {
                        food_storage.food[camp_idx] += 1;
                    }
                    food_events.delivered.push(FoodDelivered { camp_idx: town.0 as u32 });
                }
            } else if is_going_rest {
                cmds.remove::<GoingToRest>();
                cmds.insert(Resting);
            }
        }
    }

    for event in events.read() {
        for (entity, npc_idx, job, town, home, health, _faction,
             patrolling, going_rest, going_work,
             raiding, returning, _carrying, wounded, wandering) in query.iter()
        {
            if npc_idx.0 != event.npc_index { continue; }
            let idx = npc_idx.0;

            if patrolling.is_some() {
                commands.entity(entity)
                    .remove::<Patrolling>()
                    .insert(OnDuty { ticks_waiting: 0 });
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ OnDuty".into());
            } else if going_rest.is_some() {
                commands.entity(entity)
                    .remove::<GoingToRest>()
                    .insert(Resting);
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Resting".into());
            } else if going_work.is_some() {
                // Farmers: find farm at WorkPosition (not current pos - they may have been pushed)
                if *job == Job::Farmer {
                    // Use WorkPosition to find target farm (that's where we sent them)
                    let search_pos = work_positions.get(entity)
                        .map(|wp| wp.0)
                        .unwrap_or_else(|_| {
                            // Fallback to current position if no WorkPosition
                            if idx * 2 + 1 < positions.len() {
                                Vector2::new(positions[idx * 2], positions[idx * 2 + 1])
                            } else {
                                Vector2::new(0.0, 0.0)
                            }
                        });
                    if let Some((farm_idx, farm_pos)) = find_location_within_radius(search_pos, &world_data, LocationKind::Farm, FARM_ARRIVAL_RADIUS) {
                        // Reserve farm (increment occupancy)
                        if farm_idx < farm_occupancy.occupant_count.len() {
                            farm_occupancy.occupant_count[farm_idx] += 1;
                        }

                        // Transition to Working with AssignedFarm
                        commands.entity(entity)
                            .remove::<GoingToWork>()
                            .insert(Working)
                            .insert(AssignedFarm(farm_idx));
                        pop_inc_working(&mut pop_stats, *job, town.0);

                        // Set target to FARM position (not farmer position) so they return if pushed
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: farm_pos.x, y: farm_pos.y }));

                        // Check if farm is ready to harvest
                        if farm_idx < farm_states.states.len()
                            && farm_states.states[farm_idx] == FarmGrowthState::Ready
                        {
                            // Harvest: add food to town storage, reset farm
                            let town_idx = town.0 as usize;
                            if town_idx < food_storage.food.len() {
                                food_storage.food[town_idx] += 1;
                                food_events.consumed.push(FoodConsumed {
                                    location_idx: farm_idx as u32,
                                    is_camp: false,
                                });
                            }
                            farm_states.states[farm_idx] = FarmGrowthState::Growing;
                            farm_states.progress[farm_idx] = 0.0;
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Harvested → Working".into());
                        } else {
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Working (tending)".into());
                        }
                    } else {
                        // No farm found at WorkPosition - this shouldn't happen, but handle gracefully
                        commands.entity(entity)
                            .remove::<GoingToWork>()
                            .insert(Working);
                        pop_inc_working(&mut pop_stats, *job, town.0);
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: search_pos.x, y: search_pos.y }));
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Working (no farm)".into());
                    }
                } else {
                    // Non-farmers just transition to Working at current position
                    let current_pos = if idx * 2 + 1 < positions.len() {
                        Vector2::new(positions[idx * 2], positions[idx * 2 + 1])
                    } else {
                        Vector2::new(0.0, 0.0)
                    };
                    commands.entity(entity)
                        .remove::<GoingToWork>()
                        .insert(Working);
                    pop_inc_working(&mut pop_stats, *job, town.0);
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: current_pos.x, y: current_pos.y }));
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Working".into());
                }
            } else if raiding.is_some() {
                if idx * 2 + 1 < positions.len() {
                    let pos = Vector2::new(positions[idx * 2], positions[idx * 2 + 1]);

                    // Find nearest farm within arrival radius and check if Ready
                    let ready_farm = find_location_within_radius(pos, &world_data, LocationKind::Farm, FARM_ARRIVAL_RADIUS)
                        .filter(|(farm_idx, _)| {
                            *farm_idx < farm_states.states.len()
                                && farm_states.states[*farm_idx] == FarmGrowthState::Ready
                        });

                    if let Some((farm_idx, _)) = ready_farm {
                        // Farm is ready - steal food and reset farm to Growing
                        farm_states.states[farm_idx] = FarmGrowthState::Growing;
                        farm_states.progress[farm_idx] = 0.0;

                        commands.entity(entity)
                            .remove::<Raiding>()
                            .insert(CarryingFood)
                            .insert(CarriedItem(CarriedItem::FOOD))
                            .insert(Returning);
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Stole food → Returning".into());
                        // Show carried item above head
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetCarriedItem {
                            idx: npc_idx.0,
                            item_id: CarriedItem::FOOD,
                        }));
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                            idx: npc_idx.0,
                            x: home.0.x,
                            y: home.0.y,
                        }));
                    } else {
                        // No ready farm nearby - find another farm to raid
                        if let Some(farm_pos) = find_nearest_location(pos, &world_data, LocationKind::Farm) {
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                                idx: npc_idx.0,
                                x: farm_pos.x,
                                y: farm_pos.y,
                            }));
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Farm not ready, seeking another".into());
                        }
                    }
                }
            } else if returning.is_some() {
                // Arrival at wrong location (e.g., after combat chase) - re-target home.
                // Actual delivery handled by proximity check (lines 111-127).
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                    idx: npc_idx.0,
                    x: home.0.x,
                    y: home.0.y,
                }));
            } else if wandering.is_some() {
                // Wandering complete - remove marker, NPC goes back to decision_system
                commands.entity(entity).remove::<Wandering>();
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Idle".into());
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

/// Count nearby enemies and allies for threat assessment.
/// Returns (enemy_count, ally_count) within radius.
fn count_nearby_factions(
    positions: &[f32],
    factions: &[i32],
    health: &[f32],
    my_idx: usize,
    my_faction: i32,
    radius: f32,
) -> (u32, u32) {
    let radius_sq = radius * radius;
    let my_x = positions.get(my_idx * 2).copied().unwrap_or(0.0);
    let my_y = positions.get(my_idx * 2 + 1).copied().unwrap_or(0.0);

    let mut enemies = 0u32;
    let mut allies = 0u32;

    let npc_count = factions.len().min(positions.len() / 2).min(health.len());
    for i in 0..npc_count {
        if i == my_idx { continue; }

        // Skip dead NPCs
        let hp = health.get(i).copied().unwrap_or(0.0);
        if hp <= 0.0 { continue; }

        let x = positions[i * 2];
        let y = positions[i * 2 + 1];
        let dx = x - my_x;
        let dy = y - my_y;
        let dist_sq = dx * dx + dy * dy;

        if dist_sq <= radius_sq {
            let their_faction = factions.get(i).copied().unwrap_or(-1);
            if their_faction == my_faction {
                allies += 1;
            } else if their_faction >= 0 {
                enemies += 1;
            }
        }
    }

    (enemies, allies)
}

// ============================================================================
// DECISION SYSTEM
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

/// Unified decision system: ALL NPC decisions in one place with priority cascade.
///
/// Priority order (first match wins):
/// 1. InCombat + should_flee? → Flee
/// 2. InCombat + should_leash? → Leash
/// 3. InCombat → Skip (attack_system handles)
/// 4. Recovering + healed? → Resume
/// 5. Working + tired? → Stop work
/// 6. OnDuty + time_to_patrol? → Patrol
/// 7. Resting + rested? → Wake up
/// 8. Raiding (post-combat) → Re-target farm
/// 9. Idle → Score Eat/Rest/Work/Wander
pub fn decision_system(
    mut commands: Commands,
    // Main query: NPCs not in transit states
    mut query: Query<
        (Entity, &NpcIndex, &Job, &mut Energy, &Health, &Home, &Personality, &TownId, &Faction,
         Option<&InCombat>, Option<&Recovering>, Option<&Working>, Option<&OnDuty>,
         Option<&Resting>, Option<&Raiding>),
        (Without<Patrolling>, Without<GoingToWork>, Without<GoingToRest>,
         Without<Returning>, Without<Wandering>, Without<Dead>)
    >,
    // Combat data for flee/leash
    flee_data: Query<(&FleeThreshold, Option<&CarryingFood>)>,
    leash_data: Query<(&LeashRange, &CombatOrigin)>,
    // Patrol data (mutable for advancing)
    mut patrols: Query<&mut PatrolRoute>,
    // Work data
    work_positions: Query<&WorkPosition>,
    assigned_farms: Query<&AssignedFarm>,
    // Resources
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut food_storage: ResMut<FoodStorage>,
    mut farm_occupancy: ResMut<FarmOccupancy>,
    world_data: Res<WorldData>,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    mut npc_logs: ResMut<NpcLogCache>,
) {
    let frame = DECISION_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let positions = &gpu_state.positions;
    let factions_buf = &gpu_state.factions;
    let health_buf = &gpu_state.health;

    const THREAT_RADIUS: f32 = 200.0;
    const CHECK_INTERVAL: usize = 30;

    for (entity, npc_idx, job, mut energy, health, home, personality, town_id, faction,
         in_combat, recovering, working, on_duty, resting, raiding) in query.iter_mut()
    {
        let idx = npc_idx.0;

        // ====================================================================
        // Priority 1-3: Combat decisions (flee/leash/skip)
        // ====================================================================
        if in_combat.is_some() {
            // Priority 1: Should flee?
            if let Ok((flee_threshold, carrying)) = flee_data.get(entity) {
                let should_check_threat = (frame + idx) % CHECK_INTERVAL == 0;
                let effective_threshold = if should_check_threat {
                    let (enemies, allies) = count_nearby_factions(
                        positions, factions_buf, health_buf, idx, faction.0, THREAT_RADIUS
                    );
                    let ratio = (enemies as f32 + 1.0) / (allies as f32 + 1.0);
                    (flee_threshold.pct * ratio).min(1.0)
                } else {
                    flee_threshold.pct
                };

                if health.0 / 100.0 < effective_threshold {
                    let mut cmds = commands.entity(entity);
                    cmds.remove::<InCombat>().remove::<CombatOrigin>().remove::<Raiding>().insert(Returning);
                    if carrying.is_some() {
                        cmds.remove::<CarryingFood>().remove::<CarriedItem>();
                        let (r, g, b, a) = raider_faction_color(faction);
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetColor { idx, r, g, b, a }));
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetCarriedItem { idx, item_id: CarriedItem::NONE }));
                    }
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Fled combat".into());
                    continue;
                }
            }

            // Priority 2: Should leash?
            if let Ok((leash, origin)) = leash_data.get(entity) {
                if idx * 2 + 1 < positions.len() {
                    let dx = positions[idx * 2] - origin.x;
                    let dy = positions[idx * 2 + 1] - origin.y;
                    if (dx * dx + dy * dy).sqrt() > leash.distance {
                        commands.entity(entity)
                            .remove::<InCombat>().remove::<CombatOrigin>().remove::<Raiding>()
                            .insert(Returning);
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Leashed → Returning".into());
                        continue;
                    }
                }
            }

            // Priority 3: Still in combat, attack_system handles targeting
            continue;
        }

        // ====================================================================
        // Priority 4: Recovering + healed?
        // ====================================================================
        if let Some(rec) = recovering {
            if health.0 / 100.0 >= rec.threshold {
                commands.entity(entity).remove::<Recovering>();
                if resting.is_some() { commands.entity(entity).remove::<Resting>(); }
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Recovered".into());
            }
            continue;
        }

        // ====================================================================
        // Priority 5: Working + tired?
        // ====================================================================
        if working.is_some() {
            if energy.0 < ENERGY_TIRED_THRESHOLD {
                commands.entity(entity).remove::<Working>();
                if let Ok(assigned) = assigned_farms.get(entity) {
                    if assigned.0 < farm_occupancy.occupant_count.len() {
                        farm_occupancy.occupant_count[assigned.0] =
                            farm_occupancy.occupant_count[assigned.0].saturating_sub(1);
                    }
                    commands.entity(entity).remove::<AssignedFarm>();
                }
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Tired → Stopped".into());
            }
            continue;
        }

        // ====================================================================
        // Priority 6: OnDuty + time to patrol?
        // ====================================================================
        if let Some(duty) = on_duty {
            // Note: We need mutable access to increment ticks, but we have immutable here.
            // The tick increment happens via the mutable query below.
            if duty.ticks_waiting >= GUARD_PATROL_WAIT {
                if let Ok(mut patrol) = patrols.get_mut(entity) {
                    if !patrol.posts.is_empty() {
                        patrol.current = (patrol.current + 1) % patrol.posts.len();
                    }
                    commands.entity(entity).remove::<OnDuty>().insert(Patrolling);
                    if let Some(pos) = patrol.posts.get(patrol.current) {
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: pos.x, y: pos.y }));
                    }
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Patrolling".into());
                }
            }
            continue;
        }

        // ====================================================================
        // Priority 7: Resting + rested?
        // ====================================================================
        if resting.is_some() {
            if energy.0 >= ENERGY_WAKE_THRESHOLD {
                commands.entity(entity).remove::<Resting>();
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Woke up".into());
                // Fall through to make a decision
            } else {
                continue;
            }
        }

        // ====================================================================
        // Priority 8: Raiding (post-combat) → Re-target farm
        // ====================================================================
        if raiding.is_some() {
            let pos = if idx * 2 + 1 < positions.len() {
                Vector2::new(positions[idx * 2], positions[idx * 2 + 1])
            } else {
                home.0
            };
            if let Some(farm_pos) = find_nearest_location(pos, &world_data, LocationKind::Farm) {
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: farm_pos.x, y: farm_pos.y }));
            }
            continue;
        }

        // ====================================================================
        // Priority 9: Idle → Score Eat/Rest/Work/Wander
        // ====================================================================
        let en = energy.0;
        let (_fight_m, _flee_m, rest_m, eat_m, work_m, wander_m) = personality.get_multipliers();

        let town_idx = town_id.0 as usize;
        let food_available = town_idx < food_storage.food.len() && food_storage.food[town_idx] > 0;
        let mut scores: Vec<(Action, f32)> = Vec::with_capacity(4);

        if food_available {
            let eat_score = (100.0 - en) * SCORE_EAT_MULT * eat_m;
            if eat_score > 0.0 { scores.push((Action::Eat, eat_score)); }
        }

        let rest_score = (100.0 - en) * SCORE_REST_MULT * rest_m;
        if rest_score > 0.0 && home.is_valid() { scores.push((Action::Rest, rest_score)); }

        let can_work = match job {
            Job::Farmer => work_positions.get(entity).is_ok(),
            Job::Guard => patrols.get(entity).is_ok(),
            Job::Raider => true,
            Job::Fighter => false,
        };
        if can_work {
            let hp_pct = health.0 / 100.0;
            let hp_mult = if hp_pct < 0.5 { 0.0 } else { (hp_pct - 0.5) * 2.0 };
            let work_score = SCORE_WORK_BASE * work_m * hp_mult;
            if work_score > 0.0 { scores.push((Action::Work, work_score)); }
        }

        scores.push((Action::Wander, SCORE_WANDER_BASE * wander_m));

        let action = weighted_random(&scores, idx, frame);
        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
            format!("{:?} (e:{:.0} h:{:.0})", action, energy.0, health.0));

        match action {
            Action::Eat => {
                if town_idx < food_storage.food.len() && food_storage.food[town_idx] > 0 {
                    let old_energy = energy.0;
                    food_storage.food[town_idx] -= 1;
                    energy.0 = (energy.0 + ENERGY_FROM_EATING).min(100.0);
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                        format!("Ate (e:{:.0}→{:.0})", old_energy, energy.0));
                }
            }
            Action::Rest => {
                if home.is_valid() {
                    commands.entity(entity).insert(GoingToRest);
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                }
            }
            Action::Work => {
                match job {
                    Job::Farmer => {
                        if let Ok(wp) = work_positions.get(entity) {
                            commands.entity(entity).insert(GoingToWork);
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: wp.0.x, y: wp.0.y }));
                        }
                    }
                    Job::Guard => {
                        if let Ok(patrol) = patrols.get(entity) {
                            commands.entity(entity).insert(Patrolling);
                            if let Some(pos) = patrol.posts.get(patrol.current) {
                                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: pos.x, y: pos.y }));
                            }
                        }
                    }
                    Job::Raider => {
                        let pos = if idx * 2 + 1 < positions.len() {
                            Vector2::new(positions[idx * 2], positions[idx * 2 + 1])
                        } else {
                            home.0
                        };
                        if let Some(farm_pos) = find_nearest_location(pos, &world_data, LocationKind::Farm) {
                            commands.entity(entity).insert(Raiding);
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: farm_pos.x, y: farm_pos.y }));
                        }
                    }
                    Job::Fighter => {}
                }
            }
            Action::Wander => {
                if idx * 2 + 1 < positions.len() {
                    let x = positions[idx * 2];
                    let y = positions[idx * 2 + 1];
                    let offset_x = (pseudo_random(idx, frame + 1) - 0.5) * 200.0;
                    let offset_y = (pseudo_random(idx, frame + 2) - 0.5) * 200.0;
                    commands.entity(entity).insert(Wandering);
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: x + offset_x, y: y + offset_y }));
                }
            }
            Action::Fight | Action::Flee => {}
        }
    }
}

/// Increment OnDuty tick counters (runs every frame for guards at posts).
/// Separated from decision_system because we need mutable OnDuty access.
pub fn on_duty_tick_system(
    mut query: Query<&mut OnDuty, Without<InCombat>>,
) {
    for mut duty in query.iter_mut() {
        duty.ticks_waiting += 1;
    }
}
