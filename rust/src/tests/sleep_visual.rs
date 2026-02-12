//! Sleep Visual Test (3 phases)
//! Validates: Resting NPC gets sleep icon on Status layer, cleared on wake.

use bevy::prelude::*;
use crate::components::*;
use crate::gpu::NpcBufferWrites;
use crate::resources::*;

use super::{TestState, TestSetupParams};

pub fn setup(mut params: TestSetupParams, mut farm_states: ResMut<FarmStates>) {
    params.add_town("SleepTown");
    params.add_bed(380.0, 420.0);
    params.world_data.farms.push(crate::world::Farm {
        position: Vec2::new(450.0, 400.0),
        town_idx: 0,
    });
    farm_states.states.push(FarmGrowthState::Growing);
    farm_states.progress.push(0.0);
    farm_states.positions.push(Vec2::new(450.0, 400.0));
    params.init_economy(1);
    params.game_time.time_scale = 1.0;

    // Spawn 1 farmer with work position at farm
    let slot = params.slot_alloc.alloc().expect("slot alloc");
    params.spawn_events.write(crate::messages::SpawnNpcMsg {
        slot_idx: slot,
        x: 450.0, y: 400.0,
        job: 0, faction: 0, town_idx: 0,
        home_x: 380.0, home_y: 420.0,
        work_x: 450.0, work_y: 400.0,
        starting_post: -1,
        attack_type: 0,
    });

    params.test_state.phase_name = "Waiting for farmer spawn...".into();
    info!("sleep-visual: setup — 1 farmer");
}

pub fn tick(
    farmer_query: Query<(), (With<Farmer>, Without<Dead>)>,
    npc_activity_query: Query<(&NpcIndex, &Activity), (With<Farmer>, Without<Dead>)>,
    mut energy_query: Query<&mut Energy, (With<Farmer>, Without<Dead>)>,
    buffer: Res<NpcBufferWrites>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };
    if !test.require_entity(farmer_query.iter().count(), elapsed, "farmer") { return; }

    let energy = energy_query.iter().next().map(|e| e.0).unwrap_or(100.0);

    // Start energy near tired threshold so rest triggers within 30s
    if !test.get_flag("energy_set") {
        for mut e in energy_query.iter_mut() { e.0 = 35.0; }
        test.set_flag("energy_set", true);
    }

    match test.phase {
        // Phase 1: Farmer spawns alive
        1 => {
            test.phase_name = format!("e={:.0}", energy);
            if energy > 0.0 {
                test.pass_phase(elapsed, format!("Farmer alive (energy={:.0})", energy));
            } else if elapsed > 5.0 {
                test.fail_phase(elapsed, "Farmer not found");
            }
        }
        // Phase 2: Farmer rests → status_sprites should show SLEEP_SPRITE
        2 => {
            let resting = npc_activity_query.iter().find(|(_, a)| matches!(a, Activity::Resting));
            test.phase_name = format!("e={:.0} resting={}", energy, resting.is_some());

            if let Some((idx, _)) = resting {
                let j = idx.0 * 3;
                let sprite_col = buffer.status_sprites.get(j).copied().unwrap_or(-1.0);
                let sprite_atlas = buffer.status_sprites.get(j + 2).copied().unwrap_or(0.0);
                if sprite_col >= 0.0 && sprite_atlas >= 2.5 {
                    test.pass_phase(elapsed, format!("Sleep icon set (idx={}, col={:.0}, atlas={:.0})", idx.0, sprite_col, sprite_atlas));
                } else {
                    test.fail_phase(elapsed, format!("Resting but status[{}] col={:.1} atlas={:.1}, expected col>=0 atlas=3", j, sprite_col, sprite_atlas));
                }
            } else if elapsed > 45.0 {
                test.fail_phase(elapsed, format!("energy={:.0} but never rested", energy));
            }
        }
        // Phase 3: Farmer wakes → status_sprites should be cleared (-1)
        3 => {
            // Look for a farmer that was resting (phase 2 passed) and is now awake
            let awake = npc_activity_query.iter().find(|(_, a)| !matches!(a, Activity::Resting));
            test.phase_name = format!("e={:.0} awake={}", energy, awake.is_some());

            if let Some((idx, _)) = awake {
                if energy >= 80.0 {
                    let j = idx.0 * 3;
                    let sprite_col = buffer.status_sprites.get(j).copied().unwrap_or(-1.0);
                    if sprite_col == -1.0 {
                        test.pass_phase(elapsed, format!("Sleep icon cleared (idx={}, energy={:.0})", idx.0, energy));
                        test.complete(elapsed);
                    } else {
                        test.fail_phase(elapsed, format!("Awake but status_sprites[{}]={:.1}, expected -1", j, sprite_col));
                    }
                }
            }

            if elapsed > 90.0 {
                test.fail_phase(elapsed, format!("energy={:.0} never recovered", energy));
            }
        }
        _ => {}
    }
}
