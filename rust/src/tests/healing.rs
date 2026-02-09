//! Healing Aura Test (3 phases, time_scale=20)
//! Validates: damaged NPC near town → Healing marker → health increases → healing stops at max.

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
    mut faction_stats: ResMut<FactionStats>,
    mut game_time: ResMut<GameTime>,
    mut test_state: ResMut<TestState>,
) {
    // Town with healing aura (150px radius)
    world_data.towns.push(world::Town {
        name: "HealTown".into(),
        center: Vec2::new(400.0, 400.0),
        faction: 0,
        sprite_type: 0,
    });
    world_data.beds.push(world::Bed {
        position: Vec2::new(400.0, 410.0),
        town_idx: 0,
    });
    food_storage.init(1);
    faction_stats.init(1);
    game_time.time_scale = 20.0;

    // Spawn farmer RIGHT AT town center (within 150px healing radius)
    let slot = slot_alloc.alloc().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: slot,
        x: 400.0, y: 400.0, // at town center
        job: 0, faction: 0, town_idx: 0,
        home_x: 400.0, home_y: 410.0,
        work_x: -1.0, work_y: -1.0,
        starting_post: -1,
        attack_type: 0,
    });

    test_state.phase_name = "Waiting for spawn...".into();
    info!("healing: setup — 1 farmer at town center, time_scale=20");
}

pub fn tick(
    mut health_query: Query<(&mut Health, &MaxHealth, &NpcIndex), (With<Farmer>, Without<Dead>)>,
    healing_query: Query<(), (With<Healing>, With<Farmer>, Without<Dead>)>,
    mut last_ate_query: Query<&mut LastAteHour, (With<Farmer>, Without<Dead>)>,
    game_time: Res<GameTime>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    if test.passed || test.failed { return; }

    // Keep farmer fed — this test validates healing, not starvation
    for mut last_ate in last_ate_query.iter_mut() {
        last_ate.0 = game_time.total_hours();
    }

    let now = time.elapsed_secs();
    if test.start == 0.0 { test.start = now; }
    let elapsed = now - test.start;

    let Some((mut health, max_health, _)) = health_query.iter_mut().next() else {
        test.phase_name = "Waiting for farmer...".into();
        if elapsed > 3.0 { test.fail_phase(elapsed, "No farmer entity"); }
        return;
    };

    let healing = healing_query.iter().count();
    let hp = health.0;
    let max_hp = max_health.0;

    match test.phase {
        // Phase 1: Set NPC to 50 HP, wait for Healing marker
        1 => {
            if !test.get_flag("damaged") {
                health.0 = 50.0;
                test.set_flag("damaged", true);
                test.phase_name = "Damaged to 50 HP, waiting for Healing...".into();
            } else {
                test.phase_name = format!("hp={:.0} healing={}", hp, healing);
                if healing > 0 {
                    test.pass_phase(elapsed, format!("Healing marker (hp={:.0})", hp));
                } else if elapsed > 10.0 {
                    test.fail_phase(elapsed, format!("healing=0 hp={:.0}", hp));
                }
            }
        }
        // Phase 2: Health increases toward max
        2 => {
            test.phase_name = format!("hp={:.0}/{:.0}", hp, max_hp);
            if hp > 55.0 {
                test.pass_phase(elapsed, format!("hp={:.0} (increasing)", hp));
            } else if elapsed > 15.0 {
                test.fail_phase(elapsed, format!("hp={:.0} (not increasing)", hp));
            }
        }
        // Phase 3: Health reaches max → healing stops
        3 => {
            test.phase_name = format!("hp={:.0}/{:.0} healing={}", hp, max_hp, healing);
            if hp >= max_hp - 1.0 {
                test.pass_phase(elapsed, format!("hp={:.0} (healed to max)", hp));
                test.complete(elapsed);
            } else if elapsed > 30.0 {
                test.fail_phase(elapsed, format!("hp={:.0}/{:.0}", hp, max_hp));
            }
        }
        _ => {}
    }
}
