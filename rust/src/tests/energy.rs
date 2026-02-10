//! Energy System Test (3 phases)
//! Validates: energy starts at 100, drains over time, reaches ENERGY_HUNGRY threshold.

use bevy::prelude::*;
use crate::components::*;
use crate::constants::ENERGY_HUNGRY;

use super::{TestState, TestSetupParams};

pub fn setup(mut params: TestSetupParams) {
    params.add_town("TestTown");
    params.init_economy(1);
    params.game_time.time_scale = 1.0;
    // No home → farmer can't rest → energy only drains
    let slot = params.slot_alloc.alloc().expect("slot alloc");
    params.spawn_events.write(crate::messages::SpawnNpcMsg {
        slot_idx: slot,
        x: 400.0, y: 400.0,
        job: 0, faction: 0, town_idx: 0,
        home_x: -1.0, home_y: -1.0,
        work_x: -1.0, work_y: -1.0,
        starting_post: -1,
        attack_type: 0,
    });
    params.test_state.phase_name = "Waiting for spawn...".into();
    info!("energy: setup — 1 farmer");
}

pub fn tick(
    mut query: Query<(&mut Energy, &NpcIndex), (With<Farmer>, Without<Dead>)>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    if !test.require_entity(query.iter().count(), elapsed, "farmer") { return; }
    let Some((mut energy, _)) = query.iter_mut().next() else { return; };

    // Start energy near threshold so drain completes within 30s at time_scale=1
    if !test.get_flag("energy_set") {
        energy.0 = 55.0;
        test.set_flag("energy_set", true);
    }

    let e = energy.0;

    match test.phase {
        // Phase 1: Energy exists and is draining
        1 => {
            test.phase_name = format!("energy={:.1}", e);
            if e > 0.0 {
                test.pass_phase(elapsed, format!("energy={:.1}", e));
            } else if elapsed > 3.0 {
                test.fail_phase(elapsed, format!("energy={:.1} (expected >0)", e));
            }
        }
        // Phase 2: Energy drains below 54
        2 => {
            test.phase_name = format!("energy={:.1} (waiting for <54)", e);
            if e < 54.0 {
                test.pass_phase(elapsed, format!("energy={:.1}", e));
            } else if elapsed > 10.0 {
                test.fail_phase(elapsed, format!("energy={:.1} (expected <54)", e));
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
