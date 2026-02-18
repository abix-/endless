//! Farm Growth Visual Test (3 phases)
//! Validates: Growing farm has no marker → Ready farm spawns FarmReadyMarker → harvest removes it.

use bevy::prelude::*;
use crate::components::*;
use crate::resources::*;

use super::{TestState, TestSetupParams};

pub fn setup(mut params: TestSetupParams, mut farm_states: ResMut<GrowthStates>) {
    params.add_town("FarmVisTown");
    params.world_data.farms_mut().push(crate::world::PlacedBuilding::new(Vec2::new(400.0, 350.0), 0));
    farm_states.kinds.push(crate::resources::GrowthKind::Farm);
    farm_states.states.push(FarmGrowthState::Growing);
    farm_states.progress.push(0.95); // near ready so transition happens within 30s
    farm_states.positions.push(Vec2::new(400.0, 350.0));
    farm_states.town_indices.push(Some(0));
    params.add_bed(400.0, 450.0);
    params.init_economy(1);
    params.game_time.time_scale = 1.0;

    // Spawn 1 farmer to tend the farm (speeds growth to Ready)
    let slot = params.slot_alloc.alloc().expect("slot alloc");
    params.spawn_events.write(crate::messages::SpawnNpcMsg {
        slot_idx: slot,
        x: 400.0, y: 350.0,
        job: 0, faction: 0, town_idx: 0,
        home_x: 400.0, home_y: 450.0,
        work_x: 400.0, work_y: 350.0,
        starting_post: 0,
        attack_type: 0,
    });

    params.test_state.phase_name = "Waiting for farm growth...".into();
    info!("farm-visual: setup — 1 farm (progress=0.95), 1 farmer");
}

pub fn tick(
    marker_query: Query<&FarmReadyMarker>,
    farm_states: Res<GrowthStates>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

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
