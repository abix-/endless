//! Farmer Lifecycle Test (5 phases)
//! Validates: spawn from home → walk to farm → work → tired → rest at home.
//! 3 farmer homes + 3 farms → spawner system creates 3 NPCs.

use crate::components::*;
use crate::resources::*;
use bevy::prelude::*;

use super::{TestSetupParams, TestState};

const HOME_Y: f32 = 448.0;
const FARM_Y: f32 = 320.0;

/// Count live (non-dead) NPCs via EntityMap.
fn alive_npcs(entity_map: &EntityMap) -> usize {
    entity_map.iter_npcs().filter(|n| !n.dead).count()
}

pub fn setup(mut params: TestSetupParams) {
    params.add_town("TestTown");
    params.world_data.towns[0].center = Vec2::new(320.0, 384.0);

    for i in 0..3 {
        let fx = 192.0 + (i as f32 * 128.0);
        params.add_building(crate::world::BuildingKind::Farm, fx, FARM_Y, 0);
        params.set_production_ready(Vec2::new(fx, FARM_Y));
        params.add_building(crate::world::BuildingKind::FarmerHome, fx, HOME_Y, 0);
    }
    params.init_economy(1);
    params.focus_camera(320.0, 384.0);

    params.test_state.phase_name = "Waiting for spawns...".into();
    info!(
        "movement: setup — 3 farmers, homes at y={}, farms at y={}",
        HOME_Y, FARM_Y
    );
}

pub fn tick(
    entity_map: Res<EntityMap>,
    gpu_state: Res<GpuReadState>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    activity_q: Query<&Activity>,
    npc_flags_q: Query<&NpcFlags>,
    mut energy_q: Query<&mut Energy>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    let npc_count = alive_npcs(&entity_map);

    // Set energy near tired threshold so drain→rest fits in test window
    if !test.get_flag("energy_set") && npc_count >= 3 {
        for npc in entity_map.iter_npcs() {
            if !npc.dead && npc.job == Job::Farmer {
                if let Ok(mut en) = energy_q.get_mut(npc.entity) {
                    en.0 = 35.0;
                }
            }
        }
        test.set_flag("energy_set", true);
    }

    let transit = entity_map
        .iter_npcs()
        .filter(|n| !n.dead && npc_flags_q.get(n.entity).is_ok_and(|f| !f.at_destination))
        .count();
    let working = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| matches!(a.kind, ActivityKind::Work { .. }))
        })
        .count();
    let going_rest = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| matches!(a.kind, ActivityKind::Rest))
                && npc_flags_q.get(n.entity).is_ok_and(|f| !f.at_destination)
        })
        .count();
    let resting = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| matches!(a.kind, ActivityKind::Rest))
                && npc_flags_q.get(n.entity).is_ok_and(|f| f.at_destination)
        })
        .count();

    match test.phase {
        // Phase 1: Farmers spawned and heading to farms
        1 => {
            test.phase_name = format!("npcs={}/3 transit={} working={}", npc_count, transit, working);
            if transit + working >= 3 {
                test.pass_phase(elapsed, format!("transit={} working={}", transit, working));
            } else if npc_count >= 3 && elapsed > 0.5 {
                let at_dest = entity_map
                    .iter_npcs()
                    .filter(|n| {
                        !n.dead && npc_flags_q.get(n.entity).is_ok_and(|f| f.at_destination)
                    })
                    .count();
                if transit + working + at_dest >= 3 {
                    test.pass_phase(
                        elapsed,
                        format!(
                            "transit={} working={} at_dest={}",
                            transit, working, at_dest
                        ),
                    );
                }
            }
            if elapsed > 10.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "transit={} working={} npcs={}",
                        transit, working, npc_count
                    ),
                );
            }
        }
        // Phase 2: GPU positions changed (moved from HOME_Y toward FARM_Y)
        2 => {
            let positions = &gpu_state.positions;
            let mut moved_count = 0;
            for npc in entity_map.iter_npcs() {
                if npc.dead {
                    continue;
                }
                let idx = npc.slot * 2;
                if idx + 1 < positions.len() {
                    let y = positions[idx + 1];
                    if y > 0.0 && (y - HOME_Y).abs() > 5.0 {
                        moved_count += 1;
                    }
                }
            }
            test.phase_name = format!("moved={}/3 positions_len={}", moved_count, positions.len());
            if moved_count >= 1 {
                test.pass_phase(elapsed, format!("moved={}", moved_count));
            } else if elapsed > 8.0 {
                test.fail_phase(
                    elapsed,
                    format!("moved=0, len={}", positions.len()),
                );
            }
        }
        // Phase 3: Farmers working at farms
        3 => {
            test.phase_name = format!("working={}/3 transit={}", working, transit);
            if working >= 1 {
                test.pass_phase(elapsed, format!("working={}", working));
            } else if elapsed > 20.0 {
                let at_dest = entity_map
                    .iter_npcs()
                    .filter(|n| {
                        !n.dead && npc_flags_q.get(n.entity).is_ok_and(|f| f.at_destination)
                    })
                    .count();
                test.fail_phase(
                    elapsed,
                    format!("working=0 transit={} at_dest={}", transit, at_dest),
                );
            }
        }
        // Phase 4: Energy drains → going home to rest
        4 => {
            let energy = entity_map
                .iter_npcs()
                .find(|n| !n.dead)
                .and_then(|n| energy_q.get(n.entity).ok())
                .map(|e| e.0)
                .unwrap_or(100.0);
            test.phase_name = format!(
                "going_rest={} resting={} e={:.0}",
                going_rest, resting, energy
            );
            if going_rest > 0 || resting > 0 {
                test.pass_phase(
                    elapsed,
                    format!(
                        "going_rest={} resting={} (energy={:.0})",
                        going_rest, resting, energy
                    ),
                );
            } else if elapsed > 30.0 {
                test.fail_phase(
                    elapsed,
                    format!("not resting, working={} energy={:.0}", working, energy),
                );
            }
        }
        // Phase 5: Resting at home
        5 => {
            let energy = entity_map
                .iter_npcs()
                .find(|n| !n.dead)
                .and_then(|n| energy_q.get(n.entity).ok())
                .map(|e| e.0)
                .unwrap_or(100.0);
            test.phase_name = format!("resting={} e={:.0}", resting, energy);
            if resting > 0 {
                test.pass_phase(
                    elapsed,
                    format!("resting={} (energy={:.0})", resting, energy),
                );
                test.complete(elapsed);
            } else if elapsed > 35.0 {
                test.fail_phase(
                    elapsed,
                    format!("resting=0 going_rest={} energy={:.0}", going_rest, energy),
                );
            }
        }
        _ => {}
    }
}
