//! Friendly Fire Building Test (4 phases)
//! Validates: single ranged shooter can damage enemy NPCs without damaging same-faction buildings.

use bevy::prelude::*;

use crate::components::*;
use crate::constants::FARM_HP;
use crate::messages::SpawnNpcMsg;
use crate::resources::*;
use crate::world::{self, WorldCell};

use super::TestState;

const FRIENDLY_FARM_POS: Vec2 = Vec2::new(520.0, 320.0);

pub fn setup(
    mut slot_alloc: ResMut<SlotAllocator>,
    mut spawn_events: MessageWriter<SpawnNpcMsg>,
    mut world_data: ResMut<world::WorldData>,
    mut world_grid: ResMut<world::WorldGrid>,
    mut food_storage: ResMut<FoodStorage>,
    mut faction_stats: ResMut<FactionStats>,
    mut building_hp: ResMut<BuildingHpState>,
    mut test_state: ResMut<TestState>,
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

    // Friendly building in projectile lane.
    world_data.farms.push(world::Farm {
        position: FRIENDLY_FARM_POS,
        town_idx: 0,
    });
    building_hp.farms.push(FARM_HP);

    let (gc, gr) = world_grid.world_to_grid(FRIENDLY_FARM_POS);
    if let Some(cell) = world_grid.cell_mut(gc, gr) {
        cell.building = Some(world::Building::Farm { town_idx: 0 });
    }

    // Shooter (faction 0, ranged).
    let shooter = slot_alloc.alloc().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: shooter,
        x: 360.0,
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
        x: 700.0,
        y: 320.0,
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

    test_state.phase_name = "Waiting for shooter target lock...".into();
    test_state.set_flag("damage_seen", false);
    info!(
        "friendly-fire-buildings: setup complete shooter->target lane through farm at ({:.0},{:.0})",
        FRIENDLY_FARM_POS.x,
        FRIENDLY_FARM_POS.y
    );
}

pub fn tick(
    npc_query: Query<(), (With<NpcIndex>, Without<Dead>)>,
    combat_debug: Res<CombatDebug>,
    health_debug: Res<HealthDebug>,
    proj_alloc: Res<ProjSlotAllocator>,
    building_hp: Res<BuildingHpState>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    let alive = npc_query.iter().count();
    let farm_hp = building_hp.farms.first().copied().unwrap_or(0.0);

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
            test.phase_name = format!("npc_damage={} farm_hp={:.1}", health_debug.damage_processed, farm_hp);
            if health_debug.damage_processed > 0 {
                test.set_flag("damage_seen", true);
                test.pass_phase(elapsed, format!("npc_damage={}", health_debug.damage_processed));
            } else if elapsed > 30.0 {
                test.fail_phase(elapsed, "no npc damage observed");
            }
        }
        // Phase 4: friendly building HP must remain unchanged.
        4 => {
            test.phase_name = format!("farm_hp={:.1}/{:.1}", farm_hp, FARM_HP);
            if farm_hp < FARM_HP {
                test.fail_phase(
                    elapsed,
                    format!("friendly farm damaged: {:.1} -> {:.1}", FARM_HP, farm_hp),
                );
                return;
            }

            if elapsed > 40.0 && test.get_flag("damage_seen") {
                test.pass_phase(elapsed, format!("friendly farm untouched at {:.1} HP", farm_hp));
                test.complete(elapsed);
            }
        }
        _ => {}
    }
}
