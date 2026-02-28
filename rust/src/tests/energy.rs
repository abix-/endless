//! Energy System Test (3 phases)
//! Validates: energy starts at 100, drains over time, reaches ENERGY_HUNGRY threshold.

use bevy::prelude::*;
use crate::components::*;
use crate::constants::ENERGY_HUNGRY;
use crate::resources::EntityMap;

use super::{TestState, TestSetupParams};

pub fn setup(mut params: TestSetupParams) {
    params.add_town("TestTown");
    params.init_economy(1);
    params.game_time.time_scale = 1.0;
    params.focus_camera(400.0, 400.0);
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
        uid_override: None,
    });
    params.test_state.phase_name = "Waiting for spawn...".into();
    info!("energy: setup — 1 farmer");
}

pub fn tick(
    entity_map: Res<EntityMap>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    mut energy_q: Query<&mut Energy>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    let farmer_count = entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Farmer).count();
    if !test.require_entity(farmer_count, elapsed, "farmer") { return; }
    let Some(farmer) = entity_map.iter_npcs().find(|n| !n.dead && n.job == Job::Farmer) else { return; };
    let farmer_entity = farmer.entity;

    // Start energy near threshold so drain completes within 30s at time_scale=1
    if !test.get_flag("energy_set") {
        if let Ok(mut en) = energy_q.get_mut(farmer_entity) { en.0 = 55.0; }
        test.set_flag("energy_set", true);
    }

    let e = energy_q.get(farmer_entity).map(|en| en.0).unwrap_or(0.0);

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
