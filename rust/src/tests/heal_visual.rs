//! Healing Visual Test (3 phases, time_scale=20)
//! Validates: Healing NPC gets heal icon on Item equip layer, cleared when healed.

use bevy::prelude::*;
use crate::components::*;
use crate::constants::HEAL_SPRITE;
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
    // Town with healing aura (150px radius)
    world_data.towns.push(world::Town {
        name: "HealVisTown".into(),
        center: Vec2::new(400.0, 400.0),
        faction: 0,
        sprite_type: 0,
    });
    world_data.beds.push(world::Bed {
        position: Vec2::new(400.0, 410.0),
        town_idx: 0,
    });
    food_storage.init(1);
    food_storage.food[0] = 10; // enough food to prevent starvation
    faction_stats.init(1);
    game_time.time_scale = 20.0;

    // Spawn farmer at town center (inside healing radius)
    let slot = slot_alloc.alloc().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: slot,
        x: 400.0, y: 400.0,
        job: 0, faction: 0, town_idx: 0,
        home_x: 400.0, home_y: 410.0,
        work_x: -1.0, work_y: -1.0,
        starting_post: -1,
        attack_type: 0,
    });

    test_state.phase_name = "Waiting for spawn...".into();
    info!("heal-visual: setup — 1 farmer at town center, time_scale=20");
}

pub fn tick(
    mut health_query: Query<(&mut Health, &NpcIndex), (With<Farmer>, Without<Dead>)>,
    healing_query: Query<&NpcIndex, (With<Healing>, Without<Dead>)>,
    not_healing_query: Query<&NpcIndex, (With<Farmer>, Without<Healing>, Without<Dead>)>,
    buffer: Res<NpcBufferWrites>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    if test.passed || test.failed { return; }

    let now = time.elapsed_secs();
    if test.start == 0.0 { test.start = now; }
    let elapsed = now - test.start;

    let Some((mut health, npc_idx)) = health_query.iter_mut().next() else {
        test.phase_name = "Waiting for farmer...".into();
        if elapsed > 3.0 { test.fail_phase(elapsed, "No farmer entity"); }
        return;
    };

    let _idx = npc_idx.0;
    let hp = health.0;

    match test.phase {
        // Phase 1: Damage NPC, wait for Healing marker
        1 => {
            if !test.get_flag("damaged") {
                health.0 = 50.0;
                test.set_flag("damaged", true);
                test.phase_name = "Damaged to 50 HP, waiting for Healing...".into();
            } else {
                let healing = healing_query.iter().count();
                test.phase_name = format!("hp={:.0} healing={}", hp, healing);
                if healing > 0 {
                    test.pass_phase(elapsed, format!("Healing marker (hp={:.0})", hp));
                } else if elapsed > 10.0 {
                    test.fail_phase(elapsed, format!("healing=0 hp={:.0}", hp));
                }
            }
        }
        // Phase 2: Healing NPC → item_sprites should show HEAL_SPRITE
        2 => {
            let healing_idx = healing_query.iter().next().map(|n| n.0);
            test.phase_name = format!("hp={:.0} healing_idx={:?}", hp, healing_idx);

            if let Some(idx) = healing_idx {
                let sprite_col = buffer.item_sprites.get(idx * 2).copied().unwrap_or(-1.0);
                if sprite_col == HEAL_SPRITE.0 {
                    test.pass_phase(elapsed, format!("Heal icon set (idx={}, col={:.0})", idx, sprite_col));
                } else {
                    // Healing but no icon — this is the expected RED failure
                    test.fail_phase(elapsed, format!("Healing but item_sprites[{}]={:.1}, expected {:.0}", idx * 2, sprite_col, HEAL_SPRITE.0));
                }
            } else if elapsed > 15.0 {
                test.fail_phase(elapsed, format!("Lost Healing marker (hp={:.0})", hp));
            }
        }
        // Phase 3: NPC healed to full → Healing removed → item_sprites cleared
        3 => {
            let not_healing_idx = not_healing_query.iter().next().map(|n| n.0);
            test.phase_name = format!("hp={:.0} not_healing={}", hp, not_healing_idx.is_some());

            if let Some(idx) = not_healing_idx {
                if hp >= 90.0 {
                    let sprite_col = buffer.item_sprites.get(idx * 2).copied().unwrap_or(-1.0);
                    if sprite_col == -1.0 {
                        test.pass_phase(elapsed, format!("Heal icon cleared (hp={:.0})", hp));
                        test.complete(elapsed);
                    } else {
                        test.fail_phase(elapsed, format!("Healed but item_sprites[{}]={:.1}, expected -1", idx * 2, sprite_col));
                    }
                }
            }

            if elapsed > 30.0 {
                test.fail_phase(elapsed, format!("hp={:.0} healing never finished", hp));
            }
        }
        _ => {}
    }
}
