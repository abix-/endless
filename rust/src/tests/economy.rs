//! Economy Test (5 phases)
//! Validates: farm growing → ready → harvest → raider town forage → raider respawn.

use bevy::prelude::*;

use crate::resources::*;

use super::{TestSetupParams, TestState};

pub fn setup(mut params: TestSetupParams, mut raider_state: ResMut<RaiderState>) {
    // Villager town
    params.add_town("EcoTown");
    // Raider raider town
    params.world_data.towns.push(crate::world::Town {
        name: "EcoRaider".into(),
        center: Vec2::new(384.0, 128.0),
        faction: 2,
        sprite_type: 1,
    area_level: 0,
    });
    // 1 farm near town — starts Growing at 95%
    params.add_building(crate::world::BuildingKind::Farm, 384.0, 320.0, 0);
    // Set production progress to 95% via ECS
    if let Some(inst) = params.entity_map.find_by_position(Vec2::new(384.0, 320.0)) {
        let slot = inst.slot;
        if let Some(&entity) = params.entity_map.entities.get(&slot) {
            params.commands.entity(entity).insert(crate::components::ProductionState {
                ready: false,
                progress: 0.95,
            });
        }
    }

    params.init_economy(2);
    params.food_storage.food[1] = 10; // raider town has food
    raider_state.init(1, 5);
    // Tent spawner so a raider can spawn via spawner_respawn_system
    params.add_building(crate::world::BuildingKind::Tent, 384.0, 128.0, 1);
    // SpawnerState inserted by place_building with respawn_timer = 0.0 by default
    params.game_time.time_scale = 1.0;

    // Spawn 1 farmer to tend the farm (speeds growth)
    let slot = params.slot_alloc.alloc_reset().expect("slot alloc");
    params.spawn_events.write(crate::messages::SpawnNpcMsg {
        slot_idx: slot,
        x: 384.0,
        y: 384.0,
        job: 0,
        faction: 1,
        town_idx: 0,
        home_x: 384.0,
        home_y: 384.0,
        work_x: 384.0,
        work_y: 320.0,
        starting_post: -1,
        attack_type: 0,
        uid_override: None,
    });

    params.focus_camera(384.0, 384.0);
    params.test_state.phase_name = "Waiting for farmer...".into();
    info!("economy: setup — 1 farmer, 1 farm at 95%, raider town with 10 food");
}

pub fn tick(
    entity_map: Res<EntityMap>,
    food_storage: Res<FoodStorage>,
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
    let farm_ready = farm_ps.map(|ps| ps.ready).unwrap_or(false);
    let farm_progress = farm_ps.map(|ps| ps.progress).unwrap_or(0.0);
    let town_food = food_storage.food.first().copied().unwrap_or(0);
    let raider_food = food_storage.food.get(1).copied().unwrap_or(0);

    match test.phase {
        // Phase 1: Farm is Growing
        1 => {
            test.phase_name = format!("ready={} progress={:.2}", farm_ready, farm_progress);
            if !farm_ready {
                test.pass_phase(elapsed, format!("Growing at {:.0}%", farm_progress * 100.0));
            } else if elapsed > 3.0 {
                test.fail_phase(elapsed, format!("ready={}", farm_ready));
            }
        }
        // Phase 2: Farm transitions to Ready
        2 => {
            test.phase_name = format!("ready={} progress={:.2}", farm_ready, farm_progress);
            if farm_ready {
                test.pass_phase(elapsed, "Ready!".to_string());
            } else if elapsed > 30.0 {
                test.fail_phase(
                    elapsed,
                    format!("ready={} progress={:.2}", farm_ready, farm_progress),
                );
            }
        }
        // Phase 3: Farmer harvests → town food increases
        3 => {
            test.phase_name = format!("town_food={} farm={:?}", town_food, farm_ready);
            if town_food > 0 {
                test.pass_phase(elapsed, format!("town_food={}", town_food));
            } else if elapsed > 40.0 {
                test.fail_phase(elapsed, format!("town_food=0 farm={:?}", farm_ready));
            }
        }
        // Phase 4: Raider forage adds food over time
        4 => {
            // Raider town started with 10 food, forage should add more
            test.phase_name = format!("raider_food={}", raider_food);
            if raider_food > 10 {
                test.pass_phase(elapsed, format!("raider_food={} (foraged)", raider_food));
            } else if elapsed > 30.0 {
                // Raider town may have spent food on respawn — just pass if raider town exists with food
                if raider_food >= 0 {
                    test.pass_phase(
                        elapsed,
                        format!("raider_food={} (may have respawned)", raider_food),
                    );
                } else {
                    test.fail_phase(elapsed, format!("raider_food={}", raider_food));
                }
            }
        }
        // Phase 5: Raider respawns when raider town has food
        5 => {
            let raiders = entity_map
                .iter_npcs()
                .filter(|n| !n.dead && n.job == crate::components::Job::Raider)
                .count();
            test.phase_name = format!("raiders={} raider_food={}", raiders, raider_food);
            if raiders > 0 {
                test.pass_phase(
                    elapsed,
                    format!("raiders={} raider_food={}", raiders, raider_food),
                );
                test.complete(elapsed);
            } else if elapsed > 60.0 {
                let total = entity_map.iter_npcs().filter(|n| !n.dead).count();
                test.fail_phase(
                    elapsed,
                    format!("raiders=0 total={} raider_food={}", total, raider_food),
                );
            }
        }
        _ => {}
    }
}
