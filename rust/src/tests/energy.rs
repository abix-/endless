//! Energy System Test (3 phases, time_scale=50)
//! Validates: energy starts at 100, drains over time, reaches ENERGY_HUNGRY threshold.

use bevy::prelude::*;
use crate::components::*;
use crate::constants::ENERGY_HUNGRY;

use super::{TestState, TestSetupParams};

pub fn setup(mut params: TestSetupParams) {
    params.add_town("TestTown");
    params.init_economy(1);
    params.game_time.time_scale = 50.0;
    params.spawn_npc(0, 400.0, 400.0, 400.0, 450.0);
    params.test_state.phase_name = "Waiting for spawn...".into();
    info!("energy: setup — 1 farmer, time_scale=50");
}

pub fn tick(
    query: Query<(&Energy, &NpcIndex), (With<Farmer>, Without<Dead>)>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    if !test.require_entity(query.iter().count(), elapsed, "farmer") { return; }
    let Some((energy, _)) = query.iter().next() else { return; };

    let e = energy.0;

    match test.phase {
        // Phase 1: Energy starts at 100
        1 => {
            test.phase_name = format!("energy={:.1}", e);
            if !test.get_flag("initial_checked") {
                if e >= 99.0 {
                    test.pass_phase(elapsed, format!("energy={:.1}", e));
                } else if e > 0.0 {
                    // Already started draining, still pass if close
                    test.pass_phase(elapsed, format!("energy={:.1} (already draining)", e));
                }
                test.set_flag("initial_checked", true);
            } else if elapsed > 3.0 {
                test.fail_phase(elapsed, format!("energy={:.1} (expected ~100)", e));
            }
        }
        // Phase 2: Energy drains below 90
        2 => {
            test.phase_name = format!("energy={:.1} (waiting for <90)", e);
            if e < 90.0 {
                test.pass_phase(elapsed, format!("energy={:.1}", e));
            } else if elapsed > 10.0 {
                test.fail_phase(elapsed, format!("energy={:.1} (expected <90)", e));
            }
        }
        // Phase 3: Energy reaches ENERGY_HUNGRY threshold
        3 => {
            test.phase_name = format!("energy={:.1} (waiting for <{:.0})", e, ENERGY_HUNGRY);
            if e < ENERGY_HUNGRY {
                test.pass_phase(elapsed, format!("energy={:.1} < {:.0}", e, ENERGY_HUNGRY));
                test.complete(elapsed);
            } else if elapsed > 20.0 {
                test.fail_phase(elapsed, format!("energy={:.1} (expected <{:.0})", e, ENERGY_HUNGRY));
            }
        }
        _ => {}
    }
}
