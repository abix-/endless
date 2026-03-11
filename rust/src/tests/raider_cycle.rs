//! Raider Raid Cycle Test (5 phases)
//! Validates: raiders dispatched → arrive at farm → steal food → returning → deliver to raider town.

use crate::components::*;
use crate::resources::*;
use bevy::prelude::*;

use super::{TestSetupParams, TestState};

pub fn setup(mut params: TestSetupParams, mut raider_state: ResMut<RaiderState>) {
    // Villager town (faction 1) with farms
    params.add_town("FarmVille");
    // Raider raider town (faction 2)
    params.world_data.towns.push(crate::world::Town {
        name: "RaiderTown".into(),
        center: Vec2::new(384.0, 128.0),
        faction: 2,
        kind: crate::constants::TownKind::AiRaider,
    area_level: 0,
    });
    // 3 farms near villager town — all Ready so raiders can steal
    for i in 0..3 {
        let fx = 320.0 + (i as f32 * 64.0);
        params.add_building(crate::world::BuildingKind::Farm, fx, 320.0, 0);
        params.set_production_ready(Vec2::new(fx, 320.0));
    }
    params.init_economy(2);
    if let Some(mut f) = params.town_access.food_mut(0) { f.0 = 10; } // villager food
    raider_state.init(1, 5);
    params.game_time.time_scale = 1.0;

    // Spawn 3 raiders (minimum for RAID_GROUP_SIZE)
    for i in 0..3 {
        let slot = params.slot_alloc.alloc_reset().expect("slot alloc");
        params.spawn_events.write(crate::messages::SpawnNpcMsg {
            slot_idx: slot,
            x: 384.0 + (i as f32 * 20.0),
            y: 128.0,
            job: 2,
            faction: 2,
            town_idx: 1,
            home_x: 384.0,
            home_y: 128.0,
            work_x: -1.0,
            work_y: -1.0,
            starting_post: -1,
            entity_override: None,
        });
    }

    params.focus_camera(384.0, 256.0);
    params.test_state.phase_name = "Waiting for raiders...".into();
    info!("raider-cycle: setup — 3 raiders, 3 ready farms");
}

pub fn tick(
    entity_map: Res<EntityMap>,
    town_access: crate::systemparams::TownAccess,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    activity_q: Query<&Activity>,
    carried_loot_q: Query<&CarriedLoot>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };
    let alive = entity_map
        .iter_npcs()
        .filter(|n| !n.dead && n.job == crate::components::Job::Raider)
        .count();
    if !test.require_entity(alive, elapsed, "raider") {
        return;
    }

    let mut raiding = 0;
    let mut returning = 0;
    let mut carrying = 0;
    for npc in entity_map
        .iter_npcs()
        .filter(|n| !n.dead && n.job == crate::components::Job::Raider)
    {
        match activity_q.get(npc.entity).ok() {
            Some(a) if a.kind == ActivityKind::Raid => raiding += 1,
            Some(a) if matches!(a.kind, ActivityKind::ReturnLoot) => {
                returning += 1;
                if carried_loot_q
                    .get(npc.entity)
                    .is_ok_and(|cl| cl.food > 0)
                {
                    carrying += 1;
                }
            }
            _ => {}
        }
    }
    let raider_food = town_access.food(1);

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
            test.phase_name = format!(
                "raiding={} returning={} carrying={}",
                raiding, returning, carrying
            );
            if returning > 0 || carrying > 0 {
                test.pass_phase(
                    elapsed,
                    format!("returning={} carrying={}", returning, carrying),
                );
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
                test.pass_phase(
                    elapsed,
                    format!("returning={} (already delivered?)", returning),
                );
            } else if elapsed > 35.0 {
                test.fail_phase(elapsed, "carrying=0 returning=0".to_string());
            }
        }
        // Phase 4: Raiders returning home
        4 => {
            test.phase_name = format!(
                "returning={} carrying={} raider_food={}",
                returning, carrying, raider_food
            );
            if returning > 0 || raider_food > 0 {
                test.pass_phase(
                    elapsed,
                    format!("returning={} raider_food={}", returning, raider_food),
                );
            } else if elapsed > 40.0 {
                test.fail_phase(elapsed, "returning=0 raider_food=0".to_string());
            }
        }
        // Phase 5: Food delivered (raider town food increases)
        5 => {
            test.phase_name = format!("raider_food={} returning={}", raider_food, returning);
            if raider_food > 0 {
                test.pass_phase(elapsed, format!("raider_food={}", raider_food));
                test.complete(elapsed);
            } else if elapsed > 60.0 {
                test.fail_phase(elapsed, format!("raider_food=0 returning={}", returning));
            }
        }
        _ => {}
    }
}
