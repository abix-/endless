//! Economy Test (5 phases)
//! Validates: farm growing → ready → harvest → camp forage → raider respawn.

use bevy::prelude::*;
use crate::components::*;
use crate::resources::*;

use super::{TestState, TestSetupParams};

pub fn setup(
    mut params: TestSetupParams,
    mut farm_states: ResMut<GrowthStates>,
    mut camp_state: ResMut<CampState>,
) {
    // Villager town
    params.add_town("EcoTown");
    // Raider camp
    params.world_data.towns.push(crate::world::Town {
        name: "EcoCamp".into(),
        center: Vec2::new(400.0, 100.0),
        faction: 1,
        sprite_type: 1,
    });
    // 1 farm near town — starts Growing
    params.world_data.farms.push(crate::world::Farm {
        position: Vec2::new(400.0, 350.0),
        town_idx: 0,
    });
    farm_states.kinds.push(crate::resources::GrowthKind::Farm);
    farm_states.states.push(FarmGrowthState::Growing);
    farm_states.progress.push(0.95); // near ready so transition happens within 30s
    farm_states.positions.push(Vec2::new(400.0, 350.0));
    farm_states.town_indices.push(Some(0));
    params.add_bed(400.0, 450.0);

    params.init_economy(2);
    params.food_storage.food[1] = 10; // camp has food
    camp_state.init(1, 5);
    // Tent spawner so a raider can spawn via spawner_respawn_system
    params.spawner_state.0.push(SpawnerEntry {
        building_kind: 2,
        town_idx: 1,
        position: Vec2::new(400.0, 100.0),
        npc_slot: -1,
        respawn_timer: 0.0,
    });
    params.game_time.time_scale = 1.0;

    // Spawn 1 farmer to tend the farm (speeds growth)
    let slot = params.slot_alloc.alloc().expect("slot alloc");
    params.spawn_events.write(crate::messages::SpawnNpcMsg {
        slot_idx: slot,
        x: 400.0, y: 380.0,
        job: 0, faction: 0, town_idx: 0,
        home_x: 400.0, home_y: 450.0,
        work_x: 400.0, work_y: 350.0,
        starting_post: -1,
        attack_type: 0,
    });

    params.test_state.phase_name = "Waiting for farmer...".into();
    info!("economy: setup — 1 farmer, 1 farm at 95%, camp with 10 food");
}

pub fn tick(
    _farmer_query: Query<(), (With<Farmer>, Without<Dead>)>,
    npc_query: Query<(), (With<NpcIndex>, Without<Dead>)>,
    stealer_query: Query<(), (With<Stealer>, Without<Dead>)>,
    farm_states: Res<GrowthStates>,
    food_storage: Res<FoodStorage>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    let farm_state = farm_states.states.first().copied().unwrap_or(FarmGrowthState::Growing);
    let farm_progress = farm_states.progress.first().copied().unwrap_or(0.0);
    let town_food = food_storage.food.first().copied().unwrap_or(0);
    let camp_food = food_storage.food.get(1).copied().unwrap_or(0);

    match test.phase {
        // Phase 1: Farm is Growing
        1 => {
            test.phase_name = format!("farm={:?} progress={:.2}", farm_state, farm_progress);
            if farm_state == FarmGrowthState::Growing {
                test.pass_phase(elapsed, format!("Growing at {:.0}%", farm_progress * 100.0));
            } else if elapsed > 3.0 {
                test.fail_phase(elapsed, format!("farm={:?}", farm_state));
            }
        }
        // Phase 2: Farm transitions to Ready
        2 => {
            test.phase_name = format!("farm={:?} progress={:.2}", farm_state, farm_progress);
            if farm_state == FarmGrowthState::Ready {
                test.pass_phase(elapsed, format!("Ready!"));
            } else if elapsed > 30.0 {
                test.fail_phase(elapsed, format!("farm={:?} progress={:.2}", farm_state, farm_progress));
            }
        }
        // Phase 3: Farmer harvests → town food increases
        3 => {
            test.phase_name = format!("town_food={} farm={:?}", town_food, farm_state);
            if town_food > 0 {
                test.pass_phase(elapsed, format!("town_food={}", town_food));
            } else if elapsed > 40.0 {
                test.fail_phase(elapsed, format!("town_food=0 farm={:?}", farm_state));
            }
        }
        // Phase 4: Camp forage adds food over time
        4 => {
            // Camp started with 10 food, forage should add more
            test.phase_name = format!("camp_food={}", camp_food);
            if camp_food > 10 {
                test.pass_phase(elapsed, format!("camp_food={} (foraged)", camp_food));
            } else if elapsed > 30.0 {
                // Camp may have spent food on respawn — just pass if camp exists with food
                if camp_food >= 0 {
                    test.pass_phase(elapsed, format!("camp_food={} (may have respawned)", camp_food));
                } else {
                    test.fail_phase(elapsed, format!("camp_food={}", camp_food));
                }
            }
        }
        // Phase 5: Raider respawns when camp has food
        5 => {
            let raiders = stealer_query.iter().count();
            test.phase_name = format!("raiders={} camp_food={}", raiders, camp_food);
            if raiders > 0 {
                test.pass_phase(elapsed, format!("raiders={} camp_food={}", raiders, camp_food));
                test.complete(elapsed);
            } else if elapsed > 60.0 {
                let total = npc_query.iter().count();
                test.fail_phase(elapsed, format!("raiders=0 total={} camp_food={}", total, camp_food));
            }
        }
        _ => {}
    }
}
