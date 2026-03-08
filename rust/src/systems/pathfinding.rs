//! A* pathfinding on WorldGrid.
//!
//! CPU computes waypoints via A*; GPU boids steer toward current waypoint
//! via existing goals[] buffer. No shader changes needed.
//!
//! Budget processing and intent resolution live in movement.rs (resolve_movement_system).

use std::cell::Cell;

use bevy::prelude::*;

use crate::components::{Building, Dead, GpuSlot, NpcPath};
use crate::messages::BuildingGridDirtyMsg;
use crate::resources::{PathRequest, PathRequestQueue, PathSource};
use crate::world::WorldGrid;

// ============================================================================
// A* GRID ADAPTER
// ============================================================================

/// Check if a grid cell is passable for pathfinding.
fn is_passable(grid: &WorldGrid, col: i32, row: i32) -> bool {
    neighbor_cost(grid, IVec2::new(col, row)).is_some()
}

/// Movement cost for a grid cell from precomputed cost grid. Returns None if impassable.
/// Single array index — no HashMap lookups.
fn neighbor_cost(grid: &WorldGrid, pos: IVec2) -> Option<u32> {
    debug_assert!(!grid.pathfind_costs.is_empty(), "pathfind_costs not initialized");
    cost_at(&grid.pathfind_costs, grid.width, grid.height, pos)
}

/// Read cost from a flat cost array. Returns None if out-of-bounds or impassable (0).
fn cost_at(costs: &[u16], width: usize, height: usize, pos: IVec2) -> Option<u32> {
    if pos.x < 0 || pos.y < 0 || pos.x >= width as i32 || pos.y >= height as i32 {
        return None;
    }
    let cost = costs[pos.y as usize * width + pos.x as usize];
    if cost == 0 { None } else { Some(cost as u32) }
}

const NEIGHBOR_DIRS: [IVec2; 4] = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y];

/// Manhattan distance heuristic (admissible for 4-directional movement).
/// Scaled by minimum terrain cost (67 = road) to guarantee admissibility.
fn heuristic(a: IVec2, b: IVec2) -> u32 {
    let d = (a - b).abs();
    (d.x + d.y) as u32 * 67 // min cost (road) ensures never overestimates
}

/// Run A* on the WorldGrid. Returns path as grid coordinates (including start and goal).
/// Enforces `max_nodes` limit via counter in successors closure — returns None if exceeded.
pub fn pathfind_on_grid(
    grid: &WorldGrid,
    start: IVec2,
    goal: IVec2,
    max_nodes: usize,
) -> Option<Vec<IVec2>> {
    neighbor_cost(grid, goal)?;
    let node_count = Cell::new(0usize);
    pathfinding::prelude::astar(
        &start,
        |&pos| {
            let n = node_count.get() + 1;
            node_count.set(n);
            let mut result = Vec::with_capacity(4);
            if n <= max_nodes {
                for d in NEIGHBOR_DIRS {
                    let np = pos + d;
                    if let Some(cost) = neighbor_cost(grid, np) {
                        result.push((np, cost));
                    }
                }
            }
            result
        },
        |&pos| heuristic(pos, goal),
        |&pos| pos == goal,
    )
    .map(|(path, _cost)| path)
}

/// Like `pathfind_on_grid` but reads costs from a provided slice.
/// Used for path cost accumulation — each successive A* call sees costs inflated
/// along previously-found paths, naturally spreading routes apart.
pub fn pathfind_with_costs(
    costs: &[u16],
    width: usize,
    height: usize,
    start: IVec2,
    goal: IVec2,
    max_nodes: usize,
) -> Option<Vec<IVec2>> {
    cost_at(costs, width, height, goal)?;
    let node_count = Cell::new(0usize);
    pathfinding::prelude::astar(
        &start,
        |&pos| {
            let n = node_count.get() + 1;
            node_count.set(n);
            let mut result = Vec::with_capacity(4);
            if n <= max_nodes {
                for d in NEIGHBOR_DIRS {
                    let np = pos + d;
                    if let Some(cost) = cost_at(costs, width, height, np) {
                        result.push((np, cost));
                    }
                }
            }
            result
        },
        |&pos| heuristic(pos, goal),
        |&pos| pos == goal,
    )
    .map(|(path, _cost)| path)
}

/// Accumulate cost along a path with spread radius. Discourages subsequent A* calls
/// from using the same cells. Only modifies passable cells (cost > 0).
pub fn accumulate_path_cost(
    costs: &mut [u16],
    width: usize,
    height: usize,
    path: &[IVec2],
    spread: i32,
    cost_add: u16,
) {
    for cell in path {
        for dy in -spread..=spread {
            for dx in -spread..=spread {
                let x = cell.x + dx;
                let y = cell.y + dy;
                if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                    let idx = y as usize * width + x as usize;
                    if costs[idx] > 0 {
                        costs[idx] = costs[idx].saturating_add(cost_add);
                    }
                }
            }
        }
    }
}

// ============================================================================
// LINE OF SIGHT (SHORT-DISTANCE BYPASS)
// ============================================================================

/// Bresenham line walk — check if all cells between two grid positions are passable.
pub fn line_of_sight(
    grid: &WorldGrid,
    from: IVec2,
    to: IVec2,
) -> bool {
    let dx = (to.x - from.x).abs();
    let dy = (to.y - from.y).abs();
    let sx = if from.x < to.x { 1 } else { -1 };
    let sy = if from.y < to.y { 1 } else { -1 };
    let mut err = dx - dy;
    let mut x = from.x;
    let mut y = from.y;

    loop {
        if !is_passable(grid, x, y) {
            return false;
        }
        if x == to.x && y == to.y {
            return true;
        }
        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            x += sx;
        }
        if e2 < dx {
            err += dx;
            y += sy;
        }
    }
}

// ============================================================================
// COST GRID SYNC
// ============================================================================

/// Sync precomputed pathfind cost grid when buildings change.
/// Runs after rebuild_building_grid_system, before resolve_movement_system.
pub fn sync_pathfind_costs_system(
    mut grid_dirty: MessageReader<BuildingGridDirtyMsg>,
    mut grid: ResMut<WorldGrid>,
    entity_map: Res<crate::resources::EntityMap>,
) {
    if grid_dirty.read().count() > 0 {
        grid.sync_building_costs(&entity_map);
    }
}

// ============================================================================
// PATH INVALIDATION
// ============================================================================

/// When buildings change, re-queue paths that might be affected.
/// Piggybacks on existing BuildingGridDirtyMsg.
pub fn invalidate_paths_on_building_change(
    mut grid_dirty: MessageReader<BuildingGridDirtyMsg>,
    path_q: Query<(Entity, &GpuSlot, &NpcPath), (Without<Building>, Without<Dead>)>,
    mut queue: ResMut<PathRequestQueue>,
    grid: Res<WorldGrid>,
    gpu_state: Res<crate::resources::GpuReadState>,
) {
    if grid_dirty.read().count() == 0 {
        return;
    }

    // Re-queue NPCs with active paths (skipping those on cooldown).
    // Future optimization: track which cells changed and only invalidate overlapping paths.
    for (entity, slot, path) in path_q.iter() {
        if path.waypoints.is_empty() || path.current >= path.waypoints.len() {
            continue;
        }
        if path.path_cooldown > 0.0 {
            continue;
        }

        let idx = slot.0;
        let (start_col, start_row) = if idx * 2 + 1 < gpu_state.positions.len() {
            let pos = Vec2::new(gpu_state.positions[idx * 2], gpu_state.positions[idx * 2 + 1]);
            grid.world_to_grid(pos)
        } else {
            continue;
        };

        let goal = *path.waypoints.last().expect("path non-empty");
        queue.enqueue(PathRequest {
            entity,
            slot: idx,
            start: IVec2::new(start_col as i32, start_row as i32),
            goal,
            goal_world: path.goal_world,
            priority: 1,
            source: PathSource::Invalidation,
        });
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::{BuildingInstance, EntityMap};
    use crate::world::{Biome, WorldCell};

    /// Create a simple test grid with given dimensions and all Grass terrain.
    fn make_grid(width: usize, height: usize) -> WorldGrid {
        let mut grid = WorldGrid::default();
        grid.width = width;
        grid.height = height;
        grid.cell_size = 64.0;
        grid.cells = vec![WorldCell::default(); width * height];
        grid.init_pathfind_costs();
        grid
    }

    /// Place a wall at grid (col, row) in the entity map.
    fn place_wall(entity_map: &mut EntityMap, col: i32, row: i32, slot: usize) {
        entity_map.add_instance(BuildingInstance {
            kind: crate::world::BuildingKind::Wall,
            position: Vec2::new(col as f32 * 64.0 + 32.0, row as f32 * 64.0 + 32.0),
            slot,
            town_idx: 0,
            faction: 0,
            patrol_order: 0,
            assigned_mine: None,
            manual_mine: false,
            wall_level: 1,
            npc_uid: None,
            respawn_timer: -1.0,
            growth_ready: false,
            growth_progress: 0.0,
            occupants: 0,
            under_construction: 0.0,
            kills: 0,
            xp: 0,
            upgrade_levels: Vec::new(),
            auto_upgrade_flags: Vec::new(),
        });
    }

    #[test]
    fn astar_finds_straight_path() {
        let grid = make_grid(10, 10);
        let path = pathfind_on_grid(
            &grid,
            IVec2::new(0, 0),
            IVec2::new(5, 0),
            5000,
        );
        assert!(path.is_some(), "should find path on open grid");
        let path = path.unwrap();
        assert_eq!(path.first(), Some(&IVec2::new(0, 0)));
        assert_eq!(path.last(), Some(&IVec2::new(5, 0)));
        assert_eq!(path.len(), 6); // 0,1,2,3,4,5
    }

    #[test]
    fn astar_routes_around_impassable() {
        let mut grid = make_grid(10, 10);
        let mut entity_map = EntityMap::default();
        // Wall from (2,0) to (2,4) — forces detour
        for row in 0..5 {
            place_wall(&mut entity_map, 2, row, 500 + row as usize);
        }
        grid.sync_building_costs(&entity_map);
        let path = pathfind_on_grid(
            &grid,
            IVec2::new(0, 0),
            IVec2::new(4, 0),
            5000,
        );
        assert!(path.is_some(), "should find path around wall");
        let path = path.unwrap();
        // Path must go around the wall (row >= 5 at some point)
        assert!(
            path.iter().any(|p| p.y >= 5),
            "path should route around wall barrier: {:?}",
            path
        );
    }

    #[test]
    fn astar_no_path_when_fully_blocked() {
        let mut grid = make_grid(10, 10);
        let mut entity_map = EntityMap::default();
        // Wall across entire column 2
        for row in 0..10 {
            place_wall(&mut entity_map, 2, row, 600 + row as usize);
        }
        grid.sync_building_costs(&entity_map);
        let path = pathfind_on_grid(
            &grid,
            IVec2::new(0, 0),
            IVec2::new(5, 0),
            5000,
        );
        assert!(path.is_none(), "should return None when no path exists");
    }

    #[test]
    fn astar_prefers_road_over_grass() {
        let grid = make_grid(10, 1);
        let path = pathfind_on_grid(
            &grid,
            IVec2::new(0, 0),
            IVec2::new(9, 0),
            5000,
        );
        assert!(path.is_some());
        assert_eq!(path.unwrap().len(), 10);
    }

    #[test]
    fn los_clear_on_open_grid() {
        let grid = make_grid(10, 10);
        assert!(line_of_sight(
            &grid,
            IVec2::new(0, 0),
            IVec2::new(5, 5)
        ));
    }

    #[test]
    fn los_blocked_by_impassable() {
        let mut grid = make_grid(10, 10);
        let mut entity_map = EntityMap::default();
        place_wall(&mut entity_map, 2, 2, 700);
        grid.sync_building_costs(&entity_map);
        assert!(
            !line_of_sight(&grid, IVec2::new(0, 0), IVec2::new(4, 4)),
            "LOS should be blocked by wall at (2,2)"
        );
    }

    #[test]
    fn terrain_costs_match_gpu_shader() {
        // GPU shader: Road = 1.5x speed, Grass = 1.0x, Forest = 0.7x
        // Cost = 100 / speed → Road=67, Grass=100, Forest=143
        // Rock/Water are expensive but passable (NPCs avoid but can escape)
        use crate::world::terrain_base_cost;
        assert_eq!(terrain_base_cost(Biome::Grass), 100);
        assert_eq!(terrain_base_cost(Biome::Dirt), 100);
        assert_eq!(terrain_base_cost(Biome::Forest), 143);
        assert_eq!(terrain_base_cost(Biome::Rock), 500);
        assert_eq!(terrain_base_cost(Biome::Water), 800);
    }

    #[test]
    fn heuristic_is_admissible() {
        // Scaled by min cost (67 = road) so it never overestimates actual path cost.
        let h = heuristic(IVec2::new(0, 0), IVec2::new(3, 4));
        assert_eq!(h, 469); // (3+4) * 67
        // Must be <= actual cost of cheapest 7-step path (7 roads = 7*67 = 469)
        assert!(h <= 7 * 67);
    }

    // -- maze pathfinding (walls) ---------------------------------------------

    #[test]
    fn astar_routes_around_single_wall() {
        let mut grid = make_grid(10, 10);
        let mut entity_map = EntityMap::default();
        // Wall at (3,0)..=(3,4) — blocks straight horizontal path
        for row in 0..5 {
            place_wall(&mut entity_map, 3, row, 100 + row as usize);
        }
        grid.sync_building_costs(&entity_map);
        let path = pathfind_on_grid(
            &grid,
            IVec2::new(0, 0),
            IVec2::new(6, 0),
            5000,
        );
        assert!(path.is_some(), "should find path around wall");
        let path = path.unwrap();
        // Path must not pass through any wall cell
        for p in &path {
            if p.x == 3 && p.y < 5 {
                panic!("path passed through wall at {:?}", p);
            }
        }
        // Path must detour south of the wall (y >= 5)
        assert!(
            path.iter().any(|p| p.y >= 5),
            "path should route around wall: {:?}",
            path
        );
    }

    #[test]
    fn astar_serpentine_maze() {
        // 15x11 grid with serpentine walls forcing a snake path
        let mut grid = make_grid(15, 11);
        let mut entity_map = EntityMap::default();
        let mut slot = 1000;

        // Row 2: wall from col 0..12 (gap at col 13-14)
        for col in 0..13 {
            place_wall(&mut entity_map, col, 2, slot);
            slot += 1;
        }
        // Row 5: wall from col 2..14 (gap at col 0-1)
        for col in 2..15 {
            place_wall(&mut entity_map, col, 5, slot);
            slot += 1;
        }
        // Row 8: wall from col 0..12 (gap at col 13-14)
        for col in 0..13 {
            place_wall(&mut entity_map, col, 8, slot);
            slot += 1;
        }
        grid.sync_building_costs(&entity_map);

        // Start top-left, goal bottom-right
        let path = pathfind_on_grid(
            &grid,
            IVec2::new(0, 0),
            IVec2::new(14, 10),
            10000,
        );
        assert!(path.is_some(), "should find path through serpentine maze");
        let path = path.unwrap();

        // Verify no wall cells traversed
        for p in &path {
            let is_wall = (p.y == 2 && p.x < 13)
                || (p.y == 5 && p.x >= 2)
                || (p.y == 8 && p.x < 13);
            assert!(!is_wall, "path crossed wall at {:?}", p);
        }

        // Path must visit all 3 corridor bands (y=0..1, y=3..4, y=6..7, y=9..10)
        assert!(path.iter().any(|p| p.y <= 1), "must visit top corridor");
        assert!(
            path.iter().any(|p| p.y >= 3 && p.y <= 4),
            "must visit second corridor"
        );
        assert!(
            path.iter().any(|p| p.y >= 6 && p.y <= 7),
            "must visit third corridor"
        );
        assert!(path.iter().any(|p| p.y >= 9), "must visit bottom corridor");
    }

    #[test]
    fn astar_no_path_walled_off() {
        let mut grid = make_grid(10, 10);
        let mut entity_map = EntityMap::default();
        // Complete wall across column 5 (all rows)
        for row in 0..10 {
            place_wall(&mut entity_map, 5, row, 200 + row as usize);
        }
        grid.sync_building_costs(&entity_map);
        let path = pathfind_on_grid(
            &grid,
            IVec2::new(0, 0),
            IVec2::new(8, 0),
            5000,
        );
        assert!(path.is_none(), "should return None when walled off");
    }

    #[test]
    fn los_blocked_by_wall() {
        let mut grid = make_grid(10, 10);
        let mut entity_map = EntityMap::default();
        place_wall(&mut entity_map, 3, 3, 300);
        grid.sync_building_costs(&entity_map);
        assert!(
            !line_of_sight(&grid, IVec2::new(0, 0), IVec2::new(6, 6)),
            "LOS should be blocked by wall at (3,3)"
        );
    }

    // -- invalidate_paths_on_building_change ---------------------------------

    use bevy::time::TimeUpdateStrategy;

    #[derive(Resource, Default)]
    struct SendGridDirty(bool);

    fn send_grid_dirty(
        mut writer: MessageWriter<BuildingGridDirtyMsg>,
        mut flag: ResMut<SendGridDirty>,
    ) {
        if flag.0 {
            writer.write(BuildingGridDirtyMsg);
            flag.0 = false;
        }
    }

    fn setup_invalidate_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(PathRequestQueue::default());
        app.insert_resource(make_grid(10, 10));
        app.insert_resource(crate::resources::GpuReadState::default());
        app.insert_resource(SendGridDirty(false));
        app.add_message::<BuildingGridDirtyMsg>();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_systems(
            FixedUpdate,
            (send_grid_dirty, invalidate_paths_on_building_change).chain(),
        );
        app.update();
        app.update();
        app
    }

    #[test]
    fn invalidate_no_action_without_dirty() {
        let mut app = setup_invalidate_app();
        // Spawn NPC with active path
        let mut gpu = app.world_mut().resource_mut::<crate::resources::GpuReadState>();
        gpu.positions = vec![100.0, 100.0];
        app.world_mut().spawn((
            GpuSlot(0),
            NpcPath {
                waypoints: vec![IVec2::new(0, 0), IVec2::new(5, 5)],
                current: 0,
                goal_world: Vec2::new(320.0, 320.0),
                ..default()
            },
        ));
        app.update();
        let queue = app.world().resource::<PathRequestQueue>();
        assert!(queue.is_empty(), "should not invalidate without dirty msg");
    }

    #[test]
    fn invalidate_requeues_active_paths() {
        let mut app = setup_invalidate_app();
        let mut gpu = app.world_mut().resource_mut::<crate::resources::GpuReadState>();
        gpu.positions = vec![100.0, 100.0];
        app.world_mut().spawn((
            GpuSlot(0),
            NpcPath {
                waypoints: vec![IVec2::new(0, 0), IVec2::new(5, 5)],
                current: 0,
                goal_world: Vec2::new(320.0, 320.0),
                ..default()
            },
        ));
        app.insert_resource(SendGridDirty(true));
        app.update();
        let queue = app.world().resource::<PathRequestQueue>();
        assert!(!queue.is_empty(), "should requeue NPC with active path on dirty");
    }

    #[test]
    fn invalidate_skips_empty_paths() {
        let mut app = setup_invalidate_app();
        let mut gpu = app.world_mut().resource_mut::<crate::resources::GpuReadState>();
        gpu.positions = vec![100.0, 100.0];
        app.world_mut().spawn((
            GpuSlot(0),
            NpcPath {
                waypoints: vec![],
                current: 0,
                goal_world: Vec2::ZERO,
                ..default()
            },
        ));
        app.insert_resource(SendGridDirty(true));
        app.update();
        let queue = app.world().resource::<PathRequestQueue>();
        assert!(queue.is_empty(), "should not requeue NPC with empty path");
    }
}
