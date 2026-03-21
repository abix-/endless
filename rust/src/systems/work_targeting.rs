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
    production_q: Query<&ProductionState, With<Building>>,
    farm_mode_q: Query<&FarmModeComp, With<Building>>,
) {
    let msgs: Vec<_> = intents.read().collect();
    if msgs.is_empty() {
        return;
    }

    // Only Claim/Retarget need the production and cow-farm maps; skip the building scan
    // entirely for Release/MarkPresent-only batches (the common steady-state path).
    let needs_claim_data = msgs.iter().any(|WorkIntentMsg(intent)| {
        matches!(
            intent,
            WorkIntent::Claim { .. } | WorkIntent::Retarget { .. }
        )
    });

    // Single pass over all building instances to build both maps simultaneously.
    // Replaces the previous two separate iter_instances() passes.
    let mut production_map: std::collections::HashMap<usize, (bool, f32)> =
        std::collections::HashMap::new();
    let mut cow_farm_slots: std::collections::HashSet<usize> = std::collections::HashSet::new();

    if needs_claim_data {
        for inst in entity_map.iter_instances() {
            let Some(&entity) = entity_map.entities.get(&inst.slot) else {
                continue;
            };
            if let Ok(ps) = production_q.get(entity) {
                production_map.insert(inst.slot, (ps.ready, ps.progress));
            }
            if inst.kind == BuildingKind::Farm {
                if let Ok(fm) = farm_mode_q.get(entity) {
                    if fm.0 == FarmMode::Cows {
                        cow_farm_slots.insert(inst.slot);
                    }
                }
            }
        }
    }

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
                    mark_sleeping_dirty_if_resource(&mut entity_map, slot);
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
            mark_sleeping_dirty_if_resource(entity_map, slot);
            entity_map.release_for(slot, Some(entity));
        }
    }
}

/// Release by carried worksite entity (used when decision_system write-back already cleared the component).
fn release_worksite_entity(npc: Entity, worksite: Option<Entity>, entity_map: &mut EntityMap) {
    if let Some(ws_entity) = worksite {
        if let Some(slot) = entity_map.slot_for_entity(ws_entity) {
            mark_sleeping_dirty_if_resource(entity_map, slot);
            entity_map.release_for(slot, Some(npc));
        }
    }
}

#[inline]
fn mark_sleeping_dirty_if_resource(entity_map: &mut EntityMap, slot: usize) {
    if entity_map
        .get_instance(slot)
        .is_some_and(|inst| matches!(inst.kind, BuildingKind::TreeNode | BuildingKind::RockNode))
    {
        entity_map.sleeping_dirty.push(slot);
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;

    use crate::components::{
        Activity, Building, FarmMode, FarmModeComp, GpuSlot, Health, NpcWorkState, Position,
        ProductionState,
    };
    use crate::entity_map::BuildingInstance;
    use crate::messages::{WorkIntent, WorkIntentMsg};
    use crate::resources::{EntityMap, GpuSlotPool, PathRequestQueue};
    use crate::world::BuildingKind;
    use bevy::prelude::*;

    fn setup_work_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<WorkIntentMsg>();
        app.init_resource::<EntityMap>();
        app.init_resource::<PathRequestQueue>();
        app.init_resource::<GpuSlotPool>();
        {
            let mut em = app.world_mut().resource_mut::<EntityMap>();
            em.init_spatial(1600.0);
        }
        app
    }

    fn alloc_slot(app: &mut App) -> usize {
        app.world_mut()
            .resource_mut::<GpuSlotPool>()
            .alloc_reset()
            .expect("slot available")
    }

    fn register_farm(
        app: &mut App,
        slot: usize,
        x: f32,
        y: f32,
        mode: FarmMode,
        production_ready: bool,
    ) -> Entity {
        let entity = app
            .world_mut()
            .spawn((
                GpuSlot(slot),
                Position { x, y },
                Health(100.0),
                crate::components::Faction(1),
                crate::components::TownId(0),
                Building {
                    kind: BuildingKind::Farm,
                },
                ProductionState {
                    ready: production_ready,
                    progress: 1.0,
                },
                FarmModeComp(mode),
            ))
            .id();
        let mut em = app.world_mut().resource_mut::<EntityMap>();
        em.set_entity(slot, entity);
        em.add_instance(BuildingInstance {
            kind: BuildingKind::Farm,
            position: Vec2::new(x, y),
            town_idx: 0,
            slot,
            faction: 1,
        });
        entity
    }

    /// Verify the merged single-pass correctly populates cow_farm_slots:
    /// a farmer claiming a Farm should receive the Crops farm, not the Cows farm.
    /// This test would fail if the cow-farm exclusion were dropped from the merged pass.
    #[test]
    fn claim_skips_cow_farm_assigns_crops_farm() {
        let mut app = setup_work_app();

        let crops_slot = alloc_slot(&mut app);
        let cows_slot = alloc_slot(&mut app);
        let npc_slot = alloc_slot(&mut app);

        let crops_farm = register_farm(&mut app, crops_slot, 100.0, 100.0, FarmMode::Crops, true);
        let _cows_farm = register_farm(&mut app, cows_slot, 110.0, 100.0, FarmMode::Cows, true);

        let npc = app
            .world_mut()
            .spawn((
                GpuSlot(npc_slot),
                Activity::default(),
                NpcWorkState::default(),
            ))
            .id();

        let _ = app
            .world_mut()
            .run_system_once(move |mut writer: MessageWriter<WorkIntentMsg>| {
                writer.write(WorkIntentMsg(WorkIntent::Claim {
                    entity: npc,
                    kind: BuildingKind::Farm,
                    town_idx: 0,
                    from: Vec2::new(0.0, 0.0),
                }));
            });
        let _ = app.world_mut().run_system_once(resolve_work_targets);

        let ws = app.world().get::<NpcWorkState>(npc).unwrap();
        assert!(
            ws.worksite.is_some(),
            "farmer should be assigned a worksite"
        );
        assert_eq!(
            ws.worksite.unwrap(),
            crops_farm,
            "farmer must claim the Crops farm, not the Cows farm"
        );
    }

    /// Verify Release-only batch clears NpcWorkState without panicking.
    /// This test would fail if Release handling were broken by the lazy gate.
    #[test]
    fn release_clears_worksite() {
        let mut app = setup_work_app();

        let crops_slot = alloc_slot(&mut app);
        let npc_slot = alloc_slot(&mut app);

        let crops_farm = register_farm(&mut app, crops_slot, 100.0, 100.0, FarmMode::Crops, true);

        let npc = app
            .world_mut()
            .spawn((
                GpuSlot(npc_slot),
                Activity::default(),
                NpcWorkState {
                    worksite: Some(crops_farm),
                },
            ))
            .id();

        let _ = app
            .world_mut()
            .run_system_once(move |mut writer: MessageWriter<WorkIntentMsg>| {
                writer.write(WorkIntentMsg(WorkIntent::Release {
                    entity: npc,
                    worksite: Some(crops_farm),
                }));
            });
        let _ = app.world_mut().run_system_once(resolve_work_targets);

        let ws = app.world().get::<NpcWorkState>(npc).unwrap();
        assert!(
            ws.worksite.is_none(),
            "worksite should be cleared after Release"
        );
    }
}
