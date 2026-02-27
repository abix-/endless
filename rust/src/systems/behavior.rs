//! Behavior systems - Unified decision-making and state transitions
//!
//! Key systems:
//! - `arrival_system`: Minimal - marks NPCs as AtDestination, handles proximity delivery
//! - `on_duty_tick_system`: Increments guard wait counters
//! - `decision_system`: Central priority-based decision making for ALL NPCs
//!
//! The decision system is the NPC's "brain" - all decisions flow through it:
//! Priority 0: AtDestination? -> Handle arrival transition
//! Priority 1-3: Combat (flee/leash/skip)
//! Priority 4a: HealingAtFountain? -> Wake when HP recovered
//! Priority 4b: Resting? -> Wake when energy >= 90%
//! Priority 5: Working + tired? -> Stop work
//! Priority 6: OnDuty + time_to_patrol? -> Patrol
//! Priority 7: Idle -> Score Eat/Rest/Work/Wander (wounded -> fountain)

use bevy::prelude::*;

use crate::components::*;
use crate::constants::*;
use crate::messages::CombatLogMsg;
use crate::resources::{GpuReadState, GameTime, NpcLogCache, SelectedNpc, CombatEventKind, TownPolicies, WorkSchedule, OffDutyBehavior, SquadState, SystemTimings, EntityMap, MovementIntents, MovementPriority};
use crate::settings::UserSettings;
use crate::systemparams::EconomyState;
use crate::systems::economy::*;
use crate::systems::stats::UPGRADES;
use crate::constants::UpgradeStatKind;
use crate::world::{WorldData, LocationKind, find_location_within_radius, find_within_radius, BuildingKind};

// ============================================================================
// SYSTEM PARAM BUNDLES - Logical groupings for scalability
// ============================================================================

use bevy::ecs::system::SystemParam;

/// Farm-related resources
#[derive(SystemParam)]
pub struct FarmParams<'w> {
    pub world: Res<'w, WorldData>,
}

/// NPC gameplay data queries (bundled to stay under 16 params)
#[derive(SystemParam)]
pub struct NpcDataQueries<'w, 's> {
    pub home_q: Query<'w, 's, &'static Home>,
    pub personality_q: Query<'w, 's, &'static Personality>,
    pub leash_range_q: Query<'w, 's, &'static LeashRange>,
    pub work_state_q: Query<'w, 's, &'static mut NpcWorkState>,
    pub patrol_route_q: Query<'w, 's, &'static mut PatrolRoute>,
    pub carried_gold_q: Query<'w, 's, &'static mut CarriedGold>,
    pub stealer_q: Query<'w, 's, &'static Stealer>,
    pub has_energy_q: Query<'w, 's, &'static HasEnergy>,
    pub weapon_q: Query<'w, 's, &'static EquippedWeapon>,
    pub helmet_q: Query<'w, 's, &'static EquippedHelmet>,
    pub armor_q: Query<'w, 's, &'static EquippedArmor>,
}

/// NPC combat/state queries for decision_system (bundled to stay under 16 params)
#[derive(SystemParam)]
pub struct DecisionNpcState<'w, 's> {
    pub npc_flags_q: Query<'w, 's, &'static mut NpcFlags>,
    pub squad_id_q: Query<'w, 's, &'static SquadId>,
    pub manual_target_q: Query<'w, 's, &'static ManualTarget>,
    pub energy_q: Query<'w, 's, &'static mut Energy>,
    pub combat_state_q: Query<'w, 's, &'static mut CombatState>,
    pub health_q: Query<'w, 's, &'static Health, Without<Building>>,
    pub cached_stats_q: Query<'w, 's, &'static CachedStats>,
    pub activity_q: Query<'w, 's, &'static mut Activity>,
}

/// Extra resources for decision_system (bundled to stay under 16 params)
#[derive(SystemParam)]
pub struct DecisionExtras<'w> {
    pub npc_logs: ResMut<'w, NpcLogCache>,
    pub combat_log: MessageWriter<'w, CombatLogMsg>,
    pub policies: Res<'w, TownPolicies>,
    pub squad_state: Res<'w, SquadState>,
    pub timings: Res<'w, SystemTimings>,
    pub town_upgrades: Res<'w, crate::systems::stats::TownUpgrades>,
    pub selected_npc: Res<'w, SelectedNpc>,
    pub settings: Res<'w, UserSettings>,
}

/// Arrival system: proximity checks for returning NPCs and working farmers.
///
/// Responsibilities:
/// 1. Proximity-based delivery for all Returning NPCs (raiders, farmers, miners)
/// 2. Working farmer drift check + harvest → carry home (continuous, not event-based)
///
/// Arrival detection (transit -> AtDestination) is handled by gpu_position_readback.
/// All state transitions are handled by decision_system.
pub fn arrival_system(
    mut intents: ResMut<MovementIntents>,
    mut economy: EconomyState,
    world_data: Res<WorldData>,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    mut npc_logs: ResMut<NpcLogCache>,
    mut entity_map: ResMut<EntityMap>,
    mut frame_counter: Local<u32>,
    mut combat_log: MessageWriter<CombatLogMsg>,
    timings: Res<SystemTimings>,
    mut npc_q: Query<(Entity, &EntitySlot, &Job, &TownId, &mut Activity, &Home, &mut NpcWorkState), (Without<Building>, Without<Dead>)>,
) {
    let _t = timings.scope("arrival");
    let positions = &gpu_state.positions;
    const DELIVERY_RADIUS: f32 = 50.0;
    const MAX_DRIFT: f32 = 20.0;

    // ========================================================================
    // 1. Proximity-based delivery for all Returning NPCs
    // ========================================================================
    // Collect (slot, entity, loot, home, town_idx) for returning NPCs near home.
    let mut deliveries: Vec<(usize, Entity, Vec<(ItemKind, i32)>, usize)> = Vec::new();
    for (entity, slot, _job, town_id, activity, home, _work_state) in npc_q.iter() {
        let loot = match &*activity {
            Activity::Returning { loot } => loot,
            _ => continue,
        };
        let idx = slot.0;
        if idx * 2 + 1 >= positions.len() { continue; }
        let x = positions[idx * 2];
        let y = positions[idx * 2 + 1];
        let dx = x - home.0.x;
        let dy = y - home.0.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist <= DELIVERY_RADIUS {
            deliveries.push((idx, entity, loot.clone(), town_id.0 as usize));
        }
    }

    for (idx, entity, loot, town_idx) in deliveries {
        for &(item, amount) in &loot {
            if amount <= 0 { continue; }
            match item {
                ItemKind::Food => {
                    if town_idx < economy.food_storage.food.len() {
                        economy.food_storage.food[town_idx] += amount;
                    }
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                        format!("Delivered {} food", amount));
                }
                ItemKind::Gold => {
                    if town_idx < economy.gold_storage.gold.len() {
                        economy.gold_storage.gold[town_idx] += amount;
                    }
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                        format!("Delivered {} gold", amount));
                }
            }
        }
        if let Ok((_, _, _, _, mut act, _, _)) = npc_q.get_mut(entity) {
            *act = Activity::Idle;
        }
    }

    // ========================================================================
    // 2. Working farmer drift check + harvest → carry home (throttled)
    // ========================================================================
    *frame_counter = frame_counter.wrapping_add(1);
    let frame_slot = *frame_counter % 30;

    let farmer_slots: Vec<(Entity, usize, usize)> = npc_q.iter()
        .filter(|(_, slot, _job, _, activity, _, ws)| {
            matches!(&**activity, Activity::Working)
                && ws.occupied_slot.is_some()
                && (slot.0 as u32) % 30 == frame_slot
        })
        .map(|(entity, slot, _, _, _, _, ws)| (entity, slot.0, ws.occupied_slot.unwrap()))
        .collect();

    for (entity, slot, farm_slot) in farmer_slots {
        let Some(farm_pos) = entity_map.get_instance(farm_slot).map(|i| i.position) else { continue };
        let idx = slot;
        if idx * 2 + 1 >= positions.len() { continue; }
        let current = Vec2::new(positions[idx * 2], positions[idx * 2 + 1]);

        if current.distance(farm_pos) > MAX_DRIFT {
            submit_intent(&mut intents, entity, farm_pos.x, farm_pos.y, MovementPriority::JobRoute, "arrival:farm_drift");
        }

        // Harvest check
        let harvest_result = entity_map.get_instance_mut(farm_slot).and_then(|inst| {
            let food = inst.harvest();
            if food > 0 { Some((food, inst.harvest_log_msg(food))) } else { None }
        });
        if let Some((food, log_msg)) = harvest_result {
            // Read NPC data from query
            let Ok((_, _, job, town_id, _, home, _)) = npc_q.get(entity) else { continue };
            let fac = world_data.towns.get(town_id.0 as usize).map(|t| t.faction).unwrap_or(0);
            let job_val = *job;
            let town_idx = town_id.0;
            let home_pos = home.0;
            combat_log.write(CombatLogMsg { kind: CombatEventKind::Harvest, faction: fac, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: log_msg, location: None });
            entity_map.release(farm_slot);
            pop_dec_working(&mut economy.pop_stats, job_val, town_idx);
            if let Ok((_, _, _, _, mut act, _, mut ws)) = npc_q.get_mut(entity) {
                ws.occupied_slot = None;
                *act = Activity::Returning { loot: vec![(ItemKind::Food, food)] };
            }
            submit_intent(&mut intents, entity, home_pos.x, home_pos.y, MovementPriority::JobRoute, "arrival:harvest_return");
            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Harvested -> Carrying home");
        }
    }
}

// ============================================================================
// FLEE / LEASH / RECOVERY SYSTEMS (generic)
// ============================================================================

/// Unpack GPU threat counts: packed u32 -> (enemies, allies).
/// GPU computes these via spatial grid query each frame (see npc_compute.wgsl).
#[inline]
fn unpack_threat_counts(packed: u32) -> (u32, u32) {
    ((packed >> 16), packed & 0xFFFF)
}

/// Find a farm for a farmer using local expanding-radius search.
/// Preference order:
/// 1) Ready farms
/// 2) Higher growth progress
/// 3) Closer distance
/// Returns (farm_slot, farm_position, chosen_radius).
fn find_farmer_farm_target(
    from: Vec2,
    entity_map: &EntityMap,
    town_idx: u32,
) -> Option<(usize, Vec2, f32)> {
    let mut radius = 400.0_f32;
    let max_radius = 6400.0_f32;
    while radius <= max_radius {
        let mut best: Option<(usize, Vec2, bool, f32, f32)> = None; // (slot, pos, ready, growth, d2)
        entity_map.for_each_nearby(from, radius, |inst| {
            if inst.kind != BuildingKind::Farm || inst.town_idx != town_idx { return; }
            if inst.occupants >= 1 { return; }
            let d = inst.position - from;
            let d2 = d.length_squared();
            let ready = inst.growth_ready;
            let growth = inst.growth_progress;

            let better = match best {
                None => true,
                Some((_, _, b_ready, b_growth, b_d2)) => {
                    if ready != b_ready { ready && !b_ready }
                    else if (growth - b_growth).abs() > f32::EPSILON { growth > b_growth }
                    else { d2 < b_d2 }
                }
            };
            if better {
                best = Some((inst.slot, inst.position, ready, growth, d2));
            }
        });

        if let Some((slot, pos, _, _, _)) = best {
            return Some((slot, pos, radius));
        }
        radius *= 2.0;
    }
    None
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

/// Submit a movement intent through the centralized resolver.
#[inline]
fn submit_intent(
    intents: &mut MovementIntents,
    entity: Entity,
    x: f32,
    y: f32,
    priority: MovementPriority,
    source: &'static str,
) {
    intents.submit(entity, Vec2::new(x, y), priority, source);
}

/// Frame counter for pseudo-random seeding.
static DECISION_FRAME: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Unified decision system: ALL NPC decisions in one place with priority cascade.
/// This is the NPC's "brain" - all decisions and state transitions flow through here.
///
/// Priority order (first match wins):
/// 0. AtDestination -> Handle arrival transition
/// 1-3. Combat (flee/leash/skip) — runs before transit skip so fighting NPCs can flee
/// -- Skip transit NPCs --
/// 4a. HealingAtFountain? -> Wake when HP recovered
/// 4b. Resting? -> Wake when energy >= 90%
/// 5. Working + tired? -> Stop work
/// 6. OnDuty + time_to_patrol? -> Patrol
/// 7. Idle -> Score Eat/Rest/Work/Wander (wounded -> fountain, tired -> home)
pub fn decision_system(
    farms: FarmParams,
    mut economy: EconomyState,
    mut intents: ResMut<MovementIntents>,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    mut extras: DecisionExtras,
    npc_config: Res<crate::resources::NpcDecisionConfig>,
    mut entity_map: ResMut<EntityMap>,
    mut npc_state: DecisionNpcState,
    mut npc_data: NpcDataQueries,
    decision_npc_q: Query<(Entity, &EntitySlot, &Job, &TownId, &Faction), (Without<Building>, Without<Dead>)>,
) {
    let _t = extras.timings.scope("decision");
    let profiling = extras.timings.enabled;

    // Sync NPC log filter state from settings + selected NPC
    extras.npc_logs.mode = extras.settings.npc_log_mode;
    extras.npc_logs.update_selected(extras.selected_npc.0);

    let npc_logs = &mut extras.npc_logs;
    let combat_log = &mut extras.combat_log;
    let policies = &extras.policies;
    let squad_state = &extras.squad_state;
    let frame = DECISION_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let positions = &gpu_state.positions;

    const CHECK_INTERVAL: usize = 30;
    const FARM_ARRIVAL_RADIUS: f32 = 20.0;
    const HEAL_DRIFT_RADIUS: f32 = 100.0; // Re-target fountain if pushed beyond this
    const COMBAT_INTERVAL: usize = 8; // Tier 2: combat flee/leash every 8 frames (~133ms)
    let think_buckets = ((npc_config.interval * 60.0) as usize).max(1); // Tier 3: slow decisions

    // Sub-profiling accumulators (zero cost when profiler disabled)
    let mut t_arrival = std::time::Duration::ZERO;
    let mut t_combat = std::time::Duration::ZERO;
    let mut t_idle = std::time::Duration::ZERO;
    let mut t_squad = std::time::Duration::ZERO;
    let mut t_work = std::time::Duration::ZERO;
    let mut n_arrival: u32 = 0;
    let mut n_combat: u32 = 0;
    let mut n_idle: u32 = 0;
    let mut n_squad: u32 = 0;
    let mut n_work: u32 = 0;
    let mut n_transit_skip: u32 = 0;
    let mut n_total: u32 = 0;

    for (entity, slot, job, town_id, faction) in decision_npc_q.iter() {
        let idx = slot.0;
        let job = *job;
        let town_idx_i32 = town_id.0;
        let faction_i32 = faction.0;
        let mut energy = npc_state.energy_q.get(entity).map(|e| e.0).unwrap_or(100.0);
        let health = npc_state.health_q.get(entity).map(|h| h.0).unwrap_or(100.0);
        let home = npc_data.home_q.get(entity).map(|h| h.0).unwrap_or(Vec2::ZERO);
        let personality = npc_data.personality_q.get(entity).cloned().unwrap_or_default();
        let mut activity = npc_state.activity_q.get(entity).map(|a| a.clone()).unwrap_or_default();
        let mut combat_state = npc_state.combat_state_q.get(entity).map(|cs| cs.clone()).unwrap_or_default();
        let mut at_destination = npc_state.npc_flags_q.get(entity).map(|f| f.at_destination).unwrap_or(false);
        let squad_id = npc_state.squad_id_q.get(entity).ok().map(|s| s.0);
        let manual_target = npc_state.manual_target_q.get(entity).ok().cloned();
        let direct_control = npc_state.npc_flags_q.get(entity).map(|f| f.direct_control).unwrap_or(false);
        let max_hp = npc_state.cached_stats_q.get(entity).map(|s| s.max_health).unwrap_or(100.0);
        let leash_range_val = npc_data.leash_range_q.get(entity).ok().map(|lr| lr.0);
        let work_state = npc_data.work_state_q.get(entity).ok().copied().unwrap_or_default();
        let mut work_position = work_state.work_target;
        let mut assigned_farm = work_state.occupied_slot;
        let has_patrol = npc_data.patrol_route_q.get(entity).is_ok();
        let mut patrol_current = npc_data.patrol_route_q.get(entity).ok().map(|r| r.current).unwrap_or(0);

        // Capture originals for conditional writeback
        let orig_activity = std::mem::discriminant(&activity);
        let orig_energy = energy;
        let orig_combat_state = std::mem::discriminant(&combat_state);
        let orig_at_destination = at_destination;
        let orig_work_target = work_position;
        let orig_occupied_slot = assigned_farm;
        let orig_patrol_current = patrol_current;

        npc_logs.set_slot_faction(idx, faction_i32);
        let max_hp = if max_hp > 0.0 { max_hp } else { 100.0 };

        'decide: {
        // ====================================================================
        // DirectControl: absolute skip — no autonomous behavior whatsoever.
        // ====================================================================
        if direct_control {
            if at_destination {
                at_destination = false;
            }
            break 'decide;
        }
        n_total += 1;

        // ====================================================================
        // Priority 0: AtDestination -> Handle arrival transition
        // ====================================================================
        if at_destination {
            let _ps = if profiling { Some(std::time::Instant::now()) } else { None };
            at_destination = false;

            match &activity {
                Activity::Patrolling => {
                    // Squad rest: tired squad members go home instead of entering OnDuty
                    if let Some(sid) = squad_id {
                        if let Some(squad) = squad_state.squads.get(sid as usize) {
                            if squad.rest_when_tired && energy < ENERGY_TIRED_THRESHOLD && home != Vec2::ZERO {
                                activity = Activity::GoingToRest;
                                submit_intent(&mut intents, entity, home.x, home.y, MovementPriority::Survival, "arrival:squad_rest");
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Tired -> Rest (squad)");
                                if let Some(s) = _ps { t_arrival += s.elapsed(); n_arrival += 1; }
                                break 'decide;
                            }
                        }
                    }
                    activity = Activity::OnDuty { ticks_waiting: 0 };
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "-> OnDuty");
                }
                Activity::GoingToRest => {
                    activity = Activity::Resting;
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "-> Resting");
                }
                Activity::GoingToHeal => {
                    let town_idx = town_idx_i32 as usize;
                    let threshold = policies.policies.get(town_idx)
                        .map(|p| p.recovery_hp).unwrap_or(0.8);
                    activity = Activity::HealingAtFountain { recover_until: threshold };
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "-> Healing");
                }
                Activity::GoingToWork => {
                    // Farmers: find farm at work_target and start working
                    if job == Job::Farmer {
                        let reserved_slot = work_position.or(assigned_farm);
                        let search_pos = reserved_slot
                            .and_then(|wp| entity_map.get_instance(wp).map(|i| i.position))
                            .unwrap_or_else(|| {
                                if idx * 2 + 1 < positions.len() {
                                    Vec2::new(positions[idx * 2], positions[idx * 2 + 1])
                                } else {
                                    Vec2::new(0.0, 0.0)
                                }
                            });

                        let target_farm = reserved_slot
                            .and_then(|slot| entity_map.get_instance(slot)
                                .filter(|inst| inst.kind == BuildingKind::Farm && inst.town_idx == town_idx_i32 as u32)
                                .map(|inst| (slot, inst.position)))
                            .or_else(|| find_within_radius(search_pos, &entity_map, BuildingKind::Farm, FARM_ARRIVAL_RADIUS, town_idx_i32 as u32));

                        if let Some((farm_slot, farm_pos)) = target_farm {
                            let occ = entity_map.occupant_count(farm_slot);
                            let owns = assigned_farm == Some(farm_slot);
                            let occupied_by_other = if owns { occ > 1 } else { occ >= 1 };

                            if occupied_by_other {
                                // Farm already has a farmer — find a free one in own town
                                if let Some((free_slot, free_pos, radius)) =
                                    find_farmer_farm_target(search_pos, &entity_map, town_idx_i32 as u32)
                                {
                                    if let Some(old) = assigned_farm.take() {
                                        entity_map.release(old);
                                    }
                                    entity_map.claim(free_slot);
                                    activity = Activity::GoingToWork;
                                    work_position = Some(free_slot);
                                    assigned_farm = Some(free_slot);
                                    submit_intent(&mut intents, entity, free_pos.x, free_pos.y, MovementPriority::JobRoute, "arrival:farm_retarget");
                                    npc_logs.push(
                                        idx,
                                        game_time.day(),
                                        game_time.hour(),
                                        game_time.minute(),
                                        format!("Farm occupied -> local retarget (r:{:.0})", radius),
                                    );
                                } else {
                                    if let Some(old) = assigned_farm.take() {
                                        entity_map.release(old);
                                    }
                                    work_position = None;
                                    activity = Activity::Idle;
                                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "All farms occupied -> Idle");
                                }
                            } else if entity_map.get_instance(farm_slot).is_some() {
                                if !owns {
                                    entity_map.claim(farm_slot);
                                    assigned_farm = Some(farm_slot);
                                }
                                // Check if farm is ready — harvest and carry home immediately.
                                let harvest = entity_map.get_instance_mut(farm_slot).and_then(|inst| {
                                    let food = inst.harvest();
                                    if food > 0 {
                                        Some((food, inst.harvest_log_msg(food)))
                                    } else {
                                        None
                                    }
                                });
                                if let Some((food, log_msg)) = harvest {
                                    if assigned_farm == Some(farm_slot) {
                                        entity_map.release(farm_slot);
                                        assigned_farm = None;
                                    }
                                    combat_log.write(CombatLogMsg { kind: CombatEventKind::Harvest, faction: faction_i32, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: log_msg, location: None });
                                    activity = Activity::Returning { loot: vec![(ItemKind::Food, food)] };
                                    submit_intent(&mut intents, entity, home.x, home.y, MovementPriority::JobRoute, "arrival:farm_harvest_return");
                                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Harvested -> Carrying home");
                                } else {
                                    // Farm not ready — if not already reserved by us, claim now.
                                    if assigned_farm != Some(farm_slot) {
                                        entity_map.claim(farm_slot);
                                    }
                                    activity = Activity::Working;
                                    assigned_farm = Some(farm_slot);
                                    work_position = Some(farm_slot);
                                    pop_inc_working(&mut economy.pop_stats, job, town_idx_i32);
                                    submit_intent(&mut intents, entity, farm_pos.x, farm_pos.y, MovementPriority::JobRoute, "arrival:farm_work");
                                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "-> Working (tending)");
                                }
                            }
                        } else {
                            if let Some(old) = assigned_farm.take() {
                                entity_map.release(old);
                            }
                            work_position = None;
                            activity = Activity::Idle;
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "No farm nearby -> Idle");
                        }
                    } else {
                        let current_pos = if idx * 2 + 1 < positions.len() {
                            Vec2::new(positions[idx * 2], positions[idx * 2 + 1])
                        } else {
                            Vec2::new(0.0, 0.0)
                        };
                        activity = Activity::Working;
                        pop_inc_working(&mut economy.pop_stats, job, town_idx_i32);
                        submit_intent(&mut intents, entity, current_pos.x, current_pos.y, MovementPriority::JobRoute, "arrival:work_hold");
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "-> Working");
                    }
                }
                Activity::Raiding { .. } => {
                    // Raider arrived at farm - check if ready to steal
                    if idx * 2 + 1 < positions.len() {
                        let pos = Vec2::new(positions[idx * 2], positions[idx * 2 + 1]);

                        let ready_farm_pos = find_location_within_radius(pos, &entity_map, LocationKind::Farm, FARM_ARRIVAL_RADIUS)
                            .and_then(|(_, fp)| entity_map.find_farm_at(fp).filter(|i| i.growth_ready).map(|_| fp));

                        if let Some(fp) = ready_farm_pos {
                            let food = entity_map.find_farm_at_mut(fp).map(|i| {
                                let f = i.harvest();
                                if f > 0 { combat_log.write(CombatLogMsg { kind: CombatEventKind::Harvest, faction: faction_i32, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: i.harvest_log_msg(f), location: None }); }
                                f
                            }).unwrap_or(0);

                            activity = Activity::Returning { loot: vec![(ItemKind::Food, food.max(1))] };
                            submit_intent(&mut intents, entity, home.x, home.y, MovementPriority::JobRoute, "arrival:raid_return");
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Stole food -> Returning");
                        } else {
                            // Farm not ready - find a different farm nearby (exclude current one)
                            let raid_search_radius = entity_map.spatial_cell_size().max(256.0) * 8.0;
                            let mut best_d2 = f32::MAX;
                            let mut other_farm_pos: Option<Vec2> = None;
                            entity_map.for_each_nearby(pos, raid_search_radius, |f| {
                                if f.kind != BuildingKind::Farm { return; }
                                if f.position.distance(pos) <= FARM_ARRIVAL_RADIUS { return; }
                                let d2 = f.position.distance_squared(pos);
                                if d2 < best_d2 {
                                    best_d2 = d2;
                                    other_farm_pos = Some(f.position);
                                }
                            });
                            if let Some(farm_pos) = other_farm_pos {
                                activity = Activity::Raiding { target: farm_pos };
                                submit_intent(&mut intents, entity, farm_pos.x, farm_pos.y, MovementPriority::JobRoute, "arrival:raid_retarget");
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Farm not ready, seeking another");
                            } else {
                                activity = Activity::Returning { loot: vec![] };
                                submit_intent(&mut intents, entity, home.x, home.y, MovementPriority::JobRoute, "arrival:raid_no_target_return");
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "No other farms, returning");
                            }
                        }
                    }
                }
                Activity::Mining { mine_pos } => {
                    let mine_pos = *mine_pos;
                    // Arrived at gold mine — check BuildingInstance for harvest or tend
                    let mine_slot = entity_map.slot_at_position(mine_pos);
                    if let Some(inst) = entity_map.find_mine_at_mut(mine_pos) {
                        if inst.growth_ready {
                            // Mine ready — harvest immediately
                            let town_levels = extras.town_upgrades.town_levels(town_idx_i32 as usize);
                            let yield_mult = UPGRADES.stat_mult(&town_levels, "Miner", UpgradeStatKind::Yield);
                            let base_gold = inst.harvest();
                            if base_gold > 0 { combat_log.write(CombatLogMsg { kind: CombatEventKind::Harvest, faction: faction_i32, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: inst.harvest_log_msg(base_gold), location: None }); }
                            let gold_amount = ((base_gold as f32) * yield_mult).round() as i32;
                            activity = Activity::Returning { loot: vec![(ItemKind::Gold, gold_amount)] };
                            submit_intent(&mut intents, entity, home.x, home.y, MovementPriority::JobRoute, "arrival:mine_harvest_return");
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                                format!("Harvested {} gold -> Returning", gold_amount));
                        } else if let Some(ms) = mine_slot {
                            // Mine still growing — claim occupancy and tend it
                            entity_map.claim(ms);
                            activity = Activity::MiningAtMine;
                            work_position = Some(ms);
                            pop_inc_working(&mut economy.pop_stats, job, town_idx_i32);
                            submit_intent(&mut intents, entity, mine_pos.x, mine_pos.y, MovementPriority::JobRoute, "idle:work_mine");
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "-> MiningAtMine (tending)");
                        }
                    } else {
                        activity = Activity::Idle;
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "No mine nearby -> Idle");
                    }
                }
                Activity::Wandering => {
                    activity = Activity::Idle;
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "-> Idle");
                }
                Activity::Returning { .. } => {
                    // May have arrived at wrong place (e.g. after DC removal) — redirect home
                    if home != Vec2::ZERO {
                        submit_intent(&mut intents, entity, home.x, home.y, MovementPriority::JobRoute, "arrival:return_redirect");
                    } else {
                        activity = Activity::Idle;
                    }
                }
                _ => {}
            }

            if let Some(s) = _ps { t_arrival += s.elapsed(); n_arrival += 1; }
            break 'decide;
        }

        // ====================================================================
        // Squad policy hard gate: if tired, go rest before any combat/transit
        // early-returns so squad rest policy is always respected.
        // ====================================================================
        if let Some(sid) = squad_id {
            if let Some(squad) = squad_state.squads.get(sid as usize) {
                let squad_needs_rest = energy < ENERGY_TIRED_THRESHOLD
                    || (energy < ENERGY_WAKE_THRESHOLD
                        && matches!(activity, Activity::GoingToRest | Activity::Resting));
                if squad.rest_when_tired && squad_needs_rest && home != Vec2::ZERO {
                    if combat_state.is_fighting() {
                        combat_state = CombatState::None;
                    }
                    if !matches!(activity, Activity::GoingToRest | Activity::Resting) {
                        activity = Activity::GoingToRest;
                        submit_intent(&mut intents, entity, home.x, home.y, MovementPriority::Survival, "squad:rest_gate");
                    }
                    break 'decide;
                }
            }
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
            let town_idx_usize = town_idx_i32 as usize;
            let flee_pct = match job {
                Job::Raider => 0.50, // raiders always flee at 50%
                Job::Archer | Job::Crossbow => {
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
                    let packed = gpu_state.threat_counts.get(idx).copied().unwrap_or(0);
                    let (enemies, allies) = unpack_threat_counts(packed);
                    let ratio = (enemies as f32 + 1.0) / (allies as f32 + 1.0);
                    (flee_pct * ratio).min(1.0)
                } else {
                    flee_pct
                };

                if health / max_hp < effective_threshold {
                    // Clean up work state if fleeing mid-mine
                    if matches!(activity, Activity::MiningAtMine) {
                        if let Some(wp) = work_position {
                            entity_map.release(wp);
                        }
                        work_position = None;
                    }
                    combat_state = CombatState::None;
                    if !matches!(&activity, Activity::Returning { .. }) {
                        activity = Activity::Returning { loot: vec![] };
                    }
                    submit_intent(&mut intents, entity, home.x, home.y, MovementPriority::Survival, "combat:flee_home");
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Fled combat");
                    if let Some(s) = _ps { t_combat += s.elapsed(); n_combat += 1; }
                    break 'decide;
                }
            }

            // Wounded + healing policy should override leash/return behavior.
            let healing_policy_active = policies.policies.get(town_idx_usize)
                .is_some_and(|p| p.prioritize_healing && energy > 0.0 && health / max_hp < p.recovery_hp);
            if healing_policy_active {
                if let Some(town) = farms.world.towns.get(town_idx_usize) {
                    combat_state = CombatState::None;
                    if !matches!(activity, Activity::GoingToHeal | Activity::HealingAtFountain { .. }) {
                        activity = Activity::GoingToHeal;
                        submit_intent(&mut intents, entity, town.center.x, town.center.y, MovementPriority::Survival, "combat:heal_fountain");
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Combat: wounded -> Fountain");
                    }
                    if let Some(s) = _ps { t_combat += s.elapsed(); n_combat += 1; }
                    break 'decide;
                }
            }

            // Priority 2: Should leash? (per-entity LeashRange or policy archer_leash)
            let should_leash = match job {
                Job::Archer | Job::Crossbow => policies.policies.get(town_idx_usize).is_none_or(|p| p.archer_leash),
                _ => leash_range_val.is_some(),
            };
            if should_leash {
                let leash_dist = leash_range_val.unwrap_or(400.0);
                if let CombatState::Fighting { origin } = &combat_state {
                    if idx * 2 + 1 < positions.len() {
                        let dx = positions[idx * 2] - origin.x;
                        let dy = positions[idx * 2 + 1] - origin.y;
                        if (dx * dx + dy * dy).sqrt() > leash_dist {
                            if matches!(activity, Activity::MiningAtMine) {
                                if let Some(wp) = work_position {
                                    entity_map.release(wp);
                                }
                                work_position = None;
                            }
                            combat_state = CombatState::None;
                            if !matches!(&activity, Activity::Returning { .. }) {
                                activity = Activity::Returning { loot: vec![] };
                            }
                            submit_intent(&mut intents, entity, home.x, home.y, MovementPriority::Survival, "combat:leash_home");
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Leashed -> Returning");
                            if let Some(s) = _ps { t_combat += s.elapsed(); n_combat += 1; }
                            break 'decide;
                        }
                    }
                }
            }

            // Priority 3: Still in combat, attack_system handles targeting
            if let Some(s) = _ps { t_combat += s.elapsed(); n_combat += 1; }
            } // end combat_tick
            break 'decide;
        }

        // ====================================================================
        // Squad sync: apply squad target/patrol policy changes immediately
        // (before transit skip) so squad members react by next decision tick.
        // Covers all squad-assigned units: archers, crossbow, raiders, fighters.
        // ====================================================================
        let _ps_sq = if profiling && squad_id.is_some() { Some(std::time::Instant::now()) } else { None };
        if let Some(sid) = squad_id {
            // Manual micro override: player-assigned attack target takes priority.
            // Don't redirect the NPC — combat system handles ManualTarget directly.
            if manual_target.is_some() {
                // Still allow squad target to set movement destination (already done
                // when the right-click command was issued), but don't override it here.
                // Skip the rest of squad sync for this NPC.
            } else if let Some(squad) = squad_state.squads.get(sid as usize) {
                if let Some(target) = squad.target {
                    let squad_needs_rest = energy < ENERGY_TIRED_THRESHOLD
                        || (energy < ENERGY_WAKE_THRESHOLD
                            && matches!(activity, Activity::GoingToRest | Activity::Resting));
                    if squad.rest_when_tired && squad_needs_rest && home != Vec2::ZERO {
                        if !matches!(activity, Activity::GoingToRest | Activity::Resting) {
                            activity = Activity::GoingToRest;
                            submit_intent(&mut intents, entity, home.x, home.y, MovementPriority::Survival, "squad:rest_home");
                        }
                        break 'decide;
                    }
                    // Wounded: prioritize healing over squad target (prevents flee-engage oscillation)
                    let ti = town_idx_i32 as usize;
                    if let Some(p) = policies.policies.get(ti) {
                        if p.prioritize_healing && energy > 0.0 && health / max_hp < p.recovery_hp {
                            if !matches!(activity, Activity::GoingToHeal | Activity::HealingAtFountain { .. }) {
                                if let Some(town) = farms.world.towns.get(ti) {
                                    combat_state = CombatState::None;
                                    activity = Activity::GoingToHeal;
                                    submit_intent(&mut intents, entity, town.center.x, town.center.y, MovementPriority::Survival, "squad:heal_fountain");
                                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Squad: wounded -> Fountain");
                                }
                            }
                            break 'decide;
                        }
                    }
                    // Squad target — only redirect when needed (no per-frame GPU writes)
                    match activity {
                        Activity::OnDuty { .. } => {
                            // At a position — redirect only if squad target moved
                            if idx * 2 + 1 < positions.len() {
                                let dx = positions[idx * 2] - target.x;
                                let dy = positions[idx * 2 + 1] - target.y;
                                if dx * dx + dy * dy > 100.0 * 100.0 {
                                    activity = Activity::Patrolling;
                                    submit_intent(&mut intents, entity, target.x, target.y, MovementPriority::Squad, "squad:target_rejoin");
                                }
                            }
                        }
                        Activity::Patrolling | Activity::Raiding { .. } |
                        Activity::GoingToRest | Activity::Resting |
                        Activity::GoingToHeal | Activity::HealingAtFountain { .. } |
                        Activity::Returning { .. } => {
                            // Already heading to target, resting, healing, or carrying loot — no redirect
                        }
                        _ => {
                            // Idle/Wandering/Returning/other — redirect to squad target
                            activity = Activity::Patrolling;
                            submit_intent(&mut intents, entity, target.x, target.y, MovementPriority::Squad, "squad:target_assign");
                        }
                    }
                } else if !squad.patrol_enabled {
                    // No target + patrol disabled: stop and wait (gathering phase)
                    if matches!(activity, Activity::Patrolling | Activity::OnDuty { .. } | Activity::Raiding { .. }) {
                        activity = Activity::Idle;
                        if idx * 2 + 1 < positions.len() {
                            submit_intent(&mut intents, entity, positions[idx * 2], positions[idx * 2 + 1], MovementPriority::Squad, "squad:hold_position");
                        }
                    }
                }
            }
        }
        if let Some(s) = _ps_sq { t_squad += s.elapsed(); n_squad += 1; }

        // ====================================================================
        // Farmer en-route retarget: if target farm became occupied, find another
        // ====================================================================
        if job == Job::Farmer && matches!(activity, Activity::GoingToWork) && (idx + frame) % think_buckets == 0 {
            if let Some(wp) = work_position {
                let occ = entity_map.occupant_count(wp);
                let occupied_by_other = (occ > 1) || (occ >= 1 && assigned_farm != Some(wp));
                if occupied_by_other {
                    let wp_pos = entity_map.get_instance(wp).map(|i| i.position).unwrap_or_default();
                    let current_pos = if idx * 2 + 1 < positions.len() {
                        Vec2::new(positions[idx * 2], positions[idx * 2 + 1])
                    } else {
                        wp_pos
                    };
                    if let Some((free_slot, free_pos, radius)) =
                        find_farmer_farm_target(current_pos, &entity_map, town_idx_i32 as u32)
                    {
                        if let Some(old) = assigned_farm.take() {
                            entity_map.release(old);
                        }
                        entity_map.claim(free_slot);
                        work_position = Some(free_slot);
                        assigned_farm = Some(free_slot);
                        submit_intent(&mut intents, entity, free_pos.x, free_pos.y, MovementPriority::JobRoute, "farm:retarget_free");
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            format!("Farm taken -> local retarget (r:{:.0})", radius),
                        );
                    } else {
                        if let Some(old) = assigned_farm.take() {
                            entity_map.release(old);
                        }
                        work_position = None;
                        activity = Activity::Idle;
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "All farms occupied -> Idle");
                    }
                    break 'decide;
                }
            }
        }

        // ====================================================================
        // Skip NPCs in transit states (they're walking to their destination)
        // GoingToHeal proximity check runs at combat cadence (every 8 frames).
        // ====================================================================
        if activity.is_transit() {
            // Early arrival: GoingToHeal NPCs stop once inside healing range
            if (idx + frame) % COMBAT_INTERVAL == 0 && matches!(activity, Activity::GoingToHeal) {
                let town_idx = town_idx_i32 as usize;
                if let Some(town) = farms.world.towns.get(town_idx) {
                    if idx * 2 + 1 < positions.len() {
                        let current = Vec2::new(positions[idx * 2], positions[idx * 2 + 1]);
                        if current.distance(town.center) <= HEAL_DRIFT_RADIUS {
                            let threshold = policies.policies.get(town_idx)
                                .map(|p| p.recovery_hp).unwrap_or(0.8);
                            activity = Activity::HealingAtFountain { recover_until: threshold };
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "-> Healing");
                        }
                    }
                }
            }
            n_transit_skip += 1;
            break 'decide;
        }

        // ====================================================================
        // Tier 3 gate: non-combat decisions only run on this NPC's bucket
        // ====================================================================
        if (idx + frame) % think_buckets != 0 {
            break 'decide;
        }

        // ====================================================================
        // Priority 4a: HealingAtFountain? -> Wake when HP recovered
        // ====================================================================
        if let Activity::HealingAtFountain { recover_until } = &activity {
            if health / max_hp >= *recover_until {
                activity = Activity::Idle;
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Recovered");
                // Fall through to make a decision
            } else {
                // Drift check: separation physics pushes NPCs out of healing range
                let town_idx = town_idx_i32 as usize;
                if let Some(town) = farms.world.towns.get(town_idx) {
                    if idx * 2 + 1 < positions.len() {
                        let current = Vec2::new(positions[idx * 2], positions[idx * 2 + 1]);
                        if current.distance(town.center) > HEAL_DRIFT_RADIUS {
                            submit_intent(&mut intents, entity, town.center.x, town.center.y, MovementPriority::Survival, "heal:drift_retarget");
                        }
                    }
                }
                break 'decide; // still healing
            }
        }

        // ====================================================================
        // Priority 4b: Resting? -> Wake when energy recovered
        // ====================================================================
        if matches!(activity, Activity::Resting) {
            if energy >= ENERGY_WAKE_THRESHOLD {
                activity = Activity::Idle;
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Woke up");
                // Fall through to make a decision
            } else {
                break 'decide; // still resting
            }
        }

        // ====================================================================
        // Priority 5: Working/Mining + tired?
        // ====================================================================
        let _ps_wk = if profiling { Some(std::time::Instant::now()) } else { None };
        if matches!(activity, Activity::Working) {
            // Safety invariant: Working farmer must own a single valid farm reservation.
            // This self-heals stale state from older saves/logic and prevents multi-worker farms.
            let current_farm = assigned_farm.or(work_position);
            let Some(farm_slot) = current_farm else {
                activity = Activity::Idle;
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Working without farm -> Reassign");
                if let Some(s) = _ps_wk { t_work += s.elapsed(); n_work += 1; }
                break 'decide;
            };
            let valid_farm = entity_map.get_instance(farm_slot)
                .is_some_and(|inst| inst.kind == BuildingKind::Farm && inst.town_idx == town_idx_i32 as u32);
            if !valid_farm {
                if assigned_farm == Some(farm_slot) {
                    entity_map.release(farm_slot);
                }
                assigned_farm = None;
                work_position = None;
                activity = Activity::Idle;
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Working farm invalid -> Reassign");
                if let Some(s) = _ps_wk { t_work += s.elapsed(); n_work += 1; }
                break 'decide;
            }
            if assigned_farm.is_none() {
                // If another farmer already owns this farm, don't pile on.
                if entity_map.occupant_count(farm_slot) >= 1 {
                    work_position = None;
                    activity = Activity::Idle;
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Working on foreign reservation -> Reassign");
                    if let Some(s) = _ps_wk { t_work += s.elapsed(); n_work += 1; }
                    break 'decide;
                }
                entity_map.claim(farm_slot);
                assigned_farm = Some(farm_slot);
            }
            if entity_map.occupant_count(farm_slot) > 1 {
                if assigned_farm == Some(farm_slot) {
                    entity_map.release(farm_slot);
                }
                assigned_farm = None;
                if work_position == Some(farm_slot) {
                    work_position = None;
                }
                activity = Activity::Idle;
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Farm contention -> Reassign");
                if let Some(s) = _ps_wk { t_work += s.elapsed(); n_work += 1; }
                break 'decide;
            }
            let should_stop = if energy < ENERGY_TIRED_THRESHOLD {
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Tired -> Stopped");
                true
            } else {
                false
            };
            if should_stop {
                if let Some(af) = assigned_farm {
                    entity_map.release(af);
                    assigned_farm = None;
                }
                activity = Activity::Idle;
            }
            if let Some(s) = _ps_wk { t_work += s.elapsed(); n_work += 1; }
            break 'decide;
        }

        if matches!(activity, Activity::MiningAtMine) {
            if let Some(mine_slot) = work_position {
                let mine_pos = entity_map.get_instance(mine_slot).map(|i| i.position).unwrap_or_default();
                // Proximity check — if pushed away from mine, abort and re-walk
                if idx * 2 + 1 < positions.len() {
                    let current = Vec2::new(positions[idx * 2], positions[idx * 2 + 1]);
                    if current.distance(mine_pos) > MINE_WORK_RADIUS {
                        entity_map.release(mine_slot);
                        work_position = None;
                        activity = Activity::Mining { mine_pos };
                        submit_intent(&mut intents, entity, mine_pos.x, mine_pos.y, MovementPriority::JobRoute, "mining:push_return");
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Pushed from mine -> returning");
                        if let Some(s) = _ps_wk { t_work += s.elapsed(); n_work += 1; }
                        break 'decide;
                    }
                }
                // Check if mine is Ready to harvest
                let mut harvested = false;
                if let Some(inst) = entity_map.get_instance_mut(mine_slot) {
                    if inst.kind == BuildingKind::GoldMine && inst.growth_ready {
                        let town_levels = extras.town_upgrades.town_levels(town_idx_i32 as usize);
                        let yield_mult = UPGRADES.stat_mult(&town_levels, "Miner", UpgradeStatKind::Yield);
                        let base_gold = inst.harvest();
                        if base_gold > 0 { combat_log.write(CombatLogMsg { kind: CombatEventKind::Harvest, faction: faction_i32, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: inst.harvest_log_msg(base_gold), location: None }); }
                        let gold_amount = ((base_gold as f32) * yield_mult).round() as i32;
                        entity_map.release(mine_slot);
                        work_position = None;
                        activity = Activity::Returning { loot: vec![(ItemKind::Gold, gold_amount)] };
                        submit_intent(&mut intents, entity, home.x, home.y, MovementPriority::JobRoute, "mining:harvest_return");
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                            format!("Harvested {} gold -> Returning", gold_amount));
                        harvested = true;
                    }
                }
                if !harvested && energy < ENERGY_TIRED_THRESHOLD {
                    entity_map.release(mine_slot);
                    work_position = None;
                    activity = Activity::Idle;
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Mining -> Tired -> Idle");
                }
            }
            if let Some(s) = _ps_wk { t_work += s.elapsed(); n_work += 1; }
            break 'decide;
        }

        // ====================================================================
        // Priority 6: OnDuty (tired -> leave post, else patrol when ready)
        // ====================================================================
        if let Activity::OnDuty { ticks_waiting } = &activity {
            let ticks = *ticks_waiting;
            let squad_forces_stay = job.is_patrol_unit() && squad_id
                .and_then(|sid| squad_state.squads.get(sid as usize))
                .is_some_and(|s| !s.rest_when_tired);
            if energy < ENERGY_TIRED_THRESHOLD && !squad_forces_stay {
                activity = Activity::Idle;
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Tired -> Left post");
                // Fall through to idle scoring — Rest will win
            } else {
                let squad_patrol_enabled = squad_id
                    .and_then(|sid| squad_state.squads.get(sid as usize))
                    .is_none_or(|s| s.patrol_enabled);
                if ticks >= ARCHER_PATROL_WAIT && squad_patrol_enabled {
                    if let Ok(route) = npc_data.patrol_route_q.get(entity) {
                        if !route.posts.is_empty() {
                            patrol_current = (patrol_current + 1) % route.posts.len();
                            if let Some(post) = route.posts.get(patrol_current) {
                                activity = Activity::Patrolling;
                                submit_intent(&mut intents, entity, post.x, post.y, MovementPriority::JobRoute, "onduty:patrol_advance");
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "-> Patrolling");
                            }
                        }
                    }
                }
                if let Some(s) = _ps_wk { t_work += s.elapsed(); n_work += 1; }
                break 'decide;
            }
        }


        // ====================================================================
        // Priority 8: Idle -> Score Eat/Rest/Work/Wander (policy-aware)
        // ====================================================================
        let _ps_idle = if profiling { Some(std::time::Instant::now()) } else { None };
        let en = energy;
        let (_fight_m, _flee_m, rest_m, eat_m, work_m, wander_m) = personality.get_multipliers();

        let town_idx = town_idx_i32 as usize;
        let policy = policies.policies.get(town_idx);
        let food_available = policy.is_none_or(|p| p.eat_food)
            && town_idx < economy.food_storage.food.len()
            && economy.food_storage.food[town_idx] > 0;
        let mut scores: [(Action, f32); 5] = [(Action::Wander, 0.0); 5];
        let mut score_count: usize = 0;

        // Prioritize healing: wounded NPCs go to fountain before doing anything else
        // Skip if starving — HP capped at 50% until energy recovers
        if let Some(p) = policy {
            if p.prioritize_healing && energy > 0.0 && health / max_hp < p.recovery_hp {
                if let Some(town) = farms.world.towns.get(town_idx) {
                    let center = town.center;
                    activity = Activity::GoingToHeal;
                    submit_intent(&mut intents, entity, center.x, center.y, MovementPriority::Survival, "idle:heal_fountain");
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Wounded -> Fountain");
                    if let Some(s) = _ps_idle { t_idle += s.elapsed(); n_idle += 1; }
                    break 'decide;
                }
            }
        }

        if food_available && en < ENERGY_EAT_THRESHOLD {
            let eat_score = (ENERGY_EAT_THRESHOLD - en) * SCORE_EAT_MULT * eat_m;
            scores[score_count] = (Action::Eat, eat_score); score_count += 1;
        }

        if en < ENERGY_HUNGRY && home != Vec2::ZERO {
            let rest_score = (ENERGY_HUNGRY - en) * SCORE_REST_MULT * rest_m;
            scores[score_count] = (Action::Rest, rest_score); score_count += 1;
        }

        // Work schedule gate: per-job schedule
        let schedule = match job {
            Job::Farmer | Job::Miner => policy.map(|p| p.farmer_schedule).unwrap_or(WorkSchedule::Both),
            Job::Archer | Job::Crossbow => policy.map(|p| p.archer_schedule).unwrap_or(WorkSchedule::Both),
            _ => WorkSchedule::Both,
        };
        let work_allowed = match schedule {
            WorkSchedule::Both => true,
            WorkSchedule::DayOnly => game_time.is_daytime(),
            WorkSchedule::NightOnly => !game_time.is_daytime(),
        };

        let can_work = work_allowed && match job {
            Job::Farmer => true,  // dynamically find farms (same as Miner)
            Job::Miner => true,  // miners always have work (find nearest mine dynamically)
            Job::Archer | Job::Crossbow | Job::Fighter => has_patrol,
            Job::Raider => false, // squad-driven, not idle-scored
        };
        if can_work {
            let hp_pct = health / max_hp;
            let hp_mult = if hp_pct < 0.3 { 0.0 } else { (hp_pct - 0.3) * (1.0 / 0.7) };
            // Scale down work desire when tired so rest/eat can win before starvation
            let energy_factor = if en < ENERGY_TIRED_THRESHOLD {
                en / ENERGY_TIRED_THRESHOLD
            } else {
                1.0
            };
            let work_score = SCORE_WORK_BASE * work_m * hp_mult * energy_factor;
            if work_score > 0.0 { scores[score_count] = (Action::Work, work_score); score_count += 1; }
        }

        // Off-duty behavior when work is gated out by schedule
        if !work_allowed {
            let off_duty = match job {
                Job::Farmer | Job::Miner => policy.map(|p| p.farmer_off_duty).unwrap_or(OffDutyBehavior::GoToBed),
                Job::Archer | Job::Crossbow => policy.map(|p| p.archer_off_duty).unwrap_or(OffDutyBehavior::GoToBed),
                _ => OffDutyBehavior::GoToBed,
            };
            match off_duty {
                OffDutyBehavior::GoToBed => {
                    // Boost rest score so NPCs prefer going to bed
                    if home != Vec2::ZERO { scores[score_count] = (Action::Rest, 80.0 * rest_m); score_count += 1; }
                }
                OffDutyBehavior::StayAtFountain => {
                    // Go to town center (fountain)
                    if let Some(town) = farms.world.towns.get(town_idx) {
                        let center = town.center;
                        activity = Activity::Wandering;
                        submit_intent(&mut intents, entity, center.x, center.y, MovementPriority::Survival, "offduty:fountain");
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Off-duty -> Fountain");
                        if let Some(s) = _ps_idle { t_idle += s.elapsed(); n_idle += 1; }
                        break 'decide;
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
            format!("{:?} (e:{:.0} h:{:.0})", action, energy, health));

        match action {
            Action::Eat => {
                if town_idx < economy.food_storage.food.len() && economy.food_storage.food[town_idx] > 0 {
                    let old_energy = energy;
                    economy.food_storage.food[town_idx] -= 1;
                    energy = 100.0;
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                        format!("Ate (e:{:.0}->{:.0})", old_energy, energy));
                }
            }
            Action::Rest => {
                if home != Vec2::ZERO {
                    activity = Activity::GoingToRest;
                    submit_intent(&mut intents, entity, home.x, home.y, MovementPriority::Survival, "idle:rest_home");
                }
            }
            Action::Work => {
                match job {
                    Job::Farmer => {
                        // Local expanding-radius search: keep farmers working near where they are.
                        let current_pos = if idx * 2 + 1 < positions.len() {
                            Vec2::new(positions[idx * 2], positions[idx * 2 + 1])
                        } else { home };
                        if let Some((farm_slot, farm_pos, radius)) =
                            find_farmer_farm_target(current_pos, &entity_map, town_idx_i32 as u32)
                        {
                            if let Some(old) = assigned_farm.take() {
                                entity_map.release(old);
                            }
                            entity_map.claim(farm_slot);
                            activity = Activity::GoingToWork;
                            work_position = Some(farm_slot);
                            assigned_farm = Some(farm_slot);
                            submit_intent(&mut intents, entity, farm_pos.x, farm_pos.y, MovementPriority::JobRoute, "idle:work_farm");
                            npc_logs.push(
                                idx,
                                game_time.day(),
                                game_time.hour(),
                                game_time.minute(),
                                format!("Farm target local (r:{:.0})", radius),
                            );
                        }
                    }
                    Job::Miner => {
                        // Check for manually assigned mine (via miner home UI)
                        let assigned = entity_map.find_by_position(home)
                            .filter(|inst| inst.kind == BuildingKind::MinerHome)
                            .and_then(|inst| inst.assigned_mine);

                        let mine_target = if let Some(assigned_pos) = assigned {
                            // Use assigned mine directly
                            Some(assigned_pos)
                        } else {
                            // Find nearest mine that isn't occupied
                            let current_pos = if idx * 2 + 1 < positions.len() {
                                Vec2::new(positions[idx * 2], positions[idx * 2 + 1])
                            } else {
                                home
                            };
                            // Priority: ready > unoccupied > least-occupied (town-scoped, global fallback)
                            let mut best_mine: Option<(i32, i32, f32, Vec2)> = None; // (priority, occupants, dist, pos)
                            for inst in entity_map.iter_kind_for_town(BuildingKind::GoldMine, town_idx_i32 as u32) {
                                let occupant_count = inst.occupants as i32;
                                let ready = inst.growth_ready;
                                let priority = if ready { 0 } else if occupant_count == 0 { 1 } else { 2 };
                                let dist = current_pos.distance(inst.position);
                                if best_mine.is_none() || (priority, occupant_count, dist as i32) < (best_mine.unwrap().0, best_mine.unwrap().1, best_mine.unwrap().2 as i32) {
                                    best_mine = Some((priority, occupant_count, dist, inst.position));
                                }
                            }
                            // Global fallback if no mines in own town
                            if best_mine.is_none() {
                                for inst in entity_map.iter_kind(BuildingKind::GoldMine) {
                                    let occupant_count = inst.occupants as i32;
                                    let ready = inst.growth_ready;
                                    let priority = if ready { 0 } else if occupant_count == 0 { 1 } else { 2 };
                                    let dist = current_pos.distance(inst.position);
                                    if best_mine.is_none() || (priority, occupant_count, dist as i32) < (best_mine.unwrap().0, best_mine.unwrap().1, best_mine.unwrap().2 as i32) {
                                        best_mine = Some((priority, occupant_count, dist, inst.position));
                                    }
                                }
                            }
                            best_mine.map(|(_, _, _, pos)| pos)
                        };

                        if let Some(mine_pos) = mine_target {
                            activity = Activity::Mining { mine_pos };
                            submit_intent(&mut intents, entity, mine_pos.x, mine_pos.y, MovementPriority::JobRoute, "idle:work_mine");
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "-> Mining gold");
                        }
                        // No mines available — stay idle
                    }
                    Job::Archer | Job::Crossbow | Job::Fighter => {
                        // Squad override: go to squad target instead of patrolling
                        if let Some(sid) = squad_id {
                            if let Some(squad) = squad_state.squads.get(sid as usize) {
                                if let Some(target) = squad.target {
                                    activity = Activity::Patrolling;
                                    submit_intent(&mut intents, entity, target.x, target.y, MovementPriority::Squad, "idle:squad_target");
                                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                                        format!("Squad {} -> target", sid + 1));
                                    if let Some(s) = _ps_idle { t_idle += s.elapsed(); n_idle += 1; }
                                    break 'decide;
                                }
                                if !squad.patrol_enabled {
                                    if let Some(s) = _ps_idle { t_idle += s.elapsed(); n_idle += 1; }
                                    break 'decide;
                                }
                            }
                            // No target set — fall through to normal patrol
                        }
                        if let Ok(route) = npc_data.patrol_route_q.get(entity) {
                            if !route.posts.is_empty() {
                                let safe_idx = patrol_current % route.posts.len();
                                if let Some(post) = route.posts.get(safe_idx) {
                                    patrol_current = safe_idx;
                                    activity = Activity::Patrolling;
                                    submit_intent(&mut intents, entity, post.x, post.y, MovementPriority::JobRoute, "idle:patrol_route");
                                }
                            }
                        }
                    }
                    Job::Raider => {
                        // Squad-driven: squad target override handled above in squad sync.
                        // If idle with no squad target, wander near home (gathering phase).
                        if squad_id.is_some() {
                            // Squad assigned — target is managed by ai_squad_commander
                        } else {
                            // No squad — wander near town
                            let offset_x = (pseudo_random(idx, frame + 1) - 0.5) * 100.0;
                            let offset_y = (pseudo_random(idx, frame + 2) - 0.5) * 100.0;
                            activity = Activity::Wandering;
                            submit_intent(&mut intents, entity, home.x + offset_x, home.y + offset_y, MovementPriority::Wander, "idle:raider_wander");
                        }
                    }
                }
            }
            Action::Wander => {
                // Wander near home to prevent unbounded drift off the map
                let (base_x, base_y) = if home != Vec2::ZERO {
                    (home.x, home.y)
                } else if idx * 2 + 1 < positions.len() {
                    (positions[idx * 2], positions[idx * 2 + 1])
                } else {
                    if let Some(s) = _ps_idle { t_idle += s.elapsed(); n_idle += 1; }
                    break 'decide;
                };
                let offset_x = (pseudo_random(idx, frame + 1) - 0.5) * 200.0;
                let offset_y = (pseudo_random(idx, frame + 2) - 0.5) * 200.0;
                activity = Activity::Wandering;
                submit_intent(&mut intents, entity, base_x + offset_x, base_y + offset_y, MovementPriority::Wander, "idle:wander");
            }
            Action::Fight | Action::Flee => {}
        }
        if let Some(s) = _ps_idle { t_idle += s.elapsed(); n_idle += 1; }
        } // end 'decide block

        // Farmer reservation invariant:
        // a farm reservation may exist only while actively working or moving to work.
        // This prevents "ghost reservations" (farm shows occupants=1 with no active farmer).
        if job == Job::Farmer && assigned_farm.is_some()
            && !matches!(activity, Activity::Working | Activity::GoingToWork)
        {
            if let Some(slot) = assigned_farm.take() {
                entity_map.release(slot);
                if work_position == Some(slot) {
                    work_position = None;
                }
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Released farm reservation");
            }
        }

        // Conditional writeback: skip unchanged NPCs (most exit early via break 'decide)
        if std::mem::discriminant(&activity) != orig_activity {
            if let Ok(mut act) = npc_state.activity_q.get_mut(entity) { *act = activity; }
        }
        if at_destination != orig_at_destination {
            if let Ok(mut flags) = npc_state.npc_flags_q.get_mut(entity) { flags.at_destination = at_destination; }
        }
        if energy != orig_energy {
            if let Ok(mut en) = npc_state.energy_q.get_mut(entity) { en.0 = energy; }
        }
        if std::mem::discriminant(&combat_state) != orig_combat_state {
            if let Ok(mut cs) = npc_state.combat_state_q.get_mut(entity) { *cs = combat_state; }
        }
        if work_position != orig_work_target || assigned_farm != orig_occupied_slot {
            if let Ok(mut ws) = npc_data.work_state_q.get_mut(entity) {
                ws.occupied_slot = assigned_farm;
                ws.work_target = work_position;
            }
        }
        if patrol_current != orig_patrol_current {
            if let Ok(mut route) = npc_data.patrol_route_q.get_mut(entity) { route.current = patrol_current; }
        }
    }

    // Record sub-profiling results
    if profiling {
        let t = &extras.timings;
        t.record("decision/arrival", t_arrival.as_secs_f32() * 1000.0);
        t.record("decision/combat", t_combat.as_secs_f32() * 1000.0);
        t.record("decision/idle", t_idle.as_secs_f32() * 1000.0);
        t.record("decision/squad", t_squad.as_secs_f32() * 1000.0);
        t.record("decision/work", t_work.as_secs_f32() * 1000.0);
        t.record("decision/n_arrival", n_arrival as f32);
        t.record("decision/n_combat", n_combat as f32);
        t.record("decision/n_idle", n_idle as f32);
        t.record("decision/n_squad", n_squad as f32);
        t.record("decision/n_work", n_work as f32);
        t.record("decision/n_transit_skip", n_transit_skip as f32);
        t.record("decision/n_total", n_total as f32);
    }
}

/// Increment OnDuty tick counters (runs every frame for guards at posts).
/// Separated from decision_system because we need mutable Activity access.
pub fn on_duty_tick_system(
    mut q: Query<(&mut Activity, &CombatState), (Without<Building>, Without<Dead>)>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("on_duty_tick");
    for (mut activity, combat_state) in q.iter_mut() {
        if combat_state.is_fighting() { continue; }
        if let Activity::OnDuty { ticks_waiting } = activity.as_mut() {
            *ticks_waiting += 1;
        }
    }
}

/// Rebuild all guards' patrol routes when WorldData changes (waypoint added/removed/reordered).
pub fn rebuild_patrol_routes_system(
    mut entity_map: ResMut<EntityMap>,
    mut patrols_dirty: MessageReader<crate::messages::PatrolsDirtyMsg>,
    mut patrol_swaps: MessageReader<crate::messages::PatrolSwapMsg>,
    timings: Res<SystemTimings>,
    mut patrol_route_q: Query<&mut PatrolRoute>,
    mut commands: Commands,
    patrol_npc_q: Query<(Entity, &EntitySlot, &Job, &TownId), (Without<Building>, Without<Dead>)>,
) {
    let _t = timings.scope("rebuild_patrol_routes");
    if patrols_dirty.read().count() == 0 { return; }

    // Apply pending patrol order swap from UI
    if let Some(swap) = patrol_swaps.read().last() {
        let (sa, sb) = (swap.slot_a, swap.slot_b);
        let order_a = entity_map.get_instance(sa).map(|i| i.patrol_order).unwrap_or(0);
        let order_b = entity_map.get_instance(sb).map(|i| i.patrol_order).unwrap_or(0);
        if let Some(inst) = entity_map.get_instance_mut(sa) { inst.patrol_order = order_b; }
        if let Some(inst) = entity_map.get_instance_mut(sb) { inst.patrol_order = order_a; }
    }

    // Collect patrol unit slots + towns via ECS query
    let patrol_slots: Vec<(Entity, usize, i32)> = patrol_npc_q.iter()
        .filter(|(_, _, job, _)| job.is_patrol_unit())
        .map(|(entity, slot, _, town)| (entity, slot.0, town.0))
        .collect();

    // Build routes once per town (immutable entity_map access for building queries)
    let mut town_routes: std::collections::HashMap<u32, Vec<Vec2>> = std::collections::HashMap::new();
    for &(_, _, town_idx) in &patrol_slots {
        let tid = town_idx as u32;
        town_routes.entry(tid).or_insert_with(|| {
            crate::systems::spawn::build_patrol_route(&entity_map, tid)
        });
    }

    // Write routes back via ECS
    for (entity, _slot, town_idx) in patrol_slots {
        let tid = town_idx as u32;
        let Some(new_posts) = town_routes.get(&tid) else { continue };
        if new_posts.is_empty() { continue; }
        if let Ok(mut route) = patrol_route_q.get_mut(entity) {
            if route.current >= new_posts.len() { route.current = 0; }
            route.posts = new_posts.clone();
        } else {
            commands.entity(entity).insert(PatrolRoute { posts: new_posts.clone(), current: 0 });
        }
    }
}
