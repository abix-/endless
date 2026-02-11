//! World Generation Test (6 phases)
//! Validates: grid dimensions, town placement, distances, buildings, terrain, camps.

use bevy::prelude::*;
use crate::resources::*;
use crate::world;

use super::TestState;

pub fn setup(
    mut world_data: ResMut<world::WorldData>,
    mut world_grid: ResMut<world::WorldGrid>,
    config: Res<world::WorldGenConfig>,
    mut food_storage: ResMut<FoodStorage>,
    mut faction_stats: ResMut<FactionStats>,
    mut farm_states: ResMut<FarmStates>,
    mut town_grids: ResMut<world::TownGrids>,
    mut test_state: ResMut<TestState>,
) {
    // Generate the world using our config (default: 2 towns)
    town_grids.grids.clear();
    world::generate_world(&config, &mut world_grid, &mut world_data, &mut farm_states, &mut town_grids);

    // Init supporting resources based on generated world
    let total_towns = world_data.towns.len();
    food_storage.init(total_towns);
    faction_stats.init(total_towns);

    test_state.phase_name = "Checking grid dimensions...".into();
    info!("world-gen: setup — generated world with {} config towns", config.num_towns);
}

pub fn tick(
    world_data: Res<world::WorldData>,
    world_grid: Res<world::WorldGrid>,
    config: Res<world::WorldGenConfig>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    match test.phase {
        // Phase 1: Grid has correct dimensions
        1 => {
            let expected_w = (config.world_width / world_grid.cell_size) as usize;
            let expected_h = (config.world_height / world_grid.cell_size) as usize;
            let expected_cells = expected_w * expected_h;
            test.phase_name = format!("grid={}x{} cells={}", world_grid.width, world_grid.height, world_grid.cells.len());

            if world_grid.width == expected_w && world_grid.height == expected_h && world_grid.cells.len() == expected_cells {
                test.pass_phase(elapsed, format!("grid {}x{} ({} cells)", world_grid.width, world_grid.height, expected_cells));
            } else {
                test.fail_phase(elapsed, format!(
                    "grid {}x{} cells={} (expected {}x{} = {})",
                    world_grid.width, world_grid.height, world_grid.cells.len(),
                    expected_w, expected_h, expected_cells));
            }
        }
        // Phase 2: Correct number of towns (villager + raider per config town)
        2 => {
            let expected_towns = config.num_towns * 2; // villager + raider
            let actual = world_data.towns.len();
            test.phase_name = format!("towns={}/{}", actual, expected_towns);

            if actual == expected_towns {
                test.pass_phase(elapsed, format!("{} towns ({} villager + {} raider)", actual, config.num_towns, config.num_towns));
            } else {
                test.fail_phase(elapsed, format!("towns={} (expected {})", actual, expected_towns));
            }
        }
        // Phase 3: Villager towns are min distance apart
        3 => {
            let villager_towns: Vec<Vec2> = world_data.towns.iter()
                .filter(|t| t.faction == 0)
                .map(|t| t.center)
                .collect();

            let mut min_dist = f32::MAX;
            for i in 0..villager_towns.len() {
                for j in (i+1)..villager_towns.len() {
                    let d = villager_towns[i].distance(villager_towns[j]);
                    if d < min_dist { min_dist = d; }
                }
            }

            test.phase_name = format!("min_dist={:.0}", min_dist);

            if villager_towns.len() <= 1 || min_dist >= config.min_town_distance {
                test.pass_phase(elapsed, format!("min_dist={:.0} (threshold={})", min_dist, config.min_town_distance));
            } else {
                test.fail_phase(elapsed, format!("min_dist={:.0} < {}", min_dist, config.min_town_distance));
            }
        }
        // Phase 4: Each villager town has correct buildings on grid
        4 => {
            let num_vill = config.num_towns;
            // Count buildings per villager town
            let mut fountains = vec![0u32; num_vill];
            let mut farms = vec![0u32; num_vill];
            let mut posts = vec![0u32; num_vill];

            for cell in &world_grid.cells {
                if let Some(ref b) = cell.building {
                    match b {
                        world::Building::Fountain { town_idx } => {
                            if (*town_idx as usize) < num_vill { fountains[*town_idx as usize] += 1; }
                        }
                        world::Building::Farm { town_idx } => {
                            if (*town_idx as usize) < num_vill { farms[*town_idx as usize] += 1; }
                        }
                        world::Building::GuardPost { town_idx, .. } => {
                            if (*town_idx as usize) < num_vill { posts[*town_idx as usize] += 1; }
                        }
                        _ => {}
                    }
                }
            }

            let all_ok = (0..num_vill).all(|i| {
                fountains[i] == 1 && farms[i] == 2 && posts[i] == 4
            });

            test.phase_name = format!("town0: f={} farm={} post={}",
                fountains.first().unwrap_or(&0), farms.first().unwrap_or(&0),
                posts.first().unwrap_or(&0));

            if all_ok {
                test.pass_phase(elapsed, format!("all {} towns have 1 fountain, 2 farms, 4 posts", num_vill));
            } else {
                let details: Vec<String> = (0..num_vill).map(|i| {
                    format!("town{}: f={} farm={} post={}", i, fountains[i], farms[i], posts[i])
                }).collect();
                test.fail_phase(elapsed, details.join(", "));
            }
        }
        // Phase 5: Terrain near town centers is Dirt
        5 => {
            let villager_towns: Vec<Vec2> = world_data.towns.iter()
                .filter(|t| t.faction == 0)
                .map(|t| t.center)
                .collect();

            let mut dirt_near_town = true;
            for tc in &villager_towns {
                let (col, row) = world_grid.world_to_grid(*tc);
                if let Some(cell) = world_grid.cell(col, row) {
                    if cell.terrain != world::Biome::Dirt {
                        dirt_near_town = false;
                    }
                }
            }

            test.phase_name = format!("dirt_near_town={}", dirt_near_town);

            if dirt_near_town {
                test.pass_phase(elapsed, "terrain at town centers is Dirt");
            } else {
                test.fail_phase(elapsed, "terrain at town center is not Dirt");
            }
        }
        // Phase 6: Raider camps exist with correct faction
        6 => {
            let raider_towns: Vec<&world::Town> = world_data.towns.iter()
                .filter(|t| t.faction > 0)
                .collect();

            let expected = config.num_towns;
            let has_camps = world_grid.cells.iter()
                .filter(|c| matches!(c.building, Some(world::Building::Camp { .. })))
                .count();

            test.phase_name = format!("raider_towns={} camps_on_grid={}", raider_towns.len(), has_camps);

            if raider_towns.len() == expected && has_camps == expected {
                test.pass_phase(elapsed, format!("{} raider camps with correct factions", expected));
                test.complete(elapsed);
            } else {
                test.fail_phase(elapsed, format!(
                    "raider_towns={} camps={} (expected {})", raider_towns.len(), has_camps, expected));
            }
        }
        _ => {}
    }
}
