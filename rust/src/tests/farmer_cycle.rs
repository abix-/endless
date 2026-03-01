//! Farmer Occupancy Cycle Test (5 phases)
//! Validates: farmer homes spawn workers, farm occupancy caps at 1, and one extra farmer stays idle.

use crate::components::*;
use crate::resources::EntityMap;
use crate::world::BuildingKind;
use bevy::prelude::*;

use super::{TestSetupParams, TestState};

pub fn setup(mut params: TestSetupParams) {
    params.add_town("FarmTown");

    // Two farms = 2 available work slots.
    params.add_building(BuildingKind::Farm, 350.0, 340.0, 0);
    params.add_building(BuildingKind::Farm, 450.0, 340.0, 0);

    // Three farmer homes should produce 3 farmers via spawner_respawn_system.
    params.add_building(BuildingKind::FarmerHome, 300.0, 460.0, 0);
    params.add_building(BuildingKind::FarmerHome, 400.0, 460.0, 0);
    params.add_building(BuildingKind::FarmerHome, 500.0, 460.0, 0);

    params.init_economy(1);
    params.game_time.time_scale = 1.0;
    params.focus_camera(400.0, 400.0);
    params.test_state.phase_name = "Waiting for farmer-home spawns...".into();
    info!("farmer-cycle: setup - 3 farmer homes, 2 farms");
}

pub fn tick(
    entity_map: Res<EntityMap>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    activity_q: Query<&Activity>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    let farmer_count = entity_map
        .iter_npcs()
        .filter(|n| !n.dead && n.job == Job::Farmer)
        .count();

    let homes: Vec<_> = entity_map
        .iter_kind_for_town(BuildingKind::FarmerHome, 0)
        .collect();
    let farms: Vec<_> = entity_map
        .iter_kind_for_town(BuildingKind::Farm, 0)
        .collect();

    let spawned_from_homes = homes.iter().filter(|h| h.npc_uid.is_some()).count();
    let occupied_farms = farms.iter().filter(|f| f.occupants == 1).count();
    let overbooked_farms = farms.iter().filter(|f| f.occupants > 1).count();
    let total_occupants: i32 = farms.iter().map(|f| f.occupants as i32).sum();

    let going_work = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && n.job == Job::Farmer
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| matches!(*a, Activity::GoingToWork))
        })
        .count();
    let working = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && n.job == Job::Farmer
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| matches!(*a, Activity::Working))
        })
        .count();
    let idle = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && n.job == Job::Farmer
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| matches!(*a, Activity::Idle))
        })
        .count();
    let wandering = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && n.job == Job::Farmer
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| matches!(*a, Activity::Wandering))
        })
        .count();

    let idle_like = idle + wandering;
    let active_workers = going_work + working;

    match test.phase {
        // Phase 1: 3 farmers spawned from 3 farmer homes.
        1 => {
            test.phase_name = format!(
                "farmers={}/3 homes_linked={}/3",
                farmer_count, spawned_from_homes
            );
            if farmer_count == 3 && spawned_from_homes == 3 {
                test.pass_phase(elapsed, "3 farmers spawned from 3 homes");
            } else if elapsed > 12.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "farmers={} homes_linked={}",
                        farmer_count, spawned_from_homes
                    ),
                );
            }
        }
        // Phase 2: 2 farms occupied, no overbooking, exactly 2 total occupants.
        2 => {
            test.phase_name = format!(
                "occupied_farms={}/2 overbooked={} total_occ={}",
                occupied_farms, overbooked_farms, total_occupants
            );
            if occupied_farms == 2 && overbooked_farms == 0 && total_occupants == 2 {
                test.pass_phase(elapsed, "2 farm slots claimed, no overbooking");
            } else if elapsed > 18.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "occupied={} overbooked={} total_occ={}",
                        occupied_farms, overbooked_farms, total_occupants
                    ),
                );
            }
        }
        // Phase 3: only 2 farmers are actively assigned to farm work.
        3 => {
            test.phase_name = format!(
                "active_workers={}/3 (work={} going={})",
                active_workers, working, going_work
            );
            if active_workers == 2 {
                test.pass_phase(elapsed, "Only 2 farmers assigned to 2 farms");
            } else if elapsed > 22.0 {
                test.fail_phase(
                    elapsed,
                    format!("active_workers={} (expected 2)", active_workers),
                );
            }
        }
        // Phase 4: one farmer remains idle/wandering because no farm slot is free.
        4 => {
            test.phase_name = format!(
                "idle_like={} active_workers={} farmers={}",
                idle_like, active_workers, farmer_count
            );
            if idle_like >= 1 && active_workers <= 2 && farmer_count == 3 {
                test.pass_phase(
                    elapsed,
                    format!("Idle farmer confirmed (idle_like={})", idle_like),
                );
            } else if elapsed > 24.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "idle_like={} active_workers={} farmers={}",
                        idle_like, active_workers, farmer_count
                    ),
                );
            }
        }
        // Phase 5: occupancy stays stable (no contention).
        5 => {
            let stable = occupied_farms == 2
                && overbooked_farms == 0
                && total_occupants == 2
                && active_workers <= 2;
            if stable {
                test.inc("stable_ticks");
            } else {
                test.counters.insert("stable_ticks".into(), 0);
            }
            let stable_ticks = test.count("stable_ticks");
            test.phase_name = format!(
                "stable_ticks={} occupied={} overbooked={}",
                stable_ticks, occupied_farms, overbooked_farms
            );

            if stable_ticks >= 20 {
                test.pass_phase(elapsed, "Occupancy stable: 2 occupied, 0 overbooked");
                test.complete(elapsed);
            } else if elapsed > 28.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "unstable occupancy (occ={} overbooked={} active={})",
                        total_occupants, overbooked_farms, active_workers
                    ),
                );
            }
        }
        _ => {}
    }
}
