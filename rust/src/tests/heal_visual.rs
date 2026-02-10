//! Healing Visual Test (3 phases)
//! Validates: Healing NPC gets heal icon on Healing layer, cleared when healed.

use bevy::prelude::*;
use crate::components::*;
use crate::constants::HEAL_SPRITE;
use crate::gpu::NpcBufferWrites;

use super::{TestState, TestSetupParams};

pub fn setup(mut params: TestSetupParams) {
    params.add_town("HealVisTown");
    params.add_bed(400.0, 410.0);
    params.init_economy(1);
    params.food_storage.food[0] = 10; // enough food to prevent starvation
    params.game_time.time_scale = 1.0;

    // Spawn farmer at town center (inside healing radius)
    params.spawn_npc(0, 400.0, 400.0, 400.0, 410.0);

    params.test_state.phase_name = "Waiting for spawn...".into();
    info!("heal-visual: setup — 1 farmer at town center");
}

pub fn tick(
    mut health_query: Query<(&mut Health, &NpcIndex), (With<Farmer>, Without<Dead>)>,
    healing_query: Query<&NpcIndex, (With<Healing>, Without<Dead>)>,
    not_healing_query: Query<&NpcIndex, (With<Farmer>, Without<Healing>, Without<Dead>)>,
    buffer: Res<NpcBufferWrites>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    let Some((mut health, npc_idx)) = health_query.iter_mut().next() else {
        if !test.require_entity(0, elapsed, "farmer") { return; }
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
        // Phase 2: Healing NPC → healing_sprites should show HEAL_SPRITE
        2 => {
            let healing_idx = healing_query.iter().next().map(|n| n.0);
            test.phase_name = format!("hp={:.0} healing_idx={:?}", hp, healing_idx);

            if let Some(idx) = healing_idx {
                let sprite_col = buffer.healing_sprites.get(idx * 2).copied().unwrap_or(-1.0);
                if sprite_col == HEAL_SPRITE.0 {
                    test.pass_phase(elapsed, format!("Heal icon set (idx={}, col={:.0})", idx, sprite_col));
                } else {
                    // Healing but no icon — this is the expected RED failure
                    test.fail_phase(elapsed, format!("Healing but healing_sprites[{}]={:.1}, expected {:.0}", idx * 2, sprite_col, HEAL_SPRITE.0));
                }
            } else if elapsed > 15.0 {
                test.fail_phase(elapsed, format!("Lost Healing marker (hp={:.0})", hp));
            }
        }
        // Phase 3: NPC healed to full → Healing removed → healing_sprites cleared
        3 => {
            let not_healing_idx = not_healing_query.iter().next().map(|n| n.0);
            test.phase_name = format!("hp={:.0} not_healing={}", hp, not_healing_idx.is_some());

            if let Some(idx) = not_healing_idx {
                if hp >= 90.0 {
                    let sprite_col = buffer.healing_sprites.get(idx * 2).copied().unwrap_or(-1.0);
                    if sprite_col == -1.0 {
                        test.pass_phase(elapsed, format!("Heal icon cleared (hp={:.0})", hp));
                        test.complete(elapsed);
                    } else {
                        test.fail_phase(elapsed, format!("Healed but healing_sprites[{}]={:.1}, expected -1", idx * 2, sprite_col));
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
