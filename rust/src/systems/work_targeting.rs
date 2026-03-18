//! Centralized work targeting — single owner of worksite occupancy mutations.
//!
//! All systems declare intent via `WorkIntentMsg`; this system resolves them:
//! - Claim: spatial search → try_claim_worksite → update NpcWorkState + submit movement
//! - Release: entity_map.release_for (occupancy + claim-queue cleanup) → clear NpcWorkState
//! - Retarget: Release then Claim atomically
//! - MarkPresent: increments EntityMap.present when a worker physically arrives

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
    production_q: Query<(&GpuSlot, &ProductionState), With<Building>>,
    farm_mode_q: Query<(&GpuSlot, &FarmModeComp), With<Building>>,
) {
    let msgs: Vec<_> = intents.read().collect();
    if msgs.is_empty() {
        return;
    }

    // Only build the lookup maps when at least one Claim or Retarget intent exists.
    // Release and MarkPresent never use production_map or cow_farm_slots.
    let needs_claim = msgs.iter().any(|WorkIntentMsg(w)| {
        matches!(w, WorkIntent::Claim { .. } | WorkIntent::Retarget { .. })
    });

    // Build production map by iterating only buildings that have ProductionState (~1K)
    // instead of scanning all 68K instances via iter_instances().
    let production_map: std::collections::HashMap<usize, (bool, f32)> = if needs_claim {
        production_q
            .iter()
            .map(|(slot, ps)| (slot.0, (ps.ready, ps.progress)))
            .collect()
    } else {
        std::collections::HashMap::new()
    };

    // Build cow-farm set by iterating only buildings with FarmModeComp (~1K farms).
    let cow_farm_slots: std::collections::HashSet<usize> = if needs_claim {
        farm_mode_q
            .iter()
            .filter_map(|(slot, fm)| {
                if fm.0 == FarmMode::Cows {
                    Some(slot.0)
                } else {
                    None
                }
            })
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    for WorkIntentMsg(intent) in msgs {
        match intent {
            WorkIntent::Release { entity, worksite } => {
                release_worksite_entity(*entity, *worksite, &mut entity_map);
                clear_worksite(*entity, &mut work_state_q);
            }
            WorkIntent::Claim {
                entity,
                kind,
                town_idx,
                from,
            } => {
                release_worksite(*entity, &mut entity_map, &mut work_state_q);
                claim_worksite(
                    *entity,
                    *kind,
                    *town_idx,
                    *from,
                    &mut entity_map,
                    &mut work_state_q,
                    &mut activity_q,
                    &mut path_queue,
                    &production_map,
                    &cow_farm_slots,
                );
            }
            WorkIntent::Retarget {
                entity,
                kind,
                town_idx,
                from,
            } => {
                release_worksite(*entity, &mut entity_map, &mut work_state_q);
                claim_worksite(
                    *entity,
                    *kind,
                    *town_idx,
                    *from,
                    &mut entity_map,
                    &mut work_state_q,
                    &mut activity_q,
                    &mut path_queue,
                    &production_map,
                    &cow_farm_slots,
                );
            }
            WorkIntent::MarkPresent {
                entity: _,
                worksite,
            } => {
                if let Some(slot) = entity_map.entity_to_slot.get(worksite).copied() {
                    entity_map.mark_present(slot);
                }
            }
        }
    }
}

fn release_worksite(
    entity: Entity,
    entity_map: &mut EntityMap,
    work_state_q: &mut Query<&mut NpcWorkState>,
) {
    let Ok(mut ws) = work_state_q.get_mut(entity) else {
        return;
    };
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
    cow_farm_slots: &std::collections::HashSet<usize>,
) {
    let max_occupants = match building_def(kind).worksite {
        Some(ws) => ws.max_occupants,
        None => return,
    };

    // Spatial search for best worksite
    let result = match kind {
        BuildingKind::Farm => {
            find_farm_target(from, entity_map, town_idx, production_map, cow_farm_slots)
        }
        BuildingKind::GoldMine => find_mine_target(from, entity_map, town_idx, production_map),
        BuildingKind::TreeNode | BuildingKind::RockNode => {
            find_node_target(from, entity_map, town_idx, kind)
        }
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
    path_queue.submit(
        entity,
        claimed.position,
        MovementPriority::JobRoute,
        "work:claim",
    );
}

/// Find best available farm for a farmer. Returns (slot, position, search_radius).
pub(crate) fn find_farm_target(
    from: Vec2,
    entity_map: &EntityMap,
    town_idx: u32,
    production_map: &std::collections::HashMap<usize, (bool, f32)>,
    cow_farm_slots: &std::collections::HashSet<usize>,
) -> Option<(usize, Vec2, f32)> {
    let max_occ = building_def(BuildingKind::Farm)
        .worksite
        .expect("Farm has worksite")
        .max_occupants;
    entity_map
        .find_nearest_worksite(
            from,
            BuildingKind::Farm,
            town_idx,
            WorksiteFallback::TownOnly,
            6400.0,
            |inst, occ| {
                // Skip cow farms -- they don't need farmer tending
                if cow_farm_slots.contains(&inst.slot) {
                    return None;
                }
                if occ as i32 >= max_occ {
                    return None;
                }
                let (ready, progress) = production_map
                    .get(&inst.slot)
                    .copied()
                    .unwrap_or((false, 0.0));
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
    entity_map
        .find_nearest_worksite(
            from,
            BuildingKind::GoldMine,
            town_idx,
            WorksiteFallback::AnyTown,
            6400.0,
            |inst, occ| {
                let (ready, _) = production_map
                    .get(&inst.slot)
                    .copied()
                    .unwrap_or((false, 0.0));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_map::{BuildingInstance, EntityMap};
    use crate::messages::WorkIntentMsg;
    use crate::resources::PathRequestQueue;
    use bevy::time::TimeUpdateStrategy;

    fn setup_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<WorkIntentMsg>();
        app.insert_resource(EntityMap::default());
        app.insert_resource(PathRequestQueue::default());
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        ));
        app.add_systems(FixedUpdate, resolve_work_targets);
        // Prime FixedUpdate scheduler
        app.update();
        app.update();
        app
    }

    fn spawn_farm(app: &mut App, slot: usize, pos: Vec2, cow: bool) -> Entity {
        let mut inst = BuildingInstance {
            kind: BuildingKind::Farm,
            position: pos,
            town_idx: 0,
            slot,
            faction: 0,
        };
        inst.position = pos;
        let entity = app
            .world_mut()
            .spawn((
                GpuSlot(slot),
                Building {
                    kind: BuildingKind::Farm,
                },
                ProductionState::default(),
                FarmModeComp(if cow { FarmMode::Cows } else { FarmMode::Crops }),
            ))
            .id();
        let mut em = app.world_mut().resource_mut::<EntityMap>();
        em.set_entity(slot, entity);
        em.add_instance(inst);
        entity
    }

    fn spawn_farmer(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                GpuSlot(100),
                NpcWorkState::default(),
                crate::components::Activity::default(),
            ))
            .id()
    }

    /// Regression test: resolve_work_targets must skip cow farms when building
    /// cow_farm_slots via GpuSlot+FarmModeComp ECS queries.
    /// Reverts to the old iter_instances() path would not compile with the new query signature.
    #[test]
    fn cow_farm_skipped_claim_assigns_crop_farm() {
        let mut app = setup_app();

        // Slot 0: cow farm (must be skipped)
        // Slot 1: crop farm at same position (must be claimed)
        let cow_pos = Vec2::new(50.0, 50.0);
        let crop_pos = Vec2::new(60.0, 60.0);
        let from = Vec2::new(55.0, 55.0);
        app.world_mut()
            .resource_mut::<EntityMap>()
            .init_spatial(2048.0);
        spawn_farm(&mut app, 0, cow_pos, true);
        spawn_farm(&mut app, 1, crop_pos, false);

        let farmer = spawn_farmer(&mut app);
        app.world_mut().write_message(WorkIntentMsg(WorkIntent::Claim {
            entity: farmer,
            kind: BuildingKind::Farm,
            town_idx: 0,
            from,
        }));

        app.update();

        let ws = app.world().get::<NpcWorkState>(farmer).unwrap();
        assert!(
            ws.worksite.is_some(),
            "farmer should have claimed a worksite"
        );
        // The claimed entity should be the crop farm (slot 1), not the cow farm (slot 0)
        let claimed_entity = ws.worksite.unwrap();
        let em = app.world().resource::<EntityMap>();
        let claimed_slot = em.slot_for_entity(claimed_entity);
        assert_eq!(
            claimed_slot,
            Some(1),
            "farmer must claim crop farm (slot 1), not cow farm (slot 0)"
        );
    }
}

/// Find nearest available resource node (TreeNode or RockNode) for woodcutter/quarrier.
fn find_node_target(
    from: Vec2,
    entity_map: &EntityMap,
    town_idx: u32,
    kind: BuildingKind,
) -> Option<(usize, Vec2, f32)> {
    entity_map
        .find_nearest_worksite(
            from,
            kind,
            town_idx,
            WorksiteFallback::AnyTown,
            6400.0,
            |inst, occ| {
                let priority = if occ == 0 { 0u8 } else { 1 };
                Some((priority, (inst.position - from).length_squared().to_bits()))
            },
        )
        .map(|r| (r.slot, r.position, r.radius_used))
}
