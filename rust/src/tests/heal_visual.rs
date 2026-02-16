//! Healing Visual Test (3 phases)
//! Validates: Healing NPC gets halo in NpcVisualUpload equip layer 5 (atlas_id=2.0), cleared when healed.

use bevy::prelude::*;
use crate::components::*;
use crate::gpu::NpcVisualUpload;

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
    upload: Res<NpcVisualUpload>,
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
        // Phase 2: Healing NPC → equip layer 5 should show halo (col>=0, atlas=2.0)
        2 => {
            let healing_idx = healing_query.iter().next().map(|n| n.0);
            test.phase_name = format!("hp={:.0} healing_idx={:?}", hp, healing_idx);

            if let Some(idx) = healing_idx {
                let eq_base = idx * 24 + 20; // layer 5 = healing
                let col = upload.equip_data.get(eq_base).copied().unwrap_or(-1.0);
                let atlas = upload.equip_data.get(eq_base + 2).copied().unwrap_or(0.0);
                if col >= 0.0 && atlas == 2.0 {
                    test.pass_phase(elapsed, format!("Halo active (idx={}, atlas={:.0})", idx, atlas));
                } else {
                    test.fail_phase(elapsed, format!("Healing but col={:.1} atlas={:.1}, expected col>=0 atlas=2", col, atlas));
                }
            } else if elapsed > 15.0 {
                test.fail_phase(elapsed, format!("Lost Healing marker (hp={:.0})", hp));
            }
        }
        // Phase 3: NPC healed to full → Healing removed → equip layer 5 cleared
        3 => {
            let not_healing_idx = not_healing_query.iter().next().map(|n| n.0);
            test.phase_name = format!("hp={:.0} not_healing={}", hp, not_healing_idx.is_some());

            if let Some(idx) = not_healing_idx {
                if hp >= 90.0 {
                    let col = upload.equip_data.get(idx * 24 + 20).copied().unwrap_or(-1.0);
                    if col == -1.0 {
                        test.pass_phase(elapsed, format!("Halo cleared (hp={:.0})", hp));
                        test.complete(elapsed);
                    } else {
                        test.fail_phase(elapsed, format!("Healed but equip[{}]={:.1}, expected -1", idx * 24 + 20, col));
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
