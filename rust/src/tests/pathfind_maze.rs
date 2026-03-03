//! Pathfinding Maze Test
//! Validates: NPC navigates a serpentine wall maze via A* pathfinding.
//! Walls force the NPC to snake through corridors instead of walking straight.

use crate::components::*;
use crate::resources::*;
use bevy::prelude::*;

use super::{TestSetupParams, TestState};

/// Grid cell size (matches TOWN_GRID_SPACING).
const CS: f32 = 64.0;

/// Convert grid (col, row) to world center position.
fn gw(col: i32, row: i32) -> (f32, f32) {
    (col as f32 * CS + CS * 0.5, row as f32 * CS + CS * 0.5)
}

pub fn setup(mut params: TestSetupParams) {
    // Larger grid for the maze (25x25 default from add_town is fine)
    params.add_town("MazeTown");
    params.world_data.towns[0].center = Vec2::new(800.0, 800.0);

    // -- Build serpentine wall maze --
    // Grid is 25x25 (cols 0..24, rows 0..24)
    // Farmer home at (1, 1), farm at (23, 22)
    // Walls create horizontal barriers with alternating gaps:
    //   Row 4:  cols 0..21  (gap at right: cols 22-24)
    //   Row 8:  cols 3..24  (gap at left:  cols 0-2)
    //   Row 12: cols 0..21  (gap at right: cols 22-24)
    //   Row 16: cols 3..24  (gap at left:  cols 0-2)
    //   Row 20: cols 0..21  (gap at right: cols 22-24)

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

    // Farmer home top-left
    let (hx, hy) = gw(1, 1);
    params.add_building(crate::world::BuildingKind::FarmerHome, hx, hy, 0);

    // Farm bottom-right (mark as ready so farmer goes to it)
    let (fx, fy) = gw(23, 22);
    params.add_building(crate::world::BuildingKind::Farm, fx, fy, 0);
    if let Some(inst) = params.entity_map.find_farm_at_mut(Vec2::new(fx, fy)) {
        inst.growth_ready = true;
        inst.growth_progress = 1.0;
    }

    // Bed near the home for resting
    let (bx, by) = gw(2, 1);
    params.add_bed(bx, by);

    params.init_economy(1);
    // Start with food so the farmer doesn't starve
    params.food_storage.food[0] = 500;

    // Camera centered on maze
    params.focus_camera(800.0, 800.0);

    params.test_state.phase_name = "Waiting for farmer spawn...".into();
    info!(
        "pathfind-maze: setup — serpentine maze, home at ({},{}), farm at ({},{})",
        hx, hy, fx, fy
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
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    let farmers: Vec<_> = entity_map
        .iter_npcs()
        .filter(|n| !n.dead && n.job == Job::Farmer)
        .collect();
    let farmer_count = farmers.len();

    match test.phase {
        // Phase 1: Farmer spawned
        1 => {
            test.phase_name = format!("farmers={}/1", farmer_count);
            if farmer_count >= 1 {
                test.pass_phase(elapsed, format!("farmer spawned ({})", farmer_count));
            } else if elapsed > 5.0 {
                test.fail_phase(elapsed, format!("no farmer spawned"));
            }
        }
        // Phase 2: Farmer has pathfinding waypoints (A* kicked in)
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
            } else if elapsed > 8.0 {
                test.fail_phase(elapsed, format!("no pathfinding waypoints"));
            }
        }
        // Phase 3: Farmer Y position crosses first wall row (row 4 = y=288)
        // This proves the NPC went through the gap, not through the wall
        3 => {
            let crossed = farmers.iter().any(|n| {
                let idx = n.slot * 2;
                if idx + 1 < gpu_state.positions.len() {
                    let y = gpu_state.positions[idx + 1];
                    y > 4.0 * CS + CS // past row 4 wall
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
            } else if elapsed > 30.0 {
                test.fail_phase(elapsed, format!("stuck at y={:.0}", farmer_y));
            }
        }
        // Phase 4: Farmer Y position crosses second wall row (row 8 = y=544)
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
            } else if elapsed > 50.0 {
                test.fail_phase(elapsed, format!("stuck at y={:.0}", farmer_y));
            }
        }
        // Phase 5: Farmer reaches destination area (near farm at row 22)
        5 => {
            let near_farm = farmers.iter().any(|n| {
                let idx = n.slot * 2;
                if idx + 1 < gpu_state.positions.len() {
                    let y = gpu_state.positions[idx + 1];
                    y > 20.0 * CS // past row 20 wall
                } else {
                    false
                }
            });
            let is_working = farmers.iter().any(|n| {
                activity_q
                    .get(n.entity)
                    .is_ok_and(|a| matches!(*a, Activity::Working))
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
            } else if elapsed > 90.0 {
                test.fail_phase(
                    elapsed,
                    format!("didn't reach farm, y={:.0}", farmer_y),
                );
            }
        }
        _ => {}
    }
}
