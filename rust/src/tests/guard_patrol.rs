//! Guard Patrol Cycle Test (5 phases, time_scale=20)
//! Validates: OnDuty → Patrolling → OnDuty → rest when tired → resume when rested.

use bevy::prelude::*;
use crate::components::*;
use crate::constants::{ENERGY_HUNGRY, ENERGY_RESTED};
use crate::resources::*;

use super::{TestState, TestSetupParams, keep_fed};

pub fn setup(mut params: TestSetupParams) {
    params.add_town("GuardTown");
    // 4 guard posts (square patrol)
    for (order, &(gx, gy)) in [(300.0, 300.0), (500.0, 300.0), (500.0, 500.0), (300.0, 500.0)].iter().enumerate() {
        params.world_data.guard_posts.push(crate::world::GuardPost {
            position: Vec2::new(gx, gy),
            town_idx: 0,
            patrol_order: order as u32,
        });
    }
    // Beds for resting
    for i in 0..2 {
        params.add_bed(380.0 + (i as f32 * 40.0), 420.0);
    }
    params.init_economy(1);
    params.game_time.time_scale = 20.0;

    // Spawn 1 guard at post 0 (job=1, starting_post=0)
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

    params.test_state.phase_name = "Waiting for guard spawn...".into();
    info!("guard-patrol: setup — 1 guard, 4 posts, time_scale=20");
}

pub fn tick(
    on_duty_query: Query<&OnDuty, (With<Guard>, Without<Dead>)>,
    patrolling_query: Query<(), (With<Patrolling>, With<Guard>, Without<Dead>)>,
    resting_query: Query<(), (With<Resting>, With<Guard>, Without<Dead>)>,
    going_rest_query: Query<(), (With<GoingToRest>, With<Guard>, Without<Dead>)>,
    energy_query: Query<&Energy, (With<Guard>, Without<Dead>)>,
    guard_query: Query<(), (With<Guard>, Without<Dead>)>,
    mut last_ate_query: Query<&mut LastAteHour, Without<Dead>>,
    game_time: Res<GameTime>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };
    keep_fed(&mut last_ate_query, &game_time);
    if !test.require_entity(guard_query.iter().count(), elapsed, "guard") { return; }

    let energy = energy_query.iter().next().map(|e| e.0).unwrap_or(100.0);
    let on_duty = on_duty_query.iter().count();
    let patrolling = patrolling_query.iter().count();
    let resting = resting_query.iter().count();
    let going_rest = going_rest_query.iter().count();

    match test.phase {
        // Phase 1: Guard starts OnDuty at post 0
        1 => {
            test.phase_name = format!("on_duty={} patrolling={}", on_duty, patrolling);
            if on_duty > 0 {
                test.pass_phase(elapsed, format!("OnDuty (energy={:.0})", energy));
            } else if elapsed > 3.0 {
                test.fail_phase(elapsed, format!("on_duty=0 patrolling={}", patrolling));
            }
        }
        // Phase 2: After GUARD_PATROL_WAIT ticks → Patrolling
        2 => {
            test.phase_name = format!("patrolling={} on_duty={} e={:.0}", patrolling, on_duty, energy);
            if patrolling > 0 {
                test.pass_phase(elapsed, format!("Patrolling (energy={:.0})", energy));
            } else if elapsed > 15.0 {
                let ticks = on_duty_query.iter().next().map(|d| d.ticks_waiting).unwrap_or(0);
                test.fail_phase(elapsed, format!("patrolling=0 ticks={}", ticks));
            }
        }
        // Phase 3: Arrives at next post → OnDuty again
        3 => {
            test.phase_name = format!("on_duty={} patrolling={} e={:.0}", on_duty, patrolling, energy);
            // Must have been patrolling first (Phase 2 passed), now back to OnDuty
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
            } else if energy < ENERGY_HUNGRY && elapsed > 30.0 {
                test.fail_phase(elapsed, format!("energy={:.0} but not resting", energy));
            } else if elapsed > 60.0 {
                test.fail_phase(elapsed, format!("energy={:.0} never reached hungry", energy));
            }
        }
        // Phase 5: Energy > ENERGY_RESTED → resumes patrol
        5 => {
            test.phase_name = format!("e={:.0} on_duty={} patrolling={} resting={}", energy, on_duty, patrolling, resting);
            if energy >= ENERGY_RESTED && (on_duty > 0 || patrolling > 0) {
                test.pass_phase(elapsed, format!("Resumed (energy={:.0})", energy));
                test.complete(elapsed);
            } else if elapsed > 90.0 {
                test.fail_phase(elapsed, format!("energy={:.0} on_duty={} patrolling={}", energy, on_duty, patrolling));
            }
        }
        _ => {}
    }
}
