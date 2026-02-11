//! Behavior systems - Unified decision-making and state transitions
//!
//! Key systems:
//! - `arrival_system`: Minimal - marks NPCs as AtDestination, handles proximity delivery
//! - `on_duty_tick_system`: Increments guard wait counters
//! - `decision_system`: Central priority-based decision making for ALL NPCs
//!
//! The decision system is the NPC's "brain" - all decisions flow through it:
//! Priority 0: AtDestination? → Handle arrival transition
//! Priority 1-3: Combat (flee/leash/skip)
//! Priority 4: Resting? → Wake when HP recovered (if wounded) AND energy >= 90%
//! Priority 5: Working + tired? → Stop work
//! Priority 6: OnDuty + time_to_patrol? → Patrol
//! Priority 7: Idle → Score Eat/Rest/Work/Wander

use bevy::prelude::*;

use crate::components::*;
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::constants::*;
use crate::resources::{FoodEvents, FoodDelivered, FoodConsumed, PopulationStats, GpuReadState, FoodStorage, GameTime, NpcLogCache, FarmStates, FarmGrowthState, RaidQueue, CombatLog, CombatEventKind, TownPolicies, WorkSchedule, OffDutyBehavior};
use crate::systems::economy::*;
use crate::world::{WorldData, LocationKind, find_nearest_location, find_location_within_radius, FarmOccupancy, find_farm_index_by_pos, pos_to_key};

// ============================================================================
// SYSTEM PARAM BUNDLES - Logical groupings for scalability
// ============================================================================

use bevy::ecs::system::SystemParam;

/// Farm-related resources
#[derive(SystemParam)]
pub struct FarmParams<'w> {
    pub states: ResMut<'w, FarmStates>,
    pub occupancy: ResMut<'w, FarmOccupancy>,
    pub world: Res<'w, WorldData>,
}

/// Economy resources (food, population tracking)
#[derive(SystemParam)]
pub struct EconomyParams<'w> {
    pub food_storage: ResMut<'w, FoodStorage>,
    pub food_events: ResMut<'w, FoodEvents>,
    pub pop_stats: ResMut<'w, PopulationStats>,
}

/// Arrival system: proximity checks for returning raiders and working farmers.
///
/// Responsibilities:
/// 1. Proximity-based delivery for Returning raiders
/// 2. Working farmer drift check + harvest (continuous, not event-based)
///
/// Arrival detection (transit → AtDestination) is handled by gpu_position_readback.
/// All state transitions are handled by decision_system.
pub fn arrival_system(
    // Query for Returning NPCs (proximity-based delivery) — Without<AssignedFarm> for disjointness
    mut returning_query: Query<(Entity, &NpcIndex, &TownId, &Home, &Faction, &mut Activity), (Without<Dead>, Without<AssignedFarm>)>,
    // Query for Working farmers with AssignedFarm (for drift check + harvest)
    working_farmers: Query<(&NpcIndex, &AssignedFarm, &TownId, &Activity), Without<Dead>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut food_storage: ResMut<FoodStorage>,
    mut food_events: ResMut<FoodEvents>,
    world_data: Res<WorldData>,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    mut npc_logs: ResMut<NpcLogCache>,
    mut farm_states: ResMut<FarmStates>,
    mut frame_counter: Local<u32>,
    mut combat_log: ResMut<CombatLog>,
) {
    let positions = &gpu_state.positions;
    const DELIVERY_RADIUS: f32 = 150.0;     // Same as healing radius - deliver when near camp
    const MAX_DRIFT: f32 = 20.0;            // Keep farmers visually on the farm

    // ========================================================================
    // 1. Proximity-based delivery for Returning raiders
    // ========================================================================
    for (_entity, npc_idx, town, home, _faction, mut activity) in returning_query.iter_mut() {
        let has_food = matches!(*activity, Activity::Returning { has_food: true });
        if !matches!(*activity, Activity::Returning { .. }) { continue; }

        let idx = npc_idx.0;
        if idx * 2 + 1 >= positions.len() { continue; }

        let x = positions[idx * 2];
        let y = positions[idx * 2 + 1];
        let dx = x - home.0.x;
        let dy = y - home.0.y;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist <= DELIVERY_RADIUS {
            if has_food {
                let camp_idx = town.0 as usize;
                if camp_idx < food_storage.food.len() {
                    food_storage.food[camp_idx] += 1;
                }
                food_events.delivered.push(FoodDelivered { camp_idx: town.0 as u32 });
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Delivered food".into());
            }
            *activity = Activity::Idle;
        }
    }

    // ========================================================================
    // 2. Working farmer drift check + harvest (throttled: each farmer once per 30 frames)
    // ========================================================================
    *frame_counter = frame_counter.wrapping_add(1);
    let frame_slot = *frame_counter % 30;

    for (npc_idx, assigned, town, activity) in working_farmers.iter() {
        if !matches!(activity, Activity::Working) { continue; }
        if (npc_idx.0 as u32) % 30 != frame_slot { continue; }

        let farm_pos = assigned.0;
        let idx = npc_idx.0;
        if idx * 2 + 1 >= positions.len() { continue; }
        let current = Vec2::new(positions[idx * 2], positions[idx * 2 + 1]);

        // If drifted too far, re-target to farm
        if current.distance(farm_pos) > MAX_DRIFT {
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                idx, x: farm_pos.x, y: farm_pos.y
            }));
        }

        // Harvest check: if farm became Ready while working, harvest it
        if let Some(farm_idx) = find_farm_index_by_pos(&world_data.farms, farm_pos) {
            if farm_idx < farm_states.states.len()
                && farm_states.states[farm_idx] == FarmGrowthState::Ready
            {
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
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Harvested (tending)".into());
                combat_log.push(CombatEventKind::Harvest, game_time.day(), game_time.hour(), game_time.minute(),
                    format!("Farm #{} harvested", farm_idx));
            }
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
/// Uses xorshift-style mixing so both seed and frame affect the result.
fn pseudo_random(seed: usize, frame: usize) -> f32 {
    let mut h = seed ^ frame.wrapping_mul(2654435761);
    h = h.wrapping_mul(1103515245).wrapping_add(12345);
    h ^= h >> 16;
    (h & 0x7fff) as f32 / 32767.0
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
/// This is the NPC's "brain" - all decisions and state transitions flow through here.
///
/// Priority order (first match wins):
/// 0. AtDestination → Handle arrival transition
/// 1-3. Combat (flee/leash/skip)
/// 4. Resting? → Wake when HP recovered (if wounded) AND energy >= 90%
/// 5. Working + tired? → Stop work
/// 6. OnDuty + time_to_patrol? → Patrol
/// 7. Idle → Score Eat/Rest/Work/Wander (utility AI)
pub fn decision_system(
    mut commands: Commands,
    // Main query: core NPC data
    mut query: Query<
        (Entity, &NpcIndex, &Job, &mut Energy, &Health, &Home, &Personality, &TownId, &Faction,
         &mut Activity, &mut CombatState, Option<&AtDestination>),
        Without<Dead>
    >,
    // Combat config queries
    leash_query: Query<&LeashRange>,
    // Work-related queries
    work_query: Query<&WorkPosition>,
    assigned_query: Query<&AssignedFarm>,
    mut patrol_query: Query<&mut PatrolRoute>,
    // Resources
    mut farms: FarmParams,
    mut economy: EconomyParams,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    mut npc_logs: ResMut<NpcLogCache>,
    mut raid_queue: ResMut<RaidQueue>,
    mut combat_log: ResMut<CombatLog>,
    policies: Res<TownPolicies>,
) {
    let frame = DECISION_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let positions = &gpu_state.positions;
    let factions_buf = &gpu_state.factions;
    let health_buf = &gpu_state.health;

    const THREAT_RADIUS: f32 = 200.0;
    const CHECK_INTERVAL: usize = 30;
    const FARM_ARRIVAL_RADIUS: f32 = 20.0;

    for (entity, npc_idx, job, mut energy, health, home, personality, town_id, faction,
         mut activity, mut combat_state, at_destination) in query.iter_mut()
    {
        let idx = npc_idx.0;

        // ====================================================================
        // Priority 0: AtDestination → Handle arrival transition
        // ====================================================================
        if at_destination.is_some() {
            commands.entity(entity).remove::<AtDestination>();

            match &*activity {
                Activity::Patrolling => {
                    *activity = Activity::OnDuty { ticks_waiting: 0 };
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ OnDuty".into());
                }
                Activity::GoingToRest => {
                    *activity = Activity::Resting { recover_until: None };
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Resting".into());
                }
                Activity::GoingToWork => {
                    // Farmers: find farm at WorkPosition and start working
                    if *job == Job::Farmer {
                        let search_pos = work_query.get(entity)
                            .map(|wp| wp.0)
                            .unwrap_or_else(|_| {
                                if idx * 2 + 1 < positions.len() {
                                    Vec2::new(positions[idx * 2], positions[idx * 2 + 1])
                                } else {
                                    Vec2::new(0.0, 0.0)
                                }
                            });

                        if let Some((farm_idx, farm_pos)) = find_location_within_radius(search_pos, &farms.world, LocationKind::Farm, FARM_ARRIVAL_RADIUS) {
                            let farm_key = pos_to_key(farm_pos);
                            *farms.occupancy.occupants.entry(farm_key).or_insert(0) += 1;

                            *activity = Activity::Working;
                            commands.entity(entity).insert(AssignedFarm(farm_pos));
                            pop_inc_working(&mut economy.pop_stats, *job, town_id.0);
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: farm_pos.x, y: farm_pos.y }));

                            // Check if farm is ready to harvest
                            if farm_idx < farms.states.states.len()
                                && farms.states.states[farm_idx] == FarmGrowthState::Ready
                            {
                                let town_idx = town_id.0 as usize;
                                if town_idx < economy.food_storage.food.len() {
                                    economy.food_storage.food[town_idx] += 1;
                                    economy.food_events.consumed.push(FoodConsumed {
                                        location_idx: farm_idx as u32,
                                        is_camp: false,
                                    });
                                }
                                farms.states.states[farm_idx] = FarmGrowthState::Growing;
                                farms.states.progress[farm_idx] = 0.0;
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Harvested → Working".into());
                                combat_log.push(CombatEventKind::Harvest, game_time.day(), game_time.hour(), game_time.minute(),
                                    format!("Farm #{} harvested", farm_idx));
                            } else {
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Working (tending)".into());
                            }
                        } else {
                            *activity = Activity::Working;
                            pop_inc_working(&mut economy.pop_stats, *job, town_id.0);
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: search_pos.x, y: search_pos.y }));
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Working (no farm)".into());
                        }
                    } else {
                        let current_pos = if idx * 2 + 1 < positions.len() {
                            Vec2::new(positions[idx * 2], positions[idx * 2 + 1])
                        } else {
                            Vec2::new(0.0, 0.0)
                        };
                        *activity = Activity::Working;
                        pop_inc_working(&mut economy.pop_stats, *job, town_id.0);
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: current_pos.x, y: current_pos.y }));
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Working".into());
                    }
                }
                Activity::Raiding { .. } => {
                    // Raider arrived at farm - check if ready to steal
                    if idx * 2 + 1 < positions.len() {
                        let pos = Vec2::new(positions[idx * 2], positions[idx * 2 + 1]);

                        let ready_farm = find_location_within_radius(pos, &farms.world, LocationKind::Farm, FARM_ARRIVAL_RADIUS)
                            .filter(|(farm_idx, _)| {
                                *farm_idx < farms.states.states.len()
                                    && farms.states.states[*farm_idx] == FarmGrowthState::Ready
                            });

                        if let Some((farm_idx, _)) = ready_farm {
                            farms.states.states[farm_idx] = FarmGrowthState::Growing;
                            farms.states.progress[farm_idx] = 0.0;

                            *activity = Activity::Returning { has_food: true };
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Stole food → Returning".into());
                        } else {
                            // Farm not ready - find another
                            if let Some(farm_pos) = find_nearest_location(pos, &farms.world, LocationKind::Farm) {
                                *activity = Activity::Raiding { target: farm_pos };
                                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: farm_pos.x, y: farm_pos.y }));
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Farm not ready, seeking another".into());
                            } else {
                                *activity = Activity::Returning { has_food: false };
                                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "No farms, returning home".into());
                            }
                        }
                    }
                }
                Activity::Wandering => {
                    *activity = Activity::Idle;
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Idle".into());
                }
                Activity::Returning { .. } => {
                    // Arrived home (proximity delivery handled by arrival_system, this is backup)
                    *activity = Activity::Idle;
                }
                _ => {}
            }

            // Check wounded threshold on arrival (policy-driven)
            // Skip if starving — HP is capped at 50% until energy recovers, so
            // fountain healing can't reach recovery_hp. Let them rest for energy first.
            let town_idx = town_id.0 as usize;
            if energy.0 > 0.0 {
                if let Some(policy) = policies.policies.get(town_idx) {
                    let max_hp = 100.0; // TODO: use CachedStats.max_health when available in query
                    let health_pct = health.0 / max_hp;
                    if health_pct < policy.recovery_hp {
                        if matches!(*activity, Activity::Resting { .. }) {
                            // Already resting at destination — just set recovery threshold
                            *activity = Activity::Resting { recover_until: Some(policy.recovery_hp) };
                        } else if policy.prioritize_healing {
                            if let Some(town) = farms.world.towns.get(town_idx) {
                                let center = town.center;
                                *activity = Activity::GoingToRest;
                                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: center.x, y: center.y }));
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Wounded → Fountain".into());
                            } else {
                                *activity = Activity::Resting { recover_until: Some(policy.recovery_hp) };
                            }
                        } else {
                            *activity = Activity::Resting { recover_until: Some(policy.recovery_hp) };
                        }
                    }
                }
            }

            continue;
        }

        // ====================================================================
        // Skip NPCs in transit states (they're walking to their destination)
        // ====================================================================
        if activity.is_transit() {
            continue;
        }

        // ====================================================================
        // Priority 1-3: Combat decisions (flee/leash/skip)
        // ====================================================================
        if combat_state.is_fighting() {
            // Priority 1: Should flee? (policy-driven)
            let town_idx_usize = town_id.0 as usize;
            let flee_pct = match job {
                Job::Raider => 0.50, // raiders always flee at 50%
                Job::Guard => {
                    let p = policies.policies.get(town_idx_usize);
                    if p.is_some_and(|p| p.guard_aggressive) {
                        0.0 // aggressive guards never flee
                    } else {
                        p.map(|p| p.guard_flee_hp).unwrap_or(0.15)
                    }
                }
                Job::Farmer => {
                    let p = policies.policies.get(town_idx_usize);
                    if p.is_some_and(|p| p.farmer_fight_back) {
                        0.0 // fight-back farmers don't flee
                    } else {
                        p.map(|p| p.farmer_flee_hp).unwrap_or(0.30)
                    }
                }
                Job::Fighter => 0.0,
            };
            if flee_pct > 0.0 {
                let should_check_threat = (frame + idx) % CHECK_INTERVAL == 0;
                let effective_threshold = if should_check_threat {
                    let (enemies, allies) = count_nearby_factions(
                        positions, factions_buf, health_buf, idx, faction.0, THREAT_RADIUS
                    );
                    let ratio = (enemies as f32 + 1.0) / (allies as f32 + 1.0);
                    (flee_pct * ratio).min(1.0)
                } else {
                    flee_pct
                };

                if health.0 / 100.0 < effective_threshold {
                    *combat_state = CombatState::None;
                    *activity = Activity::Returning { has_food: false };
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Fled combat".into());
                    continue;
                }
            }

            // Priority 2: Should leash? (per-entity LeashRange or policy guard_leash)
            let should_leash = match job {
                Job::Guard => policies.policies.get(town_idx_usize).is_none_or(|p| p.guard_leash),
                _ => leash_query.get(entity).is_ok(),
            };
            if should_leash {
                let leash_dist = leash_query.get(entity).map(|l| l.distance).unwrap_or(400.0);
                if let CombatState::Fighting { origin } = &*combat_state {
                    if idx * 2 + 1 < positions.len() {
                        let dx = positions[idx * 2] - origin.x;
                        let dy = positions[idx * 2 + 1] - origin.y;
                        if (dx * dx + dy * dy).sqrt() > leash_dist {
                            *combat_state = CombatState::None;
                            *activity = Activity::Returning { has_food: false };
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Leashed → Returning".into());
                            continue;
                        }
                    }
                }
            }

            // Priority 3: Still in combat, attack_system handles targeting
            continue;
        }

        // ====================================================================
        // Priority 4: Resting? (energy rest + wounded recovery unified)
        // ====================================================================
        if let Activity::Resting { recover_until } = &*activity {
            // Wounded recovery: wait for HP threshold before waking
            if let Some(threshold) = recover_until {
                if health.0 / 100.0 < *threshold {
                    continue; // still recovering HP
                }
            }
            // Normal wake: energy must reach threshold
            if energy.0 >= ENERGY_WAKE_THRESHOLD {
                let msg = if recover_until.is_some() { "Recovered" } else { "Woke up" };
                *activity = Activity::Idle;
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), msg.into());
                // Fall through to make a decision
            } else {
                continue;
            }
        }

        // ====================================================================
        // Priority 5: Working + tired?
        // ====================================================================
        if matches!(*activity, Activity::Working) {
            if energy.0 < ENERGY_TIRED_THRESHOLD {
                *activity = Activity::Idle;
                if let Ok(assigned) = assigned_query.get(entity) {
                    let farm_key = pos_to_key(assigned.0);
                    if let Some(count) = farms.occupancy.occupants.get_mut(&farm_key) {
                        *count = count.saturating_sub(1);
                    }
                    commands.entity(entity).remove::<AssignedFarm>();
                }
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Tired → Stopped".into());
            }
            continue;
        }

        // ====================================================================
        // Priority 6: OnDuty (tired → leave post, else patrol when ready)
        // ====================================================================
        if let Activity::OnDuty { ticks_waiting } = &*activity {
            let ticks = *ticks_waiting;
            if energy.0 < ENERGY_TIRED_THRESHOLD {
                *activity = Activity::Idle;
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Tired → Left post".into());
                // Fall through to idle scoring — Rest will win
            } else {
                if ticks >= GUARD_PATROL_WAIT {
                    if let Ok(mut patrol) = patrol_query.get_mut(entity) {
                        if !patrol.posts.is_empty() {
                            patrol.current = (patrol.current + 1) % patrol.posts.len();
                        }
                        *activity = Activity::Patrolling;
                        if let Some(pos) = patrol.posts.get(patrol.current) {
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: pos.x, y: pos.y }));
                        }
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Patrolling".into());
                    }
                }
                continue;
            }
        }


        // ====================================================================
        // Priority 8: Idle → Score Eat/Rest/Work/Wander (policy-aware)
        // ====================================================================
        let en = energy.0;
        let (_fight_m, _flee_m, rest_m, eat_m, work_m, wander_m) = personality.get_multipliers();

        let town_idx = town_id.0 as usize;
        let policy = policies.policies.get(town_idx);
        let food_available = policy.is_none_or(|p| p.eat_food)
            && town_idx < economy.food_storage.food.len()
            && economy.food_storage.food[town_idx] > 0;
        let mut scores: Vec<(Action, f32)> = Vec::with_capacity(4);

        // Prioritize healing: wounded NPCs go to fountain before doing anything else
        // Skip if starving — HP capped at 50% until energy recovers
        if let Some(p) = policy {
            if p.prioritize_healing && energy.0 > 0.0 && health.0 / 100.0 < p.recovery_hp && *job != Job::Raider {
                if let Some(town) = farms.world.towns.get(town_idx) {
                    let center = town.center;
                    *activity = Activity::GoingToRest;
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: center.x, y: center.y }));
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Wounded → Fountain".into());
                    continue;
                }
            }
        }

        if food_available && en < ENERGY_EAT_THRESHOLD {
            let eat_score = (ENERGY_EAT_THRESHOLD - en) * SCORE_EAT_MULT * eat_m;
            scores.push((Action::Eat, eat_score));
        }

        if en < ENERGY_HUNGRY && home.is_valid() {
            let rest_score = (ENERGY_HUNGRY - en) * SCORE_REST_MULT * rest_m;
            scores.push((Action::Rest, rest_score));
        }

        // Work schedule gate: check if current time allows work
        let work_allowed = match policy.map(|p| p.work_schedule).unwrap_or(WorkSchedule::Both) {
            WorkSchedule::Both => true,
            WorkSchedule::DayOnly => game_time.is_daytime(),
            WorkSchedule::NightOnly => !game_time.is_daytime(),
        };

        let can_work = work_allowed && match job {
            Job::Farmer => work_query.get(entity).is_ok(),
            Job::Guard => patrol_query.get(entity).is_ok(),
            Job::Raider => true,
            Job::Fighter => false,
        };
        if can_work {
            let hp_pct = health.0 / 100.0;
            let hp_mult = if hp_pct < 0.5 { 0.0 } else { (hp_pct - 0.5) * 2.0 };
            let work_score = SCORE_WORK_BASE * work_m * hp_mult;
            if work_score > 0.0 { scores.push((Action::Work, work_score)); }
        }

        // Off-duty behavior when work is gated out by schedule
        if !work_allowed {
            let off_duty = match job {
                Job::Farmer => policy.map(|p| p.farmer_off_duty).unwrap_or(OffDutyBehavior::GoToBed),
                Job::Guard => policy.map(|p| p.guard_off_duty).unwrap_or(OffDutyBehavior::GoToBed),
                _ => OffDutyBehavior::GoToBed,
            };
            match off_duty {
                OffDutyBehavior::GoToBed => {
                    // Boost rest score so NPCs prefer going to bed
                    if home.is_valid() { scores.push((Action::Rest, 80.0 * rest_m)); }
                }
                OffDutyBehavior::StayAtFountain => {
                    // Go to town center (fountain)
                    if let Some(town) = farms.world.towns.get(town_idx) {
                        let center = town.center;
                        *activity = Activity::Wandering;
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: center.x, y: center.y }));
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Off-duty → Fountain".into());
                        continue;
                    }
                }
                OffDutyBehavior::WanderTown => {
                    scores.push((Action::Wander, 80.0 * wander_m));
                }
            }
        }

        scores.push((Action::Wander, SCORE_WANDER_BASE * wander_m));

        let action = weighted_random(&scores, idx, frame);
        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
            format!("{:?} (e:{:.0} h:{:.0})", action, energy.0, health.0));

        match action {
            Action::Eat => {
                if town_idx < economy.food_storage.food.len() && economy.food_storage.food[town_idx] > 0 {
                    let old_energy = energy.0;
                    economy.food_storage.food[town_idx] -= 1;
                    energy.0 = 100.0;
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                        format!("Ate (e:{:.0}→{:.0})", old_energy, energy.0));
                }
            }
            Action::Rest => {
                if home.is_valid() {
                    *activity = Activity::GoingToRest;
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                }
            }
            Action::Work => {
                match job {
                    Job::Farmer => {
                        if let Ok(wp) = work_query.get(entity) {
                            *activity = Activity::GoingToWork;
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: wp.0.x, y: wp.0.y }));
                        }
                    }
                    Job::Guard => {
                        if let Ok(patrol) = patrol_query.get(entity) {
                            *activity = Activity::Patrolling;
                            if let Some(pos) = patrol.posts.get(patrol.current) {
                                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: pos.x, y: pos.y }));
                            }
                        }
                    }
                    Job::Raider => {
                        // Add self to raid queue (only if not already in it)
                        let queue = raid_queue.waiting.entry(faction.0).or_default();
                        let already_in_queue = queue.iter().any(|(e, _)| *e == entity);
                        if !already_in_queue {
                            queue.push((entity, idx));
                        }

                        // Check if enough raiders waiting to form a group
                        if queue.len() >= RAID_GROUP_SIZE as usize {
                            let pos = if idx * 2 + 1 < positions.len() {
                                Vec2::new(positions[idx * 2], positions[idx * 2 + 1])
                            } else {
                                home.0
                            };

                            if let Some(farm_pos) = find_nearest_location(pos, &farms.world, LocationKind::Farm) {
                                let group_size = queue.len();
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                                    format!("Raid group of {} dispatched!", group_size));
                                combat_log.push(CombatEventKind::Raid, game_time.day(), game_time.hour(), game_time.minute(),
                                    format!("{} Raiders dispatched to farm", group_size));
                                for (raider_entity, raider_idx) in queue.drain(..) {
                                    // Can't mutate other entities' Activity here — use commands
                                    commands.entity(raider_entity).insert(Activity::Raiding { target: farm_pos });
                                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                                        idx: raider_idx, x: farm_pos.x, y: farm_pos.y
                                    }));
                                }
                            } else {
                                queue.clear();
                                let offset_x = (pseudo_random(idx, frame + 1) - 0.5) * 100.0;
                                let offset_y = (pseudo_random(idx, frame + 2) - 0.5) * 100.0;
                                *activity = Activity::Wandering;
                                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                                    idx, x: home.0.x + offset_x, y: home.0.y + offset_y
                                }));
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "No farms to raid".into());
                            }
                        } else if !already_in_queue {
                            // Just joined queue, not enough raiders yet - wander near camp
                            let offset_x = (pseudo_random(idx, frame + 1) - 0.5) * 100.0;
                            let offset_y = (pseudo_random(idx, frame + 2) - 0.5) * 100.0;
                            *activity = Activity::Wandering;
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                                idx, x: home.0.x + offset_x, y: home.0.y + offset_y
                            }));
                        }
                        // else: already queued, waiting — stay idle, natural Wander handles movement
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
                    *activity = Activity::Wandering;
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: x + offset_x, y: y + offset_y }));
                }
            }
            Action::Fight | Action::Flee => {}
        }
    }
}

/// Increment OnDuty tick counters (runs every frame for guards at posts).
/// Separated from decision_system because we need mutable Activity access.
pub fn on_duty_tick_system(
    mut query: Query<(&mut Activity, &CombatState), Without<Dead>>,
) {
    for (mut activity, combat) in query.iter_mut() {
        if combat.is_fighting() { continue; }
        if let Activity::OnDuty { ticks_waiting } = &mut *activity {
            *ticks_waiting += 1;
        }
    }
}
