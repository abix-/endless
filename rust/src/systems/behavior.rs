//! Behavior systems - State transitions based on energy and arrivals

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
        Entity, &NpcIndex, &Job, &TownId, &Home, &Health, &Faction,
        Option<&Patrolling>, Option<&GoingToRest>, Option<&GoingToWork>,
        Option<&Raiding>, Option<&Returning>, Option<&CarryingFood>,
        Option<&WoundedThreshold>, Option<&Wandering>,
    ), Without<Recovering>>,
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
    const FARM_ARRIVAL_RADIUS: f32 = 40.0;  // Grid spacing is 34px, keep tight to avoid false positives
    const DELIVERY_RADIUS: f32 = 150.0;     // Same as healing radius - deliver when near camp
    const MAX_DRIFT: f32 = 50.0;            // Re-target farmer if drifted this far from farm

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
                // Get farmer's current position for farm finding
                let pos = if idx * 2 + 1 < positions.len() {
                    Vector2::new(positions[idx * 2], positions[idx * 2 + 1])
                } else {
                    continue;
                };

                // Farmers: find farm, reserve it, set AssignedFarm
                if *job == Job::Farmer {
                    if let Some((farm_idx, farm_pos)) = find_location_within_radius(pos, &world_data, LocationKind::Farm, FARM_ARRIVAL_RADIUS) {
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
                        // No farm found within radius - just go to Working without farm assignment
                        commands.entity(entity)
                            .remove::<GoingToWork>()
                            .insert(Working);
                        pop_inc_working(&mut pop_stats, *job, town.0);
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: pos.x, y: pos.y }));
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Working (no farm)".into());
                    }
                } else {
                    // Non-farmers just transition to Working at current position
                    commands.entity(entity)
                        .remove::<GoingToWork>()
                        .insert(Working);
                    pop_inc_working(&mut pop_stats, *job, town.0);
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: pos.x, y: pos.y }));
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

/// Frame counter for staggered threat checks (avoid O(n²) every frame).
static FLEE_FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Flee combat when HP drops below dynamic threshold.
/// Threshold adjusts based on local enemy/ally ratio:
/// - Outnumbered 2:1 = flee at 100% HP (immediate)
/// - Even odds = flee at base threshold
/// - Winning 2:1 = flee at half base threshold
///
/// Performance: Threat check only runs every 30 frames (~0.5s) per NPC,
/// staggered by NPC index to spread load across frames.
pub fn flee_system(
    mut commands: Commands,
    query: Query<(Entity, &NpcIndex, &Health, &FleeThreshold, &Home, &Faction, Option<&CarryingFood>), With<InCombat>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    gpu_state: Res<GpuReadState>,
) {
    let positions = &gpu_state.positions;
    let factions = &gpu_state.factions;
    let health_buf = &gpu_state.health;
    const THREAT_RADIUS: f32 = 200.0;
    const CHECK_INTERVAL: u32 = 30; // Only check threat every 30 frames per NPC

    let frame = FLEE_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    for (entity, npc_idx, health, flee, home, faction, carrying) in query.iter() {
        let idx = npc_idx.0;

        // Stagger threat checks: each NPC checks on different frames
        let should_check_threat = (frame + idx as u32) % CHECK_INTERVAL == 0;

        let effective_threshold = if should_check_threat {
            // Count nearby enemies and allies (expensive, only do periodically)
            let (enemies, allies) = count_nearby_factions(
                positions, factions, health_buf, idx, faction.0, THREAT_RADIUS
            );

            // Calculate dynamic threshold: base * (enemies / max(allies, 1))
            let ratio = (enemies as f32 + 1.0) / (allies as f32 + 1.0);
            (flee.pct * ratio).min(1.0)
        } else {
            // Use base threshold on non-check frames
            flee.pct
        };

        let health_pct = health.0 / 100.0;
        if health_pct < effective_threshold {
            let mut cmds = commands.entity(entity);
            cmds.remove::<InCombat>();
            cmds.remove::<CombatOrigin>();
            cmds.remove::<Raiding>();
            cmds.insert(Returning);

            if carrying.is_some() {
                cmds.remove::<CarryingFood>();
                cmds.remove::<CarriedItem>();
                let (r, g, b, a) = raider_faction_color(faction);
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetColor {
                    idx: npc_idx.0, r, g, b, a,
                }));
                // Hide carried item (dropped when fleeing)
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetCarriedItem {
                    idx: npc_idx.0,
                    item_id: CarriedItem::NONE,
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

/// Recovery system: NPCs with Recovering resume activity when healed.
pub fn recovery_system(
    mut commands: Commands,
    query: Query<(Entity, &Health, &Recovering, Option<&Resting>)>,
) {
    for (entity, health, recovering, resting) in query.iter() {
        let health_pct = health.0 / 100.0;
        if health_pct >= recovering.threshold {
            let mut cmds = commands.entity(entity);
            cmds.remove::<Recovering>();
            if resting.is_some() {
                cmds.remove::<Resting>();
            }
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
         Without<Resting>, Without<GoingToRest>, Without<Returning>, Without<Wandering>,
         Without<InCombat>, Without<Recovering>, Without<Dead>)
    >,
    _pop_stats: ResMut<PopulationStats>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    world_data: Res<WorldData>,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    mut npc_logs: ResMut<NpcLogCache>,
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
            // Reduce work score when HP is low (below 50% = no work, 50-100% = scaled)
            let hp_pct = _health.0 / 100.0;
            let hp_mult = if hp_pct < 0.5 { 0.0 } else { (hp_pct - 0.5) * 2.0 };
            let work_score = SCORE_WORK_BASE * work_m * hp_mult;
            if work_score > 0.0 {
                scores.push((Action::Work, work_score));
            }
        }

        let wander_score = SCORE_WANDER_BASE * wander_m;
        scores.push((Action::Wander, wander_score));

        let action = weighted_random(&scores, idx, frame);

        // Log decision
        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
            format!("{:?} (e:{:.0} h:{:.0})", action, energy.0, _health.0));

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
                    commands.entity(entity).insert(Wandering);
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                        idx, x: x + offset_x, y: y + offset_y,
                    }));
                }
            }
            Action::Fight | Action::Flee => {}
        }
    }
}
