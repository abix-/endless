//! Movement & Arrival Test (3 phases)
//! Validates: NPCs get transit Activity, GPU positions update, AtDestination on arrival.

use bevy::prelude::*;
use crate::components::*;
use crate::resources::*;

use super::{TestState, TestSetupParams};

/// Farms placed 150px from spawn positions — close enough to arrive quickly.
const SPAWN_Y: f32 = 500.0;
const FARM_Y: f32 = 350.0;

pub fn setup(mut params: TestSetupParams, mut farm_states: ResMut<GrowthStates>) {
    params.add_town("TestTown");
    for i in 0..3 {
        let fx = 300.0 + (i as f32 * 100.0);
        params.world_data.farms.push(crate::world::Farm {
            position: Vec2::new(fx, FARM_Y),
            town_idx: 0,
        });
        farm_states.kinds.push(crate::resources::GrowthKind::Farm);
        farm_states.states.push(FarmGrowthState::Ready);
        farm_states.progress.push(1.0);
        farm_states.positions.push(Vec2::new(fx, FARM_Y));
        farm_states.town_indices.push(Some(0));
        params.add_bed(fx, 550.0);
    }
    params.init_economy(1);

    // Spawn 3 farmers with work positions at farms (150px away)
    for i in 0..3 {
        let fx = 300.0 + (i as f32 * 100.0);
        let slot = params.slot_alloc.alloc().expect("slot alloc");
        params.spawn_events.write(crate::messages::SpawnNpcMsg {
            slot_idx: slot,
            x: fx, y: SPAWN_Y,
            job: 0, faction: 0, town_idx: 0,
            home_x: fx, home_y: 550.0,
            work_x: fx, work_y: FARM_Y,
            starting_post: -1,
            attack_type: 0,
        });
    }

    params.test_state.phase_name = "Waiting for spawns...".into();
    info!("movement: setup — 3 farmers, 150px to farms");
}

pub fn tick(
    activity_query: Query<&Activity, (With<NpcIndex>, Without<Dead>)>,
    at_dest_query: Query<(), (With<AtDestination>, With<NpcIndex>, Without<Dead>)>,
    gpu_state: Res<GpuReadState>,
    slots: Res<SlotAllocator>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    match test.phase {
        // Phase 1: 3 NPCs in transit or working (movement initiated)
        1 => {
            let transit = activity_query.iter().filter(|a| a.is_transit()).count();
            let working = activity_query.iter().filter(|a| matches!(a, Activity::Working)).count();
            test.phase_name = format!("transit={}/3 working={}", transit, working);
            if transit + working >= 3 {
                test.pass_phase(elapsed, format!("transit={} working={}", transit, working));
            } else if slots.alive() >= 3 && elapsed > 0.5 {
                // Farmers with work_x get GoingToWork at spawn
                // If not seen, might have already arrived (unlikely at 150px)
                let at_dest = at_dest_query.iter().count();
                if transit + working + at_dest >= 3 {
                    test.pass_phase(elapsed, format!("transit={} working={} at_dest={}", transit, working, at_dest));
                }
            }
            if elapsed > 5.0 {
                test.fail_phase(elapsed, format!("transit={} working={} alive={}", transit, working, slots.alive()));
            }
        }
        // Phase 2: GPU positions have changed (not all at spawn Y)
        2 => {
            let positions = &gpu_state.positions;
            let mut moved_count = 0;
            for i in 0..3 {
                if i * 2 + 1 < positions.len() {
                    let y = positions[i * 2 + 1];
                    // NPC should have moved from SPAWN_Y toward FARM_Y
                    if y > 0.0 && (y - SPAWN_Y).abs() > 5.0 {
                        moved_count += 1;
                    }
                }
            }
            test.phase_name = format!("moved={}/3 positions_len={}", moved_count, positions.len());
            if moved_count >= 1 {
                test.pass_phase(elapsed, format!("moved={}", moved_count));
            } else if elapsed > 8.0 {
                let sample_y = positions.get(1).copied().unwrap_or(-1.0);
                test.fail_phase(elapsed, format!("moved=0, sample_y={:.1}, len={}", sample_y, positions.len()));
            }
        }
        // Phase 3: NPCs arrive (AtDestination or Working — decision_system transitions AtDest→Working)
        3 => {
            let at_dest = at_dest_query.iter().count();
            let working = activity_query.iter().filter(|a| matches!(a, Activity::Working)).count();
            let arrived = at_dest + working;
            test.phase_name = format!("arrived={}/3 (at_dest={} working={})", arrived, at_dest, working);
            if arrived >= 1 {
                test.pass_phase(elapsed, format!("arrived={} (at_dest={} working={})", arrived, at_dest, working));
                test.complete(elapsed);
            } else if elapsed > 15.0 {
                let transit = activity_query.iter().filter(|a| a.is_transit()).count();
                test.fail_phase(elapsed, format!(
                    "arrived=0 transit={}", transit));
            }
        }
        _ => {}
    }
}
