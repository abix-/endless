//! Sleep Visual Test (3 phases)
//! Validates: Resting NPC gets sleep icon in NpcVisualUpload equip layer 4, cleared on wake.

use bevy::prelude::*;
use crate::components::*;
use crate::gpu::NpcVisualUpload;
use crate::resources::EntityMap;
use super::{TestState, TestSetupParams};

pub fn setup(mut params: TestSetupParams) {
    params.add_town("SleepTown");
    params.add_bed(380.0, 420.0);
    params.add_building(crate::world::BuildingKind::Farm, 450.0, 400.0, 0);
    params.init_economy(1);
    params.game_time.time_scale = 1.0;
    params.focus_camera(400.0, 400.0);

    // Spawn 1 farmer with work position at farm
    let slot = params.slot_alloc.alloc().expect("slot alloc");
    params.spawn_events.write(crate::messages::SpawnNpcMsg {
        slot_idx: slot,
        x: 450.0, y: 400.0,
        job: 0, faction: 0, town_idx: 0,
        home_x: 380.0, home_y: 420.0,
        work_x: 450.0, work_y: 400.0,
        starting_post: -1,
        attack_type: 0,
    });

    params.test_state.phase_name = "Waiting for farmer spawn...".into();
    info!("sleep-visual: setup — 1 farmer");
}

pub fn tick(
    entity_map: Res<EntityMap>,
    upload: Res<NpcVisualUpload>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    activity_q: Query<&Activity>,
    mut energy_q: Query<&mut Energy>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };
    let farmer_count = entity_map.iter_npcs().filter(|n| !n.dead && n.job == Job::Farmer).count();
    if !test.require_entity(farmer_count, elapsed, "farmer") { return; }

    let energy = entity_map.iter_npcs().find(|n| !n.dead && n.job == Job::Farmer)
        .and_then(|n| energy_q.get(n.entity).ok()).map(|e| e.0).unwrap_or(100.0);

    // Start energy near tired threshold so rest triggers within 30s
    if !test.get_flag("energy_set") {
        for npc in entity_map.iter_npcs() {
            if !npc.dead && npc.job == Job::Farmer {
                if let Ok(mut en) = energy_q.get_mut(npc.entity) { en.0 = 35.0; }
            }
        }
        test.set_flag("energy_set", true);
    }

    match test.phase {
        // Phase 1: Farmer spawns alive
        1 => {
            test.phase_name = format!("e={:.0}", energy);
            if energy > 0.0 {
                test.pass_phase(elapsed, format!("Farmer alive (energy={:.0})", energy));
            } else if elapsed > 5.0 {
                test.fail_phase(elapsed, "Farmer not found");
            }
        }
        // Phase 2: Farmer rests → equip layer 4 should show sleep icon (atlas=3.0)
        2 => {
            let resting = entity_map.iter_npcs().find(|n| !n.dead && n.job == Job::Farmer && activity_q.get(n.entity).is_ok_and(|a| matches!(*a, Activity::Resting))).map(|n| n.slot);
            test.phase_name = format!("e={:.0} resting={}", energy, resting.is_some());

            if let Some(idx) = resting {
                let eq_base = idx * 24 + 16; // layer 4 = status
                let col = upload.equip_data.get(eq_base).copied().unwrap_or(-1.0);
                let atlas = upload.equip_data.get(eq_base + 2).copied().unwrap_or(0.0);
                if col >= 0.0 && atlas >= 2.5 {
                    test.pass_phase(elapsed, format!("Sleep icon set (idx={}, col={:.0}, atlas={:.0})", idx, col, atlas));
                } else {
                    test.fail_phase(elapsed, format!("Resting but equip[{}] col={:.1} atlas={:.1}, expected col>=0 atlas=3", eq_base, col, atlas));
                }
            } else if elapsed > 45.0 {
                test.fail_phase(elapsed, format!("energy={:.0} but never rested", energy));
            }
        }
        // Phase 3: Farmer wakes → equip layer 4 should be cleared (-1)
        3 => {
            let awake = entity_map.iter_npcs().find(|n| !n.dead && n.job == Job::Farmer && !activity_q.get(n.entity).is_ok_and(|a| matches!(*a, Activity::Resting))).map(|n| n.slot);
            test.phase_name = format!("e={:.0} awake={}", energy, awake.is_some());

            if let Some(idx) = awake {
                if energy >= 80.0 {
                    let col = upload.equip_data.get(idx * 24 + 16).copied().unwrap_or(-1.0);
                    if col == -1.0 {
                        test.pass_phase(elapsed, format!("Sleep icon cleared (idx={}, energy={:.0})", idx, energy));
                        test.complete(elapsed);
                    } else {
                        test.fail_phase(elapsed, format!("Awake but equip[{}]={:.1}, expected -1", idx * 24 + 16, col));
                    }
                }
            }

            if elapsed > 90.0 {
                test.fail_phase(elapsed, format!("energy={:.0} never recovered", energy));
            }
        }
        _ => {}
    }
}
