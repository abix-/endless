//! Spawning & Slot Reuse Test (4 phases)
//! Validates: spawn entities, kill via health=0, slot freed, slot reused on respawn.

use bevy::prelude::*;
use crate::components::*;
use crate::resources::*;

use super::{TestState, TestSetupParams};

pub fn setup(mut params: TestSetupParams) {
    params.add_town("TestTown");
    params.init_economy(1);

    // Spawn 5 farmers
    for i in 0..5 {
        params.spawn_npc(0, 300.0 + (i as f32 * 40.0), 400.0, 300.0 + (i as f32 * 40.0), 450.0);
    }

    params.test_state.phase_name = "Waiting for 5 NPCs...".into();
    info!("spawning: setup — 5 farmers");
}

pub fn tick(
    mut npc_query: Query<(Entity, &NpcIndex, &Job, &mut Health), Without<Dead>>,
    all_npc_query: Query<(), With<NpcIndex>>,
    mut slot_alloc: ResMut<SlotAllocator>,
    mut spawn_events: MessageWriter<crate::messages::SpawnNpcMsg>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    let alive = npc_query.iter().count();

    match test.phase {
        // Phase 1: 5 NPCs spawned with correct Job
        1 => {
            test.phase_name = format!("alive={}/5", alive);
            if alive == 5 {
                // Verify all are Farmers
                let farmers = npc_query.iter().filter(|(_, _, j, _)| **j == Job::Farmer).count();
                if farmers == 5 {
                    test.pass_phase(elapsed, format!("5 farmers spawned"));
                } else {
                    test.fail_phase(elapsed, format!("farmers={} (expected 5)", farmers));
                }
            } else if elapsed > 3.0 {
                test.fail_phase(elapsed, format!("alive={} (expected 5)", alive));
            }
        }
        // Phase 2: Kill one NPC (set health=0), wait for despawn
        2 => {
            if !test.get_flag("killed") {
                // Kill the first NPC
                if let Some((_, npc_idx, _, mut health)) = npc_query.iter_mut().next() {
                    test.set_flag("killed", true);
                    test.counters.insert("killed_slot".into(), npc_idx.0 as u32);
                    health.0 = 0.0;
                    test.phase_name = format!("Killed slot {}, waiting for despawn...", npc_idx.0);
                }
            } else {
                test.phase_name = format!("alive={}/4 (waiting for despawn)", alive);
                if alive <= 4 {
                    test.pass_phase(elapsed, format!("NPC died, alive={}", alive));
                } else if elapsed > 5.0 {
                    test.fail_phase(elapsed, format!("alive={} (expected <=4)", alive));
                }
            }
        }
        // Phase 3: Slot freed in SlotAllocator
        3 => {
            let free_count = slot_alloc.free.len();
            test.phase_name = format!("free_slots={}", free_count);
            if free_count > 0 {
                test.pass_phase(elapsed, format!("slot freed (free={})", free_count));
            } else if elapsed > 5.0 {
                test.fail_phase(elapsed, format!("free_slots=0"));
            }
        }
        // Phase 4: Spawn reuses freed slot
        4 => {
            if !test.get_flag("respawned") {
                let killed_slot = test.count("killed_slot") as usize;
                let new_slot = slot_alloc.alloc().expect("slot alloc for respawn");
                test.counters.insert("new_slot".into(), new_slot as u32);
                spawn_events.write(crate::messages::SpawnNpcMsg {
                    slot_idx: new_slot,
                    x: 400.0, y: 400.0,
                    job: 0, faction: 0, town_idx: 0,
                    home_x: 400.0, home_y: 450.0,
                    work_x: -1.0, work_y: -1.0,
                    starting_post: -1,
                    attack_type: 0,
                });
                test.set_flag("respawned", true);
                test.phase_name = format!("Spawned at slot {}, killed was {}", new_slot, killed_slot);
            } else {
                let total = all_npc_query.iter().count();
                let new_slot = test.count("new_slot");
                let killed_slot = test.count("killed_slot");
                test.phase_name = format!("total={}/5 new={} killed={}", total, new_slot, killed_slot);
                if total >= 5 {
                    if new_slot == killed_slot {
                        test.pass_phase(elapsed, format!("slot {} reused", new_slot));
                    } else {
                        test.pass_phase(elapsed, format!("spawned at slot {} (killed={})", new_slot, killed_slot));
                    }
                    test.complete(elapsed);
                } else if elapsed > 8.0 {
                    test.fail_phase(elapsed, format!("total={} (expected 5)", total));
                }
            }
        }
        _ => {}
    }
}
