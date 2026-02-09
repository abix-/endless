//! Farmer Work Cycle Test (5 phases, time_scale=20)
//! Validates: GoingToWork → Working → tired stops → goes home to rest → recovers and returns.

use bevy::prelude::*;
use crate::components::*;
use crate::constants::ENERGY_WAKE_THRESHOLD;
use crate::messages::SpawnNpcMsg;
use crate::resources::*;
use crate::world;

use super::TestState;

pub fn setup(
    mut slot_alloc: ResMut<SlotAllocator>,
    mut spawn_events: MessageWriter<SpawnNpcMsg>,
    mut world_data: ResMut<world::WorldData>,
    mut food_storage: ResMut<FoodStorage>,
    mut farm_states: ResMut<FarmStates>,
    mut faction_stats: ResMut<FactionStats>,
    mut game_time: ResMut<GameTime>,
    mut test_state: ResMut<TestState>,
) {
    world_data.towns.push(world::Town {
        name: "FarmTown".into(),
        center: Vec2::new(400.0, 400.0),
        faction: 0,
        sprite_type: 0,
    });
    // 1 farm near town
    world_data.farms.push(world::Farm {
        position: Vec2::new(400.0, 350.0),
        town_idx: 0,
    });
    farm_states.states.push(FarmGrowthState::Growing);
    farm_states.progress.push(0.0);
    // 1 bed (home)
    world_data.beds.push(world::Bed {
        position: Vec2::new(400.0, 450.0),
        town_idx: 0,
    });
    food_storage.init(1);
    faction_stats.init(1);
    game_time.time_scale = 20.0;

    // Spawn farmer with work position at farm
    let slot = slot_alloc.alloc().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: slot,
        x: 400.0, y: 400.0,
        job: 0, faction: 0, town_idx: 0,
        home_x: 400.0, home_y: 450.0,
        work_x: 400.0, work_y: 350.0,
        starting_post: -1,
        attack_type: 0,
    });

    test_state.phase_name = "Waiting for farmer...".into();
    info!("farmer-cycle: setup — 1 farmer, time_scale=20");
}

pub fn tick(
    going_work_query: Query<(), (With<GoingToWork>, With<Farmer>, Without<Dead>)>,
    working_query: Query<(), (With<Working>, With<Farmer>, Without<Dead>)>,
    going_rest_query: Query<(), (With<GoingToRest>, With<Farmer>, Without<Dead>)>,
    resting_query: Query<(), (With<Resting>, With<Farmer>, Without<Dead>)>,
    energy_query: Query<&Energy, (With<Farmer>, Without<Dead>)>,
    farmer_query: Query<(), (With<Farmer>, Without<Dead>)>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    if test.passed || test.failed { return; }

    let now = time.elapsed_secs();
    if test.start == 0.0 { test.start = now; }
    let elapsed = now - test.start;

    if farmer_query.iter().count() == 0 {
        test.phase_name = "Waiting for farmer...".into();
        if elapsed > 3.0 { test.fail_phase(elapsed, "No farmer entity"); }
        return;
    }

    let energy = energy_query.iter().next().map(|e| e.0).unwrap_or(100.0);
    let going_work = going_work_query.iter().count();
    let working = working_query.iter().count();
    let going_rest = going_rest_query.iter().count();
    let resting = resting_query.iter().count();

    match test.phase {
        // Phase 1: Farmer spawns with GoingToWork + HasTarget
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
                if elapsed > 60.0 {
                    test.fail_phase(elapsed, format!("working={} energy={:.0}", working, energy));
                }
            }
        }
        // Phase 4: Goes home to rest
        4 => {
            test.phase_name = format!("going_rest={} resting={} e={:.0}", going_rest, resting, energy);
            if going_rest > 0 || resting > 0 {
                test.pass_phase(elapsed, format!("going_rest={} resting={} (energy={:.0})", going_rest, resting, energy));
            } else if elapsed > 70.0 {
                test.fail_phase(elapsed, format!("not resting, energy={:.0}", energy));
            }
        }
        // Phase 5: Energy recovers → returns to work
        5 => {
            test.phase_name = format!("e={:.0} going_work={} working={} resting={}", energy, going_work, working, resting);
            if energy >= ENERGY_WAKE_THRESHOLD && (going_work > 0 || working > 0) {
                test.pass_phase(elapsed, format!("Returned to work (energy={:.0})", energy));
                test.complete(elapsed);
            } else if elapsed > 120.0 {
                test.fail_phase(elapsed, format!("energy={:.0} going_work={} working={}", energy, going_work, working));
            }
        }
        _ => {}
    }
}
