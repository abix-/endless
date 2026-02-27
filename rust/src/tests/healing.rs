//! Healing Aura Test (3 phases)
//! Validates: damaged NPC near town → Healing marker → health increases → healing stops at max.

use bevy::prelude::*;
use crate::components::*;
use crate::resources::EntityMap;

use super::{TestState, TestSetupParams};

pub fn setup(mut params: TestSetupParams) {
    params.add_town("HealTown");
    params.add_bed(400.0, 410.0);
    params.init_economy(1);
    params.game_time.time_scale = 1.0;
    params.spawn_npc(0, 400.0, 400.0, 400.0, 410.0);
    params.test_state.phase_name = "Waiting for spawn...".into();
    info!("healing: setup — 1 farmer at town center");
}

pub fn tick(
    mut entity_map: ResMut<EntityMap>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    let Some(farmer_slot) = entity_map.iter_npcs().find(|n| !n.dead && n.job == Job::Farmer).map(|n| n.slot) else {
        if !test.require_entity(0, elapsed, "farmer") { return; }
        return;
    };

    let healing = entity_map.iter_npcs().filter(|n| !n.dead && n.healing && n.job == Job::Farmer).count();
    let hp = entity_map.get_npc(farmer_slot).map(|n| n.health).unwrap_or(0.0);
    let max_hp = entity_map.get_npc(farmer_slot).map(|n| n.cached_stats.max_health).unwrap_or(100.0);

    match test.phase {
        // Phase 1: Set NPC to 50 HP, wait for Healing marker
        1 => {
            if !test.get_flag("damaged") {
                if let Some(npc) = entity_map.get_npc_mut(farmer_slot) { npc.health = 50.0; }
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
