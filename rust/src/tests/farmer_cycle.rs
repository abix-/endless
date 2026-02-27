//! Farmer Work Cycle Test (5 phases)
//! Validates: GoingToWork → Working → tired stops → goes home to rest → recovers and returns.

use bevy::prelude::*;
use crate::components::*;
use crate::constants::ENERGY_WAKE_THRESHOLD;
use crate::resources::EntityMap;
use super::{TestState, TestSetupParams};

pub fn setup(mut params: TestSetupParams) {
    params.add_town("FarmTown");
    params.add_building(crate::world::BuildingKind::Farm, 400.0, 350.0, 0);
    params.add_bed(400.0, 450.0);
    params.init_economy(1);
    params.game_time.time_scale = 1.0;

    // Spawn farmer with work position at farm
    let slot = params.slot_alloc.alloc().expect("slot alloc");
    params.spawn_events.write(crate::messages::SpawnNpcMsg {
        slot_idx: slot,
        x: 400.0, y: 400.0,
        job: 0, faction: 0, town_idx: 0,
        home_x: 400.0, home_y: 450.0,
        work_x: 400.0, work_y: 350.0,
        starting_post: -1,
        attack_type: 0,
    });

    params.test_state.phase_name = "Waiting for farmer...".into();
    info!("farmer-cycle: setup — 1 farmer");
}

pub fn tick(
    entity_map: Res<EntityMap>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    activity_q: Query<&Activity>,
    mut energy_q: Query<&mut Energy>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };
    let farmer_count = entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Farmer).count();
    if !test.require_entity(farmer_count, elapsed, "farmer") { return; }

    // Start energy near tired threshold so drain→rest→wake fits in 30s
    if !test.get_flag("energy_set") {
        for npc in entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Farmer) {
            if let Ok(mut en) = energy_q.get_mut(npc.entity) { en.0 = 35.0; }
        }
        test.set_flag("energy_set", true);
    }

    let energy = entity_map.iter_npcs().find(|n| !n.dead && n.job == Job::Farmer)
        .and_then(|n| energy_q.get(n.entity).ok()).map(|e| e.0).unwrap_or(100.0);
    let going_work = entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Farmer && activity_q.get(n.entity).is_ok_and(|a| matches!(*a, Activity::GoingToWork))).count();
    let working = entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Farmer && activity_q.get(n.entity).is_ok_and(|a| matches!(*a, Activity::Working))).count();
    let going_rest = entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Farmer && activity_q.get(n.entity).is_ok_and(|a| matches!(*a, Activity::GoingToRest))).count();
    let resting = entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Farmer && activity_q.get(n.entity).is_ok_and(|a| matches!(*a, Activity::Resting))).count();

    match test.phase {
        // Phase 1: Farmer spawns with GoingToWork
        1 => {
            test.phase_name = format!("going_work={} working={}", going_work, working);
            if going_work > 0 || working > 0 {
                test.pass_phase(elapsed, format!("going_work={} working={}", going_work, working));
            } else if elapsed > 5.0 {
                test.fail_phase(elapsed, format!("going_work=0 working=0"));
            }
        }
        // Phase 2: Arrives at farm → Working
        2 => {
            test.phase_name = format!("working={} going_work={} e={:.0}", working, going_work, energy);
            if working > 0 {
                test.pass_phase(elapsed, format!("Working (energy={:.0})", energy));
            } else if elapsed > 15.0 {
                test.fail_phase(elapsed, format!("working=0 going_work={}", going_work));
            }
        }
        // Phase 3: Energy drains → stops working
        3 => {
            test.phase_name = format!("working={} e={:.0}", working, energy);
            if test.get_flag("was_working") && working == 0 {
                test.pass_phase(elapsed, format!("Stopped working (energy={:.0})", energy));
            } else {
                if working > 0 { test.set_flag("was_working", true); }
                if elapsed > 20.0 {
                    test.fail_phase(elapsed, format!("working={} energy={:.0}", working, energy));
                }
            }
        }
        // Phase 4: Goes home to rest
        4 => {
            test.phase_name = format!("going_rest={} resting={} e={:.0}", going_rest, resting, energy);
            if going_rest > 0 || resting > 0 {
                test.pass_phase(elapsed, format!("going_rest={} resting={} (energy={:.0})", going_rest, resting, energy));
            } else if elapsed > 20.0 {
                test.fail_phase(elapsed, format!("not resting, energy={:.0}", energy));
            }
        }
        // Phase 5: Energy recovers → farmer wakes and leaves rest
        5 => {
            test.phase_name = format!("e={:.0} going_work={} working={} resting={}", energy, going_work, working, resting);
            if energy >= ENERGY_WAKE_THRESHOLD && resting == 0 {
                test.pass_phase(elapsed, format!("Woke up (energy={:.0})", energy));
                test.complete(elapsed);
            } else if elapsed > 25.0 {
                test.fail_phase(elapsed, format!("energy={:.0} resting={}", energy, resting));
            }
        }
        _ => {}
    }
}
