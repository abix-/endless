//! Vertical Slice Test
//! Validates full core loop: spawn → work → raid → combat → death → respawn.
//! 5 farmer homes + 2 archer homes + 5 tents → spawner system creates 12 NPCs.

use crate::components::*;
use crate::resources::*;
use crate::world;
use bevy::prelude::*;

use super::{TestSetupParams, TestState};

/// 64px-aligned farm positions for the vertical slice test.
const FARMS: [(f32, f32); 5] = [
    (192.0, 320.0),
    (256.0, 320.0),
    (320.0, 320.0),
    (384.0, 320.0),
    (448.0, 320.0),
];

/// Setup: place buildings, let spawner system create NPCs.
pub fn setup(mut params: TestSetupParams) {
    // World data: 2 towns on 64px-aligned centers
    params.add_town("Harvest");
    params.world_data.towns[0].center = Vec2::new(384.0, 384.0);
    params.world_data.towns.push(world::Town {
        name: "Raiders".into(),
        center: Vec2::new(384.0, 128.0),
        faction: 1,
        sprite_type: 1,
    });

    // 5 farms near town 0 (pre-grown so farmers can harvest)
    for &(fx, fy) in &FARMS {
        params.add_building(world::BuildingKind::Farm, fx, fy, 0);
        if let Some(inst) = params.entity_map.find_farm_at_mut(Vec2::new(fx, fy)) {
            inst.growth_ready = true;
            inst.growth_progress = 1.0;
        }
    }

    // 4 waypoints (square patrol around town)
    for (order, &(gx, gy)) in [
        (128.0, 128.0),
        (640.0, 128.0),
        (640.0, 640.0),
        (128.0, 640.0),
    ]
    .iter()
    .enumerate()
    {
        params.add_waypoint(gx, gy, 0, order as u32);
    }

    // Spawner buildings — the spawner_respawn_system will auto-create NPCs
    // 5 farmer homes → 5 farmers
    for i in 0..5 {
        params.add_building(
            world::BuildingKind::FarmerHome,
            192.0 + (i as f32 * 64.0),
            448.0,
            0,
        );
    }
    // 2 archer homes → 2 archers
    params.add_building(world::BuildingKind::ArcherHome, 512.0, 448.0, 0);
    params.add_building(world::BuildingKind::ArcherHome, 576.0, 448.0, 0);
    // 5 tents → 5 raiders
    for i in 0..5 {
        params.add_building(
            world::BuildingKind::Tent,
            192.0 + (i as f32 * 64.0),
            64.0,
            1,
        );
    }

    // Resources
    params.init_economy(2);
    params.food_storage.food[1] = 10;
    params.game_time.time_scale = 1.0;

    params.focus_camera(384.0, 384.0);
    params.test_state.phase_name = "Waiting for spawns...".into();
    info!("vertical-slice: setup complete — buildings placed, awaiting spawner");
}

/// Count live (non-dead) NPCs via EntityMap.
fn alive_npcs(entity_map: &EntityMap) -> usize {
    entity_map.iter_npcs().filter(|n| !n.dead).count()
}

/// Tick: phased assertions with time gates.
pub fn tick(
    time: Res<Time>,
    gpu_state: Res<GpuReadState>,
    combat_debug: Res<CombatDebug>,
    health_debug: Res<HealthDebug>,
    entity_map: Res<EntityMap>,
    mut test: ResMut<TestState>,
    activity_q: Query<&Activity>,
    npc_flags_q: Query<&NpcFlags>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    let npc_count = alive_npcs(&entity_map);

    // Track lowest NPC count for death detection
    if npc_count > 0 && npc_count < test.count("lowest_npc") as usize {
        test.counters
            .insert("lowest_npc".into(), npc_count as u32);
    }
    if !test.counters.contains_key("lowest_npc") && npc_count > 0 {
        test.counters
            .insert("lowest_npc".into(), npc_count as u32);
    }

    match test.phase {
        // Phase 1: All 12 NPCs spawned by spawner system
        1 => {
            test.phase_name = format!("npc_count={}/12", npc_count);
            if npc_count == 12 {
                test.pass_phase(elapsed, format!("npc_count={}", npc_count));
            } else if elapsed > 8.0 {
                test.fail_phase(
                    elapsed,
                    format!("npc_count={} (expected 12)", npc_count),
                );
            }
        }
        // Phase 2: GPU readback has valid positions
        2 => {
            let has_positions = gpu_state.positions.len() >= 24
                && gpu_state.positions.iter().take(24).any(|&v| v != 0.0);
            test.phase_name = format!("positions_len={}", gpu_state.positions.len());
            if has_positions {
                let p0 = (
                    gpu_state.positions.get(0).copied().unwrap_or(0.0),
                    gpu_state.positions.get(1).copied().unwrap_or(0.0),
                );
                test.pass_phase(
                    elapsed,
                    format!("positions valid, sample=({:.0},{:.0})", p0.0, p0.1),
                );
            } else if elapsed > 6.0 {
                test.fail_phase(
                    elapsed,
                    format!("positions_len={}, all zeros", gpu_state.positions.len()),
                );
            }
        }
        // Phase 3: Farmers arrive and start working
        3 => {
            let working = entity_map
                .iter_npcs()
                .filter(|n| {
                    !n.dead
                        && activity_q
                            .get(n.entity)
                            .is_ok_and(|a| matches!(*a, Activity::Working))
                })
                .count();
            let going_to_work = entity_map
                .iter_npcs()
                .filter(|n| {
                    !n.dead
                        && activity_q
                            .get(n.entity)
                            .is_ok_and(|a| matches!(*a, Activity::GoingToWork))
                })
                .count();
            test.phase_name = format!("working={} going_to_work={}", working, going_to_work);
            if working >= 3 {
                test.pass_phase(elapsed, format!("working={}", working));
            } else if elapsed > 16.0 {
                let at_dest = entity_map
                    .iter_npcs()
                    .filter(|n| {
                        !n.dead && npc_flags_q.get(n.entity).is_ok_and(|f| f.at_destination)
                    })
                    .count();
                let transit = entity_map
                    .iter_npcs()
                    .filter(|n| !n.dead && activity_q.get(n.entity).is_ok_and(|a| a.is_transit()))
                    .count();
                test.fail_phase(
                    elapsed,
                    format!(
                        "working={} going_to_work={} at_dest={} transit={}",
                        working, going_to_work, at_dest, transit
                    ),
                );
            }
        }
        // Phase 4: Raiders dispatched
        4 => {
            let raiding = entity_map
                .iter_npcs()
                .filter(|n| {
                    !n.dead
                        && activity_q
                            .get(n.entity)
                            .is_ok_and(|a| matches!(*a, Activity::Raiding { .. }))
                })
                .count();
            test.phase_name = format!("raiding={}/3", raiding);
            if raiding >= 3 {
                test.pass_phase(elapsed, format!("raiding={}", raiding));
            } else if elapsed > 30.0 {
                test.fail_phase(elapsed, format!("raiding={} (expected >=3)", raiding));
            }
        }
        // Phase 5: Combat targets acquired
        5 => {
            test.phase_name = format!("targets_found={}", combat_debug.targets_found);
            if combat_debug.targets_found > 0 {
                test.pass_phase(
                    elapsed,
                    format!("targets_found={}", combat_debug.targets_found),
                );
            } else if elapsed > 50.0 {
                test.fail_phase(elapsed, "targets_found=0");
            }
        }
        // Phase 6: Damage applied
        6 => {
            test.phase_name = format!("damage_processed={}", health_debug.damage_processed);
            if health_debug.damage_processed > 0 {
                test.pass_phase(
                    elapsed,
                    format!("damage_processed={}", health_debug.damage_processed),
                );
            } else if elapsed > 60.0 {
                test.fail_phase(elapsed, "damage_processed=0");
            }
        }
        // Phase 7: At least one death
        7 => {
            test.phase_name = format!(
                "npc_count={} deaths_frame={}",
                npc_count, health_debug.deaths_this_frame
            );
            if npc_count < 12 || health_debug.deaths_this_frame > 0 {
                test.set_flag("death_seen", true);
                test.pass_phase(
                    elapsed,
                    format!(
                        "npc_count={}, deaths_frame={}",
                        npc_count, health_debug.deaths_this_frame
                    ),
                );
            } else if elapsed > 80.0 {
                test.fail_phase(elapsed, format!("npc_count={} (no deaths)", npc_count));
            }
        }
        // Phase 8: Respawn
        8 => {
            test.phase_name = format!("npc_count={}/12 (waiting for respawn)", npc_count);
            if test.get_flag("death_seen") && npc_count >= 12 {
                test.pass_phase(elapsed, format!("npc_count={} (recovered)", npc_count));
                test.complete(elapsed);
            } else if elapsed > 180.0 {
                let lowest = test.count("lowest_npc");
                let death_seen = test.get_flag("death_seen");
                test.fail_phase(
                    elapsed,
                    format!(
                        "npc_count={}, lowest={}, death_seen={}",
                        npc_count, lowest, death_seen
                    ),
                );
            }
        }
        _ => {}
    }
}
