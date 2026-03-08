//! Farmer Occupancy Cycle Test (5 phases)
//! Validates: 20 farmer homes spawn workers, 16 farms cap at occupancy 1 each, 4 extra farmers stay idle.
//! Layout: 4x4 farm grid centered at town center, 20 farmer homes on the border.

use crate::components::*;
use crate::resources::EntityMap;
use crate::world::BuildingKind;
use bevy::prelude::*;

use super::{TestSetupParams, TestState};

const TOTAL_FARMS: usize = 16;
const TOTAL_HOMES: usize = 20;
const EXPECTED_IDLE: usize = TOTAL_HOMES - TOTAL_FARMS; // 4

pub fn setup(mut params: TestSetupParams) {
    params.add_town("FarmTown");
    params.world_data.towns[0].center = Vec2::new(384.0, 384.0);

    // 4x4 farm grid centered around town center
    for row in 0..4 {
        for col in 0..4 {
            let x = 256.0 + col as f32 * 64.0;
            let y = 256.0 + row as f32 * 64.0;
            params.add_building(BuildingKind::Farm, x, y, 0);
        }
    }

    // 20 farmer homes around the border
    // Top row (y=192): 6 homes
    for i in 0..6 {
        params.add_building(BuildingKind::FarmerHome, 192.0 + i as f32 * 64.0, 192.0, 0);
    }
    // Bottom row (y=512): 6 homes
    for i in 0..6 {
        params.add_building(BuildingKind::FarmerHome, 192.0 + i as f32 * 64.0, 512.0, 0);
    }
    // Left column (x=192): 4 homes (skip corners already placed)
    for i in 0..4 {
        params.add_building(BuildingKind::FarmerHome, 192.0, 256.0 + i as f32 * 64.0, 0);
    }
    // Right column (x=512): 4 homes
    for i in 0..4 {
        params.add_building(BuildingKind::FarmerHome, 512.0, 256.0 + i as f32 * 64.0, 0);
    }

    params.init_economy(1);
    params.game_time.time_scale = 1.0;
    params.focus_camera(384.0, 384.0);
    params.test_state.phase_name = "Waiting for farmer-home spawns...".into();
    info!(
        "farmer-cycle: setup - {} farmer homes, {} farms",
        TOTAL_HOMES, TOTAL_FARMS
    );
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
                    .is_ok_and(|a| a.kind == ActivityKind::GoingToWork)
        })
        .count();
    let working = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && n.job == Job::Farmer
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| a.kind == ActivityKind::Working)
        })
        .count();
    let idle = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && n.job == Job::Farmer
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| a.kind == ActivityKind::Idle)
        })
        .count();
    let wandering = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && n.job == Job::Farmer
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| a.kind == ActivityKind::Wandering)
        })
        .count();

    let idle_like = idle + wandering;
    let active_workers = going_work + working;

    match test.phase {
        // Phase 1: all 20 farmers spawned from 20 homes.
        1 => {
            test.phase_name = format!(
                "farmers={}/{} homes_linked={}/{}",
                farmer_count, TOTAL_HOMES, spawned_from_homes, TOTAL_HOMES
            );
            if farmer_count == TOTAL_HOMES && spawned_from_homes == TOTAL_HOMES {
                test.pass_phase(elapsed, format!("{} farmers spawned from {} homes", TOTAL_HOMES, TOTAL_HOMES));
            } else if elapsed > 25.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "farmers={} homes_linked={}",
                        farmer_count, spawned_from_homes
                    ),
                );
            }
        }
        // Phase 2: 16 farms occupied, no overbooking.
        2 => {
            test.phase_name = format!(
                "occupied_farms={}/{} overbooked={} total_occ={}",
                occupied_farms, TOTAL_FARMS, overbooked_farms, total_occupants
            );
            if occupied_farms == TOTAL_FARMS && overbooked_farms == 0 && total_occupants == TOTAL_FARMS as i32 {
                test.pass_phase(elapsed, format!("{} farm slots claimed, no overbooking", TOTAL_FARMS));
            } else if elapsed > 30.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "occupied={} overbooked={} total_occ={}",
                        occupied_farms, overbooked_farms, total_occupants
                    ),
                );
            }
        }
        // Phase 3: exactly 16 farmers actively working.
        3 => {
            test.phase_name = format!(
                "active_workers={}/{} (work={} going={})",
                active_workers, TOTAL_FARMS, working, going_work
            );
            if active_workers == TOTAL_FARMS {
                test.pass_phase(elapsed, format!("{} farmers assigned to {} farms", TOTAL_FARMS, TOTAL_FARMS));
            } else if elapsed > 35.0 {
                test.fail_phase(
                    elapsed,
                    format!("active_workers={} (expected {})", active_workers, TOTAL_FARMS),
                );
            }
        }
        // Phase 4: 4 farmers idle/wandering — no free farm slots.
        4 => {
            test.phase_name = format!(
                "idle_like={}/{} active_workers={} farmers={}",
                idle_like, EXPECTED_IDLE, active_workers, farmer_count
            );
            if idle_like >= EXPECTED_IDLE && active_workers <= TOTAL_FARMS && farmer_count == TOTAL_HOMES {
                test.pass_phase(
                    elapsed,
                    format!("{} idle farmers confirmed (idle_like={})", EXPECTED_IDLE, idle_like),
                );
            } else if elapsed > 40.0 {
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
            let stable = occupied_farms == TOTAL_FARMS
                && overbooked_farms == 0
                && total_occupants == TOTAL_FARMS as i32
                && active_workers <= TOTAL_FARMS;
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
                test.pass_phase(elapsed, format!("Occupancy stable: {} occupied, 0 overbooked", TOTAL_FARMS));
                test.complete(elapsed);
            } else if elapsed > 50.0 {
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
