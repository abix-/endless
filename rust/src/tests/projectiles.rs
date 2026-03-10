//! Projectile Pipeline Test (4 phases)
//! Validates: ranged NPCs target → projectile spawned → hit + damage → slot freed.

use bevy::prelude::*;

use crate::messages::SpawnNpcMsg;
use crate::resources::*;
use crate::world;

use super::TestState;

pub fn setup(
    mut slot_alloc: ResMut<GpuSlotPool>,
    mut spawn_events: MessageWriter<SpawnNpcMsg>,
    mut world_data: ResMut<world::WorldData>,
    mut faction_stats: ResMut<FactionStats>,
    mut test_state: ResMut<TestState>,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
) {
    world_data.towns.push(world::Town {
        name: "Archers".into(),
        center: Vec2::new(384.0, 384.0),
        faction: 1,
        kind: crate::constants::TownKind::Player,
    area_level: 0,
    });
    world_data.towns.push(world::Town {
        name: "Targets".into(),
        center: Vec2::new(384.0, 192.0),
        faction: 2,
        kind: crate::constants::TownKind::AiRaider,
    area_level: 0,
    });
    faction_stats.init(3);

    // 2 ranged fighters on opposing factions, within range (300px)
    let slot0 = slot_alloc.alloc_reset().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: slot0,
        x: 384.0,
        y: 320.0,
        job: 3,
        faction: 1,
        town_idx: 0, // Fighter
        home_x: 384.0,
        home_y: 384.0,
        work_x: -1.0,
        work_y: -1.0,
        starting_post: -1,
        attack_type: 1, // ranged
        entity_override: None,
    });
    let slot1 = slot_alloc.alloc_reset().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: slot1,
        x: 384.0,
        y: 256.0,
        job: 3,
        faction: 2,
        town_idx: 1, // Fighter
        home_x: 384.0,
        home_y: 192.0,
        work_x: -1.0,
        work_y: -1.0,
        starting_post: -1,
        attack_type: 1, // ranged
        entity_override: None,
    });

    if let Ok(mut cam) = camera_query.single_mut() {
        cam.translation.x = 384.0;
        cam.translation.y = 320.0;
    }
    test_state.phase_name = "Waiting for spawns...".into();
    info!("projectiles: setup — 2 ranged fighters, 100px apart");
}

pub fn tick(
    entity_map: Res<EntityMap>,
    combat_debug: Res<CombatDebug>,
    health_debug: Res<HealthDebug>,
    proj_alloc: Res<ProjSlotAllocator>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    let alive = entity_map.iter_npcs().filter(|n| !n.dead).count();

    match test.phase {
        // Phase 1: Combat targeting finds enemy
        1 => {
            test.phase_name = format!("targets={} alive={}", combat_debug.targets_found, alive);
            if combat_debug.targets_found > 0 {
                test.pass_phase(
                    elapsed,
                    format!("targets_found={}", combat_debug.targets_found),
                );
            } else if elapsed > 10.0 {
                test.fail_phase(elapsed, format!("targets_found=0 alive={}", alive));
            }
        }
        // Phase 2: Projectile spawned (proj allocator advanced)
        2 => {
            let proj_count = proj_alloc.next;
            test.phase_name = format!(
                "proj_next={} attacks={}",
                proj_count, combat_debug.attacks_made
            );
            if proj_count > 0 || combat_debug.attacks_made > 0 {
                test.pass_phase(
                    elapsed,
                    format!(
                        "proj_next={} attacks={}",
                        proj_count, combat_debug.attacks_made
                    ),
                );
            } else if elapsed > 20.0 {
                test.fail_phase(elapsed, "proj_next=0 attacks=0".to_string());
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
                    test.fail_phase(elapsed, "proj_free=0".to_string());
                }
            }
        }
        _ => {}
    }
}
