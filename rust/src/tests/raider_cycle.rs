//! Raider Raid Cycle Test (5 phases)
//! Validates: raiders dispatched → arrive at farm → steal food → returning → deliver to camp.

use bevy::prelude::*;
use crate::components::*;
use crate::resources::*;

use super::{TestState, TestSetupParams};

pub fn setup(
    mut params: TestSetupParams,
    mut farm_states: ResMut<GrowthStates>,
    mut camp_state: ResMut<CampState>,
) {
    // Villager town (faction 0) with farms
    params.add_town("FarmVille");
    // Raider camp (faction 1)
    params.world_data.towns.push(crate::world::Town {
        name: "RaiderCamp".into(),
        center: Vec2::new(400.0, 100.0),
        faction: 1,
        sprite_type: 1,
    });
    // 3 farms near villager town — all Ready so raiders can steal
    for i in 0..3 {
        params.world_data.farms.push(crate::world::Farm {
            position: Vec2::new(350.0 + (i as f32 * 50.0), 350.0),
            town_idx: 0,
        });
        farm_states.kinds.push(crate::resources::GrowthKind::Farm);
        farm_states.states.push(FarmGrowthState::Ready);
        farm_states.progress.push(1.0);
        farm_states.positions.push(Vec2::new(350.0 + (i as f32 * 50.0), 350.0));
        farm_states.town_indices.push(Some(0));
    }
    params.init_economy(2);
    params.food_storage.food[0] = 10; // villager food
    params.food_storage.food[1] = 0;  // raider camp starts empty
    camp_state.init(1, 5);
    params.game_time.time_scale = 1.0;

    // Spawn 3 raiders (minimum for RAID_GROUP_SIZE)
    for i in 0..3 {
        let slot = params.slot_alloc.alloc().expect("slot alloc");
        params.spawn_events.write(crate::messages::SpawnNpcMsg {
            slot_idx: slot,
            x: 380.0 + (i as f32 * 20.0), y: 100.0,
            job: 2, faction: 1, town_idx: 1,
            home_x: 400.0, home_y: 100.0,
            work_x: -1.0, work_y: -1.0,
            starting_post: -1,
            attack_type: 0,
        });
    }

    params.test_state.phase_name = "Waiting for raiders...".into();
    info!("raider-cycle: setup — 3 raiders, 3 ready farms");
}

pub fn tick(
    activity_query: Query<&Activity, (With<Stealer>, Without<Dead>)>,
    npc_query: Query<(), (With<Stealer>, Without<Dead>)>,
    food_storage: Res<FoodStorage>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };
    let alive = npc_query.iter().count();
    if !test.require_entity(alive, elapsed, "raider") { return; }

    let mut raiding = 0;
    let mut returning = 0;
    let mut carrying = 0;
    for activity in activity_query.iter() {
        match activity {
            Activity::Raiding { .. } => raiding += 1,
            Activity::Returning { has_food, .. } => {
                returning += 1;
                if *has_food { carrying += 1; }
            }
            _ => {}
        }
    }
    let camp_food = food_storage.food.get(1).copied().unwrap_or(0);

    match test.phase {
        // Phase 1: 3 raiders dispatched → Raiding
        1 => {
            test.phase_name = format!("raiding={}/3 alive={}", raiding, alive);
            if raiding >= 3 {
                test.pass_phase(elapsed, format!("raiding={}", raiding));
            } else if elapsed > 20.0 {
                test.fail_phase(elapsed, format!("raiding={} alive={}", raiding, alive));
            }
        }
        // Phase 2: Raiders arrive at farm (transitions to Returning with food)
        2 => {
            test.phase_name = format!("raiding={} returning={} carrying={}", raiding, returning, carrying);
            if returning > 0 || carrying > 0 {
                test.pass_phase(elapsed, format!("returning={} carrying={}", returning, carrying));
            } else if elapsed > 30.0 {
                test.fail_phase(elapsed, format!("raiding={} returning=0", raiding));
            }
        }
        // Phase 3: Food stolen (raider has food)
        3 => {
            test.phase_name = format!("carrying={} returning={}", carrying, returning);
            if carrying > 0 {
                test.pass_phase(elapsed, format!("carrying={}", carrying));
            } else if returning > 0 {
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
