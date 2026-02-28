//! Archer Patrol Cycle Test (5 phases)
//! Validates: OnDuty → Patrolling → OnDuty → rest when tired → resume when rested.

use bevy::prelude::*;
use crate::components::*;
use crate::constants::{ENERGY_HUNGRY, ENERGY_WAKE_THRESHOLD};
use crate::resources::EntityMap;

use super::{TestState, TestSetupParams};

pub fn setup(mut params: TestSetupParams) {
    params.add_town("ArcherTown");
    // 4 waypoints (square patrol)
    for (order, &(gx, gy)) in [(300.0, 300.0), (500.0, 300.0), (500.0, 500.0), (300.0, 500.0)].iter().enumerate() {
        params.add_waypoint(gx, gy, 0, order as u32);
    }
    // Beds for resting
    for i in 0..2 {
        params.add_bed(380.0 + (i as f32 * 40.0), 420.0);
    }
    params.init_economy(1);
    params.game_time.time_scale = 1.0;
    params.focus_camera(400.0, 400.0);

    // Spawn 1 archer at post 0 (job=1, starting_post=0)
    let slot = params.slot_alloc.alloc().expect("slot alloc");
    params.spawn_events.write(crate::messages::SpawnNpcMsg {
        slot_idx: slot,
        x: 300.0, y: 300.0,
        job: 1, faction: 0, town_idx: 0,
        home_x: 400.0, home_y: 420.0,
        work_x: -1.0, work_y: -1.0,
        starting_post: 0,
        attack_type: 0,
    });

    params.test_state.phase_name = "Waiting for archer spawn...".into();
    info!("archer-patrol: setup — 1 archer, 4 waypoints");
}

pub fn tick(
    entity_map: Res<EntityMap>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    activity_q: Query<&Activity>,
    _npc_flags_q: Query<&NpcFlags>,
    mut energy_q: Query<&mut Energy>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };
    let archer_count = entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Archer).count();
    if !test.require_entity(archer_count, elapsed, "archer") { return; }

    // Start energy near tired threshold so rest triggers within 30s
    if !test.get_flag("energy_set") {
        for npc in entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Archer) {
            if let Ok(mut en) = energy_q.get_mut(npc.entity) { en.0 = 40.0; }
        }
        test.set_flag("energy_set", true);
    }

    let energy = entity_map.iter_npcs().find(|n| !n.dead && n.job == Job::Archer)
        .and_then(|n| energy_q.get(n.entity).ok()).map(|e| e.0).unwrap_or(100.0);
    let on_duty = entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Archer && activity_q.get(n.entity).is_ok_and(|a| matches!(*a, Activity::OnDuty { .. }))).count();
    let patrolling = entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Archer && activity_q.get(n.entity).is_ok_and(|a| matches!(*a, Activity::Patrolling))).count();
    let resting = entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Archer && activity_q.get(n.entity).is_ok_and(|a| matches!(*a, Activity::Resting))).count();
    let going_rest = entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Archer && activity_q.get(n.entity).is_ok_and(|a| matches!(*a, Activity::GoingToRest))).count();

    match test.phase {
        // Phase 1: Archer starts OnDuty at post 0
        1 => {
            test.phase_name = format!("on_duty={} patrolling={}", on_duty, patrolling);
            if on_duty > 0 {
                test.pass_phase(elapsed, format!("OnDuty (energy={:.0})", energy));
            } else if elapsed > 3.0 {
                test.fail_phase(elapsed, format!("on_duty=0 patrolling={}", patrolling));
            }
        }
        // Phase 2: After ARCHER_PATROL_WAIT ticks → Patrolling
        2 => {
            test.phase_name = format!("patrolling={} on_duty={} e={:.0}", patrolling, on_duty, energy);
            if patrolling > 0 {
                test.pass_phase(elapsed, format!("Patrolling (energy={:.0})", energy));
            } else if elapsed > 15.0 {
                let ticks = entity_map.iter_npcs().find_map(|n| {
                    activity_q.get(n.entity).ok().and_then(|a| if let Activity::OnDuty { ticks_waiting } = *a { Some(ticks_waiting) } else { None })
                }).unwrap_or(0);
                test.fail_phase(elapsed, format!("patrolling=0 ticks={}", ticks));
            }
        }
        // Phase 3: Arrives at next post → OnDuty again
        3 => {
            test.phase_name = format!("on_duty={} patrolling={} e={:.0}", on_duty, patrolling, energy);
            if on_duty > 0 {
                test.pass_phase(elapsed, format!("OnDuty again (energy={:.0})", energy));
            } else if elapsed > 20.0 {
                test.fail_phase(elapsed, format!("on_duty=0 patrolling={}", patrolling));
            }
        }
        // Phase 4: Energy < ENERGY_HUNGRY → goes to rest
        4 => {
            test.phase_name = format!("e={:.0} resting={} going_rest={}", energy, resting, going_rest);
            if resting > 0 || going_rest > 0 {
                test.pass_phase(elapsed, format!("Resting (energy={:.0})", energy));
            } else if energy < ENERGY_HUNGRY && elapsed > 20.0 {
                test.fail_phase(elapsed, format!("energy={:.0} but not resting", energy));
            } else if elapsed > 25.0 {
                test.fail_phase(elapsed, format!("energy={:.0} never reached hungry", energy));
            }
        }
        // Phase 5: Energy recovers → archer wakes from rest
        5 => {
            test.phase_name = format!("e={:.0} on_duty={} patrolling={} resting={}", energy, on_duty, patrolling, resting);
            if energy >= ENERGY_WAKE_THRESHOLD && resting == 0 {
                test.pass_phase(elapsed, format!("Woke up (energy={:.0})", energy));
                test.complete(elapsed);
            } else if elapsed > 40.0 {
                test.fail_phase(elapsed, format!("energy={:.0} resting={}", energy, resting));
            }
        }
        _ => {}
    }
}
