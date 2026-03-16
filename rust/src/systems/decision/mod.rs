//! Decision system - Central priority-based decision making for ALL NPCs
//!
//! The decision system is the NPC's "brain" - all decisions flow through it:
//! Priority 0: AtDestination + Transit/Ready? -> Handle arrival transition
//! Priority 1-3: Combat (flee/leash/skip)
//! Priority 4a: Heal+Active? -> Wake when HP recovered; Heal+Transit -> skip
//! Priority 4b: Rest+Active? -> Wake when energy >= 90%; Rest+Transit -> skip
//! Priority 5: Working + tired? -> Stop work
//! Priority 6: OnDuty + time_to_patrol? -> Patrol
//! Priority 7: Idle -> Score Eat/Rest/Work/Wander (wounded -> fountain)

use crate::components::*;
use crate::constants::UpgradeStatKind;
use crate::constants::*;
use crate::messages::{CombatLogMsg, GpuUpdate, GpuUpdateMsg, WorkIntent, WorkIntentMsg};
use crate::resources::{
    CombatEventKind, DEFAULT_LOOT_THRESHOLD, EntityMap, GameTime, GpuReadState, MovementPriority,
    OffDutyBehavior, PathRequestQueue, SquadState, WorkSchedule,
};
use crate::systemparams::EconomyState;
use crate::systems::economy::*;
use crate::systems::stats::UPGRADES;
use crate::world::{
    BuildingKind, LocationKind, WorldData, find_location_within_radius, find_within_radius,
};
use bevy::prelude::*;

use super::behavior::{DecisionExtras, DecisionNpcState, NpcDataQueries};

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
    queue.submit(
        entity,
        Vec2::new(center_x + ox, center_y + oy),
        priority,
        source,
    );
}

#[inline]
fn transition_to_rest(
    activity: &mut Activity,
    queue: &mut PathRequestQueue,
    entity: Entity,
    home_valid: bool,
    home: Vec2,
    town_center: Option<Vec2>,
    idx: usize,
    frame: usize,
    priority: MovementPriority,
    home_source: &'static str,
    fountain_source: &'static str,
) -> bool {
    if home_valid {
        transition_activity(
            activity,
            ActivityKind::Rest,
            ActivityPhase::Transit,
            ActivityTarget::Home,
            home_source,
        );
        submit_intent(queue, entity, home.x, home.y, priority, home_source);
        true
    } else if let Some(center) = town_center {
        transition_activity(
            activity,
            ActivityKind::Rest,
            ActivityPhase::Transit,
            ActivityTarget::Fountain,
            fountain_source,
        );
        submit_intent_scattered(
            queue,
            entity,
            center.x,
            center.y,
            128.0,
            idx,
            frame,
            priority,
            fountain_source,
        );
        true
    } else {
        false
    }
}

#[inline]
fn has_rest_destination(home_valid: bool, town_center: Option<Vec2>) -> bool {
    home_valid || town_center.is_some()
}

#[inline]
fn loot_threshold_for_npc(squad_state: &SquadState, squad_id: Option<i32>) -> usize {
    squad_id
        .and_then(|sid| usize::try_from(sid).ok())
        .and_then(|sid| squad_state.squads.get(sid))
        .map(|squad| squad.loot_threshold)
        .unwrap_or(DEFAULT_LOOT_THRESHOLD)
}

/// Find the nearest enemy wall or gate for a raider to attack when path is blocked.
/// Uses spatial search to avoid O(n) full scan. Returns wall position if found.
fn find_nearest_enemy_wall(
    entity_map: &EntityMap,
    from: Vec2,
    attacker_faction: i32,
) -> Option<Vec2> {
    let mut best: Option<(f32, Vec2)> = None;
    // Search walls then gates
    for kind in [BuildingKind::Wall, BuildingKind::Gate] {
        for inst in entity_map.iter_kind(kind) {
            // Only target enemy walls (different faction, non-neutral)
            if inst.faction == attacker_faction || inst.faction == 0 {
                continue;
            }
            let dist_sq = from.distance_squared(inst.position);
            if best.is_none_or(|b| dist_sq < b.0) {
                best = Some((dist_sq, inst.position));
            }
        }
    }
    best.map(|b| b.1)
}

/// Transition an NPC to a new activity state. Resets ticks_waiting.
#[inline]
pub(crate) fn transition_activity(
    activity: &mut Activity,
    kind: ActivityKind,
    phase: ActivityPhase,
    target: ActivityTarget,
    reason: &'static str,
) {
    activity.kind = kind;
    activity.phase = phase;
    activity.target = target;
    activity.ticks_waiting = 0;
    activity.reason = reason;
    activity.last_frame = DECISION_FRAME.load(std::sync::atomic::Ordering::Relaxed) as u32;
    if kind != ActivityKind::Heal {
        activity.recover_until = 0.0;
    }
}

/// Transition phase only (same kind+target). Resets ticks_waiting.
#[inline]
fn transition_phase(activity: &mut Activity, phase: ActivityPhase, reason: &'static str) {
    activity.phase = phase;
    activity.ticks_waiting = 0;
    activity.reason = reason;
    activity.last_frame = DECISION_FRAME.load(std::sync::atomic::Ordering::Relaxed) as u32;
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
    world_data: Res<WorldData>,
    mut economy: EconomyState,
    mut intents: ResMut<PathRequestQueue>,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    mut extras: DecisionExtras,
    npc_config: Res<crate::resources::NpcDecisionConfig>,
    entity_map: Res<EntityMap>,
    mut npc_state: DecisionNpcState,
    mut npc_data: NpcDataQueries,
    decision_npc_q: Query<
        (Entity, &GpuSlot, &Job, &TownId, &Faction),
        (Without<Building>, Without<Dead>),
    >,
    miner_cfg_q: Query<&MinerHomeConfig>,
    mut production_q: Query<&mut ProductionState>,
    farm_mode_q: Query<&FarmModeComp>,
    mut building_health_q: Query<&mut Health, With<Building>>,
    path_q: Query<&NpcPath>,
) {
    if game_time.is_paused() {
        return;
    }

    // Sync NPC log filter state from settings + selected NPC
    extras.npc_logs.mode = extras.settings.npc_log_mode;
    extras.npc_logs.update_selected(extras.selected_npc.0);

    let npc_logs = &mut extras.npc_logs;
    let combat_log = &mut extras.combat_log;
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
    // Scale buckets down at high game speeds so decisions keep pace with movement
    let speed_scale = game_time.time_scale.max(1.0);
    let think_buckets = (think_buckets as f32 / speed_scale).max(1.0) as usize;
    const COMBAT_BUCKET: usize = 16; // ~267ms at 60fps
    let combat_bucket = (COMBAT_BUCKET as f32 / speed_scale).max(1.0) as usize;

    // Pre-build cow farm set for farmer targeting exclusion (cheap: only farm buildings)
    let cow_farm_slots: std::collections::HashSet<usize> = entity_map
        .iter_instances()
        .filter(|inst| inst.kind == crate::world::BuildingKind::Farm)
        .filter_map(|inst| {
            let e = entity_map.entities.get(&inst.slot)?;
            let fm = farm_mode_q.get(*e).ok()?;
            if fm.0 == FarmMode::Cows {
                Some(inst.slot)
            } else {
                None
            }
        })
        .collect();

    for (entity, slot, job, town_id, faction) in decision_npc_q.iter() {
        let idx = slot.0;

        // ====================================================================
        // Top-of-loop bucket gate: only process NPCs on their think cadence.
        // Fighting NPCs use a tighter bucket for responsive flee/leash.
        // ====================================================================
        let combat_state_peek = npc_state
            .combat_state_q
            .get(entity)
            .map(|cs| cs.is_fighting())
            .unwrap_or(false);
        if combat_state_peek {
            if !(idx + frame).is_multiple_of(combat_bucket) {
                continue;
            }
        } else {
            if !(idx + frame).is_multiple_of(think_buckets) {
                continue;
            }
        }

        // Full component reads — only for NPCs that passed the bucket gate
        let job = *job;
        let town_idx_i32 = town_id.0;
        let faction_i32 = faction.0;
        let mut energy = npc_state.energy_q.get(entity).map(|e| e.0).unwrap_or(100.0);
        let health = npc_state.health_q.get(entity).map(|h| h.0).unwrap_or(100.0);
        let home_comp = npc_data.home_q.get(entity).ok();
        let home = home_comp.map(|h| h.0).unwrap_or(Vec2::ZERO);
        let home_valid = home_comp.is_some_and(|h| h.is_valid());
        let personality = npc_data
            .personality_q
            .get(entity)
            .cloned()
            .unwrap_or_default();
        let mut activity = npc_state
            .activity_q
            .get(entity)
            .cloned()
            .unwrap_or_default();
        let mut combat_state = npc_state
            .combat_state_q
            .get(entity)
            .cloned()
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
            .and_then(|ws| entity_map.slot_for_entity(ws));
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
        let town_center = world_data
            .towns
            .get(town_idx_i32 as usize)
            .map(|t| t.center);
        let mut carried_loot = npc_data
            .carried_loot_q
            .get(entity)
            .cloned()
            .unwrap_or_default();

        // Capture originals for conditional writeback
        let orig_activity = activity;
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
                // Fair mining queue: direct-control miners lose their spot if moved out of range.
                if let (Some(slot), Some(current)) = (worksite, npc_pos) {
                    if let Some(inst) = entity_map.get_instance(slot) {
                        if inst.kind == BuildingKind::GoldMine {
                            let drift_radius = building_def(BuildingKind::GoldMine)
                                .worksite
                                .map(|ws| ws.drift_radius)
                                .unwrap_or(0.0);
                            if current.distance(inst.position) > drift_radius {
                                let uid = entity_map.entities.get(&slot).copied();
                                extras
                                    .work_intents
                                    .write(WorkIntentMsg(WorkIntent::Release {
                                        entity,
                                        worksite: uid,
                                    }));
                                worksite = None;
                                worksite_deferred = true;
                                npc_logs.push(
                                    idx,
                                    game_time.day(),
                                    game_time.hour(),
                                    game_time.minute(),
                                    "Direct control: out of mine range -> released queue spot",
                                );
                            }
                        }
                    }
                }
                if at_destination {
                    at_destination = false;
                }
                break 'decide;
            }

            // ====================================================================
            // Priority 0: AtDestination -> Handle arrival transition
            // ====================================================================
            if at_destination
                && activity.kind != ActivityKind::Idle
                && matches!(
                    activity.phase,
                    ActivityPhase::Transit | ActivityPhase::Ready
                )
            {
                at_destination = false;

                match activity.kind {
                    ActivityKind::Patrol | ActivityKind::SquadAttack => {
                        // Squad rest: tired squad members go home instead of entering OnDuty
                        if let Some(sid) = squad_id {
                            if let Some(squad) = squad_state.squads.get(sid as usize) {
                                if squad.rest_when_tired && energy < ENERGY_TIRED_THRESHOLD {
                                    if transition_to_rest(
                                        &mut activity,
                                        &mut intents,
                                        entity,
                                        home_valid,
                                        home,
                                        town_center,
                                        idx,
                                        frame,
                                        MovementPriority::Survival,
                                        "arrival:squad_rest",
                                        "arrival:squad_rest_fountain",
                                    ) {
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
                        }
                        // Split: Patrol arrival -> Holding at patrol post
                        //        SquadAttack arrival -> Holding at squad target
                        if activity.kind == ActivityKind::SquadAttack {
                            let squad_target = squad_id
                                .and_then(|sid| squad_state.squads.get(sid as usize))
                                .and_then(|s| s.target)
                                .unwrap_or(npc_pos.unwrap_or(home));
                            transition_activity(
                                &mut activity,
                                ActivityKind::SquadAttack,
                                ActivityPhase::Holding,
                                ActivityTarget::SquadPoint(squad_target),
                                "onduty:scatter",
                            );
                            // Scatter near squad target
                            submit_intent_scattered(
                                &mut intents,
                                entity,
                                squad_target.x,
                                squad_target.y,
                                128.0,
                                idx,
                                patrol_current,
                                MovementPriority::JobRoute,
                                "onduty:scatter",
                            );
                        } else {
                            transition_activity(
                                &mut activity,
                                ActivityKind::Patrol,
                                ActivityPhase::Holding,
                                ActivityTarget::PatrolPost {
                                    route: 0,
                                    index: patrol_current as u16,
                                },
                                "onduty:scatter",
                            );
                            // Scatter near patrol post or squad target
                            let scatter_pos =
                                squad_id
                                    .and_then(|sid| squad_state.squads.get(sid as usize))
                                    .and_then(|s| s.target)
                                    .or_else(|| {
                                        npc_data.patrol_route_q.get(entity).ok().and_then(|route| {
                                            route.posts.get(patrol_current).copied()
                                        })
                                    });
                            if let Some(spos) = scatter_pos {
                                submit_intent_scattered(
                                    &mut intents,
                                    entity,
                                    spos.x,
                                    spos.y,
                                    128.0,
                                    idx,
                                    patrol_current,
                                    MovementPriority::JobRoute,
                                    "onduty:scatter",
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
                    ActivityKind::Rest => {
                        transition_phase(&mut activity, ActivityPhase::Active, "->_onduty");
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            "-> Resting",
                        );
                    }
                    ActivityKind::Heal => {
                        let threshold = economy
                            .towns
                            .policy(town_idx_i32)
                            .map(|p| p.recovery_hp)
                            .unwrap_or(0.8)
                            .min(1.0);
                        transition_phase(&mut activity, ActivityPhase::Active, "phase_change");
                        activity.recover_until = threshold;
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            "-> Healing",
                        );
                    }
                    ActivityKind::Work => {
                        // Farmers: find farm at work_target and start working
                        if job == Job::Farmer {
                            let current_pos = npc_pos.unwrap_or(home);
                            let target_farm = worksite
                                .and_then(|slot| {
                                    entity_map
                                        .get_instance(slot)
                                        .filter(|inst| {
                                            inst.kind == BuildingKind::Farm
                                                && inst.town_idx == town_idx_i32 as u32
                                        })
                                        .map(|inst| (slot, inst.position))
                                })
                                .or_else(|| {
                                    find_within_radius(
                                        current_pos,
                                        &entity_map,
                                        BuildingKind::Farm,
                                        FARM_ARRIVAL_RADIUS,
                                        town_idx_i32 as u32,
                                    )
                                });

                            if let Some((farm_slot, farm_pos)) = target_farm {
                                let occ = entity_map.occupant_count(farm_slot);
                                let owns = worksite == Some(farm_slot);
                                let occupied_by_other = if owns { occ > 1 } else { occ >= 1 };

                                if occupied_by_other {
                                    // Farm occupied — retarget via resolver
                                    let uid =
                                        worksite.and_then(|s| entity_map.entities.get(&s).copied());
                                    extras
                                        .work_intents
                                        .write(WorkIntentMsg(WorkIntent::Release {
                                            entity,
                                            worksite: uid,
                                        }));
                                    extras.work_intents.write(WorkIntentMsg(WorkIntent::Claim {
                                        entity,
                                        kind: BuildingKind::Farm,
                                        town_idx: town_idx_i32 as u32,
                                        from: current_pos,
                                    }));
                                    worksite = None;
                                    worksite_deferred = true;
                                    npc_logs.push(
                                        idx,
                                        game_time.day(),
                                        game_time.hour(),
                                        game_time.minute(),
                                        "Farm occupied → retarget",
                                    );
                                } else if entity_map.get_instance(farm_slot).is_some() {
                                    // Check if farm ready for harvest via ECS ProductionState
                                    let farm_entity_opt =
                                        entity_map.entities.get(&farm_slot).copied();
                                    let harvest = farm_entity_opt
                                        .and_then(|e| production_q.get_mut(e).ok())
                                        .and_then(|mut ps| {
                                            let mode = farm_entity_opt
                                                .and_then(|e| farm_mode_q.get(e).ok())
                                                .map_or(FarmMode::Crops, |m| m.0);
                                            let food = ps.take_yield(BuildingKind::Farm, mode);
                                            if food > 0 {
                                                let pos = entity_map
                                                    .get_instance(farm_slot)
                                                    .map_or(Vec2::ZERO, |i| i.position);
                                                Some((
                                                    food,
                                                    ProductionState::yield_log_msg(
                                                        BuildingKind::Farm,
                                                        pos,
                                                        food,
                                                    ),
                                                ))
                                            } else {
                                                None
                                            }
                                        });
                                    if let Some((food, log_msg)) = harvest {
                                        // Harvest — release worksite, carry home
                                        let uid = worksite
                                            .and_then(|s| entity_map.entities.get(&s).copied());
                                        extras.work_intents.write(WorkIntentMsg(
                                            WorkIntent::Release {
                                                entity,
                                                worksite: uid,
                                            },
                                        ));
                                        worksite = None;
                                        worksite_deferred = true;
                                        combat_log.write(CombatLogMsg {
                                            kind: CombatEventKind::Harvest,
                                            faction: faction_i32,
                                            day: game_time.day(),
                                            hour: game_time.hour(),
                                            minute: game_time.minute(),
                                            message: log_msg,
                                            location: None,
                                        });
                                        carried_loot.food += food;
                                        transition_activity(
                                            &mut activity,
                                            ActivityKind::ReturnLoot,
                                            ActivityPhase::Transit,
                                            ActivityTarget::Dropoff,
                                            "arrival:farm_harvest_return",
                                        );
                                        submit_intent(
                                            &mut intents,
                                            entity,
                                            home.x,
                                            home.y,
                                            MovementPriority::JobRoute,
                                            "arrival:farm_harvest_return",
                                        );
                                        npc_logs.push(
                                            idx,
                                            game_time.day(),
                                            game_time.hour(),
                                            game_time.minute(),
                                            "Harvested → Returning",
                                        );
                                    } else {
                                        // Farm not ready — claim via resolver if not already owned, start working
                                        if !owns {
                                            let uid = worksite
                                                .and_then(|s| entity_map.entities.get(&s).copied());
                                            extras.work_intents.write(WorkIntentMsg(
                                                WorkIntent::Release {
                                                    entity,
                                                    worksite: uid,
                                                },
                                            ));
                                            extras.work_intents.write(WorkIntentMsg(
                                                WorkIntent::Claim {
                                                    entity,
                                                    kind: BuildingKind::Farm,
                                                    town_idx: town_idx_i32 as u32,
                                                    from: current_pos,
                                                },
                                            ));
                                            worksite = None;
                                            worksite_deferred = true;
                                        }
                                        transition_phase(
                                            &mut activity,
                                            ActivityPhase::Active,
                                            "phase_change",
                                        );

                                        pop_inc_working(&mut economy.pop_stats, job, town_idx_i32);
                                        submit_intent(
                                            &mut intents,
                                            entity,
                                            farm_pos.x,
                                            farm_pos.y,
                                            MovementPriority::JobRoute,
                                            "arrival:farm_work",
                                        );
                                        npc_logs.push(
                                            idx,
                                            game_time.day(),
                                            game_time.hour(),
                                            game_time.minute(),
                                            "→ Working (tending)",
                                        );
                                    }
                                }
                            } else {
                                // No farm nearby — release and idle
                                let uid =
                                    worksite.and_then(|s| entity_map.entities.get(&s).copied());
                                extras
                                    .work_intents
                                    .write(WorkIntentMsg(WorkIntent::Release {
                                        entity,
                                        worksite: uid,
                                    }));
                                worksite = None;
                                worksite_deferred = true;
                                transition_activity(
                                    &mut activity,
                                    ActivityKind::Idle,
                                    ActivityPhase::Ready,
                                    ActivityTarget::None,
                                    "no_farm_nearby_→_idle",
                                );
                                npc_logs.push(
                                    idx,
                                    game_time.day(),
                                    game_time.hour(),
                                    game_time.minute(),
                                    "No farm nearby → Idle",
                                );
                            }
                        } else {
                            let current_pos = npc_pos.unwrap_or(Vec2::ZERO);
                            transition_phase(&mut activity, ActivityPhase::Active, "phase_change");
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
                    ActivityKind::Raid => {
                        // Raider arrived at farm - check if ready to steal
                        if let Some(pos) = npc_pos {
                            let ready_farm_pos = find_location_within_radius(
                                pos,
                                &entity_map,
                                LocationKind::Farm,
                                FARM_ARRIVAL_RADIUS,
                            )
                            .and_then(|(_, fp)| {
                                let slot = entity_map.slot_at_position(fp)?;
                                let e = *entity_map.entities.get(&slot)?;
                                let ps = production_q.get(e).ok()?;
                                if ps.ready { Some(fp) } else { None }
                            });

                            if let Some(fp) = ready_farm_pos {
                                let farm_e = entity_map
                                    .slot_at_position(fp)
                                    .and_then(|slot| entity_map.entities.get(&slot).copied());
                                let food = farm_e
                                    .and_then(|e| production_q.get_mut(e).ok())
                                    .map(|mut ps| {
                                        let mode = farm_e
                                            .and_then(|e| farm_mode_q.get(e).ok())
                                            .map_or(FarmMode::Crops, |m| m.0);
                                        let f = ps.take_yield(BuildingKind::Farm, mode);
                                        if f > 0 {
                                            combat_log.write(CombatLogMsg {
                                                kind: CombatEventKind::Harvest,
                                                faction: faction_i32,
                                                day: game_time.day(),
                                                hour: game_time.hour(),
                                                minute: game_time.minute(),
                                                message: ProductionState::yield_log_msg(
                                                    BuildingKind::Farm,
                                                    fp,
                                                    f,
                                                ),
                                                location: None,
                                            });
                                        }
                                        f
                                    })
                                    .unwrap_or(0);

                                carried_loot.food += food.max(1);
                                transition_activity(
                                    &mut activity,
                                    ActivityKind::ReturnLoot,
                                    ActivityPhase::Transit,
                                    ActivityTarget::Dropoff,
                                    "transition",
                                );
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
                                entity_map.for_each_nearby_kind(
                                    pos,
                                    raid_search_radius,
                                    BuildingKind::Farm,
                                    |f, _| {
                                        if f.position.distance(pos) <= FARM_ARRIVAL_RADIUS {
                                            return;
                                        }
                                        let d2 = f.position.distance_squared(pos);
                                        if d2 < best_d2 {
                                            best_d2 = d2;
                                            other_farm_pos = Some(f.position);
                                        }
                                    },
                                );
                                if let Some(farm_pos) = other_farm_pos {
                                    transition_activity(
                                        &mut activity,
                                        ActivityKind::Raid,
                                        ActivityPhase::Transit,
                                        ActivityTarget::RaidPoint(farm_pos),
                                        "transition",
                                    );
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
                                    transition_activity(
                                        &mut activity,
                                        ActivityKind::ReturnLoot,
                                        ActivityPhase::Transit,
                                        ActivityTarget::Dropoff,
                                        "transition",
                                    );
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
                    ActivityKind::Mine => {
                        // Arrived at gold mine -- resolve position from worksite claim
                        let mine_pos = worksite
                            .and_then(|slot| entity_map.get_instance(slot))
                            .map(|inst| inst.position)
                            .or(npc_pos)
                            .unwrap_or(Vec2::ZERO);
                        let mine_slot = entity_map.slot_at_position(mine_pos);
                        let miner_uid = Some(entity);
                        let can_harvest_turn = mine_slot.is_none_or(|slot| {
                            miner_uid
                                .is_none_or(|uid| entity_map.is_worksite_harvest_turn(slot, uid))
                        });
                        let mine_entity =
                            mine_slot.and_then(|s| entity_map.entities.get(&s).copied());
                        let mine_ready = mine_entity
                            .and_then(|e| production_q.get(e).ok())
                            .is_some_and(|ps| ps.ready);
                        if mine_entity.is_some() {
                            if mine_ready && can_harvest_turn {
                                // Mine ready — harvest immediately
                                let town_levels = economy.towns.upgrade_levels(town_idx_i32);
                                let yield_mult = UPGRADES.stat_mult(
                                    &town_levels,
                                    "Miner",
                                    UpgradeStatKind::Yield,
                                );
                                let base_gold = mine_entity
                                    .and_then(|e| production_q.get_mut(e).ok())
                                    .map(|mut ps| {
                                        ps.take_yield(BuildingKind::GoldMine, FarmMode::Crops)
                                    })
                                    .unwrap_or(0);
                                if base_gold > 0 {
                                    combat_log.write(CombatLogMsg {
                                        kind: CombatEventKind::Harvest,
                                        faction: faction_i32,
                                        day: game_time.day(),
                                        hour: game_time.hour(),
                                        minute: game_time.minute(),
                                        message: ProductionState::yield_log_msg(
                                            BuildingKind::GoldMine,
                                            mine_pos,
                                            base_gold,
                                        ),
                                        location: None,
                                    });
                                }
                                let gold_amount = ((base_gold as f32) * yield_mult).round() as i32;
                                carried_loot.gold += gold_amount;
                                let uid =
                                    worksite.and_then(|s| entity_map.entities.get(&s).copied());
                                extras
                                    .work_intents
                                    .write(WorkIntentMsg(WorkIntent::Release {
                                        entity,
                                        worksite: uid,
                                    }));
                                worksite = None;
                                worksite_deferred = true;
                                transition_activity(
                                    &mut activity,
                                    ActivityKind::ReturnLoot,
                                    ActivityPhase::Transit,
                                    ActivityTarget::Dropoff,
                                    "transition",
                                );
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
                                // Mine not ready for this miner (still growing or queued behind others) — tend/wait
                                extras.work_intents.write(WorkIntentMsg(WorkIntent::Claim {
                                    entity,
                                    kind: BuildingKind::GoldMine,
                                    town_idx: town_idx_i32 as u32,
                                    from: mine_pos,
                                }));
                                worksite_deferred = true;
                                transition_phase(
                                    &mut activity,
                                    ActivityPhase::Holding,
                                    "phase_change",
                                );
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
                            transition_activity(
                                &mut activity,
                                ActivityKind::Idle,
                                ActivityPhase::Ready,
                                ActivityTarget::None,
                                "transition",
                            );
                            npc_logs.push(
                                idx,
                                game_time.day(),
                                game_time.hour(),
                                game_time.minute(),
                                "No mine nearby -> Idle",
                            );
                        }
                    }
                    ActivityKind::Repair => {
                        // Mason arrived at damaged building -- start repairing
                        transition_phase(
                            &mut activity,
                            ActivityPhase::Active,
                            "arrival:repair_start",
                        );
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            "Arrived -> Repairing",
                        );
                    }
                    ActivityKind::Wander => {
                        transition_activity(
                            &mut activity,
                            ActivityKind::Idle,
                            ActivityPhase::Ready,
                            ActivityTarget::None,
                            "transition",
                        );
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            "-> Idle",
                        );
                    }
                    ActivityKind::ReturnLoot => {
                        // May have arrived at wrong place (e.g. after DC removal) — redirect home
                        if home_valid {
                            submit_intent(
                                &mut intents,
                                entity,
                                home.x,
                                home.y,
                                MovementPriority::JobRoute,
                                "arrival:return_redirect",
                            );
                        } else {
                            transition_activity(
                                &mut activity,
                                ActivityKind::Idle,
                                ActivityPhase::Ready,
                                ActivityTarget::None,
                                "arrival:return_redirect",
                            );
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
                        || (energy < ENERGY_WAKE_THRESHOLD && activity.kind == ActivityKind::Rest);
                    if squad.rest_when_tired && squad_needs_rest {
                        if combat_state.is_fighting() {
                            combat_state = CombatState::None;
                        }
                        if activity.kind != ActivityKind::Rest {
                            transition_to_rest(
                                &mut activity,
                                &mut intents,
                                entity,
                                home_valid,
                                home,
                                town_center,
                                idx,
                                frame,
                                MovementPriority::Survival,
                                "squad:rest_gate",
                                "squad:rest_gate_fountain",
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
                        let p = economy.towns.policy(town_idx_i32);
                        if p.as_ref().is_some_and(|p| p.archer_aggressive) {
                            0.0 // aggressive guards never flee
                        } else {
                            p.map(|p| p.archer_flee_hp).unwrap_or(0.15)
                        }
                    }
                    Job::Farmer | Job::Miner | Job::Woodcutter | Job::Quarrier | Job::Mason => {
                        let p = economy.towns.policy(town_idx_i32);
                        if p.as_ref().is_some_and(|p| p.farmer_fight_back) {
                            0.0 // fight-back workers don't flee
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
                    let should_check_threat = (frame + idx).is_multiple_of(CHECK_INTERVAL);
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
                        if activity.kind.def().is_working {
                            let uid = worksite.and_then(|s| entity_map.entities.get(&s).copied());
                            extras
                                .work_intents
                                .write(WorkIntentMsg(WorkIntent::Release {
                                    entity,
                                    worksite: uid,
                                }));
                            worksite = None;
                            worksite_deferred = true;
                        }
                        combat_state = CombatState::None;
                        if activity.kind != ActivityKind::ReturnLoot {
                            transition_activity(
                                &mut activity,
                                ActivityKind::ReturnLoot,
                                ActivityPhase::Transit,
                                ActivityTarget::Dropoff,
                                "transition",
                            );
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
                let healing_policy_active = economy.towns.policy(town_idx_i32).is_some_and(|p| {
                    p.prioritize_healing && energy > 0.0 && health / max_hp < p.recovery_hp
                });
                if healing_policy_active {
                    if let Some(town) = world_data.towns.get(town_idx_usize) {
                        combat_state = CombatState::None;
                        if activity.kind != ActivityKind::Heal {
                            let threshold = economy
                                .towns
                                .policy(town_idx_i32)
                                .map(|p| p.recovery_hp)
                                .unwrap_or(0.8);
                            transition_activity(
                                &mut activity,
                                ActivityKind::Heal,
                                ActivityPhase::Transit,
                                ActivityTarget::Fountain,
                                "combat:heal_fountain",
                            );
                            activity.recover_until = threshold;
                            submit_intent_scattered(
                                &mut intents,
                                entity,
                                town.center.x,
                                town.center.y,
                                128.0,
                                idx,
                                frame,
                                MovementPriority::Survival,
                                "combat:heal_fountain",
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
                    Job::Archer | Job::Crossbow => economy
                        .towns
                        .policy(town_idx_i32)
                        .as_ref()
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
                                if activity.kind.def().is_working {
                                    let uid =
                                        worksite.and_then(|s| entity_map.entities.get(&s).copied());
                                    extras
                                        .work_intents
                                        .write(WorkIntentMsg(WorkIntent::Release {
                                            entity,
                                            worksite: uid,
                                        }));
                                    worksite = None;
                                    worksite_deferred = true;
                                }
                                combat_state = CombatState::None;
                                if activity.kind != ActivityKind::ReturnLoot {
                                    transition_activity(
                                        &mut activity,
                                        ActivityKind::ReturnLoot,
                                        ActivityPhase::Transit,
                                        ActivityTarget::Dropoff,
                                        "transition",
                                    );
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
                                && activity.kind == ActivityKind::Rest);
                        if squad.rest_when_tired && squad_needs_rest {
                            if activity.kind != ActivityKind::Rest {
                                transition_to_rest(
                                    &mut activity,
                                    &mut intents,
                                    entity,
                                    home_valid,
                                    home,
                                    town_center,
                                    idx,
                                    frame,
                                    MovementPriority::Survival,
                                    "squad:rest_home",
                                    "squad:rest_fountain",
                                );
                            }
                            break 'decide;
                        }
                        // Wounded: prioritize healing over squad target (prevents flee-engage oscillation)
                        let ti = town_idx_i32 as usize;
                        if let Some(p) = economy.towns.policy(town_idx_i32) {
                            if p.prioritize_healing
                                && energy > 0.0
                                && health / max_hp < p.recovery_hp
                            {
                                if activity.kind != ActivityKind::Heal {
                                    if let Some(town) = world_data.towns.get(ti) {
                                        combat_state = CombatState::None;
                                        let threshold = p.recovery_hp;
                                        transition_activity(
                                            &mut activity,
                                            ActivityKind::Heal,
                                            ActivityPhase::Transit,
                                            ActivityTarget::Fountain,
                                            "squad:heal_fountain",
                                        );
                                        activity.recover_until = threshold;
                                        submit_intent_scattered(
                                            &mut intents,
                                            entity,
                                            town.center.x,
                                            town.center.y,
                                            128.0,
                                            idx,
                                            frame,
                                            MovementPriority::Survival,
                                            "squad:heal_fountain",
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
                        // Squad target — always submit intent (single path, deterministic)
                        // Movement system deduplicates unchanged targets; priority system
                        // resolves conflicts (Survival=4 > Squad=2 > JobRoute=1).
                        //
                        // Wall-attack fallback: if pathfinding failed (path_blocked),
                        // find nearest enemy wall and target it instead.
                        let is_blocked = path_q.get(entity).is_ok_and(|p| p.path_blocked);

                        if is_blocked {
                            // Find nearest enemy wall/gate to attack
                            let npc_pos_val = npc_pos.unwrap_or(home);
                            if let Some(wall_inst) =
                                find_nearest_enemy_wall(&entity_map, npc_pos_val, faction.0)
                            {
                                submit_intent(
                                    &mut intents,
                                    entity,
                                    wall_inst.x,
                                    wall_inst.y,
                                    MovementPriority::Squad,
                                    "squad:wall_attack",
                                );
                                if at_destination {
                                    transition_activity(
                                        &mut activity,
                                        ActivityKind::SquadAttack,
                                        ActivityPhase::Holding,
                                        ActivityTarget::SquadPoint(wall_inst),
                                        "squad:wall_attack",
                                    );
                                }
                            } else {
                                // No wall found — try normal target anyway
                                submit_intent(
                                    &mut intents,
                                    entity,
                                    target.x,
                                    target.y,
                                    MovementPriority::Squad,
                                    "squad:target",
                                );
                            }
                        } else {
                            submit_intent(
                                &mut intents,
                                entity,
                                target.x,
                                target.y,
                                MovementPriority::Squad,
                                "squad:target",
                            );
                            if at_destination {
                                transition_activity(
                                    &mut activity,
                                    ActivityKind::SquadAttack,
                                    ActivityPhase::Holding,
                                    ActivityTarget::SquadPoint(target),
                                    "squad:target",
                                );
                            }
                        }
                    } else if !squad.patrol_enabled {
                        // No target + patrol disabled: stop and wait (gathering phase)
                        if matches!(
                            activity.kind,
                            ActivityKind::Patrol | ActivityKind::SquadAttack | ActivityKind::Raid
                        ) {
                            transition_activity(
                                &mut activity,
                                ActivityKind::Idle,
                                ActivityPhase::Ready,
                                ActivityTarget::None,
                                "transition",
                            );
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
                && activity.kind == ActivityKind::Work
                && activity.phase == ActivityPhase::Transit
                && (idx + frame).is_multiple_of(think_buckets)
            {
                if let Some(wp) = worksite {
                    let occ = entity_map.occupant_count(wp);
                    let occupied_by_other = (occ > 1) || (occ >= 1 && worksite != Some(wp));
                    if occupied_by_other {
                        // Retarget via resolver — release old + claim new
                        let current_pos = npc_pos.unwrap_or_default();
                        let uid = worksite.and_then(|s| entity_map.entities.get(&s).copied());
                        extras
                            .work_intents
                            .write(WorkIntentMsg(WorkIntent::Release {
                                entity,
                                worksite: uid,
                            }));
                        extras.work_intents.write(WorkIntentMsg(WorkIntent::Claim {
                            entity,
                            kind: BuildingKind::Farm,
                            town_idx: town_idx_i32 as u32,
                            from: current_pos,
                        }));
                        worksite = None;
                        worksite_deferred = true;
                        // Activity stays GoingToWork — resolver will submit path or set Idle
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            "Farm taken → retarget",
                        );
                        break 'decide;
                    }
                }
            }

            // ====================================================================
            // Skip NPCs in transit states (they're walking to their destination)
            // GoingToHeal proximity check: bucket gate ensures this runs on cadence.
            // ====================================================================
            if !at_destination {
                // Stuck-transit redirect: casual transit NPCs that haven't arrived get new scatter
                if (idx + frame).is_multiple_of(think_buckets) {
                    match activity.kind {
                        ActivityKind::Wander => {
                            // Drop to Idle so decision system re-evaluates (Work/Eat/Rest)
                            // instead of endlessly picking new wander targets.
                            transition_activity(
                                &mut activity,
                                ActivityKind::Idle,
                                ActivityPhase::Ready,
                                ActivityTarget::None,
                                "transition",
                            );
                        }
                        ActivityKind::Patrol if activity.phase == ActivityPhase::Transit => {
                            // Don't patrol around town if squad has an active target
                            let has_squad_target = squad_id
                                .and_then(|sid| squad_state.squads.get(sid as usize))
                                .is_some_and(|s| s.target.is_some());
                            if !has_squad_target {
                                if let Ok(route) = npc_data.patrol_route_q.get(entity) {
                                    if !route.posts.is_empty() {
                                        let safe_idx = patrol_current % route.posts.len();
                                        if let Some(post) = route.posts.get(safe_idx) {
                                            submit_intent_scattered(
                                                &mut intents,
                                                entity,
                                                post.x,
                                                post.y,
                                                128.0,
                                                idx,
                                                frame,
                                                MovementPriority::JobRoute,
                                                "patrol:redirect",
                                            );
                                        }
                                    }
                                }
                            }
                            break 'decide;
                        }
                        _ => {}
                    }
                }

                // Early arrival: Heal+Transit NPCs stop once inside healing range
                if activity.kind == ActivityKind::Heal && activity.phase == ActivityPhase::Transit {
                    let town_idx = town_idx_i32 as usize;
                    if let Some(town) = world_data.towns.get(town_idx) {
                        if let Some(current) = npc_pos {
                            if current.distance(town.center) <= HEAL_DRIFT_RADIUS {
                                let threshold = economy
                                    .towns
                                    .policy(town_idx_i32)
                                    .map(|p| p.recovery_hp)
                                    .unwrap_or(0.8)
                                    .min(1.0);
                                transition_phase(
                                    &mut activity,
                                    ActivityPhase::Active,
                                    "phase_change",
                                );
                                activity.recover_until = threshold;
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
            if activity.kind == ActivityKind::Heal {
                if activity.phase == ActivityPhase::Active {
                    if health / max_hp >= activity.recover_until.min(1.0) {
                        transition_activity(
                            &mut activity,
                            ActivityKind::Idle,
                            ActivityPhase::Ready,
                            ActivityTarget::None,
                            "transition",
                        );
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
                        if let Some(town) = world_data.towns.get(town_idx) {
                            if let Some(current) = npc_pos {
                                if current.distance(town.center) > HEAL_DRIFT_RADIUS {
                                    submit_intent_scattered(
                                        &mut intents,
                                        entity,
                                        town.center.x,
                                        town.center.y,
                                        128.0,
                                        idx,
                                        frame,
                                        MovementPriority::Survival,
                                        "heal:drift_retarget",
                                    );
                                }
                            }
                        }
                        break 'decide; // still healing
                    }
                } else {
                    break 'decide; // Heal+Transit: waiting for arrival
                }
            }

            // ====================================================================
            // Priority 4b: Resting? -> Wake when energy recovered
            // ====================================================================
            if activity.kind == ActivityKind::Rest {
                if activity.phase == ActivityPhase::Active {
                    if energy >= ENERGY_WAKE_THRESHOLD {
                        transition_activity(
                            &mut activity,
                            ActivityKind::Idle,
                            ActivityPhase::Ready,
                            ActivityTarget::None,
                            "transition",
                        );
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
                } else {
                    break 'decide; // Rest+Transit: waiting for arrival
                }
            }

            // ====================================================================
            // Priority 4c: Loot threshold — too much equipment, return home
            // ====================================================================
            let loot_threshold = loot_threshold_for_npc(squad_state, squad_id);
            if !npc_def(job).equip_slots.is_empty()
                && carried_loot.equipment.len() >= loot_threshold
                && activity.kind != ActivityKind::ReturnLoot
                && matches!(combat_state, CombatState::None)
            {
                let uid = worksite.and_then(|s| entity_map.entities.get(&s).copied());
                extras
                    .work_intents
                    .write(WorkIntentMsg(WorkIntent::Release {
                        entity,
                        worksite: uid,
                    }));
                worksite = None;
                worksite_deferred = true;
                transition_activity(
                    &mut activity,
                    ActivityKind::ReturnLoot,
                    ActivityPhase::Transit,
                    ActivityTarget::Dropoff,
                    "transition",
                );
                intents.submit(entity, home, MovementPriority::JobRoute, "loot:threshold");
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
            let worksite_slot = if activity.kind.def().is_working {
                worksite
            } else {
                None
            };
            if let Some(slot) = worksite_slot {
                // Look up worksite config from building registry
                let inst_snapshot = entity_map.get_instance(slot).map(|i| (i.kind, i.town_idx));
                let Some((kind, inst_town)) = inst_snapshot else {
                    // Worksite destroyed — release and reassign
                    let uid = worksite.and_then(|s| entity_map.entities.get(&s).copied());
                    extras
                        .work_intents
                        .write(WorkIntentMsg(WorkIntent::Release {
                            entity,
                            worksite: uid,
                        }));
                    worksite = None;
                    worksite_deferred = true;
                    transition_activity(
                        &mut activity,
                        ActivityKind::Idle,
                        ActivityPhase::Ready,
                        ActivityTarget::None,
                        "transition",
                    );
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
                    let uid = worksite.and_then(|s| entity_map.entities.get(&s).copied());
                    extras
                        .work_intents
                        .write(WorkIntentMsg(WorkIntent::Release {
                            entity,
                            worksite: uid,
                        }));
                    worksite = None;
                    worksite_deferred = true;
                    transition_activity(
                        &mut activity,
                        ActivityKind::Idle,
                        ActivityPhase::Ready,
                        ActivityTarget::None,
                        "transition",
                    );
                    break 'decide;
                };

                // Validate: town match (only for town-scoped worksites like farms)
                if ws.town_scoped && inst_town != town_idx_i32 as u32 {
                    let uid = worksite.and_then(|s| entity_map.entities.get(&s).copied());
                    extras
                        .work_intents
                        .write(WorkIntentMsg(WorkIntent::Release {
                            entity,
                            worksite: uid,
                        }));
                    worksite = None;
                    worksite_deferred = true;
                    transition_activity(
                        &mut activity,
                        ActivityKind::Idle,
                        ActivityPhase::Ready,
                        ActivityTarget::None,
                        "transition",
                    );
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
                    let uid = worksite.and_then(|s| entity_map.entities.get(&s).copied());
                    extras
                        .work_intents
                        .write(WorkIntentMsg(WorkIntent::Release {
                            entity,
                            worksite: uid,
                        }));
                    worksite = None;
                    worksite_deferred = true;
                    transition_activity(
                        &mut activity,
                        ActivityKind::Idle,
                        ActivityPhase::Ready,
                        ActivityTarget::None,
                        "transition",
                    );
                    if home_valid {
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
                        entity,
                        kind,
                        town_idx: town_idx_i32 as u32,
                        from: current_pos,
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
                        if kind == BuildingKind::GoldMine {
                            // Fair mining queue: leaving mine range forfeits queue position.
                            let uid = worksite.and_then(|s| entity_map.entities.get(&s).copied());
                            extras
                                .work_intents
                                .write(WorkIntentMsg(WorkIntent::Release {
                                    entity,
                                    worksite: uid,
                                }));
                            worksite = None;
                            worksite_deferred = true;
                            transition_activity(
                                &mut activity,
                                ActivityKind::Mine,
                                ActivityPhase::Transit,
                                ActivityTarget::Worksite,
                                "transition",
                            );
                            submit_intent(
                                &mut intents,
                                entity,
                                ws_pos.x,
                                ws_pos.y,
                                MovementPriority::JobRoute,
                                "mine:requeue_after_range_loss",
                            );
                            npc_logs.push(
                                idx,
                                game_time.day(),
                                game_time.hour(),
                                game_time.minute(),
                                "Out of mine range -> queue spot lost",
                            );
                            break 'decide;
                        } else {
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
                }

                // Harvest check: if production ready, harvest + apply yield mult + return home
                let mut harvested = false;
                let claimer_uid = Some(entity);
                let can_harvest_turn = if kind == BuildingKind::GoldMine {
                    claimer_uid.is_none_or(|uid| entity_map.is_worksite_harvest_turn(slot, uid))
                } else {
                    true
                };
                let ws_entity = entity_map.entities.get(&slot).copied();
                let ws_ready = ws_entity
                    .and_then(|e| production_q.get(e).ok())
                    .is_some_and(|ps| ps.ready);
                if ws_ready && can_harvest_turn {
                    if let Some(mut ps) = ws_entity.and_then(|e| production_q.get_mut(e).ok()) {
                        let town_levels = economy.towns.upgrade_levels(town_idx_i32);
                        let yield_mult = UPGRADES.stat_mult(
                            &town_levels,
                            ws.upgrade_job,
                            UpgradeStatKind::Yield,
                        );
                        let ws_pos = entity_map
                            .get_instance(slot)
                            .map_or(Vec2::ZERO, |i| i.position);
                        let mode = ws_entity
                            .and_then(|e| farm_mode_q.get(e).ok())
                            .map_or(FarmMode::Crops, |m| m.0);
                        let base_yield = ps.take_yield(kind, mode);
                        if base_yield > 0 {
                            combat_log.write(CombatLogMsg {
                                kind: CombatEventKind::Harvest,
                                faction: faction_i32,
                                day: game_time.day(),
                                hour: game_time.hour(),
                                minute: game_time.minute(),
                                message: ProductionState::yield_log_msg(kind, ws_pos, base_yield),
                                location: None,
                            });
                        }
                        let final_yield = ((base_yield as f32) * yield_mult).round() as i32;
                        let uid = worksite.and_then(|s| entity_map.entities.get(&s).copied());
                        extras
                            .work_intents
                            .write(WorkIntentMsg(WorkIntent::Release {
                                entity,
                                worksite: uid,
                            }));
                        worksite = None;
                        worksite_deferred = true;
                        // One-shot worksites (resource nodes): destroy after yield
                        if ws.one_shot {
                            if let Some(ne) = ws_entity {
                                extras.damage.write(crate::messages::DamageMsg {
                                    target: ne,
                                    amount: 9999.0,
                                    attacker: idx as i32,
                                    attacker_faction: faction_i32,
                                });
                            }
                        }
                        match ws.yield_item {
                            ResourceKind::Food => carried_loot.food += final_yield,
                            ResourceKind::Gold => carried_loot.gold += final_yield,
                            ResourceKind::Wood => carried_loot.wood += final_yield,
                            ResourceKind::Stone => carried_loot.stone += final_yield,
                        }
                        transition_activity(
                            &mut activity,
                            ActivityKind::ReturnLoot,
                            ActivityPhase::Transit,
                            ActivityTarget::Dropoff,
                            "transition",
                        );
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
                    let uid = worksite.and_then(|s| entity_map.entities.get(&s).copied());
                    extras
                        .work_intents
                        .write(WorkIntentMsg(WorkIntent::Release {
                            entity,
                            worksite: uid,
                        }));
                    worksite = None;
                    worksite_deferred = true;
                    transition_activity(
                        &mut activity,
                        ActivityKind::Idle,
                        ActivityPhase::Ready,
                        ActivityTarget::None,
                        "transition",
                    );
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
            // Priority 5b: Mason Repair active -- heal nearest damaged building
            // ====================================================================
            if activity.kind == ActivityKind::Repair && activity.phase == ActivityPhase::Active {
                // Tired -> stop repairing
                if energy < ENERGY_TIRED_THRESHOLD {
                    transition_activity(
                        &mut activity,
                        ActivityKind::Idle,
                        ActivityPhase::Ready,
                        ActivityTarget::None,
                        "repair:tired",
                    );
                    npc_logs.push(
                        idx,
                        game_time.day(),
                        game_time.hour(),
                        game_time.minute(),
                        "Tired -> Stopped repairing",
                    );
                    break 'decide;
                }
                // Find nearest damaged building at current position
                let current_pos = npc_pos.unwrap_or(home);
                let repair_radius_sq: f32 = 40.0 * 40.0;
                let mut repaired = false;
                for inst in entity_map.iter_instances() {
                    if inst.town_idx != town_idx_i32 as u32 {
                        continue;
                    }
                    if inst.position.distance_squared(current_pos) > repair_radius_sq {
                        continue;
                    }
                    let Some(bld_entity) = entity_map.entities.get(&inst.slot).copied() else {
                        continue;
                    };
                    let Ok(mut bld_hp) = building_health_q.get_mut(bld_entity) else {
                        continue;
                    };
                    let max_hp = crate::constants::building_def(inst.kind).hp;
                    if bld_hp.0 >= max_hp {
                        continue;
                    }
                    bld_hp.0 = (bld_hp.0 + MASON_REPAIR_RATE).min(max_hp);
                    repaired = true;
                    if bld_hp.0 >= max_hp {
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            format!(
                                "Repaired {} to full HP",
                                crate::constants::building_def(inst.kind).label
                            ),
                        );
                    }
                    break; // repair one building per tick
                }
                if !repaired {
                    // No damaged building nearby -- go idle
                    transition_activity(
                        &mut activity,
                        ActivityKind::Idle,
                        ActivityPhase::Ready,
                        ActivityTarget::None,
                        "repair:done",
                    );
                    npc_logs.push(
                        idx,
                        game_time.day(),
                        game_time.hour(),
                        game_time.minute(),
                        "No damaged buildings -> Idle",
                    );
                }
                break 'decide;
            }

            // ====================================================================
            // Priority 6: OnDuty (tired -> leave post, else patrol when ready)
            // ====================================================================
            if activity.kind == ActivityKind::Patrol && activity.phase == ActivityPhase::Holding {
                let ticks = activity.ticks_waiting;
                let squad_forces_stay = job.is_patrol_unit()
                    && squad_id
                        .and_then(|sid| squad_state.squads.get(sid as usize))
                        .is_some_and(|s| !s.rest_when_tired);
                if energy < ENERGY_TIRED_THRESHOLD && !squad_forces_stay {
                    transition_activity(
                        &mut activity,
                        ActivityKind::Idle,
                        ActivityPhase::Ready,
                        ActivityTarget::None,
                        "transition",
                    );
                    npc_logs.push(
                        idx,
                        game_time.day(),
                        game_time.hour(),
                        game_time.minute(),
                        "Tired -> Left post",
                    );
                    // Fall through to idle scoring -- Rest will win
                } else {
                    let squad = squad_id.and_then(|sid| squad_state.squads.get(sid as usize));
                    let has_squad_target = squad.is_some_and(|s| s.target.is_some());
                    let squad_patrol_enabled = squad.is_none_or(|s| s.patrol_enabled);
                    let jitter = (idx % 30) as u32;
                    if !has_squad_target
                        && ticks >= ARCHER_PATROL_WAIT + jitter
                        && squad_patrol_enabled
                    {
                        if let Ok(route) = npc_data.patrol_route_q.get(entity) {
                            if !route.posts.is_empty() {
                                patrol_current = (patrol_current + 1) % route.posts.len();
                                if let Some(post) = route.posts.get(patrol_current) {
                                    transition_activity(
                                        &mut activity,
                                        ActivityKind::Patrol,
                                        ActivityPhase::Transit,
                                        ActivityTarget::PatrolPost {
                                            route: 0,
                                            index: patrol_current as u16,
                                        },
                                        "onduty:patrol_advance",
                                    );
                                    submit_intent_scattered(
                                        &mut intents,
                                        entity,
                                        post.x,
                                        post.y,
                                        128.0,
                                        idx,
                                        patrol_current,
                                        MovementPriority::JobRoute,
                                        "onduty:patrol_advance",
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
            let policy = economy.towns.policy(town_idx_i32);
            let food_available =
                policy.as_ref().is_none_or(|p| p.eat_food) && economy.towns.food(town_idx_i32) > 0;
            let mut scores: [(Action, f32); 5] = [(Action::Wander, 0.0); 5];
            let mut score_count: usize = 0;

            // Prioritize healing: wounded NPCs go to fountain before doing anything else
            // Skip if starving — HP capped at 50% until energy recovers
            if let Some(p) = &policy {
                if p.prioritize_healing && energy > 0.0 && health / max_hp < p.recovery_hp {
                    if let Some(town) = world_data.towns.get(town_idx) {
                        let center = town.center;
                        let threshold = policy.map(|p| p.recovery_hp).unwrap_or(0.8);
                        transition_activity(
                            &mut activity,
                            ActivityKind::Heal,
                            ActivityPhase::Transit,
                            ActivityTarget::Fountain,
                            "idle:heal_fountain",
                        );
                        activity.recover_until = threshold;
                        submit_intent_scattered(
                            &mut intents,
                            entity,
                            center.x,
                            center.y,
                            128.0,
                            idx,
                            frame,
                            MovementPriority::Survival,
                            "idle:heal_fountain",
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

            if en < ENERGY_HUNGRY && has_rest_destination(home_valid, town_center) {
                let rest_score = (ENERGY_HUNGRY - en) * SCORE_REST_MULT * rest_m;
                scores[score_count] = (Action::Rest, rest_score);
                score_count += 1;
            }

            // Work schedule gate: per-job schedule
            let schedule = match job {
                Job::Farmer | Job::Miner => policy
                    .as_ref()
                    .map(|p| p.farmer_schedule)
                    .unwrap_or(WorkSchedule::Both),
                Job::Archer | Job::Crossbow => policy
                    .as_ref()
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
                    Job::Farmer | Job::Mason => true,
                    Job::Miner => true,
                    Job::Woodcutter | Job::Quarrier => true,
                    Job::Archer | Job::Crossbow | Job::Fighter => has_patrol,
                    Job::Raider | Job::Boat => false,
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
                        .as_ref()
                        .map(|p| p.farmer_off_duty)
                        .unwrap_or(OffDutyBehavior::GoToBed),
                    Job::Archer | Job::Crossbow => policy
                        .as_ref()
                        .map(|p| p.archer_off_duty)
                        .unwrap_or(OffDutyBehavior::GoToBed),
                    _ => OffDutyBehavior::GoToBed,
                };
                match off_duty {
                    OffDutyBehavior::GoToBed => {
                        // Boost rest score so NPCs prefer going to bed
                        if has_rest_destination(home_valid, town_center) {
                            scores[score_count] = (Action::Rest, 80.0 * rest_m);
                            score_count += 1;
                        }
                    }
                    OffDutyBehavior::StayAtFountain => {
                        // Go to town center (fountain)
                        if let Some(town) = world_data.towns.get(town_idx) {
                            let center = town.center;
                            transition_activity(
                                &mut activity,
                                ActivityKind::Wander,
                                ActivityPhase::Transit,
                                ActivityTarget::None,
                                "transition",
                            );
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
                    if let Some(mut f) = economy.towns.food_mut(town_idx_i32) {
                        if f.0 > 0 {
                            let old_energy = energy;
                            f.0 -= 1;
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
                }
                Action::Rest => {
                    if transition_to_rest(
                        &mut activity,
                        &mut intents,
                        entity,
                        home_valid,
                        home,
                        town_center,
                        idx,
                        frame,
                        MovementPriority::Survival,
                        "idle:rest_home",
                        "idle:rest_fountain_homeless",
                    ) && activity.target == ActivityTarget::Fountain
                    {
                        npc_logs.push(
                            idx,
                            game_time.day(),
                            game_time.hour(),
                            game_time.minute(),
                            "Homeless -> Rest at fountain",
                        );
                    }
                }
                Action::Work => {
                    match job {
                        Job::Farmer => {
                            let current_pos = npc_pos.unwrap_or(home);
                            // Probe for available farm (read-only); defer claim to resolver
                            // Probe only — production state doesn't affect availability check
                            let empty_map = std::collections::HashMap::new();
                            if find_farmer_farm_target(
                                current_pos,
                                &entity_map,
                                town_idx_i32 as u32,
                                &empty_map,
                                &cow_farm_slots,
                            )
                            .is_some()
                            {
                                extras.work_intents.write(WorkIntentMsg(WorkIntent::Claim {
                                    entity,
                                    kind: BuildingKind::Farm,
                                    town_idx: town_idx_i32 as u32,
                                    from: current_pos,
                                }));
                                transition_activity(
                                    &mut activity,
                                    ActivityKind::Work,
                                    ActivityPhase::Transit,
                                    ActivityTarget::Worksite,
                                    "farm_claim_->_resolver",
                                );
                                npc_logs.push(
                                    idx,
                                    game_time.day(),
                                    game_time.hour(),
                                    game_time.minute(),
                                    "Farm claim -> resolver",
                                );
                            } else {
                                // No available farm — clear stale target and wander
                                worksite = None;
                                let base = if home_valid {
                                    home
                                } else if let Some(pos) = npc_pos {
                                    pos
                                } else {
                                    break 'decide;
                                };
                                let offset_x = (pseudo_random(idx, frame + 1) - 0.5) * 200.0;
                                let offset_y = (pseudo_random(idx, frame + 2) - 0.5) * 200.0;
                                transition_activity(
                                    &mut activity,
                                    ActivityKind::Wander,
                                    ActivityPhase::Transit,
                                    ActivityTarget::None,
                                    "idle:wander_no_farm",
                                );
                                submit_intent(
                                    &mut intents,
                                    entity,
                                    base.x + offset_x,
                                    base.y + offset_y,
                                    MovementPriority::Wander,
                                    "idle:wander_no_farm",
                                );
                            }
                        }
                        Job::Miner => {
                            // Check for manually assigned mine (via miner home ECS component)
                            let assigned = entity_map
                                .find_by_position(home)
                                .filter(|inst| inst.kind == BuildingKind::MinerHome)
                                .and_then(|inst| entity_map.entities.get(&inst.slot).copied())
                                .and_then(|e| miner_cfg_q.get(e).ok())
                                .and_then(|cfg| cfg.assigned_mine);

                            let mine_target = if let Some(assigned_pos) = assigned {
                                Some(assigned_pos)
                            } else {
                                let current_pos = npc_pos.unwrap_or(home);
                                // Pre-collect production readiness for score closure
                                // (closure borrows entity_map, can't query ECS from inside)
                                let mine_ready: std::collections::HashMap<usize, bool> = entity_map
                                    .iter_kind(BuildingKind::GoldMine)
                                    .filter_map(|inst| {
                                        let e = *entity_map.entities.get(&inst.slot)?;
                                        let ready = production_q.get(e).is_ok_and(|ps| ps.ready);
                                        Some((inst.slot, ready))
                                    })
                                    .collect();
                                // Spatial cell-ring search: ready > unoccupied > occupied, then nearest
                                entity_map
                                    .find_nearest_worksite(
                                        current_pos,
                                        BuildingKind::GoldMine,
                                        town_idx_i32 as u32,
                                        crate::resources::WorksiteFallback::AnyTown,
                                        6400.0,
                                        |inst, occ| {
                                            let ready = mine_ready
                                                .get(&inst.slot)
                                                .copied()
                                                .unwrap_or(false);
                                            let priority = if ready {
                                                0u8
                                            } else if occ == 0 {
                                                1
                                            } else {
                                                2
                                            };
                                            Some((
                                                priority,
                                                occ as u16,
                                                inst.position
                                                    .distance_squared(current_pos)
                                                    .to_bits(),
                                            ))
                                        },
                                    )
                                    .map(|r| r.position)
                            };

                            if let Some(mine_pos) = mine_target {
                                transition_activity(
                                    &mut activity,
                                    ActivityKind::Mine,
                                    ActivityPhase::Transit,
                                    ActivityTarget::Worksite,
                                    "transition",
                                );
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
                                        transition_activity(
                                            &mut activity,
                                            ActivityKind::SquadAttack,
                                            ActivityPhase::Transit,
                                            ActivityTarget::SquadPoint(target),
                                            "transition",
                                        );
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
                                        transition_activity(
                                            &mut activity,
                                            ActivityKind::Patrol,
                                            ActivityPhase::Transit,
                                            ActivityTarget::PatrolPost {
                                                route: 0,
                                                index: patrol_current as u16,
                                            },
                                            "idle:patrol_route",
                                        );
                                        submit_intent_scattered(
                                            &mut intents,
                                            entity,
                                            post.x,
                                            post.y,
                                            128.0,
                                            idx,
                                            patrol_current,
                                            MovementPriority::JobRoute,
                                            "idle:patrol_route",
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
                                transition_activity(
                                    &mut activity,
                                    ActivityKind::Wander,
                                    ActivityPhase::Transit,
                                    ActivityTarget::None,
                                    "transition",
                                );
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
                        Job::Woodcutter => {
                            let current_pos = npc_pos.unwrap_or(home);
                            extras.work_intents.write(WorkIntentMsg(WorkIntent::Claim {
                                entity,
                                kind: BuildingKind::TreeNode,
                                town_idx: town_idx_i32 as u32,
                                from: current_pos,
                            }));
                            transition_activity(
                                &mut activity,
                                ActivityKind::Work,
                                ActivityPhase::Transit,
                                ActivityTarget::Worksite,
                                "work_claim_tree",
                            );
                        }
                        Job::Quarrier => {
                            let current_pos = npc_pos.unwrap_or(home);
                            extras.work_intents.write(WorkIntentMsg(WorkIntent::Claim {
                                entity,
                                kind: BuildingKind::RockNode,
                                town_idx: town_idx_i32 as u32,
                                from: current_pos,
                            }));
                            transition_activity(
                                &mut activity,
                                ActivityKind::Work,
                                ActivityPhase::Transit,
                                ActivityTarget::Worksite,
                                "work_claim_rock",
                            );
                        }
                        Job::Boat => {} // CPU-driven movement, no behavior
                        Job::Mason => {
                            let current_pos = npc_pos.unwrap_or(home);
                            let max_dist_sq = MASON_SEARCH_RADIUS * MASON_SEARCH_RADIUS;
                            let mut best: Option<(f32, Vec2)> = None;
                            for inst in entity_map.iter_instances() {
                                if inst.town_idx != town_idx_i32 as u32 {
                                    continue;
                                }
                                let dist_sq = inst.position.distance_squared(current_pos);
                                if dist_sq > max_dist_sq {
                                    continue;
                                }
                                let Some(bld_entity) = entity_map.entities.get(&inst.slot).copied()
                                else {
                                    continue;
                                };
                                let Ok(bld_hp) = building_health_q.get(bld_entity) else {
                                    continue;
                                };
                                let max_hp = crate::constants::building_def(inst.kind).hp;
                                if bld_hp.0 >= max_hp {
                                    continue;
                                }
                                if best.as_ref().is_none_or(|b| dist_sq < b.0) {
                                    best = Some((dist_sq, inst.position));
                                }
                            }
                            if let Some((_, target_pos)) = best {
                                transition_activity(
                                    &mut activity,
                                    ActivityKind::Repair,
                                    ActivityPhase::Transit,
                                    ActivityTarget::None,
                                    "transition",
                                );
                                submit_intent(
                                    &mut intents,
                                    entity,
                                    target_pos.x,
                                    target_pos.y,
                                    MovementPriority::JobRoute,
                                    "idle:work_repair",
                                );
                            }
                        }
                    }
                }
                Action::Wander => {
                    // Wander from current position, clamped to stay near home
                    let base = if let Some(pos) = npc_pos {
                        pos
                    } else if home_valid {
                        home
                    } else {
                        break 'decide;
                    };
                    let offset_x = (pseudo_random(idx, frame + 1) - 0.5) * 128.0;
                    let offset_y = (pseudo_random(idx, frame + 2) - 0.5) * 128.0;
                    let mut target = Vec2::new(base.x + offset_x, base.y + offset_y);
                    if home_valid {
                        let diff = target - home;
                        let dist = diff.length();
                        if dist > 200.0 {
                            target = home + diff * (200.0 / dist);
                        }
                    }
                    transition_activity(
                        &mut activity,
                        ActivityKind::Wander,
                        ActivityPhase::Transit,
                        ActivityTarget::None,
                        "transition",
                    );
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
            && activity.kind != ActivityKind::Work
        {
            let uid = worksite.and_then(|s| entity_map.entities.get(&s).copied());
            extras
                .work_intents
                .write(WorkIntentMsg(WorkIntent::Release {
                    entity,
                    worksite: uid,
                }));
            worksite = None;
            worksite_deferred = true;
            npc_logs.push(
                idx,
                game_time.day(),
                game_time.hour(),
                game_time.minute(),
                "Released stale worksite",
            );
        }

        // Conditional writeback: skip unchanged NPCs (most exit early via break 'decide)
        let new_visual_key = (activity.visual_key(), carried_loot.visual_key());
        if activity != orig_activity {
            // Clear stale GPU target when going Idle -- prevents oscillation with nearby NPCs
            if activity.kind == ActivityKind::Idle && activity.kind != orig_activity.kind {
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
            let orig_cl = npc_data.carried_loot_q.get(entity).ok();
            let changed = orig_cl.as_ref().is_none_or(|cl| {
                cl.food != carried_loot.food
                    || cl.gold != carried_loot.gold
                    || cl.wood != carried_loot.wood
                    || cl.stone != carried_loot.stone
            });
            if changed {
                if let Ok(mut cl) = npc_data.carried_loot_q.get_mut(entity) {
                    cl.food = carried_loot.food;
                    cl.gold = carried_loot.gold;
                    cl.wood = carried_loot.wood;
                    cl.stone = carried_loot.stone;
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
                ws.worksite = worksite.and_then(|s| entity_map.entities.get(&s).copied());
            }
        }
        if patrol_current != orig_patrol_current {
            if let Ok(mut route) = npc_data.patrol_route_q.get_mut(entity) {
                route.current = patrol_current;
            }
        }
    }
}

#[cfg(test)]
mod tests;
