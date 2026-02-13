//! Terrain & Building Visual Test
//! Displays all terrain biomes and building types in a labeled grid.
//! Stays on screen until user clicks Back.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::render::{MainCamera, TilemapSpawned};
use crate::world::{self, Biome, Building, WorldCell, WorldGrid};

use super::TestState;

// Grid layout: 11 cols × 5 rows
const GRID_COLS: usize = 11;
const GRID_ROWS: usize = 5;

// Terrain row (row 3 in grid = second from top)
const TERRAIN_ROW: usize = 3;
const TERRAIN_LABELS: [&str; GRID_COLS] = [
    "Grass A", "Grass B", "Forest A", "Forest B", "Forest C",
    "Forest D", "Forest E", "Forest F", "Water", "Rock", "Dirt",
];

// Building row (row 1 in grid)
const BUILDING_ROW: usize = 1;
const BUILDING_LABELS: [&str; 8] = [
    "Fountain", "Bed", "GuardPost", "Farm", "Camp", "House", "Barracks", "Tent",
];

pub fn setup(
    mut grid: ResMut<WorldGrid>,
    mut test: ResMut<TestState>,
    mut game_time: ResMut<crate::resources::GameTime>,
) {
    game_time.time_scale = 0.0;

    // Populate a small grid
    grid.width = GRID_COLS;
    grid.height = GRID_ROWS;
    grid.cell_size = 32.0;
    grid.cells = vec![WorldCell { terrain: Biome::Dirt, building: None }; GRID_COLS * GRID_ROWS];

    // Row 3: terrain showcase — each column gets a distinct biome
    for col in 0..GRID_COLS {
        let idx = TERRAIN_ROW * GRID_COLS + col;
        grid.cells[idx].terrain = match col {
            0 => Biome::Grass,  // tileset_index uses cell_index % 2
            1 => Biome::Grass,
            2..=7 => Biome::Forest,
            8 => Biome::Water,
            9 => Biome::Rock,
            10 => Biome::Dirt,
            _ => Biome::Dirt,
        };
    }

    // Row 1: buildings on Dirt background
    let buildings: [Building; 8] = [
        Building::Fountain { town_idx: 0 },
        Building::Bed { town_idx: 0 },
        Building::GuardPost { town_idx: 0, patrol_order: 0 },
        Building::Farm { town_idx: 0 },
        Building::Camp { town_idx: 0 },
        Building::House { town_idx: 0 },
        Building::Barracks { town_idx: 0 },
        Building::Tent { town_idx: 0 },
    ];
    for (col, building) in buildings.iter().enumerate() {
        let idx = BUILDING_ROW * GRID_COLS + col;
        grid.cells[idx].building = Some(*building);
    }

    test.phase_name = "Waiting for tilemap...".into();
    info!("terrain-visual: setup — {}x{} grid, {} terrain biomes, {} buildings",
        GRID_COLS, GRID_ROWS, GRID_COLS, buildings.len());
}

pub fn tick(
    tilemap_spawned: Res<TilemapSpawned>,
    mut test: ResMut<TestState>,
    time: Res<Time>,
    grid: Res<WorldGrid>,
    mut camera_query: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
    mut contexts: EguiContexts,
    windows: Query<&Window>,
    mut positioned: Local<bool>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    // Phase 1: wait for tilemap to spawn
    if test.phase == 1 && !*positioned {
        if !tilemap_spawned.0 {
            test.phase_name = format!("Waiting for tilemap... ({:.1}s)", elapsed);
            if elapsed > 10.0 {
                test.fail_phase(elapsed, "Tilemap never spawned");
            }
            return;
        }

        // Position camera centered on grid
        let world_w = grid.width as f32 * grid.cell_size;
        let world_h = grid.height as f32 * grid.cell_size;
        if let Ok((mut transform, mut projection)) = camera_query.single_mut() {
            transform.translation.x = world_w / 2.0;
            transform.translation.y = world_h / 2.0;
            if let Projection::Orthographic(ref mut ortho) = *projection {
                ortho.scale = 0.15; // ~6.7x zoom to see 32px tiles clearly
            }
        }

        *positioned = true;
        test.pass_phase(elapsed, format!("Tilemap spawned, {}x{} grid", grid.width, grid.height));
        return;
    }

    // Egui overlay: labels for terrain and building tiles
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let Ok(window) = windows.single() else { return };
    let Ok((cam_transform, cam_projection)) = camera_query.single() else { return };
    let Projection::Orthographic(ref ortho) = *cam_projection else { return };

    let zoom = 1.0 / ortho.scale;
    let cam_pos = cam_transform.translation.truncate();
    let viewport = Vec2::new(window.width(), window.height());

    let world_to_screen = |world_pos: Vec2| -> egui::Pos2 {
        let offset = (world_pos - cam_pos) * zoom;
        egui::Pos2::new(
            offset.x + viewport.x / 2.0,
            viewport.y / 2.0 - offset.y,
        )
    };

    let cell_center = |col: usize, row: usize| -> Vec2 {
        Vec2::new(
            col as f32 * grid.cell_size + grid.cell_size * 0.5,
            row as f32 * grid.cell_size + grid.cell_size * 0.5,
        )
    };

    // Terrain labels (above terrain row)
    for col in 0..GRID_COLS {
        let pos = cell_center(col, TERRAIN_ROW) + Vec2::new(0.0, grid.cell_size * 0.7);
        let screen = world_to_screen(pos);
        egui::Area::new(egui::Id::new(format!("terrain_label_{}", col)))
            .fixed_pos(screen)
            .pivot(egui::Align2::CENTER_BOTTOM)
            .interactable(false)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(TERRAIN_LABELS[col]).strong().size(11.0).color(egui::Color32::WHITE));
            });
    }

    // Row label for terrain
    let terrain_label_pos = cell_center(0, TERRAIN_ROW) - Vec2::new(grid.cell_size * 1.5, 0.0);
    let screen = world_to_screen(terrain_label_pos);
    egui::Area::new(egui::Id::new("terrain_row_label"))
        .fixed_pos(screen)
        .pivot(egui::Align2::CENTER_CENTER)
        .interactable(false)
        .show(ctx, |ui| {
            ui.label(egui::RichText::new("Terrain").strong().size(14.0).color(egui::Color32::from_rgb(102, 255, 102)));
        });

    // Building labels (below building row)
    for col in 0..BUILDING_LABELS.len() {
        let pos = cell_center(col, BUILDING_ROW) - Vec2::new(0.0, grid.cell_size * 0.7);
        let screen = world_to_screen(pos);
        egui::Area::new(egui::Id::new(format!("building_label_{}", col)))
            .fixed_pos(screen)
            .pivot(egui::Align2::CENTER_TOP)
            .interactable(false)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(BUILDING_LABELS[col]).strong().size(11.0).color(egui::Color32::WHITE));
            });
    }

    // Row label for buildings
    let building_label_pos = cell_center(0, BUILDING_ROW) - Vec2::new(grid.cell_size * 1.5, 0.0);
    let screen = world_to_screen(building_label_pos);
    egui::Area::new(egui::Id::new("building_row_label"))
        .fixed_pos(screen)
        .pivot(egui::Align2::CENTER_CENTER)
        .interactable(false)
        .show(ctx, |ui| {
            ui.label(egui::RichText::new("Buildings").strong().size(14.0).color(egui::Color32::from_rgb(255, 204, 102)));
        });

    // Atlas coordinate labels under each terrain tile
    for col in 0..GRID_COLS {
        let idx = TERRAIN_ROW * GRID_COLS + col;
        let tile_idx = grid.cells[idx].terrain.tileset_index(idx);
        let label = match world::TERRAIN_TILES[tile_idx as usize] {
            world::TileSpec::Single(c, r) => format!("({},{})", c, r),
            world::TileSpec::Quad(q) => format!("2x2@({},{})", q[0].0, q[0].1),
        };
        let pos = cell_center(col, TERRAIN_ROW) - Vec2::new(0.0, grid.cell_size * 0.6);
        let screen = world_to_screen(pos);
        egui::Area::new(egui::Id::new(format!("terrain_atlas_{}", col)))
            .fixed_pos(screen)
            .pivot(egui::Align2::CENTER_TOP)
            .interactable(false)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(label)
                    .size(9.0).color(egui::Color32::GRAY));
            });
    }

    // Atlas coordinate labels above each building tile
    for col in 0..BUILDING_LABELS.len() {
        let idx = BUILDING_ROW * GRID_COLS + col;
        if let Some(ref building) = grid.cells[idx].building {
            let tile_idx = building.tileset_index();
            let label = match world::BUILDING_TILES[tile_idx as usize] {
                world::TileSpec::Single(c, r) => format!("({},{})", c, r),
                world::TileSpec::Quad(q) => format!("2x2@({},{})", q[0].0, q[0].1),
            };
            let pos = cell_center(col, BUILDING_ROW) + Vec2::new(0.0, grid.cell_size * 0.6);
            let screen = world_to_screen(pos);
            egui::Area::new(egui::Id::new(format!("building_atlas_{}", col)))
                .fixed_pos(screen)
                .pivot(egui::Align2::CENTER_BOTTOM)
                .interactable(false)
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new(label)
                        .size(9.0).color(egui::Color32::GRAY));
                });
        }
    }
}
