//! Miner Work Cycle Test (5 phases)
//! Validates: Idle → Mining (walk to mine) → MiningAtMine (tend) → harvest gold → Returning →
//! deliver gold at home → energy drain → rest → wake.

use crate::components::*;
use crate::constants::ENERGY_WAKE_THRESHOLD;
use crate::resources::*;
use bevy::prelude::*;

use super::{TestSetupParams, TestState};

pub fn setup(mut params: TestSetupParams, mut gold_storage: ResMut<GoldStorage>) {
    params.add_town("MinerTown");
    params.init_economy(1);
    gold_storage.init(1);
    params.game_time.time_scale = 1.0;

    // Place MinerHome building at (384,384)
    params.add_building(crate::world::BuildingKind::MinerHome, 384.0, 384.0, 0);

    // Place GoldMine building at (384,256)
    params.add_building(crate::world::BuildingKind::GoldMine, 384.0, 256.0, 0);

    // Spawn miner (job=4) at town center, home at MinerHome
    let slot = params.slot_alloc.alloc_reset().expect("slot alloc");
    params.spawn_events.write(crate::messages::SpawnNpcMsg {
        slot_idx: slot,
        x: 384.0,
        y: 384.0,
        job: 4,
        faction: 1,
        town_idx: 0,
        home_x: 384.0,
        home_y: 384.0,
        work_x: 384.0,
        work_y: 256.0,
        starting_post: -1,
        attack_type: 0,
        uid_override: None,
    });

    params.focus_camera(384.0, 320.0);
    params.test_state.phase_name = "Waiting for miner...".into();
    info!("miner-cycle: setup — 1 miner, 1 gold mine");
}

pub fn tick(
    entity_map: Res<EntityMap>,
    gold_storage: Res<GoldStorage>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    activity_q: Query<&Activity>,
    mut energy_q: Query<&mut Energy>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };
    let miner_count = entity_map
        .iter_npcs()
        .filter(|n| !n.dead && n.job == Job::Miner)
        .count();
    if !test.require_entity(miner_count, elapsed, "miner") {
        return;
    }

    let energy = entity_map
        .iter_npcs()
        .find(|n| !n.dead && n.job == Job::Miner)
        .and_then(|n| energy_q.get(n.entity).ok())
        .map(|e| e.0)
        .unwrap_or(100.0);
    let gold = gold_storage.gold.first().copied().unwrap_or(0);

    let mining = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && n.job == Job::Miner
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| a.kind == ActivityKind::Mining)
        })
        .count();
    let mining_at_mine = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && n.job == Job::Miner
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| a.kind == ActivityKind::MiningAtMine)
        })
        .count();
    let returning = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && n.job == Job::Miner
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| a.kind == ActivityKind::Returning)
        })
        .count();
    let idle = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && n.job == Job::Miner
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| a.kind == ActivityKind::Idle)
        })
        .count();
    let going_rest = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && n.job == Job::Miner
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| a.kind == ActivityKind::GoingToRest)
        })
        .count();
    let resting = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && n.job == Job::Miner
                && activity_q
                    .get(n.entity)
                    .is_ok_and(|a| a.kind == ActivityKind::Resting)
        })
        .count();

    match test.phase {
        // Phase 1: Miner starts heading to mine
        1 => {
            test.phase_name = format!("mining={} at_mine={}", mining, mining_at_mine);
            if mining > 0 || mining_at_mine > 0 {
                test.pass_phase(
                    elapsed,
                    format!("mining={} at_mine={}", mining, mining_at_mine),
                );
            } else if elapsed > 5.0 {
                test.fail_phase(elapsed, "no Mining activity");
            }
        }
        // Phase 2: Arrives at mine → MiningAtMine (tending)
        2 => {
            test.phase_name = format!("at_mine={} mining={}", mining_at_mine, mining);
            if mining_at_mine > 0 {
                test.pass_phase(elapsed, "MiningAtMine (tending)".to_string());
            } else if elapsed > 10.0 {
                test.fail_phase(elapsed, format!("mining={} at_mine=0", mining));
            }
        }
        // Phase 3: Mine becomes Ready → miner harvests → Returning with gold
        3 => {
            test.phase_name = format!(
                "returning={} gold={} at_mine={}",
                returning, gold, mining_at_mine
            );
            if returning > 0 {
                test.pass_phase(elapsed, "Returning with gold".to_string());
            } else if elapsed > 15.0 {
                test.fail_phase(elapsed, format!("at_mine={} returning=0", mining_at_mine));
            }
        }
        // Phase 4: Gold delivered at home → gold_storage > 0
        4 => {
            test.phase_name = format!("gold={} idle={} returning={}", gold, idle, returning);
            if gold > 0 {
                test.pass_phase(elapsed, format!("Delivered {} gold", gold));
                // Now set energy low so tired→rest happens within test window
                for npc in entity_map
                    .iter_npcs()
                    .filter(|n| !n.dead && n.job == Job::Miner)
                {
                    if let Ok(mut en) = energy_q.get_mut(npc.entity) {
                        en.0 = 35.0;
                    }
                }
            } else if elapsed > 20.0 {
                test.fail_phase(elapsed, format!("gold=0 returning={}", returning));
            }
        }
        // Phase 5: Energy drains → rests → wakes up
        5 => {
            test.phase_name = format!(
                "e={:.0} resting={} going_rest={}",
                energy, resting, going_rest
            );
            if test.get_flag("was_resting") && resting == 0 && energy >= ENERGY_WAKE_THRESHOLD {
                test.pass_phase(elapsed, format!("Woke up (energy={:.0})", energy));
                test.complete(elapsed);
            } else {
                if resting > 0 || going_rest > 0 {
                    test.set_flag("was_resting", true);
                }
                if elapsed > 30.0 {
                    test.fail_phase(elapsed, format!("energy={:.0} resting={}", energy, resting));
                }
            }
        }
        _ => {}
    }
}
