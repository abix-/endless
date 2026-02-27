//! Farmer Lifecycle Test (5 phases)
//! Validates: spawn from home → walk to farm → work → tired → rest at home.

use bevy::prelude::*;
use crate::components::*;
use crate::resources::*;

use super::{TestState, TestSetupParams};

const HOME_Y: f32 = 550.0;
const FARM_Y: f32 = 350.0;

pub fn setup(mut params: TestSetupParams) {
    params.add_town("TestTown");
    for i in 0..3 {
        let fx = 300.0 + (i as f32 * 100.0);
        params.add_building(crate::world::BuildingKind::Farm, fx, FARM_Y, 0);
        if let Some(inst) = params.entity_map.find_farm_at_mut(Vec2::new(fx, FARM_Y)) {
            inst.growth_ready = true;
            inst.growth_progress = 1.0;
        }
        params.add_building(crate::world::BuildingKind::FarmerHome, fx, HOME_Y, 0);
    }
    params.init_economy(1);

    params.test_state.phase_name = "Waiting for spawns...".into();
    info!("movement: setup — 3 farmers, homes at y={}, farms at y={}", HOME_Y, FARM_Y);
}

pub fn tick(
    activity_query: Query<&Activity, (With<EntitySlot>, Without<Dead>)>,
    at_dest_query: Query<(), (With<AtDestination>, With<EntitySlot>, Without<Dead>)>,
    mut energy_query: Query<&mut Energy, (With<Farmer>, Without<Dead>)>,
    gpu_state: Res<GpuReadState>,
    slots: Res<EntitySlots>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    // Set energy near tired threshold so drain→rest fits in test window
    if !test.get_flag("energy_set") && slots.alive() >= 3 {
        for mut e in energy_query.iter_mut() { e.0 = 35.0; }
        test.set_flag("energy_set", true);
    }

    let transit = activity_query.iter().filter(|a| a.is_transit()).count();
    let working = activity_query.iter().filter(|a| matches!(a, Activity::Working)).count();
    let going_rest = activity_query.iter().filter(|a| matches!(a, Activity::GoingToRest)).count();
    let resting = activity_query.iter().filter(|a| matches!(a, Activity::Resting)).count();

    match test.phase {
        // Phase 1: Farmers in transit to farms
        1 => {
            test.phase_name = format!("transit={}/3 working={}", transit, working);
            if transit + working >= 3 {
                test.pass_phase(elapsed, format!("transit={} working={}", transit, working));
            } else if slots.alive() >= 3 && elapsed > 0.5 {
                let at_dest = at_dest_query.iter().count();
                if transit + working + at_dest >= 3 {
                    test.pass_phase(elapsed, format!("transit={} working={} at_dest={}", transit, working, at_dest));
                }
            }
            if elapsed > 10.0 {
                test.fail_phase(elapsed, format!("transit={} working={} alive={}", transit, working, slots.alive()));
            }
        }
        // Phase 2: GPU positions changed (moved from HOME_Y toward FARM_Y)
        2 => {
            let positions = &gpu_state.positions;
            let mut moved_count = 0;
            for i in 0..3 {
                if i * 2 + 1 < positions.len() {
                    let y = positions[i * 2 + 1];
                    if y > 0.0 && (y - HOME_Y).abs() > 5.0 {
                        moved_count += 1;
                    }
                }
            }
            test.phase_name = format!("moved={}/3 positions_len={}", moved_count, positions.len());
            if moved_count >= 1 {
                test.pass_phase(elapsed, format!("moved={}", moved_count));
            } else if elapsed > 8.0 {
                let sample_y = positions.get(1).copied().unwrap_or(-1.0);
                test.fail_phase(elapsed, format!("moved=0, sample_y={:.1}, len={}", sample_y, positions.len()));
            }
        }
        // Phase 3: Farmers working at farms
        3 => {
            test.phase_name = format!("working={}/3 transit={}", working, transit);
            if working >= 1 {
                test.pass_phase(elapsed, format!("working={}", working));
            } else if elapsed > 15.0 {
                let at_dest = at_dest_query.iter().count();
                test.fail_phase(elapsed, format!("working=0 transit={} at_dest={}", transit, at_dest));
            }
        }
        // Phase 4: Energy drains → going home to rest
        4 => {
            let energy = energy_query.iter().next().map(|e| e.0).unwrap_or(100.0);
            test.phase_name = format!("going_rest={} resting={} e={:.0}", going_rest, resting, energy);
            if going_rest > 0 || resting > 0 {
                test.pass_phase(elapsed, format!("going_rest={} resting={} (energy={:.0})", going_rest, resting, energy));
            } else if elapsed > 25.0 {
                test.fail_phase(elapsed, format!("not resting, working={} energy={:.0}", working, energy));
            }
        }
        // Phase 5: Resting at home
        5 => {
            let energy = energy_query.iter().next().map(|e| e.0).unwrap_or(100.0);
            test.phase_name = format!("resting={} e={:.0}", resting, energy);
            if resting > 0 {
                test.pass_phase(elapsed, format!("resting={} (energy={:.0})", resting, energy));
                test.complete(elapsed);
            } else if elapsed > 30.0 {
                test.fail_phase(elapsed, format!("resting=0 going_rest={} energy={:.0}", going_rest, energy));
            }
        }
        _ => {}
    }
}
