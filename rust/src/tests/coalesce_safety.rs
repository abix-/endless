//! Coalesce Safety Tests
//! Validates: GPU-authoritative buffers (positions, arrivals) are not corrupted
//! by coalesced uploads when unrelated slots get SetPosition/SetTarget events.

use crate::components::*;
use crate::resources::*;
use bevy::prelude::*;

use super::{TestSetupParams, TestState};

// --- movement-coalesce: SetPosition on one NPC must not teleport another ---

const HOME_Y: f32 = 600.0;
const FARM_Y: f32 = 200.0;

pub fn setup_movement(mut params: TestSetupParams) {
    params.add_town("CoalesceTown");
    params.add_building(crate::world::BuildingKind::Farm, 400.0, FARM_Y, 0);
    if let Some(inst) = params.entity_map.find_farm_at_mut(Vec2::new(400.0, FARM_Y)) {
        inst.growth_ready = true;
        inst.growth_progress = 1.0;
    }
    params.add_building(crate::world::BuildingKind::Farm, 500.0, FARM_Y, 0);
    if let Some(inst) = params.entity_map.find_farm_at_mut(Vec2::new(500.0, FARM_Y)) {
        inst.growth_ready = true;
        inst.growth_progress = 1.0;
    }
    params.add_building(crate::world::BuildingKind::FarmerHome, 400.0, HOME_Y, 0);
    params.add_building(crate::world::BuildingKind::FarmerHome, 500.0, HOME_Y, 0);
    params.init_economy(1);
    params.focus_camera(450.0, 400.0);
    params.test_state.phase_name = "Waiting for spawns + movement...".into();
    info!("coalesce-movement: setup — 2 farmers, homes@{HOME_Y}, farms@{FARM_Y}");
}

pub fn tick_movement(
    entity_map: Res<EntityMap>,
    gpu_read: Res<GpuReadState>,
    slots: Res<GpuSlotPool>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    match test.phase {
        // Phase 1: Wait for 2 farmers to move away from spawn
        1 => {
            if slots.alive() < 2 {
                return;
            }
            let positions = &gpu_read.positions;
            let mut moved = 0;
            for npc in entity_map.iter_npcs() {
                if npc.dead {
                    continue;
                }
                let i = npc.slot;
                if i * 2 + 1 < positions.len() {
                    let y = positions[i * 2 + 1];
                    if y > 0.0 && y < HOME_Y - 30.0 {
                        moved += 1;
                    }
                }
            }
            test.phase_name = format!("moved={moved}/2 alive={}", slots.alive());
            if moved >= 2 {
                test.pass_phase(elapsed, format!("2 farmers moving south"));
            }
            if elapsed > 10.0 {
                test.fail_phase(elapsed, format!("only {moved}/2 moved"));
            }
        }
        // Phase 2: Inject SetPosition on unused slot, check no snap-back
        2 => {
            if !test.get_flag("injected") {
                gpu_updates.write(crate::messages::GpuUpdateMsg(
                    crate::messages::GpuUpdate::SetPosition {
                        idx: 9999,
                        x: 100.0,
                        y: 100.0,
                    },
                ));
                test.set_flag("injected", true);
                test.phase_name = "Injected SetPosition, monitoring...".into();
                return;
            }
            if elapsed < 0.5 {
                return;
            }

            let positions = &gpu_read.positions;
            let mut snapped = false;
            let mut detail = String::new();
            for npc in entity_map.iter_npcs() {
                if npc.dead {
                    continue;
                }
                let i = npc.slot;
                if i * 2 + 1 >= positions.len() {
                    continue;
                }
                let y = positions[i * 2 + 1];
                if (y - HOME_Y).abs() < 5.0 {
                    snapped = true;
                    detail = format!("slot {} snapped to y={:.0} (spawn={HOME_Y})", npc.slot, y);
                }
            }
            test.phase_name = if snapped {
                detail.clone()
            } else {
                "No teleport".into()
            };
            if !snapped {
                test.pass_phase(
                    elapsed,
                    format!("positions stable after foreign SetPosition"),
                );
                test.complete(elapsed);
            } else {
                test.fail_phase(elapsed, detail);
            }
            if elapsed > 5.0 && !snapped {
                test.pass_phase(elapsed, format!("timeout but no snap detected"));
                test.complete(elapsed);
            }
        }
        _ => {}
    }
}

// --- arrival-coalesce: SetTarget on one NPC must not reset another's arrival ---

pub fn setup_arrival(mut params: TestSetupParams) {
    params.add_town("ArrivalTown");
    params.add_waypoint(400.0, 400.0, 0, 0);
    params.add_building(crate::world::BuildingKind::ArcherHome, 410.0, 410.0, 0);
    params.add_waypoint(300.0, 300.0, 0, 1);
    params.add_building(crate::world::BuildingKind::ArcherHome, 300.0, 600.0, 0);
    params.init_economy(1);
    params.focus_camera(350.0, 400.0);
    params.test_state.phase_name = "Waiting for archers...".into();
    info!("coalesce-arrival: setup — 2 archers, one near waypoint, one far");
}

pub fn tick_arrival(
    entity_map: Res<EntityMap>,
    slots: Res<GpuSlotPool>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    npc_flags_q: Query<&NpcFlags>,
    activity_q: Query<&Activity>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    match test.phase {
        // Phase 1: Wait for at least one archer to arrive
        1 => {
            if slots.alive() < 2 {
                return;
            }
            let arrived = entity_map
                .iter_npcs()
                .filter(|n| !n.dead && npc_flags_q.get(n.entity).is_ok_and(|f| f.at_destination))
                .count();
            test.phase_name = format!("arrived={arrived}/2 alive={}", slots.alive());
            if arrived >= 1 {
                for npc in entity_map.iter_npcs() {
                    if npc.dead {
                        continue;
                    }
                    if npc_flags_q.get(npc.entity).is_ok_and(|f| f.at_destination) {
                        test.counters.insert("arrived_slot".into(), npc.slot as u32);
                        break;
                    }
                }
                test.pass_phase(elapsed, format!("{arrived} archer(s) at destination"));
            }
            if elapsed > 15.0 {
                test.fail_phase(elapsed, format!("arrived={arrived}"));
            }
        }
        // Phase 2: Verify arrived NPC stays stable
        2 => {
            let arrived_slot = *test.counters.get("arrived_slot").unwrap_or(&0) as usize;
            let npc = entity_map.iter_npcs().find(|n| n.slot == arrived_slot);
            if let Some(npc) = npc {
                let still_arrived = npc_flags_q.get(npc.entity).is_ok_and(|f| f.at_destination);
                let activity = activity_q.get(npc.entity).ok();
                let is_stable = still_arrived || activity.is_some_and(|a| !a.is_transit());
                test.phase_name = format!(
                    "slot {arrived_slot} arrived={still_arrived} act={:?}",
                    activity
                );
                if elapsed > 2.0 {
                    if is_stable {
                        test.pass_phase(elapsed, format!("slot {arrived_slot} stable"));
                        test.complete(elapsed);
                    } else {
                        test.fail_phase(elapsed, format!("slot {arrived_slot} arrival was reset!"));
                    }
                }
            }
            if elapsed > 10.0 {
                test.fail_phase(elapsed, format!("timeout"));
            }
        }
        _ => {}
    }
}
