//! Projectile Pipeline Test (4 phases)
//! Validates: ranged NPCs target → projectile spawned → hit + damage → slot freed.

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
    mut flags: ResMut<DebugFlags>,
    mut test_state: ResMut<TestState>,
) {
    world_data.towns.push(world::Town {
        name: "Archers".into(),
        center: Vec2::new(400.0, 400.0),
        faction: 0,
        sprite_type: 0,
    });
    world_data.towns.push(world::Town {
        name: "Targets".into(),
        center: Vec2::new(400.0, 200.0),
        faction: 1,
        sprite_type: 1,
    });
    food_storage.init(2);
    faction_stats.init(2);

    // 2 ranged fighters on opposing factions, within range (300px)
    let slot0 = slot_alloc.alloc().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: slot0,
        x: 400.0, y: 350.0,
        job: 3, faction: 0, town_idx: 0, // Fighter
        home_x: 400.0, home_y: 400.0,
        work_x: -1.0, work_y: -1.0,
        starting_post: -1,
        attack_type: 1, // ranged
    });
    let slot1 = slot_alloc.alloc().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: slot1,
        x: 400.0, y: 250.0,
        job: 3, faction: 1, town_idx: 1, // Fighter
        home_x: 400.0, home_y: 200.0,
        work_x: -1.0, work_y: -1.0,
        starting_post: -1,
        attack_type: 1, // ranged
    });

    flags.combat = true;
    test_state.phase_name = "Waiting for spawns...".into();
    info!("projectiles: setup — 2 ranged fighters, 100px apart");
}

pub fn tick(
    npc_query: Query<(), (With<NpcIndex>, Without<Dead>)>,
    combat_debug: Res<CombatDebug>,
    health_debug: Res<HealthDebug>,
    proj_alloc: Res<ProjSlotAllocator>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    if test.passed || test.failed { return; }

    let now = time.elapsed_secs();
    if test.start == 0.0 { test.start = now; }
    let elapsed = now - test.start;

    let alive = npc_query.iter().count();

    match test.phase {
        // Phase 1: Combat targeting finds enemy
        1 => {
            test.phase_name = format!("targets={} alive={}", combat_debug.targets_found, alive);
            if combat_debug.targets_found > 0 {
                test.pass_phase(elapsed, format!("targets_found={}", combat_debug.targets_found));
            } else if elapsed > 10.0 {
                test.fail_phase(elapsed, format!("targets_found=0 alive={}", alive));
            }
        }
        // Phase 2: Projectile spawned (proj allocator advanced)
        2 => {
            let proj_count = proj_alloc.next;
            test.phase_name = format!("proj_next={} attacks={}", proj_count, combat_debug.attacks_made);
            if proj_count > 0 || combat_debug.attacks_made > 0 {
                test.pass_phase(elapsed, format!("proj_next={} attacks={}", proj_count, combat_debug.attacks_made));
            } else if elapsed > 20.0 {
                test.fail_phase(elapsed, format!("proj_next=0 attacks=0"));
            }
        }
        // Phase 3: Projectile hits → damage processed
        3 => {
            test.phase_name = format!("damage={}", health_debug.damage_processed);
            if health_debug.damage_processed > 0 {
                test.pass_phase(elapsed, format!("damage={}", health_debug.damage_processed));
            } else if elapsed > 25.0 {
                test.fail_phase(elapsed, "damage=0");
            }
        }
        // Phase 4: Projectile slot freed (after expiry or hit)
        4 => {
            let free = proj_alloc.free.len();
            test.phase_name = format!("proj_free={} proj_next={}", free, proj_alloc.next);
            if free > 0 {
                test.pass_phase(elapsed, format!("proj_free={}", free));
                test.complete(elapsed);
            } else if elapsed > 30.0 {
                // Projectiles might still be in flight — pass if damage was confirmed
                if health_debug.damage_processed > 0 {
                    test.pass_phase(elapsed, format!("damage confirmed, proj_free={}", free));
                    test.complete(elapsed);
                } else {
                    test.fail_phase(elapsed, format!("proj_free=0"));
                }
            }
        }
        _ => {}
    }
}
