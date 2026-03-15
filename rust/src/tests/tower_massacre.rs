//! Tower Massacre stress test: 25K towers vs 50K raiders.
//! Measures per-frame death processing cost during realistic mass combat.
//! Validates frame-capped death queue under extreme load.

use bevy::prelude::*;

use crate::components::{
    FoodStore, GoldStore, StoneStore, TownAreaLevel, TownEquipment, TownMarker, TownPolicy,
    TownUpgradeLevel, WoodStore,
};
use crate::messages::SpawnNpcMsg;
use crate::resources::*;
use crate::world;

use super::TestState;

const NUM_TOWERS: usize = 25_000;
const NUM_RAIDERS: usize = 50_000;
const TOWER_SPACING: f32 = 128.0;
const GRID_COLS: usize = 200;

pub fn setup(
    mut slot_alloc: ResMut<GpuSlotPool>,
    mut spawn_events: MessageWriter<SpawnNpcMsg>,
    mut world_data: ResMut<world::WorldData>,
    mut entity_map: ResMut<crate::entity_map::EntityMap>,
    mut faction_stats: ResMut<FactionStats>,
    mut test_state: ResMut<TestState>,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut commands: Commands,
    mut town_index: ResMut<TownIndex>,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
    mut world_grid: ResMut<world::WorldGrid>,
) {
    let center = Vec2::new(16000.0, 16000.0);

    // Initialize grid
    let grid_size = 500;
    world_grid.width = grid_size;
    world_grid.height = grid_size;
    world_grid.cell_size = 64.0;
    world_grid.cells = vec![world::WorldCell::default(); grid_size * grid_size];
    world_grid.init_pathfind_costs();
    world_grid.init_town_buildable();

    // Player town (towers)
    world_data.towns.push(world::Town {
        name: "Tower Town".into(),
        center,
        faction: 1,
        kind: crate::constants::TownKind::Player,
    });
    // Raider town
    world_data.towns.push(world::Town {
        name: "Raider Horde".into(),
        center: center + Vec2::new(0.0, 1000.0),
        faction: 2,
        kind: crate::constants::TownKind::AiRaider,
    });
    faction_stats.init(3);

    // Spawn town entities
    for (i, _) in world_data.towns.iter().enumerate() {
        let entity = commands
            .spawn((
                TownMarker,
                TownAreaLevel(0),
                FoodStore(100_000),
                GoldStore(100_000),
                WoodStore(0),
                StoneStore(0),
                TownPolicy(PolicySet::default()),
                TownUpgradeLevel::default(),
                TownEquipment::default(),
            ))
            .id();
        town_index.0.insert(i as i32, entity);
    }

    // Place towers in a grid around town center
    let mut towers_placed = 0;
    for i in 0..NUM_TOWERS {
        let col = i % GRID_COLS;
        let row = i / GRID_COLS;
        let pos = Vec2::new(
            center.x - (GRID_COLS as f32 * TOWER_SPACING / 2.0) + col as f32 * TOWER_SPACING,
            center.y - 500.0 + row as f32 * TOWER_SPACING,
        );
        // Mark cells as buildable for this town
        let (gc, gr) = world_grid.world_to_grid(pos);
        world_grid.add_town_buildable(gc, gr, 0);
        if world::place_building(
            &mut slot_alloc,
            &mut entity_map,
            &mut commands,
            &mut gpu_updates,
            world::BuildingKind::Tower,
            pos,
            0, // town_idx
            1, // faction
            &world::BuildingOverrides::default(),
            None, // no BuildContext (free placement)
            None, // no dirty writers
        )
        .is_ok()
        {
            towers_placed += 1;
        }
    }

    // Spawn raiders in tower range
    let mut raiders_spawned = 0;
    for i in 0..NUM_RAIDERS {
        let Some(slot) = slot_alloc.alloc_reset() else {
            break;
        };
        let col = i % 250;
        let row = i / 250;
        let x = center.x - 8000.0 + col as f32 * 64.0;
        let y = center.y - 8000.0 + row as f32 * 64.0;
        spawn_events.write(SpawnNpcMsg {
            slot_idx: slot,
            x,
            y,
            job: 2, // Raider
            faction: 2,
            town_idx: 1,
            home_x: center.x,
            home_y: center.y + 1000.0,
            work_x: -1.0,
            work_y: -1.0,
            starting_post: -1,
            entity_override: None,
        });
        raiders_spawned += 1;
    }

    if let Ok(mut cam) = camera_query.single_mut() {
        cam.translation.x = center.x;
        cam.translation.y = center.y;
    }

    test_state.phase_name = format!(
        "Setup: {} towers, {} raiders",
        towers_placed, raiders_spawned
    );
    info!(
        "tower-massacre: placed {} towers, spawning {} raiders",
        towers_placed, raiders_spawned
    );
}

pub fn tick(
    time: Res<Time>,
    entity_map: Res<crate::entity_map::EntityMap>,
    mut test: ResMut<TestState>,
    death_queue: Res<crate::systems::DeathQueue>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    // Count alive raiders (faction 2)
    let alive_raiders = entity_map
        .iter_npcs()
        .filter(|npc| npc.faction == 2 && !npc.dead)
        .count();
    let dead_raiders = entity_map
        .iter_npcs()
        .filter(|npc| npc.faction == 2 && npc.dead)
        .count();
    let queue_pending = death_queue.pending.len();

    if test.phase == 1 {
        // Wait for spawning to complete
        let total = entity_map
            .iter_npcs()
            .filter(|npc| npc.faction == 2)
            .count();
        if total >= NUM_RAIDERS / 2 {
            test.pass_phase(
                elapsed,
                format!("{} raiders spawned, combat starting", total),
            );
        }
    } else if test.phase == 2 {
        // Combat phase -- track deaths
        test.phase_name = format!(
            "Combat: {} alive, {} dead, {} queued, {:.1}s",
            alive_raiders, dead_raiders, queue_pending, elapsed
        );

        if alive_raiders == 0 && queue_pending == 0 {
            test.pass_phase(
                elapsed,
                format!("All {} raiders killed in {:.1}s", dead_raiders, elapsed),
            );
            test.complete(elapsed);
        }

        // Timeout after 120 seconds
        if elapsed > 120.0 {
            test.fail_phase(
                elapsed,
                format!(
                    "Timeout: {} alive, {} dead, {} queued after {:.0}s",
                    alive_raiders, dead_raiders, queue_pending, elapsed
                ),
            );
        }
    }
}
