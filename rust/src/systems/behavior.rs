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

use crate::components::*;
use crate::constants::UpgradeStatKind;
use crate::constants::*;
use crate::messages::{CombatLogMsg, GpuUpdate, GpuUpdateMsg, WorkIntent, WorkIntentMsg};
use crate::resources::{
    CombatEventKind, EntityMap, GameTime, GpuReadState, MovementPriority, PathRequestQueue,
    NpcLogCache, OffDutyBehavior, SelectedNpc, SquadState, TownPolicies, WorkSchedule,
};
use crate::settings::UserSettings;
use crate::systemparams::EconomyState;
use crate::systems::economy::*;
use crate::systems::stats::UPGRADES;
use crate::world::{
    BuildingKind, LocationKind, WorldData, find_location_within_radius, find_within_radius,
};
use bevy::prelude::*;

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
    pub carried_loot_q: Query<'w, 's, &'static mut CarriedLoot>,
    pub stealer_q: Query<'w, 's, &'static Stealer>,
    pub has_energy_q: Query<'w, 's, &'static HasEnergy>,
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
    pub gpu_updates: MessageWriter<'w, GpuUpdateMsg>,
    pub work_intents: MessageWriter<'w, WorkIntentMsg>,
    pub policies: Res<'w, TownPolicies>,
    pub squad_state: Res<'w, SquadState>,
    pub town_upgrades: Res<'w, crate::systems::stats::TownUpgrades>,
    pub selected_npc: Res<'w, SelectedNpc>,
    pub settings: Res<'w, UserSettings>,
}

/// Arrival system: proximity-based delivery for Returning NPCs.
///
/// When a Returning NPC is within delivery radius of home, deposit CarriedLoot and go Idle.
/// Arrival detection (transit -> AtDestination) is handled by gpu_position_readback.
/// Farm occupancy and harvest are handled exclusively by decision_system.
pub fn arrival_system(
    mut economy: EconomyState,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    mut npc_logs: ResMut<NpcLogCache>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut carried_loot_q: Query<&mut CarriedLoot>,
    mut npc_q: Query<
        (
            Entity,
            &GpuSlot,
            &Job,
            &TownId,
            &mut Activity,
            &Home,
            &mut NpcWorkState,
        ),
        (Without<Building>, Without<Dead>),
    >,
) {
    if game_time.is_paused() {
        return;
    }
    let positions = &gpu_state.positions;
    const DELIVERY_RADIUS: f32 = 50.0;

    // ========================================================================
    // 1. Proximity-based delivery for all Returning NPCs
    // ========================================================================
    // Collect (slot, entity, town_idx) for returning NPCs near home.
    let mut deliveries: Vec<(usize, Entity, usize)> = Vec::new();
    for (entity, slot, _job, town_id, activity, home, _work_state) in npc_q.iter() {
        if !matches!(*activity, Activity::Returning) {
            continue;
        }
        let idx = slot.0;
        if idx * 2 + 1 >= positions.len() {
            continue;
        }
        let x = positions[idx * 2];
        let y = positions[idx * 2 + 1];
        let dx = x - home.0.x;
        let dy = y - home.0.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist <= DELIVERY_RADIUS {
            deliveries.push((idx, entity, town_id.0 as usize));
        }
    }

    for (idx, entity, town_idx) in deliveries {
        // Read and drain CarriedLoot
        if let Ok(mut loot) = carried_loot_q.get_mut(entity) {
            if loot.food > 0 {
                if town_idx < economy.food_storage.food.len() {
                    economy.food_storage.food[town_idx] += loot.food;
                }
                npc_logs.push(
                    idx,
                    game_time.day(),
                    game_time.hour(),
                    game_time.minute(),
                    format!("Delivered {} food", loot.food),
                );
            }
            if loot.gold > 0 {
                if town_idx < economy.gold_storage.gold.len() {
                    economy.gold_storage.gold[town_idx] += loot.gold;
                }
                npc_logs.push(
                    idx,
                    game_time.day(),
                    game_time.hour(),
                    game_time.minute(),
                    format!("Delivered {} gold", loot.gold),
                );
            }
            if !loot.equipment.is_empty() {
                let count = loot.equipment.len();
                for item in loot.equipment.drain(..) {
                    economy.town_inventory.add(town_idx, item);
                }
                npc_logs.push(
                    idx,
                    game_time.day(),
                    game_time.hour(),
                    game_time.minute(),
                    format!("Delivered {} equipment", count),
                );
            }
            loot.food = 0;
            loot.gold = 0;
        }
        if let Ok((_, slot, _, _, mut act, _, mut ws)) = npc_q.get_mut(entity) {
            *act = Activity::Idle;
            // Clear stale work_target so idle farmers don't carry a phantom target.
            // worksite is NOT cleared here — decision_system owns occupancy
            // release via entity_map and handles it before setting Returning.
            ws.worksite = None;
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::MarkVisualDirty { idx: slot.0 }));
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

/// Find a farm for a farmer using cell-ring expansion with kind-filtered spatial index.
/// Preference order (min-order tuple):
/// 1) Ready farms first (ready=0, not_ready=1)
use crate::systems::work_targeting::find_farm_target as find_farmer_farm_target;

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
    queue: &mut PathRequestQueue,
    entity: Entity,
    x: f32,
    y: f32,
    priority: MovementPriority,
    source: &'static str,
) {
    queue.submit(entity, Vec2::new(x, y), priority, source);
}

/// Submit a movement intent with a deterministic scatter offset around a center point.
#[inline]
fn submit_intent_scattered(
    queue: &mut PathRequestQueue,
    entity: Entity,
    center_x: f32,
    center_y: f32,
    scatter: f32,
    idx: usize,
    frame: usize,
    priority: MovementPriority,
    source: &'static str,
) {
    let ox = (pseudo_random(idx, frame + 5) - 0.5) * scatter;
    let oy = (pseudo_random(idx, frame + 6) - 0.5) * scatter;
    queue.submit(entity, Vec2::new(center_x + ox, center_y + oy), priority, source);
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
    mut intents: ResMut<PathRequestQueue>,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    mut extras: DecisionExtras,
    npc_config: Res<crate::resources::NpcDecisionConfig>,
    mut entity_map: ResMut<EntityMap>,
    mut npc_state: DecisionNpcState,
    mut npc_data: NpcDataQueries,
    decision_npc_q: Query<
        (Entity, &GpuSlot, &Job, &TownId, &Faction),
        (Without<Building>, Without<Dead>),
    >,
) {
    if game_time.is_paused() {
        return;
    }

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
    // Adaptive bucket count — scales with population to cap per-frame decisions
    let npc_count = entity_map.npc_count();
    let interval_buckets = (npc_config.interval * 60.0) as usize;
    let min_buckets = npc_count / npc_config.max_decisions_per_frame.max(1);
    let think_buckets = interval_buckets.max(min_buckets).max(1);

    for (entity, slot, job, town_id, faction) in decision_npc_q.iter() {
        let idx = slot.0;

        // ====================================================================
        // Top-of-loop bucket gate: only process NPCs on their think cadence.
        // Fighting NPCs use a tighter bucket for responsive flee/leash.
        // ====================================================================
        const COMBAT_BUCKET: usize = 16; // ~267ms at 60fps
        let combat_state_peek = npc_state
            .combat_state_q
            .get(entity)
            .map(|cs| cs.is_fighting())
            .unwrap_or(false);
        if combat_state_peek {
            if (idx + frame) % COMBAT_BUCKET != 0 {
                continue;
            }
        } else {
            if (idx + frame) % think_buckets != 0 {
                continue;
            }
        }

        // Full component reads — only for NPCs that passed the bucket gate
        let job = *job;
        let town_idx_i32 = town_id.0;
        let faction_i32 = faction.0;
        let mut energy = npc_state.energy_q.get(entity).map(|e| e.0).unwrap_or(100.0);
        let health = npc_state.health_q.get(entity).map(|h| h.0).unwrap_or(100.0);
        let home = npc_data
            .home_q
            .get(entity)
            .map(|h| h.0)
            .unwrap_or(Vec2::ZERO);
        let personality = npc_data
            .personality_q
            .get(entity)
            .cloned()
            .unwrap_or_default();
        let mut activity = npc_state
            .activity_q
            .get(entity)
            .map(|a| a.clone())
            .unwrap_or_default();
        let mut combat_state = npc_state
            .combat_state_q
            .get(entity)
            .map(|cs| cs.clone())
            .unwrap_or_default();
        let mut at_destination = npc_state
            .npc_flags_q
            .get(entity)
            .map(|f| f.at_destination)
            .unwrap_or(false);
        let squad_id = npc_state.squad_id_q.get(entity).ok().map(|s| s.0);
        let manual_target = npc_state.manual_target_q.get(entity).ok().cloned();
        let direct_control = npc_state
            .npc_flags_q
            .get(entity)
            .map(|f| f.direct_control)
            .unwrap_or(false);
        let max_hp = npc_state
            .cached_stats_q
            .get(entity)
            .map(|s| s.max_health)
            .unwrap_or(100.0);
        let leash_range_val = npc_data.leash_range_q.get(entity).ok().map(|lr| lr.0);
        let work_state = npc_data
            .work_state_q
            .get(entity)
            .ok()
            .copied()
            .unwrap_or_default();
        let mut worksite = work_state
            .worksite
            .and_then(|uid| entity_map.slot_for_uid(uid));
        let has_patrol = npc_data.patrol_route_q.get(entity).is_ok();
        let mut patrol_current = npc_data
            .patrol_route_q
            .get(entity)
            .ok()
            .map(|r| r.current)
            .unwrap_or(0);
        let npc_pos = if idx * 2 + 1 < positions.len() {
            Some(Vec2::new(positions[idx * 2], positions[idx * 2 + 1]))
        } else {
            None
        };
        let mut carried_loot = npc_data
            .carried_loot_q
            .get(entity)
            .map(|cl| cl.clone())
            .unwrap_or_default();

        // Capture originals for conditional writeback
        let orig_activity = std::mem::discriminant(&activity);
        let orig_visual_key = (activity.visual_key(), carried_loot.visual_key());
        let orig_energy = energy;
        let orig_combat_state = std::mem::discriminant(&combat_state);
        let orig_at_destination = at_destination;
        let orig_worksite = worksite;
        let orig_patrol_current = patrol_current;

        npc_logs.set_slot_faction(idx, faction_i32);
        let max_hp = if max_hp > 0.0 { max_hp } else { 100.0 };

        // When true, skip NpcWorkState write-back (resolver owns it for this NPC this frame)
        let mut worksite_deferred = false;

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

            // ====================================================================
            // Priority 0: AtDestination -> Handle arrival transition
            // ====================================================================
            if at_destination {
                at_destination = false;

                match &activity {
                    Activity::Patrolling => {
                        // Squad rest: tired squad members go home instead of entering OnDuty
                        if let Some(sid) = squad_id {
                            if let Some(squad) = squad_state.squads.get(sid as usize) {
                                if squad.rest_when_tired
                                    && energy < ENERGY_TIRED_THRESHOLD
                                    && home != Vec2::ZERO
                                {
                                    activity = Activity::GoingToRest;
                                    submit_intent(
                                        &mut intents,
                                        entity,
                                        home.x,
                                        home.y,
                                        MovementPriority::Survival,
                                        "arrival:squad_rest",
                                    );
                                    npc_logs.push(
                                        idx,
                                        game_time.day(),
                                        game_time.hour(),
                                        game_time.minute(),
                                        "Tired -> Rest (squad)",
                                    );
                                    break 'decide;
                                }
                            }
                        }
                        activity = Activity::OnDuty { ticks_waiting: 0 };
                        // Scatter near waypoint so guards don't stack on the exact same spot
                        if let Ok(route) = npc_data.patrol_route_q.get(entity) {
                            if let Some(post) = route.posts.get(patrol_current) {
                                submit_intent_scattered(
                                    &mut intents, entity, post.x, post.y, 128.0,
                                    idx, patrol_current, MovementPriority::JobRoute, "onduty:scatter",
                                );
                            }
                        }
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            "-> OnDuty",
                        );
                    }
                    Activity::GoingToRest => {
                        activity = Activity::Resting;
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            "-> Resting",
                        );
                    }
                    Activity::GoingToHeal => {
                        let town_idx = town_idx_i32 as usize;
                        let threshold = policies
                            .policies
                            .get(town_idx)
                            .map(|p| p.recovery_hp)
                            .unwrap_or(0.8);
                        activity = Activity::HealingAtFountain {
                            recover_until: threshold,
                        };
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            "-> Healing",
                        );
                    }
                    Activity::GoingToWork => {
                        // Farmers: find farm at work_target and start working
                        if job == Job::Farmer {
                            let current_pos = npc_pos.unwrap_or(home);
                            let target_farm = worksite
                                .and_then(|slot| {
                                    entity_map
                                        .get_instance(slot)
                                        .filter(|inst| inst.kind == BuildingKind::Farm && inst.town_idx == town_idx_i32 as u32)
                                        .map(|inst| (slot, inst.position))
                                })
                                .or_else(|| {
                                    find_within_radius(current_pos, &entity_map, BuildingKind::Farm, FARM_ARRIVAL_RADIUS, town_idx_i32 as u32)
                                });

                            if let Some((farm_slot, farm_pos)) = target_farm {
                                let occ = entity_map.occupant_count(farm_slot);
                                let owns = worksite == Some(farm_slot);
                                let occupied_by_other = if owns { occ > 1 } else { occ >= 1 };

                                if occupied_by_other {
                                    // Farm occupied — retarget via resolver
                                    let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                                    extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                                    extras.work_intents.write(WorkIntentMsg(WorkIntent::Claim {
                                        entity, kind: BuildingKind::Farm, town_idx: town_idx_i32 as u32, from: current_pos,
                                    }));
                                    worksite = None;
                                    worksite_deferred = true;
                                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Farm occupied → retarget");
                                } else if entity_map.get_instance(farm_slot).is_some() {
                                    // Check if farm ready for harvest
                                    let harvest = entity_map.get_instance_mut(farm_slot).and_then(|inst| {
                                        let food = inst.harvest();
                                        if food > 0 { Some((food, inst.harvest_log_msg(food))) } else { None }
                                    });
                                    if let Some((food, log_msg)) = harvest {
                                        // Harvest — release worksite, carry home
                                        let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                                        extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                                        worksite = None;
                                        worksite_deferred = true;
                                        combat_log.write(CombatLogMsg {
                                            kind: CombatEventKind::Harvest, faction: faction_i32,
                                            day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(),
                                            message: log_msg, location: None,
                                        });
                                        carried_loot.food += food;
                                        activity = Activity::Returning;
                                        submit_intent(&mut intents, entity, home.x, home.y, MovementPriority::JobRoute, "arrival:farm_harvest_return");
                                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Harvested → Returning");
                                    } else {
                                        // Farm not ready — claim via resolver if not already owned, start working
                                        if !owns {
                                            let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                                            extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                                            extras.work_intents.write(WorkIntentMsg(WorkIntent::Claim {
                                                entity, kind: BuildingKind::Farm, town_idx: town_idx_i32 as u32, from: current_pos,
                                            }));
                                            worksite = None;
                                            worksite_deferred = true;
                                        }
                                        activity = Activity::Working;
                                        pop_inc_working(&mut economy.pop_stats, job, town_idx_i32);
                                        submit_intent(&mut intents, entity, farm_pos.x, farm_pos.y, MovementPriority::JobRoute, "arrival:farm_work");
                                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Working (tending)");
                                    }
                                }
                            } else {
                                // No farm nearby — release and idle
                                let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                                extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                                worksite = None;
                                worksite_deferred = true;
                                activity = Activity::Idle;
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "No farm nearby → Idle");
                            }
                        } else {
                            let current_pos = npc_pos.unwrap_or(Vec2::ZERO);
                            activity = Activity::Working;
                            pop_inc_working(&mut economy.pop_stats, job, town_idx_i32);
                            submit_intent(
                                &mut intents,
                                entity,
                                current_pos.x,
                                current_pos.y,
                                MovementPriority::JobRoute,
                                "arrival:work_hold",
                            );
                            npc_logs.push(
                                idx,
                                game_time.day(),
                                game_time.hour(),
                                game_time.minute(),
                                "-> Working",
                            );
                        }
                    }
                    Activity::Raiding { .. } => {
                        // Raider arrived at farm - check if ready to steal
                        if let Some(pos) = npc_pos {
                            let ready_farm_pos = find_location_within_radius(
                                pos,
                                &entity_map,
                                LocationKind::Farm,
                                FARM_ARRIVAL_RADIUS,
                            )
                            .and_then(|(_, fp)| {
                                entity_map
                                    .find_farm_at(fp)
                                    .filter(|i| i.growth_ready)
                                    .map(|_| fp)
                            });

                            if let Some(fp) = ready_farm_pos {
                                let food = entity_map
                                    .find_farm_at_mut(fp)
                                    .map(|i| {
                                        let f = i.harvest();
                                        if f > 0 {
                                            combat_log.write(CombatLogMsg {
                                                kind: CombatEventKind::Harvest,
                                                faction: faction_i32,
                                                day: game_time.day(),
                                                hour: game_time.hour(),
                                                minute: game_time.minute(),
                                                message: i.harvest_log_msg(f),
                                                location: None,
                                            });
                                        }
                                        f
                                    })
                                    .unwrap_or(0);

                                carried_loot.food += food.max(1);
                                activity = Activity::Returning;
                                submit_intent(
                                    &mut intents,
                                    entity,
                                    home.x,
                                    home.y,
                                    MovementPriority::JobRoute,
                                    "arrival:raid_return",
                                );
                                npc_logs.push(
                                    idx,
                                    game_time.day(),
                                    game_time.hour(),
                                    game_time.minute(),
                                    "Stole food -> Returning",
                                );
                            } else {
                                // Farm not ready - find a different farm nearby (exclude current one)
                                let raid_search_radius =
                                    entity_map.spatial_cell_size().max(256.0) * 8.0;
                                let mut best_d2 = f32::MAX;
                                let mut other_farm_pos: Option<Vec2> = None;
                                entity_map.for_each_nearby(pos, raid_search_radius, |f| {
                                    if f.kind != BuildingKind::Farm {
                                        return;
                                    }
                                    if f.position.distance(pos) <= FARM_ARRIVAL_RADIUS {
                                        return;
                                    }
                                    let d2 = f.position.distance_squared(pos);
                                    if d2 < best_d2 {
                                        best_d2 = d2;
                                        other_farm_pos = Some(f.position);
                                    }
                                });
                                if let Some(farm_pos) = other_farm_pos {
                                    activity = Activity::Raiding { target: farm_pos };
                                    submit_intent(
                                        &mut intents,
                                        entity,
                                        farm_pos.x,
                                        farm_pos.y,
                                        MovementPriority::JobRoute,
                                        "arrival:raid_retarget",
                                    );
                                    npc_logs.push(
                                        idx,
                                        game_time.day(),
                                        game_time.hour(),
                                        game_time.minute(),
                                        "Farm not ready, seeking another",
                                    );
                                } else {
                                    activity = Activity::Returning;
                                    submit_intent(
                                        &mut intents,
                                        entity,
                                        home.x,
                                        home.y,
                                        MovementPriority::JobRoute,
                                        "arrival:raid_no_target_return",
                                    );
                                    npc_logs.push(
                                        idx,
                                        game_time.day(),
                                        game_time.hour(),
                                        game_time.minute(),
                                        "No other farms, returning",
                                    );
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
                                let town_levels =
                                    extras.town_upgrades.town_levels(town_idx_i32 as usize);
                                let yield_mult = UPGRADES.stat_mult(
                                    &town_levels,
                                    "Miner",
                                    UpgradeStatKind::Yield,
                                );
                                let base_gold = inst.harvest();
                                if base_gold > 0 {
                                    combat_log.write(CombatLogMsg {
                                        kind: CombatEventKind::Harvest,
                                        faction: faction_i32,
                                        day: game_time.day(),
                                        hour: game_time.hour(),
                                        minute: game_time.minute(),
                                        message: inst.harvest_log_msg(base_gold),
                                        location: None,
                                    });
                                }
                                let gold_amount = ((base_gold as f32) * yield_mult).round() as i32;
                                carried_loot.gold += gold_amount;
                                let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                                extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                                worksite = None;
                                worksite_deferred = true;
                                activity = Activity::Returning;
                                submit_intent(
                                    &mut intents,
                                    entity,
                                    home.x,
                                    home.y,
                                    MovementPriority::JobRoute,
                                    "arrival:mine_harvest_return",
                                );
                                npc_logs.push(
                                    idx,
                                    game_time.day(),
                                    game_time.hour(),
                                    game_time.minute(),
                                    format!("Harvested {} gold -> Returning", gold_amount),
                                );
                            } else if mine_slot.is_some() {
                                // Mine still growing — claim via resolver and start tending
                                extras.work_intents.write(WorkIntentMsg(WorkIntent::Claim {
                                    entity, kind: BuildingKind::GoldMine, town_idx: town_idx_i32 as u32, from: mine_pos,
                                }));
                                worksite_deferred = true;
                                activity = Activity::MiningAtMine;
                                pop_inc_working(&mut economy.pop_stats, job, town_idx_i32);
                                submit_intent(
                                    &mut intents,
                                    entity,
                                    mine_pos.x,
                                    mine_pos.y,
                                    MovementPriority::JobRoute,
                                    "idle:work_mine",
                                );
                                npc_logs.push(
                                    idx,
                                    game_time.day(),
                                    game_time.hour(),
                                    game_time.minute(),
                                    "-> MiningAtMine (tending)",
                                );
                            }
                        } else {
                            activity = Activity::Idle;
                            npc_logs.push(
                                idx,
                                game_time.day(),
                                game_time.hour(),
                                game_time.minute(),
                                "No mine nearby -> Idle",
                            );
                        }
                    }
                    Activity::Wandering => {
                        activity = Activity::Idle;
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            "-> Idle",
                        );
                    }
                    Activity::Returning => {
                        // May have arrived at wrong place (e.g. after DC removal) — redirect home
                        if home != Vec2::ZERO {
                            submit_intent(
                                &mut intents,
                                entity,
                                home.x,
                                home.y,
                                MovementPriority::JobRoute,
                                "arrival:return_redirect",
                            );
                        } else {
                            activity = Activity::Idle;
                        }
                    }
                    _ => {}
                }

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
                            submit_intent(
                                &mut intents,
                                entity,
                                home.x,
                                home.y,
                                MovementPriority::Survival,
                                "squad:rest_gate",
                            );
                        }
                        break 'decide;
                    }
                }
            }

            // ====================================================================
            // Priority 1-3: Combat decisions (flee/leash/skip)
            // Runs BEFORE transit skip so fighting NPCs in transit (e.g. Raiding)
            // can still flee or leash back. Bucket-gated at COMBAT_BUCKET (16 frames).
            // ====================================================================
            if combat_state.is_fighting() {
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
                    Job::Fighter | Job::Boat => 0.0,
                };
                // Personality modifies flee threshold (Brave: never flees, Coward: flees sooner)
                let flee_mods = personality.get_behavior_mods();
                let flee_pct = if flee_mods.never_flees {
                    0.0
                } else {
                    (flee_pct + flee_mods.flee_threshold_add).clamp(0.0, 1.0)
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
                        // Clean up work state if fleeing mid-work
                        if matches!(activity, Activity::MiningAtMine | Activity::Working) {
                            let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                            extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                            worksite = None;
                            worksite_deferred = true;
                        }
                        combat_state = CombatState::None;
                        if !matches!(activity, Activity::Returning) {
                            activity = Activity::Returning;
                        }
                        submit_intent(
                            &mut intents,
                            entity,
                            home.x,
                            home.y,
                            MovementPriority::Survival,
                            "combat:flee_home",
                        );
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            "Fled combat",
                        );
                        break 'decide;
                    }
                }

                // Wounded + healing policy should override leash/return behavior.
                let healing_policy_active =
                    policies.policies.get(town_idx_usize).is_some_and(|p| {
                        p.prioritize_healing && energy > 0.0 && health / max_hp < p.recovery_hp
                    });
                if healing_policy_active {
                    if let Some(town) = farms.world.towns.get(town_idx_usize) {
                        combat_state = CombatState::None;
                        if !matches!(
                            activity,
                            Activity::GoingToHeal | Activity::HealingAtFountain { .. }
                        ) {
                            activity = Activity::GoingToHeal;
                            submit_intent_scattered(
                                &mut intents, entity, town.center.x, town.center.y, 128.0,
                                idx, frame, MovementPriority::Survival, "combat:heal_fountain",
                            );
                            npc_logs.push(
                                idx,
                                game_time.day(),
                                game_time.hour(),
                                game_time.minute(),
                                "Combat: wounded -> Fountain",
                            );
                        }
                        break 'decide;
                    }
                }

                // Priority 2: Should leash? (per-entity LeashRange or policy archer_leash)
                let should_leash = match job {
                    Job::Archer | Job::Crossbow => policies
                        .policies
                        .get(town_idx_usize)
                        .is_none_or(|p| p.archer_leash),
                    _ => leash_range_val.is_some(),
                };
                if should_leash {
                    let leash_dist = leash_range_val.unwrap_or(400.0);
                    if let CombatState::Fighting { origin } = &combat_state {
                        if let Some(pos) = npc_pos {
                            let dx = pos.x - origin.x;
                            let dy = pos.y - origin.y;
                            if (dx * dx + dy * dy).sqrt() > leash_dist {
                                if matches!(activity, Activity::MiningAtMine | Activity::Working) {
                                    let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                                    extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                                    worksite = None;
                                    worksite_deferred = true;
                                }
                                combat_state = CombatState::None;
                                if !matches!(activity, Activity::Returning) {
                                    activity = Activity::Returning;
                                }
                                submit_intent(
                                    &mut intents,
                                    entity,
                                    home.x,
                                    home.y,
                                    MovementPriority::Survival,
                                    "combat:leash_home",
                                );
                                npc_logs.push(
                                    idx,
                                    game_time.day(),
                                    game_time.hour(),
                                    game_time.minute(),
                                    "Leashed -> Returning",
                                );
                                break 'decide;
                            }
                        }
                    }
                }

                // Priority 3: Still in combat, attack_system handles targeting
                break 'decide;
            }

            // ====================================================================
            // Squad sync: apply squad target/patrol policy changes immediately
            // (before transit skip) so squad members react by next decision tick.
            // Covers all squad-assigned units: archers, crossbow, raiders, fighters.
            // ====================================================================
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
                                submit_intent(
                                    &mut intents,
                                    entity,
                                    home.x,
                                    home.y,
                                    MovementPriority::Survival,
                                    "squad:rest_home",
                                );
                            }
                            break 'decide;
                        }
                        // Wounded: prioritize healing over squad target (prevents flee-engage oscillation)
                        let ti = town_idx_i32 as usize;
                        if let Some(p) = policies.policies.get(ti) {
                            if p.prioritize_healing
                                && energy > 0.0
                                && health / max_hp < p.recovery_hp
                            {
                                if !matches!(
                                    activity,
                                    Activity::GoingToHeal | Activity::HealingAtFountain { .. }
                                ) {
                                    if let Some(town) = farms.world.towns.get(ti) {
                                        combat_state = CombatState::None;
                                        activity = Activity::GoingToHeal;
                                        submit_intent_scattered(
                                            &mut intents, entity, town.center.x, town.center.y, 128.0,
                                            idx, frame, MovementPriority::Survival, "squad:heal_fountain",
                                        );
                                        npc_logs.push(
                                            idx,
                                            game_time.day(),
                                            game_time.hour(),
                                            game_time.minute(),
                                            "Squad: wounded -> Fountain",
                                        );
                                    }
                                }
                                break 'decide;
                            }
                        }
                        // Squad target — only redirect when needed (no per-frame GPU writes)
                        match activity {
                            Activity::OnDuty { .. } => {
                                // At a position — redirect only if squad target moved
                                if let Some(pos) = npc_pos {
                                    let dx = pos.x - target.x;
                                    let dy = pos.y - target.y;
                                    if dx * dx + dy * dy > 100.0 * 100.0 {
                                        activity = Activity::Patrolling;
                                        submit_intent(
                                            &mut intents,
                                            entity,
                                            target.x,
                                            target.y,
                                            MovementPriority::Squad,
                                            "squad:target_rejoin",
                                        );
                                    }
                                }
                            }
                            Activity::Patrolling
                            | Activity::Raiding { .. }
                            | Activity::GoingToRest
                            | Activity::Resting
                            | Activity::GoingToHeal
                            | Activity::HealingAtFountain { .. }
                            | Activity::Returning => {
                                // Already heading to target, resting, healing, or carrying loot — no redirect
                            }
                            _ => {
                                // Idle/Wandering/Returning/other — redirect to squad target
                                activity = Activity::Patrolling;
                                submit_intent(
                                    &mut intents,
                                    entity,
                                    target.x,
                                    target.y,
                                    MovementPriority::Squad,
                                    "squad:target_assign",
                                );
                            }
                        }
                    } else if !squad.patrol_enabled {
                        // No target + patrol disabled: stop and wait (gathering phase)
                        if matches!(
                            activity,
                            Activity::Patrolling
                                | Activity::OnDuty { .. }
                                | Activity::Raiding { .. }
                        ) {
                            activity = Activity::Idle;
                            if let Some(pos) = npc_pos {
                                submit_intent(
                                    &mut intents,
                                    entity,
                                    pos.x,
                                    pos.y,
                                    MovementPriority::Squad,
                                    "squad:hold_position",
                                );
                            }
                        }
                    }
                }
            }

            // ====================================================================
            // Farmer en-route retarget: if target farm became occupied, find another
            // ====================================================================
            if job == Job::Farmer
                && matches!(activity, Activity::GoingToWork)
                && (idx + frame) % think_buckets == 0
            {
                if let Some(wp) = worksite {
                    let occ = entity_map.occupant_count(wp);
                    let occupied_by_other = (occ > 1) || (occ >= 1 && worksite != Some(wp));
                    if occupied_by_other {
                        // Retarget via resolver — release old + claim new
                        let current_pos = npc_pos.unwrap_or_default();
                        let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                        extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                        extras.work_intents.write(WorkIntentMsg(WorkIntent::Claim {
                            entity, kind: BuildingKind::Farm, town_idx: town_idx_i32 as u32, from: current_pos,
                        }));
                        worksite = None;
                        worksite_deferred = true;
                        // Activity stays GoingToWork — resolver will submit path or set Idle
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Farm taken → retarget");
                        break 'decide;
                    }
                }
            }

            // ====================================================================
            // Skip NPCs in transit states (they're walking to their destination)
            // GoingToHeal proximity check: bucket gate ensures this runs on cadence.
            // ====================================================================
            if activity.is_transit() {
                // Stuck-transit redirect: casual transit NPCs that haven't arrived get new scatter
                if (idx + frame) % think_buckets == 0 {
                    match &activity {
                        Activity::Wandering => {
                            if let Some(pos) = npc_pos {
                                let offset_x = (pseudo_random(idx, frame + 3) - 0.5) * 128.0;
                                let offset_y = (pseudo_random(idx, frame + 4) - 0.5) * 128.0;
                                let mut target = Vec2::new(pos.x + offset_x, pos.y + offset_y);
                                if home != Vec2::ZERO {
                                    let diff = target - home;
                                    let dist = diff.length();
                                    if dist > 200.0 { target = home + diff * (200.0 / dist); }
                                }
                                submit_intent(&mut intents, entity, target.x, target.y,
                                    MovementPriority::Wander, "wander:redirect");
                            }
                            break 'decide;
                        }
                        Activity::Patrolling => {
                            if let Ok(route) = npc_data.patrol_route_q.get(entity) {
                                if !route.posts.is_empty() {
                                    let safe_idx = patrol_current % route.posts.len();
                                    if let Some(post) = route.posts.get(safe_idx) {
                                        submit_intent_scattered(
                                            &mut intents, entity, post.x, post.y, 128.0,
                                            idx, frame, MovementPriority::JobRoute, "patrol:redirect",
                                        );
                                    }
                                }
                            }
                            break 'decide;
                        }
                        _ => {}
                    }
                }

                // Early arrival: GoingToHeal NPCs stop once inside healing range
                if matches!(activity, Activity::GoingToHeal) {
                    let town_idx = town_idx_i32 as usize;
                    if let Some(town) = farms.world.towns.get(town_idx) {
                        if let Some(current) = npc_pos {
                            if current.distance(town.center) <= HEAL_DRIFT_RADIUS {
                                let threshold = policies
                                    .policies
                                    .get(town_idx)
                                    .map(|p| p.recovery_hp)
                                    .unwrap_or(0.8);
                                activity = Activity::HealingAtFountain {
                                    recover_until: threshold,
                                };
                                npc_logs.push(
                                    idx,
                                    game_time.day(),
                                    game_time.hour(),
                                    game_time.minute(),
                                    "-> Healing",
                                );
                            }
                        }
                    }
                }
                break 'decide;
            }

            // (Tier 3 bucket gate removed — gating now happens at top of loop)

            // ====================================================================
            // Priority 4a: HealingAtFountain? -> Wake when HP recovered
            // ====================================================================
            if let Activity::HealingAtFountain { recover_until } = &activity {
                if health / max_hp >= *recover_until {
                    activity = Activity::Idle;
                    npc_logs.push(
                        idx,
                        game_time.day(),
                        game_time.hour(),
                        game_time.minute(),
                        "Recovered",
                    );
                    // Fall through to make a decision
                } else {
                    // Drift check: separation physics pushes NPCs out of healing range
                    let town_idx = town_idx_i32 as usize;
                    if let Some(town) = farms.world.towns.get(town_idx) {
                        if let Some(current) = npc_pos {
                            if current.distance(town.center) > HEAL_DRIFT_RADIUS {
                                submit_intent_scattered(
                                    &mut intents, entity, town.center.x, town.center.y, 128.0,
                                    idx, frame, MovementPriority::Survival, "heal:drift_retarget",
                                );
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
                    npc_logs.push(
                        idx,
                        game_time.day(),
                        game_time.hour(),
                        game_time.minute(),
                        "Woke up",
                    );
                    // Fall through to make a decision
                } else {
                    break 'decide; // still resting
                }
            }

            // ====================================================================
            // Priority 4c: Loot threshold — too much equipment, return home
            // ====================================================================
            if !npc_def(job).equip_slots.is_empty()
                && carried_loot.equipment.len() >= crate::constants::LOOT_CARRY_THRESHOLD
                && !matches!(activity, Activity::Returning)
                && matches!(combat_state, CombatState::None)
            {
                let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                worksite = None;
                worksite_deferred = true;
                activity = Activity::Returning;
                intents.submit(
                    entity,
                    home,
                    MovementPriority::JobRoute,
                    "loot:threshold",
                );
                npc_logs.push(
                    idx,
                    game_time.day(),
                    game_time.hour(),
                    game_time.minute(),
                    format!(
                        "Carrying {} equipment, returning home",
                        carried_loot.equipment.len()
                    ),
                );
                break 'decide;
            }

            // ====================================================================
            // Priority 5: Working/Mining + tired?
            // ====================================================================
            // Priority 5: Working at worksite (farm or mine)
            let worksite_slot = if matches!(activity, Activity::Working | Activity::MiningAtMine) {
                worksite
            } else {
                None
            };
            if let Some(slot) = worksite_slot {
                // Look up worksite config from building registry
                let inst_snapshot = entity_map.get_instance(slot).map(|i| (i.kind, i.town_idx));
                let Some((kind, inst_town)) = inst_snapshot else {
                    // Worksite destroyed — release and reassign
                    let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                    extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                    worksite = None;
                    worksite_deferred = true;
                    activity = Activity::Idle;
                    npc_logs.push(
                        idx,
                        game_time.day(),
                        game_time.hour(),
                        game_time.minute(),
                        "Worksite destroyed -> Idle",
                    );
                    break 'decide;
                };
                let def = building_def(kind);
                let Some(ws) = def.worksite else {
                    let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                    extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                    worksite = None;
                    worksite_deferred = true;
                    activity = Activity::Idle;
                    break 'decide;
                };

                // Validate: town match (only for town-scoped worksites like farms)
                if ws.town_scoped && inst_town != town_idx_i32 as u32 {
                    let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                    extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                    worksite = None;
                    worksite_deferred = true;
                    activity = Activity::Idle;
                    npc_logs.push(
                        idx,
                        game_time.day(),
                        game_time.hour(),
                        game_time.minute(),
                        "Worksite wrong town -> Idle",
                    );
                    break 'decide;
                }

                // Contention: too many occupants → release and go home
                if entity_map.occupant_count(slot) > ws.max_occupants {
                    let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                    extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                    worksite = None;
                    worksite_deferred = true;
                    activity = Activity::Idle;
                    if home != Vec2::ZERO {
                        submit_intent(
                            &mut intents,
                            entity,
                            home.x,
                            home.y,
                            MovementPriority::JobRoute,
                            "worksite:contention_home",
                        );
                    }
                    npc_logs.push(
                        idx,
                        game_time.day(),
                        game_time.hour(),
                        game_time.minute(),
                        "Worksite contention -> Reassign",
                    );
                    break 'decide;
                }

                // Claim repair: if we don't have a claim, try to get one via resolver
                if worksite.is_none() {
                    let current_pos = npc_pos.unwrap_or_default();
                    extras.work_intents.write(WorkIntentMsg(WorkIntent::Claim {
                        entity, kind, town_idx: town_idx_i32 as u32, from: current_pos,
                    }));
                    worksite_deferred = true;
                    // If resolver fails to claim, it sets Activity::Idle
                }

                // Drift check: push NPC back to worksite if too far
                let ws_pos = entity_map
                    .get_instance(slot)
                    .map(|i| i.position)
                    .unwrap_or_default();
                if let Some(current) = npc_pos {
                    if current.distance(ws_pos) > ws.drift_radius {
                        submit_intent(
                            &mut intents,
                            entity,
                            ws_pos.x,
                            ws_pos.y,
                            MovementPriority::JobRoute,
                            "worksite:drift",
                        );
                    }
                }

                // Harvest check: if growth_ready, harvest + apply yield mult + return home
                let mut harvested = false;
                if let Some(inst) = entity_map.get_instance_mut(slot) {
                    if inst.growth_ready {
                        let town_levels =
                            extras.town_upgrades.town_levels(town_idx_i32 as usize);
                        let yield_mult =
                            UPGRADES.stat_mult(&town_levels, ws.upgrade_job, UpgradeStatKind::Yield);
                        let base_yield = inst.harvest();
                        if base_yield > 0 {
                            combat_log.write(CombatLogMsg {
                                kind: CombatEventKind::Harvest,
                                faction: faction_i32,
                                day: game_time.day(),
                                hour: game_time.hour(),
                                minute: game_time.minute(),
                                message: inst.harvest_log_msg(base_yield),
                                location: None,
                            });
                        }
                        let final_yield = ((base_yield as f32) * yield_mult).round() as i32;
                        let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                        extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                        worksite = None;
                        worksite_deferred = true;
                        match ws.harvest_item {
                            ItemKind::Food => carried_loot.food += final_yield,
                            ItemKind::Gold => carried_loot.gold += final_yield,
                        }
                        activity = Activity::Returning;
                        submit_intent(
                            &mut intents,
                            entity,
                            home.x,
                            home.y,
                            MovementPriority::JobRoute,
                            "worksite:harvest_return",
                        );
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            format!("Harvested {} {} -> Returning", final_yield, def.label),
                        );
                        harvested = true;
                    }
                }
                if harvested {
                    break 'decide;
                }

                // Tired check: release worksite and go idle
                if energy < ENERGY_TIRED_THRESHOLD {
                    let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
                    extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
                    worksite = None;
                    worksite_deferred = true;
                    activity = Activity::Idle;
                    npc_logs.push(
                        idx,
                        game_time.day(),
                        game_time.hour(),
                        game_time.minute(),
                        "Tired -> Stopped working",
                    );
                }
                break 'decide;
            }

            // ====================================================================
            // Priority 6: OnDuty (tired -> leave post, else patrol when ready)
            // ====================================================================
            if let Activity::OnDuty { ticks_waiting } = &activity {
                let ticks = *ticks_waiting;
                let squad_forces_stay = job.is_patrol_unit()
                    && squad_id
                        .and_then(|sid| squad_state.squads.get(sid as usize))
                        .is_some_and(|s| !s.rest_when_tired);
                if energy < ENERGY_TIRED_THRESHOLD && !squad_forces_stay {
                    activity = Activity::Idle;
                    npc_logs.push(
                        idx,
                        game_time.day(),
                        game_time.hour(),
                        game_time.minute(),
                        "Tired -> Left post",
                    );
                    // Fall through to idle scoring — Rest will win
                } else {
                    let squad_patrol_enabled = squad_id
                        .and_then(|sid| squad_state.squads.get(sid as usize))
                        .is_none_or(|s| s.patrol_enabled);
                    let jitter = (idx % 30) as u32;
                    if ticks >= ARCHER_PATROL_WAIT + jitter && squad_patrol_enabled {
                        if let Ok(route) = npc_data.patrol_route_q.get(entity) {
                            if !route.posts.is_empty() {
                                patrol_current = (patrol_current + 1) % route.posts.len();
                                if let Some(post) = route.posts.get(patrol_current) {
                                    activity = Activity::Patrolling;
                                    submit_intent_scattered(
                                        &mut intents, entity, post.x, post.y, 128.0,
                                        idx, patrol_current, MovementPriority::JobRoute, "onduty:patrol_advance",
                                    );
                                    npc_logs.push(
                                        idx,
                                        game_time.day(),
                                        game_time.hour(),
                                        game_time.minute(),
                                        "-> Patrolling",
                                    );
                                }
                            }
                        }
                    }
                    break 'decide;
                }
            }

            // ====================================================================
            // Priority 8: Idle -> Score Eat/Rest/Work/Wander (policy-aware)
            // ====================================================================
            let en = energy;
            let behavior_mods = personality.get_behavior_mods();
            let rest_m = behavior_mods.rest;
            let eat_m = behavior_mods.eat;
            let work_m = behavior_mods.work;
            let wander_m = behavior_mods.wander;

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
                        submit_intent_scattered(
                            &mut intents, entity, center.x, center.y, 128.0,
                            idx, frame, MovementPriority::Survival, "idle:heal_fountain",
                        );
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            "Wounded -> Fountain",
                        );
                        break 'decide;
                    }
                }
            }

            if food_available && en < ENERGY_EAT_THRESHOLD {
                let eat_score = (ENERGY_EAT_THRESHOLD - en) * SCORE_EAT_MULT * eat_m;
                scores[score_count] = (Action::Eat, eat_score);
                score_count += 1;
            }

            if en < ENERGY_HUNGRY && home != Vec2::ZERO {
                let rest_score = (ENERGY_HUNGRY - en) * SCORE_REST_MULT * rest_m;
                scores[score_count] = (Action::Rest, rest_score);
                score_count += 1;
            }

            // Work schedule gate: per-job schedule
            let schedule = match job {
                Job::Farmer | Job::Miner => policy
                    .map(|p| p.farmer_schedule)
                    .unwrap_or(WorkSchedule::Both),
                Job::Archer | Job::Crossbow => policy
                    .map(|p| p.archer_schedule)
                    .unwrap_or(WorkSchedule::Both),
                _ => WorkSchedule::Both,
            };
            let work_allowed = match schedule {
                WorkSchedule::Both => true,
                WorkSchedule::DayOnly => game_time.is_daytime(),
                WorkSchedule::NightOnly => !game_time.is_daytime(),
            };

            let can_work = work_allowed
                && match job {
                    Job::Farmer => true, // dynamically find farms (same as Miner)
                    Job::Miner => true,  // miners always have work (find nearest mine dynamically)
                    Job::Archer | Job::Crossbow | Job::Fighter => has_patrol,
                    Job::Raider | Job::Boat => false, // squad-driven / non-behavioral
                };
            if can_work {
                let hp_pct = health / max_hp;
                let hp_mult = if hp_pct < 0.3 {
                    0.0
                } else {
                    (hp_pct - 0.3) * (1.0 / 0.7)
                };
                // Scale down work desire when tired so rest/eat can win before starvation
                let energy_factor = if en < ENERGY_TIRED_THRESHOLD {
                    en / ENERGY_TIRED_THRESHOLD
                } else {
                    1.0
                };
                let work_score = SCORE_WORK_BASE * work_m * hp_mult * energy_factor;
                if work_score > 0.0 {
                    scores[score_count] = (Action::Work, work_score);
                    score_count += 1;
                }
            }

            // Off-duty behavior when work is gated out by schedule
            if !work_allowed {
                let off_duty = match job {
                    Job::Farmer | Job::Miner => policy
                        .map(|p| p.farmer_off_duty)
                        .unwrap_or(OffDutyBehavior::GoToBed),
                    Job::Archer | Job::Crossbow => policy
                        .map(|p| p.archer_off_duty)
                        .unwrap_or(OffDutyBehavior::GoToBed),
                    _ => OffDutyBehavior::GoToBed,
                };
                match off_duty {
                    OffDutyBehavior::GoToBed => {
                        // Boost rest score so NPCs prefer going to bed
                        if home != Vec2::ZERO {
                            scores[score_count] = (Action::Rest, 80.0 * rest_m);
                            score_count += 1;
                        }
                    }
                    OffDutyBehavior::StayAtFountain => {
                        // Go to town center (fountain)
                        if let Some(town) = farms.world.towns.get(town_idx) {
                            let center = town.center;
                            activity = Activity::Wandering;
                            submit_intent(
                                &mut intents,
                                entity,
                                center.x,
                                center.y,
                                MovementPriority::Survival,
                                "offduty:fountain",
                            );
                            npc_logs.push(
                                idx,
                                game_time.day(),
                                game_time.hour(),
                                game_time.minute(),
                                "Off-duty -> Fountain",
                            );
                            break 'decide;
                        }
                    }
                    OffDutyBehavior::WanderTown => {
                        scores[score_count] = (Action::Wander, 80.0 * wander_m);
                        score_count += 1;
                    }
                }
            }

            scores[score_count] = (Action::Wander, SCORE_WANDER_BASE * wander_m);
            score_count += 1;

            let action = weighted_random(&scores[..score_count], idx, frame);
            npc_logs.push(
                idx,
                game_time.day(),
                game_time.hour(),
                game_time.minute(),
                format!("{:?} (e:{:.0} h:{:.0})", action, energy, health),
            );

            match action {
                Action::Eat => {
                    if town_idx < economy.food_storage.food.len()
                        && economy.food_storage.food[town_idx] > 0
                    {
                        let old_energy = energy;
                        economy.food_storage.food[town_idx] -= 1;
                        energy = 100.0;
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            format!("Ate (e:{:.0}->{:.0})", old_energy, energy),
                        );
                    }
                }
                Action::Rest => {
                    if home != Vec2::ZERO {
                        activity = Activity::GoingToRest;
                        submit_intent(
                            &mut intents,
                            entity,
                            home.x,
                            home.y,
                            MovementPriority::Survival,
                            "idle:rest_home",
                        );
                    }
                }
                Action::Work => {
                    match job {
                        Job::Farmer => {
                            let current_pos = npc_pos.unwrap_or(home);
                            // Probe for available farm (read-only); defer claim to resolver
                            if find_farmer_farm_target(current_pos, &entity_map, town_idx_i32 as u32).is_some() {
                                extras.work_intents.write(WorkIntentMsg(WorkIntent::Claim {
                                    entity,
                                    kind: BuildingKind::Farm,
                                    town_idx: town_idx_i32 as u32,
                                    from: current_pos,
                                }));
                                activity = Activity::GoingToWork;
                                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Farm claim → resolver");
                            } else {
                                // No available farm — clear stale target and wander
                                worksite = None;
                                let base = if home != Vec2::ZERO { home } else if let Some(pos) = npc_pos { pos } else { break 'decide; };
                                let offset_x = (pseudo_random(idx, frame + 1) - 0.5) * 200.0;
                                let offset_y = (pseudo_random(idx, frame + 2) - 0.5) * 200.0;
                                activity = Activity::Wandering;
                                submit_intent(&mut intents, entity, base.x + offset_x, base.y + offset_y, MovementPriority::Wander, "idle:wander_no_farm");
                            }
                        }
                        Job::Miner => {
                            // Check for manually assigned mine (via miner home UI)
                            let assigned = entity_map
                                .find_by_position(home)
                                .filter(|inst| inst.kind == BuildingKind::MinerHome)
                                .and_then(|inst| inst.assigned_mine);

                            let mine_target = if let Some(assigned_pos) = assigned {
                                Some(assigned_pos)
                            } else {
                                let current_pos = npc_pos.unwrap_or(home);
                                // Spatial cell-ring search: ready > unoccupied > occupied, then nearest
                                entity_map
                                    .find_nearest_worksite(
                                        current_pos,
                                        BuildingKind::GoldMine,
                                        town_idx_i32 as u32,
                                        crate::resources::WorksiteFallback::AnyTown,
                                        6400.0,
                                        |inst| {
                                            let priority = if inst.growth_ready {
                                                0u8
                                            } else if inst.occupants == 0 {
                                                1
                                            } else {
                                                2
                                            };
                                            Some((
                                                priority,
                                                inst.occupants as u16,
                                                inst.position
                                                    .distance_squared(current_pos)
                                                    .to_bits(),
                                            ))
                                        },
                                    )
                                    .map(|r| r.position)
                            };

                            if let Some(mine_pos) = mine_target {
                                activity = Activity::Mining { mine_pos };
                                submit_intent(
                                    &mut intents,
                                    entity,
                                    mine_pos.x,
                                    mine_pos.y,
                                    MovementPriority::JobRoute,
                                    "idle:work_mine",
                                );
                                npc_logs.push(
                                    idx,
                                    game_time.day(),
                                    game_time.hour(),
                                    game_time.minute(),
                                    "-> Mining gold",
                                );
                            }
                        }
                        Job::Archer | Job::Crossbow | Job::Fighter => {
                            // Squad override: go to squad target instead of patrolling
                            if let Some(sid) = squad_id {
                                if let Some(squad) = squad_state.squads.get(sid as usize) {
                                    if let Some(target) = squad.target {
                                        activity = Activity::Patrolling;
                                        submit_intent(
                                            &mut intents,
                                            entity,
                                            target.x,
                                            target.y,
                                            MovementPriority::Squad,
                                            "idle:squad_target",
                                        );
                                        npc_logs.push(
                                            idx,
                                            game_time.day(),
                                            game_time.hour(),
                                            game_time.minute(),
                                            format!("Squad {} -> target", sid + 1),
                                        );
                                        break 'decide;
                                    }
                                    if !squad.patrol_enabled {
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
                                        submit_intent_scattered(
                                            &mut intents, entity, post.x, post.y, 128.0,
                                            idx, patrol_current, MovementPriority::JobRoute, "idle:patrol_route",
                                        );
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
                                submit_intent(
                                    &mut intents,
                                    entity,
                                    home.x + offset_x,
                                    home.y + offset_y,
                                    MovementPriority::Wander,
                                    "idle:raider_wander",
                                );
                            }
                        }
                        Job::Boat => {} // CPU-driven movement, no behavior
                    }
                }
                Action::Wander => {
                    // Wander from current position, clamped to stay near home
                    let base = if let Some(pos) = npc_pos {
                        pos
                    } else if home != Vec2::ZERO {
                        home
                    } else {
                        break 'decide;
                    };
                    let offset_x = (pseudo_random(idx, frame + 1) - 0.5) * 128.0;
                    let offset_y = (pseudo_random(idx, frame + 2) - 0.5) * 128.0;
                    let mut target = Vec2::new(base.x + offset_x, base.y + offset_y);
                    if home != Vec2::ZERO {
                        let diff = target - home;
                        let dist = diff.length();
                        if dist > 200.0 { target = home + diff * (200.0 / dist); }
                    }
                    activity = Activity::Wandering;
                    submit_intent(
                        &mut intents,
                        entity,
                        target.x,
                        target.y,
                        MovementPriority::Wander,
                        "idle:wander",
                    );
                }
                Action::Fight | Action::Flee => {}
            }
        } // end 'decide block

        // Farmer reservation invariant:
        // a farm reservation may exist only while actively working or moving to work.
        // Release stale worksite if farmer isn't actively working or en-route.
        // Skip if we deferred to resolver this frame (it will handle cleanup).
        if !worksite_deferred
            && job == Job::Farmer
            && worksite.is_some()
            && !matches!(activity, Activity::Working | Activity::GoingToWork)
        {
            let uid = worksite.and_then(|s| entity_map.uid_for_slot(s));
            extras.work_intents.write(WorkIntentMsg(WorkIntent::Release { entity, uid }));
            worksite = None;
            worksite_deferred = true;
            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Released stale worksite");
        }

        // Conditional writeback: skip unchanged NPCs (most exit early via break 'decide)
        let new_visual_key = (activity.visual_key(), carried_loot.visual_key());
        if std::mem::discriminant(&activity) != orig_activity {
            // Clear stale GPU target when going Idle — prevents oscillation with nearby NPCs
            if matches!(activity, Activity::Idle) {
                if let Some(pos) = npc_pos {
                    intents.submit(entity, pos, MovementPriority::Wander, "idle:stop");
                }
            }
            if let Ok(mut act) = npc_state.activity_q.get_mut(entity) {
                *act = activity;
            }
        }
        // Write back carried loot if changed
        {
            let orig_cl = npc_data
                .carried_loot_q
                .get(entity)
                .ok();
            let changed = orig_cl.as_ref().map_or(true, |cl| cl.food != carried_loot.food || cl.gold != carried_loot.gold);
            if changed {
                if let Ok(mut cl) = npc_data.carried_loot_q.get_mut(entity) {
                    cl.food = carried_loot.food;
                    cl.gold = carried_loot.gold;
                }
            }
        }
        if new_visual_key != orig_visual_key {
            extras
                .gpu_updates
                .write(GpuUpdateMsg(GpuUpdate::MarkVisualDirty { idx }));
        }
        if at_destination != orig_at_destination {
            if let Ok(mut flags) = npc_state.npc_flags_q.get_mut(entity) {
                flags.at_destination = at_destination;
            }
        }
        if energy != orig_energy {
            if let Ok(mut en) = npc_state.energy_q.get_mut(entity) {
                en.0 = energy;
            }
        }
        if std::mem::discriminant(&combat_state) != orig_combat_state {
            if let Ok(mut cs) = npc_state.combat_state_q.get_mut(entity) {
                *cs = combat_state;
            }
        }
        if !worksite_deferred && worksite != orig_worksite {
            if let Ok(mut ws) = npc_data.work_state_q.get_mut(entity) {
                ws.worksite = worksite.and_then(|s| entity_map.uid_for_slot(s));
            }
        }
        if patrol_current != orig_patrol_current {
            if let Ok(mut route) = npc_data.patrol_route_q.get_mut(entity) {
                route.current = patrol_current;
            }
        }
    }
}

/// Increment OnDuty tick counters (runs every frame for guards at posts).
/// Separated from decision_system because we need mutable Activity access.
pub fn on_duty_tick_system(
    game_time: Res<GameTime>,
    mut q: Query<(&mut Activity, &CombatState), (Without<Building>, Without<Dead>)>,
) {
    if game_time.is_paused() {
        return;
    }
    for (mut activity, combat_state) in q.iter_mut() {
        if combat_state.is_fighting() {
            continue;
        }
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
    mut patrol_route_q: Query<&mut PatrolRoute>,
    mut commands: Commands,
    patrol_npc_q: Query<(Entity, &GpuSlot, &Job, &TownId), (Without<Building>, Without<Dead>)>,
) {
    if patrols_dirty.read().count() == 0 {
        return;
    }

    // Apply pending patrol order swap from UI
    if let Some(swap) = patrol_swaps.read().last() {
        let (sa, sb) = (swap.slot_a, swap.slot_b);
        let order_a = entity_map
            .get_instance(sa)
            .map(|i| i.patrol_order)
            .unwrap_or(0);
        let order_b = entity_map
            .get_instance(sb)
            .map(|i| i.patrol_order)
            .unwrap_or(0);
        if let Some(inst) = entity_map.get_instance_mut(sa) {
            inst.patrol_order = order_b;
        }
        if let Some(inst) = entity_map.get_instance_mut(sb) {
            inst.patrol_order = order_a;
        }
    }

    // Collect patrol unit slots + towns via ECS query
    let patrol_slots: Vec<(Entity, usize, i32)> = patrol_npc_q
        .iter()
        .filter(|(_, _, job, _)| job.is_patrol_unit())
        .map(|(entity, slot, _, town)| (entity, slot.0, town.0))
        .collect();

    // Build routes once per town (immutable entity_map access for building queries)
    let mut town_routes: std::collections::HashMap<u32, Vec<Vec2>> =
        std::collections::HashMap::new();
    for &(_, _, town_idx) in &patrol_slots {
        let tid = town_idx as u32;
        town_routes
            .entry(tid)
            .or_insert_with(|| crate::systems::spawn::build_patrol_route(&entity_map, tid));
    }

    // Write routes back via ECS
    for (entity, _slot, town_idx) in patrol_slots {
        let tid = town_idx as u32;
        let Some(new_posts) = town_routes.get(&tid) else {
            continue;
        };
        if new_posts.is_empty() {
            continue;
        }
        if let Ok(mut route) = patrol_route_q.get_mut(entity) {
            if route.current >= new_posts.len() {
                route.current = 0;
            }
            route.posts = new_posts.clone();
        } else {
            commands.entity(entity).insert(PatrolRoute {
                posts: new_posts.clone(),
                current: 0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Activity, Building, CombatState, Dead};
    use crate::resources::GameTime;
    use bevy::time::TimeUpdateStrategy;

    // ========================================================================
    // on_duty_tick_system tests
    // ========================================================================

    fn setup_on_duty_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(GameTime::default());
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_systems(FixedUpdate, on_duty_tick_system);
        app.update();
        app.update();
        app
    }

    #[test]
    fn on_duty_increments_ticks_waiting() {
        let mut app = setup_on_duty_app();
        let npc = app.world_mut().spawn((
            Activity::OnDuty { ticks_waiting: 0 },
            CombatState::None,
        )).id();

        app.update();
        let activity = app.world().get::<Activity>(npc).unwrap();
        if let Activity::OnDuty { ticks_waiting } = activity {
            assert!(*ticks_waiting > 0, "ticks_waiting should increment: {ticks_waiting}");
        } else {
            panic!("activity should still be OnDuty");
        }
    }

    #[test]
    fn on_duty_fighting_skipped() {
        let mut app = setup_on_duty_app();
        let npc = app.world_mut().spawn((
            Activity::OnDuty { ticks_waiting: 0 },
            CombatState::Fighting { origin: Vec2::ZERO },
        )).id();

        app.update();
        let activity = app.world().get::<Activity>(npc).unwrap();
        if let Activity::OnDuty { ticks_waiting } = activity {
            assert_eq!(*ticks_waiting, 0, "fighting NPCs should not increment ticks");
        } else {
            panic!("activity should still be OnDuty");
        }
    }

    #[test]
    fn on_duty_paused_no_change() {
        let mut app = setup_on_duty_app();
        app.world_mut().resource_mut::<GameTime>().paused = true;
        let npc = app.world_mut().spawn((
            Activity::OnDuty { ticks_waiting: 5 },
            CombatState::None,
        )).id();

        app.update();
        let activity = app.world().get::<Activity>(npc).unwrap();
        if let Activity::OnDuty { ticks_waiting } = activity {
            assert_eq!(*ticks_waiting, 5, "paused should not increment: {ticks_waiting}");
        }
    }

    #[test]
    fn on_duty_dead_excluded() {
        let mut app = setup_on_duty_app();
        let npc = app.world_mut().spawn((
            Activity::OnDuty { ticks_waiting: 0 },
            CombatState::None,
            Dead,
        )).id();

        app.update();
        let activity = app.world().get::<Activity>(npc).unwrap();
        if let Activity::OnDuty { ticks_waiting } = activity {
            assert_eq!(*ticks_waiting, 0, "dead NPC should not increment");
        }
    }

    #[test]
    fn on_duty_buildings_excluded() {
        let mut app = setup_on_duty_app();
        let bld = app.world_mut().spawn((
            Activity::OnDuty { ticks_waiting: 0 },
            CombatState::None,
            Building { kind: crate::world::BuildingKind::Tower },
        )).id();

        app.update();
        let activity = app.world().get::<Activity>(bld).unwrap();
        if let Activity::OnDuty { ticks_waiting } = activity {
            assert_eq!(*ticks_waiting, 0, "buildings should not increment");
        }
    }

    #[test]
    fn non_on_duty_activity_unchanged() {
        let mut app = setup_on_duty_app();
        let npc = app.world_mut().spawn((
            Activity::Working,
            CombatState::None,
        )).id();

        app.update();
        let activity = app.world().get::<Activity>(npc).unwrap();
        assert!(matches!(activity, Activity::Working), "non-OnDuty activity should not change");
    }
}
