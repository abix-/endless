//! Centralized work targeting — single owner of worksite occupancy mutations.
//!
//! All systems declare intent via `WorkIntentMsg`; this system resolves them:
//! - Claim: spatial search → try_claim_worksite → update NpcWorkState + submit movement
//! - Release: entity_map.release_for (occupancy + claim-queue cleanup) → clear NpcWorkState
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
    production_q: Query<&ProductionState, With<Building>>,
) {
    let msgs: Vec<_> = intents.read().collect();
    if msgs.is_empty() {
        return;
    }

    // Pre-collect production state only when there are messages to process.
    // Only Claim/Retarget need it, but building the map is cheaper than checking each message type.
    let production_map: std::collections::HashMap<usize, (bool, f32)> = entity_map
        .iter_instances()
        .filter_map(|inst| {
            let entity = entity_map.entities.get(&inst.slot)?;
            let ps = production_q.get(*entity).ok()?;
            Some((inst.slot, (ps.ready, ps.progress)))
        })
        .collect();
    for WorkIntentMsg(intent) in msgs {
        match intent {
            WorkIntent::Release { entity, worksite } => {
                release_worksite_entity(*entity, *worksite, &mut entity_map);
                clear_worksite(*entity, &mut work_state_q);
            }
            WorkIntent::Claim { entity, kind, town_idx, from } => {
                release_worksite(*entity, &mut entity_map, &mut work_state_q);
                claim_worksite(*entity, *kind, *town_idx, *from, &mut entity_map, &mut work_state_q, &mut activity_q, &mut path_queue, &production_map);
            }
            WorkIntent::Retarget { entity, kind, town_idx, from } => {
                release_worksite(*entity, &mut entity_map, &mut work_state_q);
                claim_worksite(*entity, *kind, *town_idx, *from, &mut entity_map, &mut work_state_q, &mut activity_q, &mut path_queue, &production_map);
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
    if let Some(ws_entity) = ws.worksite.take() {
        if let Some(slot) = entity_map.slot_for_entity(ws_entity) {
            entity_map.release_for(slot, Some(entity));
        }
    }
}

/// Release by carried worksite entity (used when decision_system write-back already cleared the component).
fn release_worksite_entity(npc: Entity, worksite: Option<Entity>, entity_map: &mut EntityMap) {
    if let Some(ws_entity) = worksite {
        if let Some(slot) = entity_map.slot_for_entity(ws_entity) {
            entity_map.release_for(slot, Some(npc));
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
    production_map: &std::collections::HashMap<usize, (bool, f32)>,
) {
    let max_occupants = match building_def(kind).worksite {
        Some(ws) => ws.max_occupants,
        None => return,
    };

    // Spatial search for best worksite
    let result = match kind {
        BuildingKind::Farm => find_farm_target(from, entity_map, town_idx, production_map),
        BuildingKind::GoldMine => find_mine_target(from, entity_map, town_idx, production_map),
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
        Some(entity),
    ) else {
        // Claim failed — revert Activity to Idle
        if let Ok(mut act) = activity_q.get_mut(entity) {
            *act = crate::components::Activity::default();
        }
        return;
    };

    // Update NpcWorkState
    if let Ok(mut ws) = work_state_q.get_mut(entity) {
        ws.worksite = entity_map.entities.get(&claimed.slot).copied();
    }

    // Submit movement intent
    path_queue.submit(entity, claimed.position, MovementPriority::JobRoute, "work:claim");
}

/// Find best available farm for a farmer. Returns (slot, position, search_radius).
pub(crate) fn find_farm_target(
    from: Vec2,
    entity_map: &EntityMap,
    town_idx: u32,
    production_map: &std::collections::HashMap<usize, (bool, f32)>,
) -> Option<(usize, Vec2, f32)> {
    let max_occ = building_def(BuildingKind::Farm).worksite.expect("Farm has worksite").max_occupants;
    entity_map.find_nearest_worksite(
        from,
        BuildingKind::Farm,
        town_idx,
        WorksiteFallback::TownOnly,
        6400.0,
        |inst, occ| {
            if occ as i32 >= max_occ {
                return None;
            }
            let (ready, progress) = production_map.get(&inst.slot).copied().unwrap_or((false, 0.0));
            let not_ready: u8 = if ready { 0 } else { 1 };
            let inv_growth = (1.0 - progress).to_bits();
            let d2 = (inst.position - from).length_squared().to_bits();
            Some((not_ready, inv_growth, d2))
        },
    )
    .map(|r| (r.slot, r.position, r.radius_used))
}

fn find_mine_target(
    from: Vec2,
    entity_map: &EntityMap,
    town_idx: u32,
    production_map: &std::collections::HashMap<usize, (bool, f32)>,
) -> Option<(usize, Vec2, f32)> {
    entity_map.find_nearest_worksite(
        from,
        BuildingKind::GoldMine,
        town_idx,
        WorksiteFallback::AnyTown,
        6400.0,
        |inst, occ| {
            let (ready, _) = production_map.get(&inst.slot).copied().unwrap_or((false, 0.0));
            let priority = if ready {
                0u8
            } else if occ == 0 {
                1
            } else {
                2
            };
            Some((priority, (inst.position - from).length_squared().to_bits()))
        },
    )
    .map(|r| (r.slot, r.position, r.radius_used))
}
