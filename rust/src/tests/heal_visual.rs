//! Healing Visual Test (3 phases)
//! Validates: Healing NPC gets halo in NpcVisualUpload equip layer 5 (atlas_id=2.0), cleared when healed.

use crate::components::*;
use crate::gpu::NpcVisualUpload;
use crate::resources::EntityMap;
use bevy::prelude::*;

use super::{TestSetupParams, TestState};

pub fn setup(mut params: TestSetupParams) {
    params.add_town("HealVisTown");
    params.init_economy(1);
    if let Some(mut f) = params.town_access.food_mut(0) { f.0 = 10; }
    params.game_time.time_scale = 1.0;
    params.focus_camera(384.0, 384.0);

    // Spawn farmer at town center (inside healing radius)
    params.spawn_npc(0, 384.0, 384.0, 384.0, 384.0);

    params.test_state.phase_name = "Waiting for spawn...".into();
    info!("heal-visual: setup — 1 farmer at town center");
}

pub fn tick(
    entity_map: Res<EntityMap>,
    upload: Res<NpcVisualUpload>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    mut health_q: Query<&mut Health, Without<Building>>,
    npc_flags_q: Query<&NpcFlags>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    let Some(farmer) = entity_map
        .iter_npcs()
        .find(|n| !n.dead && n.job == Job::Farmer)
    else {
        if !test.require_entity(0, elapsed, "farmer") {
            return;
        }
        return;
    };
    let farmer_entity = farmer.entity;

    let hp = health_q.get(farmer_entity).map(|h| h.0).unwrap_or(0.0);

    match test.phase {
        // Phase 1: Damage NPC, wait for Healing marker
        1 => {
            if !test.get_flag("damaged") {
                if let Ok(mut h) = health_q.get_mut(farmer_entity) {
                    h.0 = 50.0;
                }
                test.set_flag("damaged", true);
                test.phase_name = "Damaged to 50 HP, waiting for Healing...".into();
            } else {
                let healing = entity_map
                    .iter_npcs()
                    .filter(|n| !n.dead && npc_flags_q.get(n.entity).is_ok_and(|f| f.healing))
                    .count();
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
            let healing_idx = entity_map
                .iter_npcs()
                .find(|n| !n.dead && npc_flags_q.get(n.entity).is_ok_and(|f| f.healing))
                .map(|n| n.slot);
            test.phase_name = format!("hp={:.0} healing_idx={:?}", hp, healing_idx);

            if let Some(idx) = healing_idx {
                let eq_base = idx * 28 + 20; // layer 5 = healing (7 layers × 4 floats = 28)
                let col = upload.equip_data.get(eq_base).copied().unwrap_or(-1.0);
                let atlas = upload.equip_data.get(eq_base + 2).copied().unwrap_or(0.0);
                if col >= 0.0 && atlas == 2.0 {
                    test.pass_phase(
                        elapsed,
                        format!("Halo active (idx={}, atlas={:.0})", idx, atlas),
                    );
                } else {
                    test.fail_phase(
                        elapsed,
                        format!(
                            "Healing but col={:.1} atlas={:.1}, expected col>=0 atlas=2",
                            col, atlas
                        ),
                    );
                }
            } else if elapsed > 15.0 {
                test.fail_phase(elapsed, format!("Lost Healing marker (hp={:.0})", hp));
            }
        }
        // Phase 3: NPC healed to full → Healing removed → equip layer 5 cleared
        3 => {
            let not_healing_idx = entity_map
                .iter_npcs()
                .find(|n| {
                    !n.dead
                        && n.job == Job::Farmer
                        && !npc_flags_q.get(n.entity).is_ok_and(|f| f.healing)
                })
                .map(|n| n.slot);
            test.phase_name = format!("hp={:.0} not_healing={}", hp, not_healing_idx.is_some());

            if let Some(idx) = not_healing_idx {
                if hp >= 90.0 {
                    let col = upload
                        .equip_data
                        .get(idx * 28 + 20)
                        .copied()
                        .unwrap_or(-1.0);
                    if col == -1.0 {
                        test.pass_phase(elapsed, format!("Halo cleared (hp={:.0})", hp));
                        test.complete(elapsed);
                    } else {
                        test.fail_phase(
                            elapsed,
                            format!(
                                "Healed but equip[{}]={:.1}, expected -1",
                                idx * 28 + 20,
                                col
                            ),
                        );
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
