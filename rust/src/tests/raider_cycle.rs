//! Raider Raid Cycle Test (5 phases, time_scale=20)
//! Validates: raiders dispatched → arrive at farm → steal food → returning → deliver to camp.

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
    mut camp_state: ResMut<CampState>,
    mut test_state: ResMut<TestState>,
) {
    // Villager town (faction 0) with farms
    world_data.towns.push(world::Town {
        name: "FarmVille".into(),
        center: Vec2::new(400.0, 400.0),
        faction: 0,
        sprite_type: 0,
    });
    // Raider camp (faction 1)
    world_data.towns.push(world::Town {
        name: "RaiderCamp".into(),
        center: Vec2::new(400.0, 100.0),
        faction: 1,
        sprite_type: 1,
    });
    // 3 farms near villager town — all Ready so raiders can steal
    for i in 0..3 {
        world_data.farms.push(world::Farm {
            position: Vec2::new(350.0 + (i as f32 * 50.0), 350.0),
            town_idx: 0,
        });
        farm_states.states.push(FarmGrowthState::Ready);
        farm_states.progress.push(1.0);
    }
    food_storage.init(2);
    food_storage.food[0] = 10; // villager food
    food_storage.food[1] = 0;  // raider camp starts empty
    faction_stats.init(2);
    camp_state.init(1, 5);
    game_time.time_scale = 20.0;

    // Spawn 3 raiders (minimum for RAID_GROUP_SIZE)
    for i in 0..3 {
        let slot = slot_alloc.alloc().expect("slot alloc");
        spawn_events.write(SpawnNpcMsg {
            slot_idx: slot,
            x: 380.0 + (i as f32 * 20.0), y: 100.0,
            job: 2, faction: 1, town_idx: 1,
            home_x: 400.0, home_y: 100.0,
            work_x: -1.0, work_y: -1.0,
            starting_post: -1,
            attack_type: 0,
        });
    }

    test_state.phase_name = "Waiting for raiders...".into();
    info!("raider-cycle: setup — 3 raiders, 3 ready farms, time_scale=20");
}

pub fn tick(
    raiding_query: Query<(), (With<Raiding>, Without<Dead>)>,
    returning_query: Query<(), (With<Returning>, Without<Dead>)>,
    carrying_query: Query<(), (With<CarryingFood>, Without<Dead>)>,
    npc_query: Query<(), (With<Stealer>, Without<Dead>)>,
    food_storage: Res<FoodStorage>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    if test.passed || test.failed { return; }

    let now = time.elapsed_secs();
    if test.start == 0.0 { test.start = now; }
    let elapsed = now - test.start;

    let alive = npc_query.iter().count();
    if alive == 0 {
        test.phase_name = "Waiting for raiders...".into();
        if elapsed > 3.0 { test.fail_phase(elapsed, "No raider entities"); }
        return;
    }

    let raiding = raiding_query.iter().count();
    let returning = returning_query.iter().count();
    let carrying = carrying_query.iter().count();
    let camp_food = food_storage.food.get(1).copied().unwrap_or(0);

    match test.phase {
        // Phase 1: 3 raiders dispatched → Raiding marker
        1 => {
            test.phase_name = format!("raiding={}/3 alive={}", raiding, alive);
            if raiding >= 3 {
                test.pass_phase(elapsed, format!("raiding={}", raiding));
            } else if elapsed > 20.0 {
                test.fail_phase(elapsed, format!("raiding={} alive={}", raiding, alive));
            }
        }
        // Phase 2: Raiders arrive at farm (raiding count stays or transitions)
        2 => {
            // Track when any raider arrives (transitions to Returning with CarryingFood)
            test.phase_name = format!("raiding={} returning={} carrying={}", raiding, returning, carrying);
            if returning > 0 || carrying > 0 {
                test.pass_phase(elapsed, format!("returning={} carrying={}", returning, carrying));
            } else if elapsed > 30.0 {
                test.fail_phase(elapsed, format!("raiding={} returning=0", raiding));
            }
        }
        // Phase 3: Food stolen (farm food decreases is implicit — raider took it)
        3 => {
            test.phase_name = format!("carrying={} returning={}", carrying, returning);
            if carrying > 0 {
                test.pass_phase(elapsed, format!("carrying={}", carrying));
            } else if returning > 0 {
                // Already returning without carry flag = already delivered
                test.pass_phase(elapsed, format!("returning={} (already delivered?)", returning));
            } else if elapsed > 35.0 {
                test.fail_phase(elapsed, format!("carrying=0 returning=0"));
            }
        }
        // Phase 4: Raiders returning home
        4 => {
            test.phase_name = format!("returning={} carrying={} camp_food={}", returning, carrying, camp_food);
            if returning > 0 || camp_food > 0 {
                test.pass_phase(elapsed, format!("returning={} camp_food={}", returning, camp_food));
            } else if elapsed > 40.0 {
                test.fail_phase(elapsed, format!("returning=0 camp_food=0"));
            }
        }
        // Phase 5: Food delivered (camp food increases)
        5 => {
            test.phase_name = format!("camp_food={} returning={}", camp_food, returning);
            if camp_food > 0 {
                test.pass_phase(elapsed, format!("camp_food={}", camp_food));
                test.complete(elapsed);
            } else if elapsed > 60.0 {
                test.fail_phase(elapsed, format!("camp_food=0 returning={}", returning));
            }
        }
        _ => {}
    }
}
