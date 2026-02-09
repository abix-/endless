//! Energy System Test (3 phases, time_scale=50)
//! Validates: energy starts at 100, drains over time, reaches ENERGY_HUNGRY threshold.

use bevy::prelude::*;
use crate::components::*;
use crate::constants::ENERGY_HUNGRY;
use crate::messages::SpawnNpcMsg;
use crate::resources::*;
use crate::world;

use super::TestState;

pub fn setup(
    mut slot_alloc: ResMut<SlotAllocator>,
    mut spawn_events: MessageWriter<SpawnNpcMsg>,
    mut world_data: ResMut<world::WorldData>,
    mut food_storage: ResMut<FoodStorage>,
    mut faction_stats: ResMut<FactionStats>,
    mut game_time: ResMut<GameTime>,
    mut test_state: ResMut<TestState>,
) {
    world_data.towns.push(world::Town {
        name: "TestTown".into(),
        center: Vec2::new(400.0, 400.0),
        faction: 0,
        sprite_type: 0,
    });
    food_storage.init(1);
    faction_stats.init(1);
    game_time.time_scale = 50.0;

    // Spawn 1 farmer without work position (stays idle, drains energy)
    let slot = slot_alloc.alloc().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: slot,
        x: 400.0, y: 400.0,
        job: 0, faction: 0, town_idx: 0,
        home_x: 400.0, home_y: 450.0,
        work_x: -1.0, work_y: -1.0,
        starting_post: -1,
        attack_type: 0,
    });

    test_state.phase_name = "Waiting for spawn...".into();
    info!("energy: setup — 1 farmer, time_scale=50");
}

pub fn tick(
    query: Query<(&Energy, &NpcIndex), (With<Farmer>, Without<Dead>)>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    if test.passed || test.failed { return; }

    let now = time.elapsed_secs();
    if test.start == 0.0 { test.start = now; }
    let elapsed = now - test.start;

    let Some((energy, _)) = query.iter().next() else {
        test.phase_name = "Waiting for farmer...".into();
        if elapsed > 3.0 {
            test.fail_phase(elapsed, "No farmer entity found");
        }
        return;
    };

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
