//! Farm Growth Visual Test (3 phases, time_scale=50)
//! Validates: Growing farm has no marker → Ready farm spawns FarmReadyMarker → harvest removes it.

use bevy::prelude::*;
use crate::components::*;
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
        name: "FarmVisTown".into(),
        center: Vec2::new(400.0, 400.0),
        faction: 0,
        sprite_type: 0,
    });
    world_data.farms.push(world::Farm {
        position: Vec2::new(400.0, 350.0),
        town_idx: 0,
    });
    farm_states.states.push(FarmGrowthState::Growing);
    farm_states.progress.push(0.8); // almost ready
    world_data.beds.push(world::Bed {
        position: Vec2::new(400.0, 450.0),
        town_idx: 0,
    });

    food_storage.init(1);
    faction_stats.init(1);
    game_time.time_scale = 50.0;

    // Spawn 1 farmer to tend the farm (speeds growth to Ready)
    let slot = slot_alloc.alloc().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: slot,
        x: 400.0, y: 350.0,
        job: 0, faction: 0, town_idx: 0,
        home_x: 400.0, home_y: 450.0,
        work_x: 400.0, work_y: 350.0,
        starting_post: 0,
        attack_type: 0,
    });

    test_state.phase_name = "Waiting for farm growth...".into();
    info!("farm-visual: setup — 1 farm (progress=0.8), 1 farmer, time_scale=50");
}

pub fn tick(
    marker_query: Query<&FarmReadyMarker>,
    farm_states: Res<FarmStates>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    if test.passed || test.failed { return; }

    let now = time.elapsed_secs();
    if test.start == 0.0 { test.start = now; }
    let elapsed = now - test.start;

    let farm_state = farm_states.states.first().copied();
    let farm_progress = farm_states.progress.first().copied().unwrap_or(0.0);
    let marker_count = marker_query.iter().count();

    match test.phase {
        // Phase 1: Farm is Growing, no FarmReadyMarker entities
        1 => {
            test.phase_name = format!("state={:?} prog={:.2} markers={}", farm_state, farm_progress, marker_count);
            if farm_state == Some(FarmGrowthState::Growing) && marker_count == 0 {
                test.pass_phase(elapsed, "Farm Growing, no markers");
            } else if elapsed > 5.0 {
                test.fail_phase(elapsed, format!("state={:?} markers={}", farm_state, marker_count));
            }
        }
        // Phase 2: Farm reaches Ready → FarmReadyMarker entity exists
        2 => {
            test.phase_name = format!("state={:?} prog={:.2} markers={}", farm_state, farm_progress, marker_count);
            if farm_state == Some(FarmGrowthState::Ready) {
                if marker_count > 0 {
                    test.pass_phase(elapsed, format!("FarmReadyMarker spawned (count={})", marker_count));
                } else {
                    // Ready but no marker — this is the expected RED failure
                    test.fail_phase(elapsed, "Farm Ready but no FarmReadyMarker entity");
                }
            } else if elapsed > 30.0 {
                test.fail_phase(elapsed, format!("Farm never reached Ready (prog={:.2})", farm_progress));
            }
        }
        // Phase 3: Farmer harvests → FarmReadyMarker despawned, farm back to Growing
        3 => {
            test.phase_name = format!("state={:?} markers={}", farm_state, marker_count);
            if farm_state == Some(FarmGrowthState::Growing) && marker_count == 0 {
                test.pass_phase(elapsed, "Harvested: marker removed, farm Growing again");
                test.complete(elapsed);
            } else if elapsed > 45.0 {
                test.fail_phase(elapsed, format!("state={:?} markers={}", farm_state, marker_count));
            }
        }
        _ => {}
    }
}
