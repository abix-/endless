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
//! Priority 4a: HealingAtFountain? → Wake when HP recovered
//! Priority 4b: Resting? → Wake when energy >= 90%
//! Priority 5: Working + tired? → Stop work
//! Priority 6: OnDuty + time_to_patrol? → Patrol
//! Priority 7: Idle → Score Eat/Rest/Work/Wander (wounded → fountain)

use bevy::prelude::*;

use crate::components::*;
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::constants::*;
use crate::resources::{FoodEvents, FoodDelivered, PopulationStats, GpuReadState, FoodStorage, GameTime, NpcLogCache, FarmStates, FarmGrowthState, RaidQueue, CombatLog, CombatEventKind, TownPolicies, WorkSchedule, OffDutyBehavior, SquadState, SystemTimings};
use crate::systems::economy::*;
use crate::world::{WorldData, LocationKind, find_nearest_location, find_nearest_free, find_location_within_radius, find_within_radius, BuildingOccupancy, find_by_pos, BuildingSpatialGrid, BuildingKind};

// ============================================================================
// SYSTEM PARAM BUNDLES - Logical groupings for scalability
// ============================================================================

use bevy::ecs::system::SystemParam;

/// Farm-related resources
#[derive(SystemParam)]
pub struct FarmParams<'w> {
    pub states: ResMut<'w, FarmStates>,
    pub occupancy: ResMut<'w, BuildingOccupancy>,
    pub world: Res<'w, WorldData>,
}

/// Economy resources (food, population tracking)
#[derive(SystemParam)]
pub struct EconomyParams<'w> {
    pub food_storage: ResMut<'w, FoodStorage>,
    pub gold_storage: ResMut<'w, crate::resources::GoldStorage>,
    pub mine_states: ResMut<'w, crate::resources::MineStates>,
    pub food_events: ResMut<'w, FoodEvents>,
    pub pop_stats: ResMut<'w, PopulationStats>,
}

/// Extra resources for decision_system (bundled to stay under 16 params)
#[derive(SystemParam)]
pub struct DecisionExtras<'w> {
    pub npc_logs: ResMut<'w, NpcLogCache>,
    pub raid_queue: ResMut<'w, RaidQueue>,
    pub combat_log: ResMut<'w, CombatLog>,
    pub policies: Res<'w, TownPolicies>,
    pub squad_state: Res<'w, SquadState>,
    pub timings: Res<'w, SystemTimings>,
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
    mut gold_storage: ResMut<crate::resources::GoldStorage>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("arrival");
    let positions = &gpu_state.positions;
    const DELIVERY_RADIUS: f32 = 150.0;     // Same as healing radius - deliver when near camp
    const MAX_DRIFT: f32 = 20.0;            // Keep farmers visually on the farm

    // ========================================================================
    // 1. Proximity-based delivery for Returning raiders
    // ========================================================================
    for (_entity, npc_idx, town, home, _faction, mut activity) in returning_query.iter_mut() {
        let (has_food, carried_gold) = match &*activity {
            Activity::Returning { has_food, gold } => (*has_food, *gold),
            _ => continue,
        };

        let idx = npc_idx.0;
        if idx * 2 + 1 >= positions.len() { continue; }

        let x = positions[idx * 2];
        let y = positions[idx * 2 + 1];
        let dx = x - home.0.x;
        let dy = y - home.0.y;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist <= DELIVERY_RADIUS {
            let town_idx = town.0 as usize;
            if has_food {
                if town_idx < food_storage.food.len() {
                    food_storage.food[town_idx] += 1;
                }
                food_events.delivered.push(FoodDelivered { camp_idx: town.0 as u32 });
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Delivered food");
            }
            if carried_gold > 0 {
                if town_idx < gold_storage.gold.len() {
                    gold_storage.gold[town_idx] += carried_gold;
                }
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                    format!("Delivered {} gold", carried_gold));
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
        if let Some(farm_idx) = find_by_pos(&world_data.farms, farm_pos) {
            if farm_states.harvest(farm_idx, Some(town.0 as usize), &mut food_storage, &mut food_events, &mut combat_log, &game_time) {
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Harvested (tending)");
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
/// 1-3. Combat (flee/leash/skip) — runs before transit skip so fighting NPCs can flee
/// -- Skip transit NPCs --
/// 4a. HealingAtFountain? → Wake when HP recovered
/// 4b. Resting? → Wake when energy >= 90%
/// 5. Working + tired? → Stop work
/// 6. OnDuty + time_to_patrol? → Patrol
/// 7. Idle → Score Eat/Rest/Work/Wander (wounded → fountain, tired → home)
pub fn decision_system(
    mut commands: Commands,
    // Main query: core NPC data (SquadId is Optional — only on squad-assigned guards)
    mut query: Query<
        (Entity, &NpcIndex, &Job, &mut Energy, &Health, &Home, &Personality, &TownId, &Faction,
         &mut Activity, &mut CombatState, Option<&AtDestination>, Option<&SquadId>),
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
    mut extras: DecisionExtras,
    npc_config: Res<crate::resources::NpcDecisionConfig>,
    bgrid: Res<BuildingSpatialGrid>,
) {
    let _t = extras.timings.scope("decision");
    let profiling = extras.timings.enabled;
    let npc_logs = &mut extras.npc_logs;
    let raid_queue = &mut extras.raid_queue;
    let combat_log = &mut extras.combat_log;
    let policies = &extras.policies;
    let squad_state = &extras.squad_state;
    let frame = DECISION_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let positions = &gpu_state.positions;
    let factions_buf = &gpu_state.factions;
    let health_buf = &gpu_state.health;

    const THREAT_RADIUS: f32 = 200.0;
    const CHECK_INTERVAL: usize = 30;
    const FARM_ARRIVAL_RADIUS: f32 = 20.0;
    const HEAL_DRIFT_RADIUS: f32 = 100.0; // Re-target fountain if pushed beyond this
    const COMBAT_INTERVAL: usize = 8; // Tier 2: combat flee/leash every 8 frames (~133ms)
    let think_buckets = ((npc_config.interval * 60.0) as usize).max(1); // Tier 3: slow decisions

    // Sub-profiling accumulators (zero cost when profiler disabled)
    let mut t_arrival = std::time::Duration::ZERO;
    let mut t_combat = std::time::Duration::ZERO;
    let mut t_idle = std::time::Duration::ZERO;
    let mut n_arrival: u32 = 0;
    let mut n_combat: u32 = 0;
    let mut n_idle: u32 = 0;

    for (entity, npc_idx, job, mut energy, health, home, personality, town_id, faction,
         mut activity, mut combat_state, at_destination, squad_id) in query.iter_mut()
    {
        let idx = npc_idx.0;

        // ====================================================================
        // Priority 0: AtDestination → Handle arrival transition
        // ====================================================================
        if at_destination.is_some() {
            let _ps = if profiling { Some(std::time::Instant::now()) } else { None };
            commands.entity(entity).remove::<AtDestination>();

            match &*activity {
                Activity::Patrolling => {
                    *activity = Activity::OnDuty { ticks_waiting: 0 };
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ OnDuty");
                }
                Activity::GoingToRest => {
                    *activity = Activity::Resting;
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Resting");
                }
                Activity::GoingToHeal => {
                    let town_idx = town_id.0 as usize;
                    let threshold = policies.policies.get(town_idx)
                        .map(|p| p.recovery_hp).unwrap_or(0.8);
                    *activity = Activity::HealingAtFountain { recover_until: threshold };
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Healing");
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

                        if let Some((farm_idx, farm_pos)) = find_within_radius(search_pos, &bgrid, BuildingKind::Farm, FARM_ARRIVAL_RADIUS, town_id.0 as u32) {
                            let occupied = farms.occupancy.is_occupied(farm_pos);

                            if occupied {
                                // Farm already has a farmer — find a free one in own town
                                if let Some(free_pos) = find_nearest_free(search_pos, &bgrid, BuildingKind::Farm, &farms.occupancy, Some(town_id.0 as u32)) {
                                    *activity = Activity::GoingToWork;
                                    commands.entity(entity).insert(WorkPosition(free_pos));
                                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: free_pos.x, y: free_pos.y }));
                                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Farm occupied, going to free farm");
                                } else {
                                    *activity = Activity::Idle;
                                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "All farms occupied → Idle");
                                }
                            } else {
                                farms.occupancy.claim(farm_pos);

                                *activity = Activity::Working;
                                commands.entity(entity).insert(AssignedFarm(farm_pos));
                                pop_inc_working(&mut economy.pop_stats, *job, town_id.0);
                                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: farm_pos.x, y: farm_pos.y }));

                                // Check if farm is ready to harvest
                                if farms.states.harvest(farm_idx, Some(town_id.0 as usize), &mut economy.food_storage, &mut economy.food_events, combat_log, &game_time) {
                                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Harvested → Working");
                                } else {
                                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Working (tending)");
                                }
                            }
                        } else {
                            *activity = Activity::Idle;
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "No farm nearby → Idle");
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
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Working");
                    }
                }
                Activity::Raiding { .. } => {
                    // Raider arrived at farm - check if ready to steal
                    if idx * 2 + 1 < positions.len() {
                        let pos = Vec2::new(positions[idx * 2], positions[idx * 2 + 1]);

                        let ready_farm = find_location_within_radius(pos, &bgrid, LocationKind::Farm, FARM_ARRIVAL_RADIUS)
                            .filter(|(farm_idx, _)| {
                                *farm_idx < farms.states.states.len()
                                    && farms.states.states[*farm_idx] == FarmGrowthState::Ready
                            });

                        if let Some((farm_idx, _)) = ready_farm {
                            farms.states.harvest(farm_idx, None, &mut economy.food_storage, &mut economy.food_events, combat_log, &game_time);

                            *activity = Activity::Returning { has_food: true, gold: 0 };
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Stole food → Returning");
                        } else {
                            // Farm not ready - find a different farm (exclude current one)
                            let other_farm = farms.world.farms.iter()
                                .filter(|f| f.position.x > -9000.0) // skip tombstoned
                                .filter(|f| f.position.distance(pos) > FARM_ARRIVAL_RADIUS)
                                .min_by(|a, b| {
                                    a.position.distance_squared(pos)
                                        .partial_cmp(&b.position.distance_squared(pos))
                                        .unwrap_or(std::cmp::Ordering::Equal)
                                });
                            if let Some(farm) = other_farm {
                                *activity = Activity::Raiding { target: farm.position };
                                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: farm.position.x, y: farm.position.y }));
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Farm not ready, seeking another");
                            } else {
                                *activity = Activity::Returning { has_food: false, gold: 0 };
                                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "No other farms, returning");
                            }
                        }
                    }
                }
                Activity::Mining { mine_pos } => {
                    let mine_pos = *mine_pos;
                    // Arrived at gold mine — start mining if gold available
                    if let Some(mine_idx) = economy.mine_states.positions.iter().position(|p| {
                        (*p - mine_pos).length() < 30.0
                    }) {
                        if mine_idx < economy.mine_states.gold.len() && economy.mine_states.gold[mine_idx] > 0.0 {
                            farms.occupancy.claim(mine_pos);
                            *activity = Activity::MiningAtMine;
                            commands.entity(entity).insert(WorkPosition(mine_pos));
                            pop_inc_working(&mut economy.pop_stats, *job, town_id.0);
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: mine_pos.x, y: mine_pos.y }));
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ MiningAtMine");
                        } else {
                            // Mine depleted
                            *activity = Activity::Idle;
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Mine depleted → Idle");
                        }
                    } else {
                        *activity = Activity::Idle;
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "No mine nearby → Idle");
                    }
                }
                Activity::Wandering => {
                    *activity = Activity::Idle;
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Idle");
                }
                Activity::Returning { .. } => {
                    // Arrived home (proximity delivery handled by arrival_system, this is backup)
                    *activity = Activity::Idle;
                }
                _ => {}
            }

            if let Some(s) = _ps { t_arrival += s.elapsed(); n_arrival += 1; }
            continue;
        }

        // ====================================================================
        // Priority 1-3: Combat decisions (flee/leash/skip)
        // Runs BEFORE transit skip so fighting NPCs in transit (e.g. Raiding)
        // can still flee or leash back. Tier 2: every COMBAT_INTERVAL frames.
        // ====================================================================
        if combat_state.is_fighting() {
            let combat_tick = (idx + frame) % COMBAT_INTERVAL == 0;
            if combat_tick {
            let _ps = if profiling { Some(std::time::Instant::now()) } else { None };
            // Priority 1: Should flee? (policy-driven)
            let town_idx_usize = town_id.0 as usize;
            let flee_pct = match job {
                Job::Raider => 0.50, // raiders always flee at 50%
                Job::Archer => {
                    let p = policies.policies.get(town_idx_usize);
                    if p.is_some_and(|p| p.archer_aggressive) {
                        0.0 // aggressive guards never flee
                    } else {
                        p.map(|p| p.archer_flee_hp).unwrap_or(0.15)
                    }
                }
                Job::Farmer | Job::Miner => {
                    let p = policies.policies.get(town_idx_usize);
                    if p.is_some_and(|p| p.farmer_fight_back) {
                        0.0 // fight-back farmers/miners don't flee
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
                    *activity = Activity::Returning { has_food: false, gold: 0 };
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Fled combat");
                    if let Some(s) = _ps { t_combat += s.elapsed(); n_combat += 1; }
                    continue;
                }
            }

            // Priority 2: Should leash? (per-entity LeashRange or policy archer_leash)
            let should_leash = match job {
                Job::Archer => policies.policies.get(town_idx_usize).is_none_or(|p| p.archer_leash),
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
                            *activity = Activity::Returning { has_food: false, gold: 0 };
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Leashed → Returning");
                            if let Some(s) = _ps { t_combat += s.elapsed(); n_combat += 1; }
                            continue;
                        }
                    }
                }
            }

            // Priority 3: Still in combat, attack_system handles targeting
            if let Some(s) = _ps { t_combat += s.elapsed(); n_combat += 1; }
            } // end combat_tick
            continue;
        }

        // ====================================================================
        // Squad sync: apply squad target/patrol policy changes immediately
        // (before transit skip) so archers react by next decision tick.
        // ====================================================================
        if *job == Job::Archer {
            if let Some(sid) = squad_id {
                if let Some(squad) = squad_state.squads.get(sid.0 as usize) {
                    if let Some(target) = squad.target {
                        // Squad target always overrides patrol route.
                        if !matches!(*activity, Activity::Patrolling) {
                            *activity = Activity::Patrolling;
                        }
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: target.x, y: target.y }));
                    } else if !squad.patrol_enabled {
                        // Patrol disabled and no squad target: stop moving now.
                        if matches!(*activity, Activity::Patrolling | Activity::OnDuty { .. }) {
                            *activity = Activity::Idle;
                            if idx * 2 + 1 < positions.len() {
                                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                                    idx,
                                    x: positions[idx * 2],
                                    y: positions[idx * 2 + 1],
                                }));
                            }
                        }
                    }
                }
            }
        }

        // ====================================================================
        // Skip NPCs in transit states (they're walking to their destination)
        // GoingToHeal proximity check runs at combat cadence (every 8 frames).
        // ====================================================================
        if activity.is_transit() {
            // Early arrival: GoingToHeal NPCs stop once inside healing range
            if (idx + frame) % COMBAT_INTERVAL == 0 && matches!(*activity, Activity::GoingToHeal) {
                let town_idx = town_id.0 as usize;
                if let Some(town) = farms.world.towns.get(town_idx) {
                    if idx * 2 + 1 < positions.len() {
                        let current = Vec2::new(positions[idx * 2], positions[idx * 2 + 1]);
                        if current.distance(town.center) <= HEAL_DRIFT_RADIUS {
                            let threshold = policies.policies.get(town_idx)
                                .map(|p| p.recovery_hp).unwrap_or(0.8);
                            *activity = Activity::HealingAtFountain { recover_until: threshold };
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Healing");
                        }
                    }
                }
            }
            continue;
        }

        // ====================================================================
        // Tier 3 gate: non-combat decisions only run on this NPC's bucket
        // ====================================================================
        if (idx + frame) % think_buckets != 0 {
            continue;
        }

        // ====================================================================
        // Priority 4a: HealingAtFountain? → Wake when HP recovered
        // ====================================================================
        if let Activity::HealingAtFountain { recover_until } = &*activity {
            if health.0 / 100.0 >= *recover_until {
                *activity = Activity::Idle;
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Recovered");
                // Fall through to make a decision
            } else {
                // Drift check: separation physics pushes NPCs out of healing range
                let town_idx = town_id.0 as usize;
                if let Some(town) = farms.world.towns.get(town_idx) {
                    if idx * 2 + 1 < positions.len() {
                        let current = Vec2::new(positions[idx * 2], positions[idx * 2 + 1]);
                        if current.distance(town.center) > HEAL_DRIFT_RADIUS {
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                                idx, x: town.center.x, y: town.center.y
                            }));
                        }
                    }
                }
                continue; // still healing
            }
        }

        // ====================================================================
        // Priority 4b: Resting? → Wake when energy recovered
        // ====================================================================
        if matches!(*activity, Activity::Resting) {
            if energy.0 >= ENERGY_WAKE_THRESHOLD {
                *activity = Activity::Idle;
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Woke up");
                // Fall through to make a decision
            } else {
                continue; // still resting
            }
        }

        // ====================================================================
        // Priority 5: Working/Mining + tired?
        // ====================================================================
        if matches!(*activity, Activity::Working) {
            if energy.0 < ENERGY_TIRED_THRESHOLD {
                *activity = Activity::Idle;
                if let Ok(assigned) = assigned_query.get(entity) {
                    farms.occupancy.release(assigned.0);
                    commands.entity(entity).remove::<AssignedFarm>();
                }
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Tired → Stopped");
            }
            continue;
        }

        if matches!(*activity, Activity::MiningAtMine) {
            if energy.0 < ENERGY_TIRED_THRESHOLD {
                // Extract gold before leaving
                let gold_amount = crate::constants::MINE_EXTRACT_PER_CYCLE;
                if let Ok(wp) = work_query.get(entity) {
                    let mine_pos = wp.0;
                    // Deduct from mine
                    if let Some(mine_idx) = economy.mine_states.positions.iter().position(|p| {
                        (*p - mine_pos).length() < 30.0
                    }) {
                        if mine_idx < economy.mine_states.gold.len() {
                            let extracted = (economy.mine_states.gold[mine_idx] as i32).min(gold_amount);
                            economy.mine_states.gold[mine_idx] = (economy.mine_states.gold[mine_idx] - extracted as f32).max(0.0);
                            farms.occupancy.release(mine_pos);
                            commands.entity(entity).remove::<WorkPosition>();
                            *activity = Activity::Returning { has_food: false, gold: extracted };
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                                format!("Mined {} gold → Returning", extracted));
                            continue;
                        }
                    }
                    // Couldn't find mine — just leave
                    farms.occupancy.release(mine_pos);
                    commands.entity(entity).remove::<WorkPosition>();
                }
                *activity = Activity::Idle;
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Mining tired → Idle");
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
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Tired → Left post");
                // Fall through to idle scoring — Rest will win
            } else {
                let squad_patrol_enabled = squad_id
                    .and_then(|sid| squad_state.squads.get(sid.0 as usize))
                    .is_none_or(|s| s.patrol_enabled);
                if ticks >= ARCHER_PATROL_WAIT && squad_patrol_enabled {
                    if let Ok(mut patrol) = patrol_query.get_mut(entity) {
                        if !patrol.posts.is_empty() {
                            patrol.current = (patrol.current + 1) % patrol.posts.len();
                        }
                        *activity = Activity::Patrolling;
                        if let Some(pos) = patrol.posts.get(patrol.current) {
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: pos.x, y: pos.y }));
                        }
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Patrolling");
                    }
                }
                continue;
            }
        }


        // ====================================================================
        // Priority 8: Idle → Score Eat/Rest/Work/Wander (policy-aware)
        // ====================================================================
        let _ps_idle = if profiling { Some(std::time::Instant::now()) } else { None };
        let en = energy.0;
        let (_fight_m, _flee_m, rest_m, eat_m, work_m, wander_m) = personality.get_multipliers();

        let town_idx = town_id.0 as usize;
        let policy = policies.policies.get(town_idx);
        let food_available = policy.is_none_or(|p| p.eat_food)
            && town_idx < economy.food_storage.food.len()
            && economy.food_storage.food[town_idx] > 0;
        let mut scores: [(Action, f32); 5] = [(Action::Wander, 0.0); 5];
        let mut score_count: usize = 0;

        // Prioritize healing: wounded NPCs go to fountain before doing anything else
        // Skip if starving — HP capped at 50% until energy recovers
        if let Some(p) = policy {
            if p.prioritize_healing && energy.0 > 0.0 && health.0 / 100.0 < p.recovery_hp {
                if let Some(town) = farms.world.towns.get(town_idx) {
                    let center = town.center;
                    *activity = Activity::GoingToHeal;
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: center.x, y: center.y }));
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Wounded → Fountain");
                    if let Some(s) = _ps_idle { t_idle += s.elapsed(); n_idle += 1; }
                    continue;
                }
            }
        }

        if food_available && en < ENERGY_EAT_THRESHOLD {
            let eat_score = (ENERGY_EAT_THRESHOLD - en) * SCORE_EAT_MULT * eat_m;
            scores[score_count] = (Action::Eat, eat_score); score_count += 1;
        }

        if en < ENERGY_HUNGRY && home.is_valid() {
            let rest_score = (ENERGY_HUNGRY - en) * SCORE_REST_MULT * rest_m;
            scores[score_count] = (Action::Rest, rest_score); score_count += 1;
        }

        // Work schedule gate: per-job schedule
        let schedule = match job {
            Job::Farmer | Job::Miner => policy.map(|p| p.farmer_schedule).unwrap_or(WorkSchedule::Both),
            Job::Archer => policy.map(|p| p.archer_schedule).unwrap_or(WorkSchedule::Both),
            _ => WorkSchedule::Both,
        };
        let work_allowed = match schedule {
            WorkSchedule::Both => true,
            WorkSchedule::DayOnly => game_time.is_daytime(),
            WorkSchedule::NightOnly => !game_time.is_daytime(),
        };

        let can_work = work_allowed && match job {
            Job::Farmer => work_query.get(entity).is_ok(),
            Job::Miner => true,  // miners always have work (find nearest mine dynamically)
            Job::Archer => patrol_query.get(entity).is_ok(),
            Job::Raider => true,
            Job::Fighter => false,
        };
        if can_work {
            let hp_pct = health.0 / 100.0;
            let hp_mult = if hp_pct < 0.3 { 0.0 } else { (hp_pct - 0.3) * (1.0 / 0.7) };
            let work_score = SCORE_WORK_BASE * work_m * hp_mult;
            if work_score > 0.0 { scores[score_count] = (Action::Work, work_score); score_count += 1; }
        }

        // Off-duty behavior when work is gated out by schedule
        if !work_allowed {
            let off_duty = match job {
                Job::Farmer | Job::Miner => policy.map(|p| p.farmer_off_duty).unwrap_or(OffDutyBehavior::GoToBed),
                Job::Archer => policy.map(|p| p.archer_off_duty).unwrap_or(OffDutyBehavior::GoToBed),
                _ => OffDutyBehavior::GoToBed,
            };
            match off_duty {
                OffDutyBehavior::GoToBed => {
                    // Boost rest score so NPCs prefer going to bed
                    if home.is_valid() { scores[score_count] = (Action::Rest, 80.0 * rest_m); score_count += 1; }
                }
                OffDutyBehavior::StayAtFountain => {
                    // Go to town center (fountain)
                    if let Some(town) = farms.world.towns.get(town_idx) {
                        let center = town.center;
                        *activity = Activity::Wandering;
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: center.x, y: center.y }));
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Off-duty → Fountain");
                        if let Some(s) = _ps_idle { t_idle += s.elapsed(); n_idle += 1; }
                        continue;
                    }
                }
                OffDutyBehavior::WanderTown => {
                    scores[score_count] = (Action::Wander, 80.0 * wander_m); score_count += 1;
                }
            }
        }

        scores[score_count] = (Action::Wander, SCORE_WANDER_BASE * wander_m); score_count += 1;

        let action = weighted_random(&scores[..score_count], idx, frame);
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
                    Job::Miner => {
                        // Find nearest mine with gold > 0
                        let current_pos = if idx * 2 + 1 < positions.len() {
                            Vec2::new(positions[idx * 2], positions[idx * 2 + 1])
                        } else {
                            home.0
                        };
                        let mut best_mine: Option<(f32, Vec2)> = None;
                        for (mi, mpos) in economy.mine_states.positions.iter().enumerate() {
                            if mi < economy.mine_states.gold.len() && economy.mine_states.gold[mi] > 0.0 {
                                let dist = current_pos.distance(*mpos);
                                if best_mine.is_none() || dist < best_mine.unwrap().0 {
                                    best_mine = Some((dist, *mpos));
                                }
                            }
                        }
                        if let Some((_, mine_pos)) = best_mine {
                            *activity = Activity::Mining { mine_pos };
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: mine_pos.x, y: mine_pos.y }));
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Mining gold");
                        }
                        // No mines available — stay idle
                    }
                    Job::Archer => {
                        // Squad override: go to squad target instead of patrolling
                        if let Some(sid) = squad_id {
                            if let Some(squad) = squad_state.squads.get(sid.0 as usize) {
                                if let Some(target) = squad.target {
                                    *activity = Activity::Patrolling;
                                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: target.x, y: target.y }));
                                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                                        format!("Squad {} → target", sid.0 + 1));
                                    if let Some(s) = _ps_idle { t_idle += s.elapsed(); n_idle += 1; }
                                    continue;
                                }
                                if !squad.patrol_enabled {
                                    if let Some(s) = _ps_idle { t_idle += s.elapsed(); n_idle += 1; }
                                    continue;
                                }
                            }
                            // No target set — fall through to normal patrol
                        }
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

                            if let Some(farm_pos) = find_nearest_location(pos, &bgrid, LocationKind::Farm) {
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
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "No farms to raid");
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
                // Wander near home to prevent unbounded drift off the map
                let (base_x, base_y) = if home.is_valid() {
                    (home.0.x, home.0.y)
                } else if idx * 2 + 1 < positions.len() {
                    (positions[idx * 2], positions[idx * 2 + 1])
                } else {
                    if let Some(s) = _ps_idle { t_idle += s.elapsed(); n_idle += 1; }
                    continue;
                };
                let offset_x = (pseudo_random(idx, frame + 1) - 0.5) * 200.0;
                let offset_y = (pseudo_random(idx, frame + 2) - 0.5) * 200.0;
                *activity = Activity::Wandering;
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: base_x + offset_x, y: base_y + offset_y }));
            }
            Action::Fight | Action::Flee => {}
        }
        if let Some(s) = _ps_idle { t_idle += s.elapsed(); n_idle += 1; }
    }

    // Record sub-profiling results
    if profiling {
        let t = &extras.timings;
        t.record("decision/arrival", t_arrival.as_secs_f32() * 1000.0);
        t.record("decision/combat", t_combat.as_secs_f32() * 1000.0);
        t.record("decision/idle", t_idle.as_secs_f32() * 1000.0);
        t.record("decision/n_arrival", n_arrival as f32);
        t.record("decision/n_combat", n_combat as f32);
        t.record("decision/n_idle", n_idle as f32);
    }
}

/// Increment OnDuty tick counters (runs every frame for guards at posts).
/// Separated from decision_system because we need mutable Activity access.
pub fn on_duty_tick_system(
    mut query: Query<(&mut Activity, &CombatState), Without<Dead>>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("on_duty_tick");
    for (mut activity, combat) in query.iter_mut() {
        if combat.is_fighting() { continue; }
        if let Activity::OnDuty { ticks_waiting } = &mut *activity {
            *ticks_waiting += 1;
        }
    }
}

/// Rebuild all guards' patrol routes when WorldData changes (guard post added/removed/reordered).
pub fn rebuild_patrol_routes_system(
    world_data: Res<WorldData>,
    mut guards: Query<(&mut PatrolRoute, &TownId, &Job), Without<Dead>>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("rebuild_patrol_routes");
    if !world_data.is_changed() { return; }
    for (mut route, town_id, job) in guards.iter_mut() {
        if *job != Job::Archer { continue; }
        let new_posts = crate::systems::spawn::build_patrol_route(&world_data, town_id.0 as u32);
        if new_posts.is_empty() { continue; }
        // Clamp current index to new route length
        route.current = if route.current < new_posts.len() { route.current } else { 0 };
        route.posts = new_posts;
    }
}
