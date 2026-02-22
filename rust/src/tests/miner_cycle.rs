//! Miner Work Cycle Test (5 phases)
//! Validates: Idle → Mining (walk to mine) → MiningAtMine (tend) → harvest gold → Returning →
//! deliver gold at home → energy drain → rest → wake.

use bevy::prelude::*;
use crate::components::*;
use crate::constants::ENERGY_WAKE_THRESHOLD;
use crate::resources::*;

use super::{TestState, TestSetupParams};

pub fn setup(
    mut params: TestSetupParams,
    mut farm_states: ResMut<GrowthStates>,
    mut gold_storage: ResMut<GoldStorage>,
) {
    params.add_town("MinerTown");
    params.add_bed(400.0, 450.0);
    params.init_economy(1);
    gold_storage.init(1);
    params.game_time.time_scale = 1.0;

    // Place MinerHome building at (380,400)
    params.world_data.miner_homes_mut().push(crate::world::PlacedBuilding::new(Vec2::new(380.0, 400.0), 0));

    // Place GoldMine building at (400,300) + register in GrowthStates
    params.world_data.gold_mines_mut().push(crate::world::PlacedBuilding::new(Vec2::new(400.0, 300.0), 0));
    farm_states.push_mine(Vec2::new(400.0, 300.0));

    // Spawn miner (job=4) at town center, home at MinerHome
    let slot = params.slot_alloc.alloc().expect("slot alloc");
    params.spawn_events.write(crate::messages::SpawnNpcMsg {
        slot_idx: slot,
        x: 400.0, y: 400.0,
        job: 4, faction: 0, town_idx: 0,
        home_x: 380.0, home_y: 400.0,
        work_x: 400.0, work_y: 300.0,
        starting_post: -1,
        attack_type: 0,
    });

    params.test_state.phase_name = "Waiting for miner...".into();
    info!("miner-cycle: setup — 1 miner, 1 gold mine");
}

pub fn tick(
    activity_query: Query<&Activity, (With<Miner>, Without<Dead>)>,
    mut energy_query: Query<&mut Energy, (With<Miner>, Without<Dead>)>,
    miner_query: Query<(), (With<Miner>, Without<Dead>)>,
    gold_storage: Res<GoldStorage>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };
    if !test.require_entity(miner_query.iter().count(), elapsed, "miner") { return; }

    let energy = energy_query.iter().next().map(|e| e.0).unwrap_or(100.0);
    let gold = gold_storage.gold.first().copied().unwrap_or(0);

    let mining = activity_query.iter().filter(|a| matches!(a, Activity::Mining { .. })).count();
    let mining_at_mine = activity_query.iter().filter(|a| matches!(a, Activity::MiningAtMine)).count();
    let returning = activity_query.iter().filter(|a| matches!(a, Activity::Returning { .. })).count();
    let idle = activity_query.iter().filter(|a| matches!(a, Activity::Idle)).count();
    let going_rest = activity_query.iter().filter(|a| matches!(a, Activity::GoingToRest)).count();
    let resting = activity_query.iter().filter(|a| matches!(a, Activity::Resting)).count();

    match test.phase {
        // Phase 1: Miner starts heading to mine
        1 => {
            test.phase_name = format!("mining={} at_mine={}", mining, mining_at_mine);
            if mining > 0 || mining_at_mine > 0 {
                test.pass_phase(elapsed, format!("mining={} at_mine={}", mining, mining_at_mine));
            } else if elapsed > 5.0 {
                test.fail_phase(elapsed, "no Mining activity");
            }
        }
        // Phase 2: Arrives at mine → MiningAtMine (tending)
        2 => {
            test.phase_name = format!("at_mine={} mining={}", mining_at_mine, mining);
            if mining_at_mine > 0 {
                test.pass_phase(elapsed, format!("MiningAtMine (tending)"));
            } else if elapsed > 10.0 {
                test.fail_phase(elapsed, format!("mining={} at_mine=0", mining));
            }
        }
        // Phase 3: Mine becomes Ready → miner harvests → Returning with gold
        3 => {
            test.phase_name = format!("returning={} gold={} at_mine={}", returning, gold, mining_at_mine);
            if returning > 0 {
                test.pass_phase(elapsed, format!("Returning with gold"));
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
                for mut e in energy_query.iter_mut() { e.0 = 35.0; }
            } else if elapsed > 20.0 {
                test.fail_phase(elapsed, format!("gold=0 returning={}", returning));
            }
        }
        // Phase 5: Energy drains → rests → wakes up
        5 => {
            test.phase_name = format!("e={:.0} resting={} going_rest={}", energy, resting, going_rest);
            if test.get_flag("was_resting") && resting == 0 && energy >= ENERGY_WAKE_THRESHOLD {
                test.pass_phase(elapsed, format!("Woke up (energy={:.0})", energy));
                test.complete(elapsed);
            } else {
                if resting > 0 || going_rest > 0 { test.set_flag("was_resting", true); }
                if elapsed > 30.0 {
                    test.fail_phase(elapsed, format!("energy={:.0} resting={}", energy, resting));
                }
            }
        }
        _ => {}
    }
}
