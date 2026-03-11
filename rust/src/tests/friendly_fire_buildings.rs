//! Friendly Fire Building Test (4 phases)
//! Validates: single ranged shooter can damage enemy NPCs without damaging same-faction buildings.

use bevy::prelude::*;

use crate::constants::building_def;
use crate::messages::{GpuUpdate, GpuUpdateMsg, SpawnNpcMsg};
use crate::render::MainCamera;
use crate::resources::*;
use crate::world::BuildingKind;
use crate::world::{self, WorldCell};

use super::TestState;

const FARM_WALL_X: f32 = 512.0;
const FARM_WALL_Y: [f32; 7] = [
    192.0, 256.0, 320.0, 384.0, 448.0, 512.0, 576.0,
];
const TARGET_X: f32 = 576.0;
const TARGET_Y: f32 = 320.0;

pub fn setup(
    mut slot_alloc: ResMut<GpuSlotPool>,
    mut spawn_events: MessageWriter<SpawnNpcMsg>,
    mut world_data: ResMut<world::WorldData>,
    mut entity_map: ResMut<EntityMap>,
    mut world_grid: ResMut<world::WorldGrid>,
    mut faction_stats: ResMut<FactionStats>,
    mut test_state: ResMut<TestState>,
    mut camera_query: Query<&mut Transform, With<MainCamera>>,
    mut commands: Commands,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
) {
    // Grid must exist so building spatial grid rebuild runs in normal systems.
    world_grid.width = 40;
    world_grid.height = 30;
    world_grid.cell_size = crate::constants::TOWN_GRID_SPACING;
    world_grid.cells = vec![WorldCell::default(); world_grid.width * world_grid.height];

    world_data.towns.push(world::Town {
        name: "Blue".into(),
        center: Vec2::new(384.0, 384.0),
        faction: 1,
        kind: crate::constants::TownKind::Player,
    });
    world_data.towns.push(world::Town {
        name: "Red".into(),
        center: Vec2::new(768.0, 320.0),
        faction: 2,
        kind: crate::constants::TownKind::AiRaider,
    });
    faction_stats.init(3);

    // Friendly vertical farm wall in projectile lane.
    for y in FARM_WALL_Y {
        let pos = Vec2::new(FARM_WALL_X, y);
        let _ = world::place_building(
            &mut slot_alloc, &mut entity_map, &mut commands, &mut gpu_updates,
            world::BuildingKind::Farm, pos, 0, 0, &Default::default(), None, None,
        );
    }

    // Shooter (faction 1, ranged).
    let shooter = slot_alloc.alloc_reset().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: shooter,
        x: 448.0,
        y: 320.0,
        job: 3, // fighter
        faction: 1,
        town_idx: 0,
        home_x: 384.0,
        home_y: 384.0,
        work_x: -1.0,
        work_y: -1.0,
        starting_post: -1,
        entity_override: None,
    });

    // Target dummy (faction 2, melee) so only one side shoots.
    let target = slot_alloc.alloc_reset().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: target,
        x: TARGET_X,
        y: TARGET_Y,
        job: 0, // farmer (not dedicated ranged combat)
        faction: 2,
        town_idx: 1,
        home_x: 768.0,
        home_y: 320.0,
        work_x: -1.0,
        work_y: -1.0,
        starting_post: -1,
        entity_override: None,
    });

    if let Ok(mut cam) = camera_query.single_mut() {
        // Center on the shooter/farm/target lane so test behavior is visible immediately.
        cam.translation.x = 512.0;
        cam.translation.y = 320.0;
    }

    test_state.phase_name = "Waiting for shooter target lock...".into();
    test_state.set_flag("damage_seen", false);
    info!(
        "friendly-fire-buildings: setup complete shooter->target through {}-farm wall at x={:.0}",
        FARM_WALL_Y.len(),
        FARM_WALL_X
    );
}

pub fn tick(
    entity_map: Res<EntityMap>,
    combat_debug: Res<CombatDebug>,
    health_debug: Res<HealthDebug>,
    proj_alloc: Res<ProjSlotAllocator>,
    building_query: Query<
        (&crate::components::Building, &crate::components::Health),
        Without<crate::components::Dead>,
    >,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    // Pin target dummy so it can't flee/chase and invalidate the lane geometry.
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
        idx: 1, // setup alloc order: shooter=0, target=1
        x: TARGET_X,
        y: TARGET_Y,
    }));

    let alive = entity_map.iter_npcs().filter(|n| !n.dead).count();
    let max_farm_hp = building_def(BuildingKind::Farm).hp;
    let farm_entities: Vec<f32> = building_query
        .iter()
        .filter(|(b, _)| b.kind == BuildingKind::Farm)
        .map(|(_, h)| h.0)
        .collect();
    let damaged_farms = farm_entities.iter().filter(|&&hp| hp < max_farm_hp).count();
    let min_farm_hp = farm_entities.iter().copied().fold(max_farm_hp, f32::min);

    match test.phase {
        // Phase 1: target acquired.
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
        // Phase 2: projectile activity observed.
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
        // Phase 3: NPC damage confirms real hits.
        3 => {
            test.phase_name = format!(
                "npc_damage={} damaged_farms={} min_farm_hp={:.1}",
                health_debug.damage_processed, damaged_farms, min_farm_hp
            );
            if damaged_farms > 0 && health_debug.damage_processed == 0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "bug reproduced: {} friendly farms damaged before any npc damage (min_hp {:.1})",
                        damaged_farms, min_farm_hp
                    ),
                );
                return;
            }
            if health_debug.damage_processed > 0 {
                test.set_flag("damage_seen", true);
                test.pass_phase(
                    elapsed,
                    format!("npc_damage={}", health_debug.damage_processed),
                );
            } else if elapsed > 30.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "no npc damage observed (damaged_farms={}, min_farm_hp={:.1}, proj_next={}, attacks={})",
                        damaged_farms, min_farm_hp, proj_alloc.next, combat_debug.attacks_made
                    ),
                );
            }
        }
        // Phase 4: friendly building HP must remain unchanged.
        4 => {
            test.phase_name = format!(
                "damaged_farms={} min_farm_hp={:.1}/{:.1}",
                damaged_farms,
                min_farm_hp,
                building_def(BuildingKind::Farm).hp
            );
            if damaged_farms > 0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "friendly farm line damaged: {} farms, min_hp {:.1}",
                        damaged_farms, min_farm_hp
                    ),
                );
                return;
            }

            if elapsed > 40.0 && test.get_flag("damage_seen") {
                test.pass_phase(elapsed, "friendly farm line untouched".to_string());
                test.complete(elapsed);
            }
        }
        _ => {}
    }
}
