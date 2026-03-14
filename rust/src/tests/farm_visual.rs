//! Farm Growth Visual Test (3 phases)
//! Validates: Growing farm has no marker → Ready farm spawns FarmReadyMarker → harvest removes it.

use crate::components::*;
use crate::resources::*;
use bevy::prelude::*;

use super::{TestSetupParams, TestState};

pub fn setup(mut params: TestSetupParams) {
    params.add_town("FarmVisTown");
    params.add_building(crate::world::BuildingKind::Farm, 384.0, 320.0, 0);
    // Set progress near ready so transition happens within 30s
    if let Some(inst) = params.entity_map.find_by_position(Vec2::new(384.0, 320.0)) {
        let slot = inst.slot;
        if let Some(&entity) = params.entity_map.entities.get(&slot) {
            params
                .commands
                .entity(entity)
                .insert(crate::components::ProductionState {
                    ready: false,
                    progress: 0.95,
                });
        }
    }
    params.init_economy(1);
    params.game_time.time_scale = 1.0;
    params.focus_camera(384.0, 384.0);

    // Spawn 1 farmer to tend the farm (speeds growth to Ready)
    let slot = params.slot_alloc.alloc_reset().expect("slot alloc");
    params.spawn_events.write(crate::messages::SpawnNpcMsg {
        slot_idx: slot,
        x: 384.0,
        y: 320.0,
        job: 0,
        faction: 1,
        town_idx: 0,
        home_x: 384.0,
        home_y: 384.0,
        work_x: 384.0,
        work_y: 320.0,
        starting_post: 0,
        entity_override: None,
    });

    params.test_state.phase_name = "Waiting for farm growth...".into();
    info!("farm-visual: setup — 1 farm (progress=0.95), 1 farmer");
}

pub fn tick(
    marker_query: Query<&FarmReadyMarker>,
    entity_map: Res<EntityMap>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    production_q: Query<&crate::components::ProductionState, With<crate::components::Building>>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    let farm_inst = entity_map
        .iter_kind(crate::world::BuildingKind::Farm)
        .next();
    let farm_ps = farm_inst
        .and_then(|i| entity_map.entities.get(&i.slot))
        .and_then(|&e| production_q.get(e).ok());
    let farm_ready = farm_ps.map(|ps| ps.ready);
    let farm_progress = farm_ps.map(|ps| ps.progress).unwrap_or(0.0);
    let marker_count = marker_query.iter().count();

    match test.phase {
        // Phase 1: Farm is Growing, no FarmReadyMarker entities
        1 => {
            test.phase_name = format!(
                "ready={:?} prog={:.2} markers={}",
                farm_ready, farm_progress, marker_count
            );
            if farm_ready == Some(false) && marker_count == 0 {
                test.pass_phase(elapsed, "Farm Growing, no markers");
            } else if elapsed > 5.0 {
                test.fail_phase(
                    elapsed,
                    format!("ready={:?} markers={}", farm_ready, marker_count),
                );
            }
        }
        // Phase 2: Farm reaches Ready → FarmReadyMarker entity exists
        2 => {
            test.phase_name = format!(
                "ready={:?} prog={:.2} markers={}",
                farm_ready, farm_progress, marker_count
            );
            if farm_ready == Some(true) {
                if marker_count > 0 {
                    test.pass_phase(
                        elapsed,
                        format!("FarmReadyMarker spawned (count={})", marker_count),
                    );
                } else {
                    // Ready but no marker — this is the expected RED failure
                    test.fail_phase(elapsed, "Farm Ready but no FarmReadyMarker entity");
                }
            } else if elapsed > 30.0 {
                test.fail_phase(
                    elapsed,
                    format!("Farm never reached Ready (prog={:.2})", farm_progress),
                );
            }
        }
        // Phase 3: Farmer harvests → FarmReadyMarker despawned, farm back to Growing
        3 => {
            test.phase_name = format!("ready={:?} markers={}", farm_ready, marker_count);
            if farm_ready == Some(false) && marker_count == 0 {
                test.pass_phase(elapsed, "Harvested: marker removed, farm Growing again");
                test.complete(elapsed);
            } else if elapsed > 45.0 {
                test.fail_phase(
                    elapsed,
                    format!("ready={:?} markers={}", farm_ready, marker_count),
                );
            }
        }
        _ => {}
    }
}
