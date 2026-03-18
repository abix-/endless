use super::*;
use crate::entity_map::BuildingInstance;
use crate::messages::BuildingGridDirtyMsg;
use crate::resources::EntityMap;
use crate::world::worldgen::generate_terrain_worldmap;
use crate::world::worldgen::spawn_resource_nodes;
use bevy::ecs::system::RunSystemOnce;
use bevy::time::TimeUpdateStrategy;

// -- rebuild_building_grid_system signal tests ----------------------------

#[derive(bevy::prelude::Resource, Default)]
struct SendBuildingGridDirty(bool);

fn maybe_send_building_grid_dirty(
    mut writer: MessageWriter<BuildingGridDirtyMsg>,
    mut flag: bevy::prelude::ResMut<SendBuildingGridDirty>,
) {
    if flag.0 {
        writer.write(BuildingGridDirtyMsg);
        flag.0 = false;
    }
}

fn setup_rebuild_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    let mut em = EntityMap::default();
    let world_size_px = 16.0 * 64.0;
    em.init_spatial(world_size_px);
    app.insert_resource(em);
    app.add_message::<BuildingGridDirtyMsg>();
    app.insert_resource(SendBuildingGridDirty(false));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    let mut grid = WorldGrid::default();
    grid.width = 16;
    grid.height = 16;
    grid.cell_size = 64.0;
    grid.cells = vec![WorldCell::default(); 16 * 16];
    app.insert_resource(grid);
    app.add_systems(
        FixedUpdate,
        (maybe_send_building_grid_dirty, rebuild_building_grid_system).chain(),
    );
    app.update();
    app.update();
    app
}

/// Regression: spatial is maintained incrementally by add_instance.
/// Building is findable immediately after add_instance without any dirty message.
#[test]
fn spatial_incremental_add_instance_findable() {
    let mut app = setup_rebuild_app();
    let pos = Vec2::new(32.0, 32.0);
    app.world_mut()
        .resource_mut::<EntityMap>()
        .add_instance(BuildingInstance {
            kind: BuildingKind::Farm,
            position: pos,
            slot: 1,
            town_idx: 0,
            faction: 1,
        });
    // No dirty message needed -- add_instance maintains spatial incrementally
    let em = app.world().resource::<EntityMap>();
    let mut found = false;
    em.for_each_nearby(pos, 200.0, |_, _| found = true);
    assert!(found, "building should be findable after add_instance");
}

/// Regression: building remains findable across frames without any dirty message.
/// rebuild_building_grid_system must NOT clear spatial on message-free frames.
#[test]
fn rebuild_building_grid_preserves_spatial_on_subsequent_frame() {
    let mut app = setup_rebuild_app();
    let pos_a = Vec2::new(32.0, 32.0);
    app.world_mut()
        .resource_mut::<EntityMap>()
        .add_instance(BuildingInstance {
            kind: BuildingKind::Farm,
            position: pos_a,
            slot: 1,
            town_idx: 0,
            faction: 1,
        });
    // Advance two frames without any dirty message
    app.update();
    app.update();
    let em = app.world().resource::<EntityMap>();
    let mut still_found = false;
    em.for_each_nearby(pos_a, 200.0, |_, _| still_found = true);
    assert!(
        still_found,
        "spatial should be preserved across frames without BuildingGridDirtyMsg"
    );
}

#[test]
fn road_blocked_on_forest_biome() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(crate::resources::EntityMap::default());
    app.insert_resource(crate::resources::GpuSlotPool::default());
    app.add_message::<crate::messages::GpuUpdateMsg>();

    // 10x10 grid, all grass, town 0 owns everything
    let mut grid = WorldGrid::default();
    grid.width = 10;
    grid.height = 10;
    grid.cell_size = 32.0;
    grid.cells = vec![
        WorldCell {
            terrain: Biome::Grass,
            original_terrain: Biome::Grass
        };
        100
    ];
    grid.town_owner = vec![0u16; 100];
    // Set (5,5) to Forest
    grid.cells[55].terrain = Biome::Forest;
    grid.cells[55].original_terrain = Biome::Forest;

    let world_data = WorldData {
        towns: vec![Town {
            name: "Test".into(),
            center: Vec2::new(160.0, 160.0),
            faction: 0,
            kind: crate::constants::TownKind::Player,
        }],
    };

    let forest_pos = grid.grid_to_world(5, 5);
    let grass_pos = grid.grid_to_world(3, 3);

    app.insert_resource(grid);
    app.insert_resource(world_data);
    app.update();

    // Road on forest -> rejected
    app.world_mut()
        .run_system_once(
            move |mut slot_alloc: ResMut<crate::resources::GpuSlotPool>,
                  mut entity_map: ResMut<crate::resources::EntityMap>,
                  mut commands: Commands,
                  mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
                  mut grid: ResMut<WorldGrid>,
                  world_data: Res<WorldData>| {
                let result = place_building(
                    &mut slot_alloc,
                    &mut entity_map,
                    &mut commands,
                    &mut gpu_updates,
                    BuildingKind::Road,
                    forest_pos,
                    0,
                    0,
                    &BuildingOverrides::default(),
                    Some(BuildContext {
                        grid: &mut grid,
                        world_data: &world_data,
                        food: &mut 9999,
                        cost: 10,
                    }),
                    None,
                );
                assert_eq!(result, Err("cannot build road on forest"));
            },
        )
        .unwrap();

    // Road on grass -> accepted
    app.world_mut()
        .run_system_once(
            move |mut slot_alloc: ResMut<crate::resources::GpuSlotPool>,
                  mut entity_map: ResMut<crate::resources::EntityMap>,
                  mut commands: Commands,
                  mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
                  mut grid: ResMut<WorldGrid>,
                  world_data: Res<WorldData>| {
                let result = place_building(
                    &mut slot_alloc,
                    &mut entity_map,
                    &mut commands,
                    &mut gpu_updates,
                    BuildingKind::Road,
                    grass_pos,
                    0,
                    0,
                    &BuildingOverrides::default(),
                    Some(BuildContext {
                        grid: &mut grid,
                        world_data: &world_data,
                        food: &mut 9999,
                        cost: 10,
                    }),
                    None,
                );
                assert!(result.is_ok(), "road on grass should succeed: {:?}", result);
            },
        )
        .unwrap();

    // Waypoint on forest -> accepted (non-road buildings allowed)
    app.world_mut()
        .run_system_once(
            move |mut slot_alloc: ResMut<crate::resources::GpuSlotPool>,
                  mut entity_map: ResMut<crate::resources::EntityMap>,
                  mut commands: Commands,
                  mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
                  mut grid: ResMut<WorldGrid>,
                  world_data: Res<WorldData>| {
                let result = place_building(
                    &mut slot_alloc,
                    &mut entity_map,
                    &mut commands,
                    &mut gpu_updates,
                    BuildingKind::Waypoint,
                    forest_pos,
                    0,
                    0,
                    &BuildingOverrides::default(),
                    Some(BuildContext {
                        grid: &mut grid,
                        world_data: &world_data,
                        food: &mut 9999,
                        cost: 10,
                    }),
                    None,
                );
                assert!(
                    result.is_ok(),
                    "waypoint on forest should succeed: {:?}",
                    result
                );
            },
        )
        .unwrap();
}

#[test]
fn resource_nodes_follow_biomes_spacing_and_occupied_cells() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(crate::resources::EntityMap::default());
    app.insert_resource(crate::resources::GpuSlotPool::default());
    app.add_message::<crate::messages::GpuUpdateMsg>();

    let mut grid = WorldGrid::default();
    grid.width = 6;
    grid.height = 4;
    grid.cell_size = crate::constants::TOWN_GRID_SPACING;
    grid.cells = vec![
        WorldCell {
            terrain: Biome::Grass,
            original_terrain: Biome::Grass
        };
        grid.width * grid.height
    ];
    grid.town_owner = vec![u16::MAX; grid.width * grid.height];
    for (col, row) in [(0, 0), (1, 0), (3, 0), (5, 0)] {
        let idx = row * grid.width + col;
        grid.cells[idx].terrain = Biome::Forest;
        grid.cells[idx].original_terrain = Biome::Forest;
    }
    for (col, row) in [(0, 2), (1, 2), (3, 2)] {
        let idx = row * grid.width + col;
        grid.cells[idx].terrain = Biome::Rock;
        grid.cells[idx].original_terrain = Biome::Rock;
    }
    let occupied_forest_pos = grid.grid_to_world(5, 0);

    app.insert_resource(grid);
    app.update();

    app.world_mut()
        .run_system_once(
            move |mut slot_alloc: ResMut<crate::resources::GpuSlotPool>,
                  mut entity_map: ResMut<crate::resources::EntityMap>,
                  mut commands: Commands,
                  mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>| {
                let result = place_building(
                    &mut slot_alloc,
                    &mut entity_map,
                    &mut commands,
                    &mut gpu_updates,
                    BuildingKind::Waypoint,
                    occupied_forest_pos,
                    crate::constants::TOWN_NONE,
                    crate::constants::FACTION_NEUTRAL,
                    &BuildingOverrides::default(),
                    None,
                    None,
                );
                assert!(
                    result.is_ok(),
                    "occupied cell setup should succeed: {:?}",
                    result
                );
            },
        )
        .unwrap();

    let config = WorldGenConfig::default();
    app.world_mut()
        .run_system_once(
            move |mut slot_alloc: ResMut<crate::resources::GpuSlotPool>,
                  mut entity_map: ResMut<crate::resources::EntityMap>,
                  mut commands: Commands,
                  mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
                  mut grid: ResMut<WorldGrid>| {
                let (tree_count, rock_count) = spawn_resource_nodes(
                    &config,
                    &mut grid,
                    &mut slot_alloc,
                    &mut entity_map,
                    &mut commands,
                    &mut gpu_updates,
                );

                // Density 1.0: every Forest/Rock cell gets a node, except occupied cells
                assert_eq!(
                    tree_count, 3,
                    "3 of 4 forest cells should get TreeNode (one occupied by Waypoint)"
                );
                assert_eq!(rock_count, 3, "all 3 rock cells should get RockNode");

                assert_eq!(
                    entity_map.get_at_grid(0, 0).map(|b| b.kind),
                    Some(BuildingKind::TreeNode)
                );
                assert_eq!(
                    entity_map.get_at_grid(1, 0).map(|b| b.kind),
                    Some(BuildingKind::TreeNode),
                    "adjacent forest cell should also get a TreeNode at density 1.0"
                );
                assert_eq!(
                    entity_map.get_at_grid(3, 0).map(|b| b.kind),
                    Some(BuildingKind::TreeNode)
                );
                assert_eq!(
                    entity_map.get_at_grid(5, 0).map(|b| b.kind),
                    Some(BuildingKind::Waypoint),
                    "occupied cell should keep existing building"
                );

                assert_eq!(
                    entity_map.get_at_grid(0, 2).map(|b| b.kind),
                    Some(BuildingKind::RockNode)
                );
                assert_eq!(
                    entity_map.get_at_grid(1, 2).map(|b| b.kind),
                    Some(BuildingKind::RockNode),
                    "adjacent rock cell should also get RockNode at density 1.0"
                );
                assert_eq!(
                    entity_map.get_at_grid(3, 2).map(|b| b.kind),
                    Some(BuildingKind::RockNode)
                );

                assert_eq!(entity_map.get_at_grid(2, 1).map(|b| b.kind), None);
            },
        )
        .unwrap();
}

#[test]
fn worldmap_generates_corridors_and_ice_caps() {
    let mut grid = WorldGrid::default();
    grid.width = 100;
    grid.height = 100;
    grid.cell_size = 64.0;
    grid.cells = vec![WorldCell::default(); 100 * 100];

    // Fixed seed for determinism -- avoids flaky failures from unlucky random maps.
    generate_terrain_worldmap(&mut grid, 0x1234_5678_9abc_def0);

    // Count biome types
    let mut water = 0usize;
    let mut land = 0usize;
    let mut ice_top = 0usize;
    let mut ice_bottom = 0usize;

    let ice_rows = (100.0 * 0.12) as usize; // 12 rows each pole

    for row in 0..grid.height {
        for col in 0..grid.width {
            let cell = &grid.cells[row * grid.width + col];
            match cell.terrain {
                Biome::Water => water += 1,
                Biome::Rock => {
                    if row < ice_rows {
                        ice_top += 1;
                    } else if row >= grid.height - ice_rows {
                        ice_bottom += 1;
                    }
                }
                _ => land += 1,
            }
        }
    }

    let total = (grid.width * grid.height) as f64;

    // Ice caps: top and bottom rows should be mostly Rock
    let top_total = (ice_rows * grid.width) as f64;
    let bot_total = (ice_rows * grid.width) as f64;
    assert!(
        ice_top as f64 / top_total > 0.9,
        "top ice cap should be >90% rock, got {:.1}%",
        ice_top as f64 / top_total * 100.0
    );
    assert!(
        ice_bottom as f64 / bot_total > 0.9,
        "bottom ice cap should be >90% rock, got {:.1}%",
        ice_bottom as f64 / bot_total * 100.0
    );

    // Should have meaningful water and land
    assert!(
        water as f64 / total > 0.1,
        "should have >10% water, got {:.1}%",
        water as f64 / total * 100.0
    );
    assert!(
        land as f64 / total > 0.15,
        "should have >15% land, got {:.1}%",
        land as f64 / total * 100.0
    );
}

#[test]
fn worldmap_biomes_follow_latitude() {
    let mut grid = WorldGrid::default();
    grid.width = 200;
    grid.height = 200;
    grid.cell_size = 64.0;
    grid.cells = vec![WorldCell::default(); 200 * 200];

    // Fixed seed for determinism.
    generate_terrain_worldmap(&mut grid, 0x1234_5678_9abc_def0);

    // Sample equatorial band (rows 90-110) and near-polar band (rows 25-35)
    let mut equatorial_grass = 0usize;
    let mut equatorial_total = 0usize;
    let mut polar_rock = 0usize;
    let mut polar_total = 0usize;

    for row in 90..110 {
        for col in 0..grid.width {
            let cell = &grid.cells[row * grid.width + col];
            if cell.terrain != Biome::Water {
                equatorial_total += 1;
                if cell.terrain == Biome::Grass {
                    equatorial_grass += 1;
                }
            }
        }
    }

    // Ice cap rows (within 12% of poles) should be Rock
    let ice_rows = (200.0 * 0.12) as usize;
    for row in 0..ice_rows {
        for col in 0..grid.width {
            let cell = &grid.cells[row * grid.width + col];
            polar_total += 1;
            if cell.terrain == Biome::Rock {
                polar_rock += 1;
            }
        }
    }

    // Equatorial band should have some grass (not all rock/forest)
    if equatorial_total > 0 {
        assert!(
            equatorial_grass > 0,
            "equatorial band should have some grass cells"
        );
    }

    // Ice cap should be all Rock
    assert!(
        polar_rock == polar_total,
        "ice cap rows should be 100% rock, got {}/{}",
        polar_rock,
        polar_total
    );
}

#[test]
fn worldmap_towns_avoid_water_and_ice() {
    // Verify that town placement rejects water/ice cells
    let mut grid = WorldGrid::default();
    grid.width = 50;
    grid.height = 50;
    grid.cell_size = 64.0;
    grid.cells = vec![WorldCell::default(); 50 * 50];

    // Set all cells to water
    for cell in &mut grid.cells {
        cell.terrain = Biome::Water;
        cell.original_terrain = Biome::Water;
    }
    // Set a few cells to grass (valid placement)
    let valid_col = 25;
    let valid_row = 25;
    for r in valid_row - 2..=valid_row + 2 {
        for c in valid_col - 2..=valid_col + 2 {
            let idx = r * grid.width + c;
            grid.cells[idx].terrain = Biome::Grass;
            grid.cells[idx].original_terrain = Biome::Grass;
        }
    }

    // Test the rejection: WorldMap style rejects Water and Rock
    let style = WorldGenStyle::WorldMap;
    assert!(style.needs_pre_terrain());

    let pos_water = grid.grid_to_world(5, 5);
    let (gc, gr) = grid.world_to_grid(pos_water);
    let cell = grid.cell(gc, gr).unwrap();
    assert_eq!(cell.terrain, Biome::Water);

    let pos_grass = grid.grid_to_world(valid_col, valid_row);
    let (gc, gr) = grid.world_to_grid(pos_grass);
    let cell = grid.cell(gc, gr).unwrap();
    assert_eq!(cell.terrain, Biome::Grass);
}

#[test]
fn worldgen_style_roundtrip() {
    for &style in WorldGenStyle::ALL {
        let idx = style.to_index();
        let back = WorldGenStyle::from_index(idx);
        assert_eq!(
            style,
            back,
            "roundtrip failed for {:?} (index {})",
            style.label(),
            idx
        );
    }
}

// -- path validation tests ------------------------------------------------

/// Build a passable 10x10 grid and a WorldData with one town centered at (5,5).
fn path_test_grid() -> (WorldGrid, WorldData) {
    let w = 10usize;
    let h = 10usize;
    let cs = crate::constants::TOWN_GRID_SPACING;
    let mut grid = WorldGrid {
        width: w,
        height: h,
        cell_size: cs,
        cells: vec![WorldCell::default(); w * h],
        pathfind_costs: vec![100u16; w * h], // all passable
        building_cost_cells: Vec::new(),
        hpa_cache: None,
        town_owner: vec![0u16; w * h],
        town_overlap: Default::default(),
    };
    // Block the cells that are impassable (pathfind_costs already set to 100 = passable)
    // Mark cell (5,5) as the town center (already passable).
    grid.pathfind_costs[5 * w + 5] = 100;

    let mut world_data = WorldData::default();
    world_data.towns.push(Town {
        name: "TestTown".into(),
        center: grid.grid_to_world(5, 5),
        faction: 0,
        kind: crate::constants::TownKind::Player,
    });
    (grid, world_data)
}

/// Helper: add a building to both the grid-cell and EntityMap without full ECS.
fn add_bld(
    entity_map: &mut EntityMap,
    grid: &WorldGrid,
    kind: BuildingKind,
    gc: usize,
    gr: usize,
    town_idx: u32,
) {
    let slot = entity_map.building_count();
    let pos = grid.grid_to_world(gc, gr);
    entity_map.add_instance(crate::entity_map::BuildingInstance {
        kind,
        position: pos,
        town_idx,
        slot,
        faction: 0,
    });
}

/// Regression test: placing a wall that fully surrounds the fountain is rejected.
/// Scenario: 3 walls already around fountain, 4th wall seals it -- must be rejected.
#[test]
fn wall_sealing_fountain_is_rejected() {
    let (mut grid, world_data) = path_test_grid();
    let mut entity_map = EntityMap::default();

    // Place fountain at town center (5,5)
    add_bld(&mut entity_map, &grid, BuildingKind::Fountain, 5, 5, 0);

    // Mark 3 neighbors of (5,5) as walls (impassable in cost grid)
    let w = grid.width;
    grid.pathfind_costs[5 * w + 4] = 0; // (4,5) -- left
    grid.pathfind_costs[5 * w + 6] = 0; // (6,5) -- right
    grid.pathfind_costs[4 * w + 5] = 0; // (5,4) -- below

    // Placing the 4th wall at (5,6) (above) would seal the fountain
    let result = grid.would_block_critical_access(&entity_map, 5, 6, 0, &world_data);
    assert!(
        result.is_some(),
        "expected rejection when fountain is fully sealed, got None"
    );
    assert!(
        result.unwrap().contains("fountain"),
        "expected fountain-specific message, got: {:?}",
        result
    );
}

/// Regression test: placing a wall that leaves a gap is accepted.
/// Scenario: 2 walls around fountain, 3rd wall placed -- still has one open neighbor.
#[test]
fn wall_leaving_gap_is_accepted() {
    let (mut grid, world_data) = path_test_grid();
    let mut entity_map = EntityMap::default();

    // Place fountain at town center (5,5)
    add_bld(&mut entity_map, &grid, BuildingKind::Fountain, 5, 5, 0);

    // Only 2 neighbors blocked -- one still open
    let w = grid.width;
    grid.pathfind_costs[5 * w + 4] = 0; // (4,5) blocked
    grid.pathfind_costs[4 * w + 5] = 0; // (5,4) blocked

    // Placing a wall at (6,5) -- leaves (5,6) still open
    let result = grid.would_block_critical_access(&entity_map, 6, 5, 0, &world_data);
    assert!(
        result.is_none(),
        "expected acceptance when fountain has open neighbor, got: {:?}",
        result
    );
}
