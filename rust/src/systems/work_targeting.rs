//! Centralized work targeting — single owner of worksite occupancy mutations.
//!
//! All systems declare intent via `WorkIntentMsg`; this system resolves them:
//! - Claim: spatial search → try_claim_worksite → update NpcWorkState + submit movement
//! - Release: entity_map.release → clear NpcWorkState
//! - Retarget: Release then Claim atomically

use bevy::prelude::*;

use crate::components::*;
use crate::constants::building_def;
use crate::messages::{WorkIntent, WorkIntentMsg};
use crate::resources::{EntityMap, MovementPriority, PathRequestQueue, WorksiteFallback};
use crate::world::BuildingKind;

/// Drain all `WorkIntentMsg` and execute claim/release/retarget as the single authority.
pub fn resolve_work_targets(
    mut intents: MessageReader<WorkIntentMsg>,
    mut entity_map: ResMut<EntityMap>,
    mut work_state_q: Query<&mut NpcWorkState>,
    mut activity_q: Query<&mut crate::components::Activity>,
    mut path_queue: ResMut<PathRequestQueue>,
) {
    for WorkIntentMsg(intent) in intents.read() {
        match intent {
            WorkIntent::Release { entity, uid } => {
                release_worksite_uid(*uid, &mut entity_map);
                clear_worksite(*entity, &mut work_state_q);
            }
            WorkIntent::Claim { entity, kind, town_idx, from } => {
                release_worksite(*entity, &mut entity_map, &mut work_state_q);
                claim_worksite(*entity, *kind, *town_idx, *from, &mut entity_map, &mut work_state_q, &mut activity_q, &mut path_queue);
            }
            WorkIntent::Retarget { entity, kind, town_idx, from } => {
                release_worksite(*entity, &mut entity_map, &mut work_state_q);
                claim_worksite(*entity, *kind, *town_idx, *from, &mut entity_map, &mut work_state_q, &mut activity_q, &mut path_queue);
            }
        }
    }
}

fn release_worksite(
    entity: Entity,
    entity_map: &mut EntityMap,
    work_state_q: &mut Query<&mut NpcWorkState>,
) {
    let Ok(mut ws) = work_state_q.get_mut(entity) else { return };
    if let Some(uid) = ws.worksite.take() {
        if let Some(slot) = entity_map.slot_for_uid(uid) {
            entity_map.release(slot);
        }
    }
}

/// Release by carried UID (used when decision_system write-back already cleared the component).
fn release_worksite_uid(uid: Option<EntityUid>, entity_map: &mut EntityMap) {
    if let Some(uid) = uid {
        if let Some(slot) = entity_map.slot_for_uid(uid) {
            entity_map.release(slot);
        }
    }
}

/// Clear NpcWorkState.worksite (idempotent if write-back already cleared it).
fn clear_worksite(entity: Entity, work_state_q: &mut Query<&mut NpcWorkState>) {
    if let Ok(mut ws) = work_state_q.get_mut(entity) {
        ws.worksite = None;
    }
}

fn claim_worksite(
    entity: Entity,
    kind: BuildingKind,
    town_idx: u32,
    from: Vec2,
    entity_map: &mut EntityMap,
    work_state_q: &mut Query<&mut NpcWorkState>,
    activity_q: &mut Query<&mut crate::components::Activity>,
    path_queue: &mut PathRequestQueue,
) {
    let max_occupants = match building_def(kind).worksite {
        Some(ws) => ws.max_occupants,
        None => return,
    };

    // Spatial search for best worksite
    let result = match kind {
        BuildingKind::Farm => find_farm_target(from, entity_map, town_idx),
        BuildingKind::GoldMine => find_mine_target(from, entity_map, town_idx),
        _ => return,
    };

    let Some((target_slot, ..)) = result else {
        // No worksite available — revert Activity to Idle
        if let Ok(mut act) = activity_q.get_mut(entity) {
            *act = crate::components::Activity::default();
        }
        return;
    };

    // Authoritative claim
    let Some(claimed) = entity_map.try_claim_worksite(
        target_slot,
        kind,
        Some(town_idx),
        max_occupants,
    ) else {
        // Claim failed — revert Activity to Idle
        if let Ok(mut act) = activity_q.get_mut(entity) {
            *act = crate::components::Activity::default();
        }
        return;
    };

    // Update NpcWorkState
    if let Ok(mut ws) = work_state_q.get_mut(entity) {
        let uid = entity_map.uid_for_slot(claimed.slot);
        ws.worksite = uid;
    }

    // Submit movement intent
    path_queue.submit(entity, claimed.position, MovementPriority::JobRoute, "work:claim");
}

/// Find best available farm for a farmer. Returns (slot, position, search_radius).
pub(crate) fn find_farm_target(from: Vec2, entity_map: &EntityMap, town_idx: u32) -> Option<(usize, Vec2, f32)> {
    let max_occ = building_def(BuildingKind::Farm).worksite.expect("Farm has worksite").max_occupants;
    entity_map.find_nearest_worksite(
        from,
        BuildingKind::Farm,
        town_idx,
        WorksiteFallback::TownOnly,
        6400.0,
        |inst| {
            if inst.occupants as i32 >= max_occ {
                return None;
            }
            let not_ready: u8 = if inst.growth_ready { 0 } else { 1 };
            let inv_growth = (1.0 - inst.growth_progress).to_bits();
            let d2 = (inst.position - from).length_squared().to_bits();
            Some((not_ready, inv_growth, d2))
        },
    )
    .map(|r| (r.slot, r.position, r.radius_used))
}

fn find_mine_target(from: Vec2, entity_map: &EntityMap, town_idx: u32) -> Option<(usize, Vec2, f32)> {
    entity_map.find_nearest_worksite(
        from,
        BuildingKind::GoldMine,
        town_idx,
        WorksiteFallback::AnyTown,
        6400.0,
        |inst| {
            let priority = if inst.growth_ready {
                0u8
            } else if inst.occupants == 0 {
                1
            } else {
                2
            };
            Some((priority, (inst.position - from).length_squared().to_bits()))
        },
    )
    .map(|r| (r.slot, r.position, r.radius_used))
}
