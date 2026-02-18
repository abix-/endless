//! Friendly Fire Building Test (4 phases)
//! Validates: single ranged shooter can damage enemy NPCs without damaging same-faction buildings.

use bevy::prelude::*;

use crate::components::*;
use crate::constants::building_def;
use crate::world::BuildingKind;
use crate::messages::{GpuUpdate, GpuUpdateMsg, SpawnNpcMsg};
use crate::render::MainCamera;
use crate::resources::*;
use crate::world::{self, WorldCell};

use super::TestState;

const FARM_WALL_X: f32 = 500.0;
const FARM_WALL_Y: [f32; 10] = [230.0, 250.0, 270.0, 290.0, 310.0, 330.0, 350.0, 370.0, 390.0, 410.0];
const TARGET_X: f32 = 555.0;
const TARGET_Y: f32 = 320.0;

pub fn setup(
    mut slot_alloc: ResMut<SlotAllocator>,
    mut spawn_events: MessageWriter<SpawnNpcMsg>,
    mut world_data: ResMut<world::WorldData>,
    mut world_grid: ResMut<world::WorldGrid>,
    mut food_storage: ResMut<FoodStorage>,
    mut faction_stats: ResMut<FactionStats>,
    mut building_hp: ResMut<BuildingHpState>,
    mut test_state: ResMut<TestState>,
    mut camera_query: Query<&mut Transform, With<MainCamera>>,
) {
    // Grid must exist so building spatial grid rebuild runs in normal systems.
    world_grid.width = 40;
    world_grid.height = 30;
    world_grid.cell_size = 32.0;
    world_grid.cells = vec![WorldCell::default(); world_grid.width * world_grid.height];

    world_data.towns.push(world::Town {
        name: "Blue".into(),
        center: Vec2::new(320.0, 320.0),
        faction: 0,
        sprite_type: 0,
    });
    world_data.towns.push(world::Town {
        name: "Red".into(),
        center: Vec2::new(780.0, 320.0),
        faction: 1,
        sprite_type: 1,
    });
    food_storage.init(2);
    faction_stats.init(2);

    // Friendly vertical farm wall in projectile lane.
    for y in FARM_WALL_Y {
        let pos = Vec2::new(FARM_WALL_X, y);
        world_data.farms.push(world::Farm {
            position: pos,
            town_idx: 0,
        });
        building_hp.farms.push(building_def(BuildingKind::Farm).hp);

        let (gc, gr) = world_grid.world_to_grid(pos);
        if let Some(cell) = world_grid.cell_mut(gc, gr) {
            cell.building = Some(world::Building::Farm { town_idx: 0 });
        }
    }

    // Shooter (faction 0, ranged).
    let shooter = slot_alloc.alloc().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: shooter,
        x: 425.0,
        y: 320.0,
        job: 3, // fighter
        faction: 0,
        town_idx: 0,
        home_x: 320.0,
        home_y: 320.0,
        work_x: -1.0,
        work_y: -1.0,
        starting_post: -1,
        attack_type: 1, // ranged
    });

    // Target dummy (faction 1, melee) so only one side shoots.
    let target = slot_alloc.alloc().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: target,
        x: TARGET_X,
        y: TARGET_Y,
        job: 0, // farmer (not dedicated ranged combat)
        faction: 1,
        town_idx: 1,
        home_x: 780.0,
        home_y: 320.0,
        work_x: -1.0,
        work_y: -1.0,
        starting_post: -1,
        attack_type: 0, // melee
    });

    if let Ok(mut cam) = camera_query.single_mut() {
        // Center on the shooter/farm/target lane so test behavior is visible immediately.
        cam.translation.x = 500.0;
        cam.translation.y = 320.0;
    }

    test_state.phase_name = "Waiting for shooter target lock...".into();
    test_state.set_flag("damage_seen", false);
    info!(
        "friendly-fire-buildings: setup complete shooter->target lane through {}-farm vertical wall at x={:.0}",
        FARM_WALL_Y.len(),
        FARM_WALL_X
    );
}

pub fn tick(
    npc_query: Query<(), (With<NpcIndex>, Without<Dead>)>,
    combat_debug: Res<CombatDebug>,
    health_debug: Res<HealthDebug>,
    proj_alloc: Res<ProjSlotAllocator>,
    building_hp: Res<BuildingHpState>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    // Pin target dummy so it can't flee/chase and invalidate the lane geometry.
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
        idx: 1, // setup alloc order: shooter=0, target=1
        x: TARGET_X,
        y: TARGET_Y,
    }));

    let alive = npc_query.iter().count();
    let damaged_farms = building_hp.farms.iter().filter(|&&hp| hp < building_def(BuildingKind::Farm).hp).count();
    let min_farm_hp = building_hp
        .farms
        .iter()
        .copied()
        .fold(building_def(BuildingKind::Farm).hp, f32::min);

    match test.phase {
        // Phase 1: target acquired.
        1 => {
            test.phase_name = format!("targets={} alive={}", combat_debug.targets_found, alive);
            if combat_debug.targets_found > 0 {
                test.pass_phase(elapsed, format!("targets_found={}", combat_debug.targets_found));
            } else if elapsed > 10.0 {
                test.fail_phase(elapsed, format!("targets_found=0 alive={}", alive));
            }
        }
        // Phase 2: projectile activity observed.
        2 => {
            test.phase_name = format!("proj_next={} attacks={}", proj_alloc.next, combat_debug.attacks_made);
            if proj_alloc.next > 0 || combat_debug.attacks_made > 0 {
                test.pass_phase(
                    elapsed,
                    format!("proj_next={} attacks={}", proj_alloc.next, combat_debug.attacks_made),
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
                test.pass_phase(elapsed, format!("npc_damage={}", health_debug.damage_processed));
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
                damaged_farms, min_farm_hp, building_def(BuildingKind::Farm).hp
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

