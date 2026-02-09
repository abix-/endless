//! Sleep Visual Test (3 phases, time_scale=20)
//! Validates: Resting NPC gets sleep icon on Item equip layer, cleared on wake.

use bevy::prelude::*;
use crate::components::*;
use crate::constants::SLEEP_SPRITE;
use crate::gpu::NpcBufferWrites;
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
        name: "SleepTown".into(),
        center: Vec2::new(400.0, 400.0),
        faction: 0,
        sprite_type: 0,
    });
    world_data.beds.push(world::Bed {
        position: Vec2::new(380.0, 420.0),
        town_idx: 0,
    });
    world_data.farms.push(world::Farm {
        position: Vec2::new(450.0, 400.0),
        town_idx: 0,
    });
    food_storage.init(1);
    faction_stats.init(1);
    game_time.time_scale = 20.0;

    // Spawn 1 farmer
    let slot = slot_alloc.alloc().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: slot,
        x: 450.0, y: 400.0,
        job: 0, faction: 0, town_idx: 0,
        home_x: 380.0, home_y: 420.0,
        work_x: 450.0, work_y: 400.0,
        starting_post: 0,
        attack_type: 0,
    });

    test_state.phase_name = "Waiting for farmer spawn...".into();
    info!("sleep-visual: setup — 1 farmer, time_scale=20");
}

pub fn tick(
    farmer_query: Query<(), (With<Farmer>, Without<Dead>)>,
    resting_query: Query<&NpcIndex, (With<Resting>, Without<Dead>)>,
    not_resting_query: Query<&NpcIndex, (With<Farmer>, Without<Resting>, Without<Dead>)>,
    energy_query: Query<&Energy, (With<Farmer>, Without<Dead>)>,
    buffer: Res<NpcBufferWrites>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    if test.passed || test.failed { return; }

    let now = time.elapsed_secs();
    if test.start == 0.0 { test.start = now; }
    let elapsed = now - test.start;

    let farmer_exists = farmer_query.iter().count() > 0;
    if !farmer_exists {
        test.phase_name = "Waiting for farmer...".into();
        if elapsed > 3.0 { test.fail_phase(elapsed, "No farmer entity"); }
        return;
    }

    let energy = energy_query.iter().next().map(|e| e.0).unwrap_or(100.0);

    match test.phase {
        // Phase 1: Farmer spawns with energy 100
        1 => {
            test.phase_name = format!("e={:.0}", energy);
            if energy >= 90.0 {
                test.pass_phase(elapsed, format!("Farmer alive (energy={:.0})", energy));
            } else if elapsed > 5.0 {
                test.fail_phase(elapsed, "Farmer not at full energy");
            }
        }
        // Phase 2: Farmer rests → item_sprites should show SLEEP_SPRITE
        2 => {
            let resting_idx = resting_query.iter().next().map(|n| n.0);
            test.phase_name = format!("e={:.0} resting={}", energy, resting_idx.is_some());

            if let Some(idx) = resting_idx {
                let sprite_col = buffer.item_sprites.get(idx * 2).copied().unwrap_or(-1.0);
                if sprite_col == SLEEP_SPRITE.0 {
                    test.pass_phase(elapsed, format!("Sleep icon set (idx={}, col={:.0})", idx, sprite_col));
                } else {
                    // Resting but no sleep icon — this is the expected RED failure
                    test.fail_phase(elapsed, format!("Resting but item_sprites[{}]={:.1}, expected {:.0}", idx * 2, sprite_col, SLEEP_SPRITE.0));
                }
            } else if elapsed > 45.0 {
                test.fail_phase(elapsed, format!("energy={:.0} but never rested", energy));
            }
        }
        // Phase 3: Farmer wakes → item_sprites should be cleared (-1)
        3 => {
            // Look for a farmer that was resting (phase 2 passed) and is now awake
            let awake_idx = not_resting_query.iter().next().map(|n| n.0);
            test.phase_name = format!("e={:.0} awake={}", energy, awake_idx.is_some());

            if let Some(idx) = awake_idx {
                if energy >= 80.0 {
                    let sprite_col = buffer.item_sprites.get(idx * 2).copied().unwrap_or(-1.0);
                    if sprite_col == -1.0 {
                        test.pass_phase(elapsed, format!("Sleep icon cleared (idx={}, energy={:.0})", idx, energy));
                        test.complete(elapsed);
                    } else {
                        test.fail_phase(elapsed, format!("Awake but item_sprites[{}]={:.1}, expected -1", idx * 2, sprite_col));
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
