//! Pathfinding Maze Test
//! Validates: NPCs navigate a serpentine wall maze via A* pathfinding.
//! Walls force NPCs to snake through corridors instead of walking straight.
//! Configurable NPC count (1-5000) via slider UI.

use crate::components::*;
use crate::resources::*;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use super::{TestSetupParams, TestState};

/// Grid cell size (matches TOWN_GRID_SPACING).
const CS: f32 = 64.0;

/// Convert grid (col, row) to world center position.
fn gw(col: i32, row: i32) -> (f32, f32) {
    (col as f32 * CS + CS * 0.5, row as f32 * CS + CS * 0.5)
}

#[derive(Resource)]
pub struct PathfindMazeConfig {
    pub npc_count: usize,
    input_buf: String,
}

impl Default for PathfindMazeConfig {
    fn default() -> Self {
        Self {
            npc_count: 1,
            input_buf: "1".into(),
        }
    }
}

pub fn setup(mut params: TestSetupParams, config: Res<PathfindMazeConfig>) {
    let npc_count = config.npc_count.max(1);

    // Expanded grid: 100x100 to fit maze (cols 0-24) + homes (cols 26+)
    params.world_grid.width = 100;
    params.world_grid.height = 100;
    params.world_grid.cell_size = crate::constants::TOWN_GRID_SPACING;
    params.world_grid.cells =
        vec![crate::world::WorldCell::default(); 100 * 100];

    params.add_town("MazeTown");
    params.world_data.towns[0].center = Vec2::new(3200.0, 3200.0);
    if params.entity_map.spatial_cell_size() <= 0.0 {
        let world_size_px = 100.0 * CS;
        params.entity_map.init_spatial(world_size_px);
    }

    // -- Build serpentine wall maze (cols 0-24, rows 0-24) --
    let wall_rows: &[(i32, i32, i32)] = &[
        (4, 0, 22),   // row 4: col 0..21
        (8, 3, 25),   // row 8: col 3..24
        (12, 0, 22),  // row 12: col 0..21
        (16, 3, 25),  // row 16: col 3..24
        (20, 0, 22),  // row 20: col 0..21
    ];

    for &(row, col_start, col_end) in wall_rows {
        for col in col_start..col_end {
            let (wx, wy) = gw(col, row);
            params.add_building(crate::world::BuildingKind::Wall, wx, wy, 0);
        }
    }

    // Place N FarmerHomes in a grid starting at col 26, row 0
    for i in 0..npc_count {
        let col = 26 + (i % 74) as i32;
        let row = (i / 74) as i32;
        let (hx, hy) = gw(col, row);
        params.add_building(crate::world::BuildingKind::FarmerHome, hx, hy, 0);
    }

    // Farm bottom-right of maze (mark as ready so farmers go to it)
    let (fx, fy) = gw(23, 22);
    params.add_building(crate::world::BuildingKind::Farm, fx, fy, 0);
    params.set_production_ready(Vec2::new(fx, fy));

    // Bed near the home area for resting
    let (bx, by) = gw(2, 1);
    params.add_bed(bx, by);

    params.init_economy(1);
    if let Some(mut f) = params.town_access.food_mut(0) { f.0 = 500; }

    // Camera centered on maze
    params.focus_camera(800.0, 800.0);

    params.test_state.phase_name = format!("Waiting for {} farmer(s)...", npc_count);
    info!(
        "pathfind-maze: setup — serpentine maze, {} farmer homes, farm at ({},{})",
        npc_count, fx, fy
    );
}

pub fn tick(
    entity_map: Res<EntityMap>,
    gpu_state: Res<GpuReadState>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    activity_q: Query<&Activity>,
    path_q: Query<&NpcPath>,
    _job_q: Query<&Job>,
    config: Res<PathfindMazeConfig>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    let npc_count = config.npc_count.max(1);
    // Scale timeouts with NPC count: more NPCs need more time to spawn/path
    let time_scale = 1.0 + (npc_count as f32 / 100.0).min(10.0);

    let farmers: Vec<_> = entity_map
        .iter_npcs()
        .filter(|n| !n.dead && n.job == Job::Farmer)
        .collect();
    let farmer_count = farmers.len();

    match test.phase {
        // Phase 1: All farmers spawned
        1 => {
            test.phase_name = format!("farmers={}/{}", farmer_count, npc_count);
            if farmer_count >= npc_count {
                test.pass_phase(elapsed, format!("farmers spawned ({})", farmer_count));
            } else if elapsed > 5.0 * time_scale {
                test.fail_phase(elapsed, format!("only {}/{} farmers", farmer_count, npc_count));
            }
        }
        // Phase 2: At least one farmer has A* waypoints
        2 => {
            let has_path = farmers.iter().any(|n| {
                path_q
                    .get(n.entity)
                    .is_ok_and(|p| p.waypoints.len() >= 2)
            });
            test.phase_name = format!("has_path={}", has_path);
            if has_path {
                let wp_count = farmers
                    .iter()
                    .filter_map(|n| path_q.get(n.entity).ok())
                    .map(|p| p.waypoints.len())
                    .max()
                    .unwrap_or(0);
                test.pass_phase(elapsed, format!("A* path found ({} waypoints)", wp_count));
            } else if elapsed > 8.0 * time_scale {
                test.fail_phase(elapsed, "no pathfinding waypoints".to_string());
            }
        }
        // Phase 3: Any farmer crosses first wall row (row 4 = y=288)
        3 => {
            let crossed = farmers.iter().any(|n| {
                let idx = n.slot * 2;
                if idx + 1 < gpu_state.positions.len() {
                    let y = gpu_state.positions[idx + 1];
                    y > 4.0 * CS + CS
                } else {
                    false
                }
            });
            let farmer_y = farmers
                .first()
                .and_then(|n| {
                    let idx = n.slot * 2;
                    (idx + 1 < gpu_state.positions.len())
                        .then(|| gpu_state.positions[idx + 1])
                })
                .unwrap_or(0.0);
            test.phase_name = format!("y={:.0} crossed_wall1={}", farmer_y, crossed);
            if crossed {
                test.pass_phase(
                    elapsed,
                    format!("crossed first wall row (y={:.0})", farmer_y),
                );
            } else if elapsed > 30.0 * time_scale {
                test.fail_phase(elapsed, format!("stuck at y={:.0}", farmer_y));
            }
        }
        // Phase 4: Any farmer crosses second wall row (row 8 = y=544)
        4 => {
            let farmer_y = farmers
                .first()
                .and_then(|n| {
                    let idx = n.slot * 2;
                    (idx + 1 < gpu_state.positions.len())
                        .then(|| gpu_state.positions[idx + 1])
                })
                .unwrap_or(0.0);
            let crossed = farmer_y > 8.0 * CS + CS;
            test.phase_name = format!("y={:.0} crossed_wall2={}", farmer_y, crossed);
            if crossed {
                test.pass_phase(
                    elapsed,
                    format!("crossed second wall row (y={:.0})", farmer_y),
                );
            } else if elapsed > 50.0 * time_scale {
                test.fail_phase(elapsed, format!("stuck at y={:.0}", farmer_y));
            }
        }
        // Phase 5: Any farmer reaches farm area (near farm at row 22)
        5 => {
            let near_farm = farmers.iter().any(|n| {
                let idx = n.slot * 2;
                if idx + 1 < gpu_state.positions.len() {
                    let y = gpu_state.positions[idx + 1];
                    y > 20.0 * CS
                } else {
                    false
                }
            });
            let is_working = farmers.iter().any(|n| {
                activity_q
                    .get(n.entity)
                    .is_ok_and(|a| a.kind == ActivityKind::Work)
            });
            let farmer_y = farmers
                .first()
                .and_then(|n| {
                    let idx = n.slot * 2;
                    (idx + 1 < gpu_state.positions.len())
                        .then(|| gpu_state.positions[idx + 1])
                })
                .unwrap_or(0.0);
            test.phase_name = format!("y={:.0} near_farm={} working={}", farmer_y, near_farm, is_working);
            if near_farm || is_working {
                test.pass_phase(
                    elapsed,
                    format!(
                        "reached farm area (y={:.0}, working={})",
                        farmer_y, is_working
                    ),
                );
                test.complete(elapsed);
            } else if elapsed > 90.0 * time_scale {
                test.fail_phase(
                    elapsed,
                    format!("didn't reach farm, y={:.0}", farmer_y),
                );
            }
        }
        _ => {}
    }
}

/// Maze config panel — slider for NPC count + restart button.
pub(super) fn ui(
    mut contexts: EguiContexts,
    mut config: ResMut<PathfindMazeConfig>,
    mut test_state: ResMut<TestState>,
    mut next_state: ResMut<NextState<crate::AppState>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let mut restart = false;
    egui::Window::new("Maze Config")
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 300.0])
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui: &mut egui::Ui| {
            ui.horizontal(|ui: &mut egui::Ui| {
                ui.label("NPCs:");
                let mut count = config.npc_count as u32;
                if ui
                    .add(egui::Slider::new(&mut count, 1..=5000).logarithmic(true))
                    .changed()
                {
                    config.npc_count = count as usize;
                    config.input_buf = count.to_string();
                }
            });
            ui.label(format!("{} farmer homes", config.npc_count));
            ui.add_space(4.0);
            if ui.button("Restart").clicked() {
                restart = true;
            }
        });
    if restart {
        test_state.pending_relaunch = Some("pathfind-maze".into());
        next_state.set(crate::AppState::TestMenu);
    }
    Ok(())
}
