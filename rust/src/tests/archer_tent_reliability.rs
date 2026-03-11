//! Archer vs Tent Reliability Test (5 phases)
//! Validates: archer acquires enemy tent, shoots projectiles, and tent HP drops consistently.

use bevy::prelude::*;

use crate::components::{Building, Dead, GpuSlot, Health, Job};
use crate::render::MainCamera;
use crate::resources::*;
use crate::world::{self, BuildingKind, WorldCell};

use super::TestState;

const TENT_POS: Vec2 = Vec2::new(576.0, 320.0);
const ARCHER_HOME_A: Vec2 = Vec2::new(384.0, 320.0);
const ARCHER_HOME_B: Vec2 = Vec2::new(448.0, 320.0);

pub fn setup(
    mut slot_alloc: ResMut<GpuSlotPool>,
    mut world_data: ResMut<world::WorldData>,
    mut entity_map: ResMut<EntityMap>,
    mut world_grid: ResMut<world::WorldGrid>,
    mut faction_stats: ResMut<FactionStats>,
    mut test_state: ResMut<TestState>,
    mut camera_query: Query<&mut Transform, With<MainCamera>>,
    mut commands: Commands,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
) {
    // Ensure world/grid visuals are initialized so building + extras atlases
    // are composited and sampled correctly in this test scene.
    world_grid.width = 40;
    world_grid.height = 30;
    world_grid.cell_size = crate::constants::TOWN_GRID_SPACING;
    world_grid.cells = vec![WorldCell::default(); world_grid.width * world_grid.height];

    world_data.towns.push(world::Town {
        name: "Archers".into(),
        center: Vec2::new(384.0, 320.0),
        faction: 1,
        kind: crate::constants::TownKind::Player,
    area_level: 0,
    });
    world_data.towns.push(world::Town {
        name: "TentTown".into(),
        center: Vec2::new(576.0, 320.0),
        faction: 2,
        kind: crate::constants::TownKind::AiRaider,
    area_level: 0,
    });
    faction_stats.init(3);

    let _home_a = world::place_building(
        &mut slot_alloc, &mut entity_map, &mut commands, &mut gpu_updates,
        BuildingKind::ArcherHome, ARCHER_HOME_A, 0, 0, &Default::default(), None, None,
    )
    .expect("archer home A slot alloc");
    let _home_b = world::place_building(
        &mut slot_alloc, &mut entity_map, &mut commands, &mut gpu_updates,
        BuildingKind::ArcherHome, ARCHER_HOME_B, 0, 0, &Default::default(), None, None,
    )
    .expect("archer home B slot alloc");

    let tent_slot = world::place_building(
        &mut slot_alloc, &mut entity_map, &mut commands, &mut gpu_updates,
        BuildingKind::Tent, TENT_POS, 1, 1, &Default::default(), None, None,
    )
    .expect("tent slot alloc");
    test_state
        .counters
        .insert("tent_slot".into(), tent_slot as u32);

    if let Ok(mut cam) = camera_query.single_mut() {
        cam.translation.x = 512.0;
        cam.translation.y = 320.0;
    }

    test_state.phase_name = "Waiting for tent target lock...".into();
    test_state.counters.insert("last_hp_x10".into(), 0);
    test_state.counters.insert("first_damage_tenths".into(), 0);
    info!(
        "archer-tent-reliability: setup homes@({:.0},{:.0})+({:.0},{:.0}) tent@({:.0},{:.0}) slot={}",
        ARCHER_HOME_A.x,
        ARCHER_HOME_A.y,
        ARCHER_HOME_B.x,
        ARCHER_HOME_B.y,
        TENT_POS.x,
        TENT_POS.y,
        tent_slot
    );
}

pub fn tick(
    entity_map: Res<EntityMap>,
    combat_debug: Res<CombatDebug>,
    proj_alloc: Res<ProjSlotAllocator>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    building_q: Query<(&GpuSlot, &Building, &Health), Without<Dead>>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    let Some(&tent_slot_u32) = test.counters.get("tent_slot") else {
        test.fail_phase(elapsed, "missing tent slot in test state");
        return;
    };
    let tent_slot = tent_slot_u32 as usize;

    let tent_hp = building_q
        .iter()
        .find(|(slot, b, _)| slot.0 == tent_slot && b.kind == BuildingKind::Tent)
        .map(|(_, _, h)| h.0)
        .unwrap_or(-1.0);
    let max_hp = crate::constants::building_def(BuildingKind::Tent).hp;
    if tent_hp < 0.0 {
        test.fail_phase(
            elapsed,
            format!("tent entity missing for slot {}", tent_slot),
        );
        return;
    }
    let hp_x10 = (tent_hp * 10.0).round() as u32;
    let last_hp_x10 = test.counters.get("last_hp_x10").copied().unwrap_or(hp_x10);
    if last_hp_x10 == 0 {
        test.counters.insert("last_hp_x10".into(), hp_x10);
    } else if hp_x10 < last_hp_x10 {
        test.inc("hp_drop_events");
        test.counters.insert("last_hp_x10".into(), hp_x10);
    }

    if tent_hp < max_hp
        && test
            .counters
            .get("first_damage_tenths")
            .copied()
            .unwrap_or(0)
            == 0
    {
        test.counters.insert(
            "first_damage_tenths".into(),
            (elapsed * 10.0).round() as u32,
        );
    }

    let archer_count = entity_map
        .iter_npcs()
        .filter(|n| !n.dead && n.job == Job::Archer && n.faction == crate::constants::FACTION_PLAYER)
        .count();

    match test.phase {
        // Phase 1: archer spawners produced units and targeting started
        1 => {
            test.phase_name = format!(
                "archers={} targets_found={}",
                archer_count, combat_debug.targets_found
            );
            if archer_count >= 2 && combat_debug.targets_found > 0 {
                test.pass_phase(
                    elapsed,
                    format!(
                        "archers={} targets_found={}",
                        archer_count, combat_debug.targets_found
                    ),
                );
            } else if elapsed > 80.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "archers={} targets_found={}",
                        archer_count, combat_debug.targets_found
                    ),
                );
            }
        }
        // Phase 2: projectile activity observed
        2 => {
            test.phase_name = format!(
                "proj_next={} attacks={}",
                proj_alloc.next, combat_debug.attacks_made
            );
            if proj_alloc.next > 0 || combat_debug.attacks_made > 0 {
                test.pass_phase(
                    elapsed,
                    format!(
                        "proj_next={} attacks={}",
                        proj_alloc.next, combat_debug.attacks_made
                    ),
                );
            } else if elapsed > 20.0 {
                test.fail_phase(elapsed, "no projectile activity");
            }
        }
        // Phase 3: first tent damage observed
        3 => {
            test.phase_name = format!("tent_hp={:.1}/{:.1}", tent_hp, max_hp);
            if tent_hp < max_hp {
                test.pass_phase(
                    elapsed,
                    format!("tent damaged {:.1}->{:.1}", max_hp, tent_hp),
                );
            } else if elapsed > 25.0 {
                test.fail_phase(elapsed, "tent never took damage");
            }
        }
        // Phase 4: continued damage (not a one-off graze)
        4 => {
            let drops = test.count("hp_drop_events");
            test.phase_name = format!("tent_hp={:.1}/{:.1} hp_drops={}", tent_hp, max_hp, drops);
            if tent_hp <= max_hp - 25.0 || drops >= 2 {
                test.pass_phase(
                    elapsed,
                    format!("sustained damage tent_hp={:.1} drops={}", tent_hp, drops),
                );
            } else if elapsed > 35.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "damage too inconsistent tent_hp={:.1} drops={}",
                        tent_hp, drops
                    ),
                );
            }
        }
        // Phase 5: eventual kill expected in this lane setup
        5 => {
            test.phase_name = format!("tent_hp={:.1}", tent_hp);
            if tent_hp <= 0.0 {
                test.pass_phase(elapsed, "tent destroyed");
                test.complete(elapsed);
            } else if elapsed > 50.0 {
                test.fail_phase(elapsed, format!("tent survived too long hp={:.1}", tent_hp));
            }
        }
        _ => {}
    }
}
