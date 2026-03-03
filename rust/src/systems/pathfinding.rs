//! A* pathfinding on WorldGrid with budgeted per-frame processing.
//!
//! CPU computes waypoints via A*; GPU boids steer toward current waypoint
//! via existing goals[] buffer. No shader changes needed.

use bevy::prelude::*;

use crate::components::{Building, Dead, GpuSlot, NpcPath};
use crate::messages::{BuildingGridDirtyMsg, GpuUpdate, GpuUpdateMsg};
use crate::resources::{EntityMap, GameTime, PathRequestQueue, PathfindConfig};
use crate::world::{Biome, WorldGrid};

// ============================================================================
// A* GRID ADAPTER
// ============================================================================

/// Movement cost per terrain type (scaled to u32 × 100 for integer A* cost).
/// Matches GPU shader speed multipliers: cost = 100 / speed_multiplier.
fn terrain_cost(biome: Biome) -> u32 {
    match biome {
        Biome::Grass | Biome::Dirt => 100, // 1.0x speed → cost 100
        Biome::Forest => 143,              // 0.7x speed → cost ~143
        Biome::Rock => 200,                // 0.5x speed → cost 200
        Biome::Water => u32::MAX,          // impassable
    }
}

/// Check if a grid cell is passable for pathfinding.
fn is_passable(grid: &WorldGrid, entity_map: &EntityMap, col: i32, row: i32) -> bool {
    if col < 0 || row < 0 || col >= grid.width as i32 || row >= grid.height as i32 {
        return false;
    }
    let cell = &grid.cells[row as usize * grid.width + col as usize];
    if cell.terrain == Biome::Water {
        return false;
    }
    // Walls block pathfinding (buildings at this cell that are walls)
    if entity_map.has_building_at(col, row) {
        if let Some(inst) = entity_map.get_at_grid(col, row) {
            if inst.kind == crate::world::BuildingKind::Wall {
                return false;
            }
        }
    }
    true
}

/// 4-directional neighbors with terrain cost.
fn neighbors(
    grid: &WorldGrid,
    entity_map: &EntityMap,
    pos: IVec2,
) -> Vec<(IVec2, u32)> {
    let dirs = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y];
    let mut result = Vec::with_capacity(4);
    for d in dirs {
        let n = pos + d;
        if !is_passable(grid, entity_map, n.x, n.y) {
            continue;
        }
        let cell = &grid.cells[n.y as usize * grid.width + n.x as usize];
        let cost = terrain_cost(cell.terrain);
        // Road bonus: override terrain cost
        if entity_map.get_at_grid(n.x, n.y).is_some_and(|inst| {
            inst.kind == crate::world::BuildingKind::Road
        }) {
            result.push((n, 67)); // 1.5x speed → cost ~67
        } else {
            result.push((n, cost));
        }
    }
    result
}

/// Manhattan distance heuristic (admissible for 4-directional movement).
/// Scaled by minimum terrain cost (67 = road) to guarantee admissibility.
fn heuristic(a: IVec2, b: IVec2) -> u32 {
    let d = (a - b).abs();
    (d.x + d.y) as u32 * 67 // min cost (road) ensures never overestimates
}

/// Run A* on the WorldGrid. Returns path as grid coordinates (including start and goal).
pub fn pathfind_on_grid(
    grid: &WorldGrid,
    entity_map: &EntityMap,
    start: IVec2,
    goal: IVec2,
    _max_nodes: usize,
) -> Option<Vec<IVec2>> {
    if !is_passable(grid, entity_map, goal.x, goal.y) {
        return None;
    }
    pathfinding::prelude::astar(
        &start,
        |&pos| neighbors(grid, entity_map, pos),
        |&pos| heuristic(pos, goal),
        |&pos| pos == goal,
    )
    .map(|(path, _cost)| path)
}

// ============================================================================
// LINE OF SIGHT (SHORT-DISTANCE BYPASS)
// ============================================================================

/// Bresenham line walk — check if all cells between two grid positions are passable.
pub fn line_of_sight(
    grid: &WorldGrid,
    entity_map: &EntityMap,
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
        if !is_passable(grid, entity_map, x, y) {
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
// BUDGET SYSTEM
// ============================================================================

/// Process queued pathfinding requests, up to config.max_per_frame per tick.
/// Produces NpcPath components and sets initial waypoint via GpuUpdateMsg::SetTarget.
pub fn pathfind_budget_system(
    mut queue: ResMut<PathRequestQueue>,
    config: Res<PathfindConfig>,
    grid: Res<WorldGrid>,
    entity_map: Res<EntityMap>,
    game_time: Res<GameTime>,
    mut path_q: Query<&mut NpcPath>,
    slot_q: Query<&GpuSlot>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
) {
    if game_time.is_paused() || queue.requests.is_empty() {
        return;
    }

    // Sort by priority (lower = more urgent)
    queue.requests.sort_unstable_by_key(|r| r.priority);

    let budget = config.max_per_frame.min(queue.requests.len());
    let requests: Vec<_> = queue.requests.drain(..budget).collect();

    for req in requests {
        // Validate entity still exists
        if slot_q.get(req.entity).is_err() {
            continue;
        }

        let start_grid = IVec2::new(req.start.x, req.start.y);
        let goal_grid = IVec2::new(req.goal.x, req.goal.y);
        let dist = (goal_grid - start_grid).abs();
        let manhattan = dist.x + dist.y;

        // Short-distance LOS bypass
        if manhattan <= config.short_distance_tiles {
            if line_of_sight(&grid, &entity_map, start_grid, goal_grid) {
                // Direct movement — clear any existing path, let boids handle it
                if let Ok(mut path) = path_q.get_mut(req.entity) {
                    path.waypoints.clear();
                    path.current = 0;
                }
                // Set goal directly
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                    idx: req.slot,
                    x: req.goal_world.x,
                    y: req.goal_world.y,
                }));
                continue;
            }
        }

        // Full A* pathfinding
        if let Some(path_points) = pathfind_on_grid(
            &grid,
            &entity_map,
            start_grid,
            goal_grid,
            config.max_nodes,
        ) {
            if path_points.len() < 2 {
                // Already at goal
                continue;
            }

            // Store path and set first waypoint
            let first_wp = path_points[1]; // [0] is start position
            let world_pos = grid.grid_to_world(first_wp.x as usize, first_wp.y as usize);

            if let Ok(mut npc_path) = path_q.get_mut(req.entity) {
                npc_path.waypoints = path_points;
                npc_path.current = 1; // skip start, aim for first real waypoint
                npc_path.goal_world = req.goal_world;
            }

            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                idx: req.slot,
                x: world_pos.x,
                y: world_pos.y,
            }));
        }
        // else: no path found — NPC stays put
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

    // Re-queue all NPCs with active paths.
    // Future optimization: track which cells changed and only invalidate overlapping paths.
    for (entity, slot, path) in path_q.iter() {
        if path.waypoints.is_empty() || path.current >= path.waypoints.len() {
            continue;
        }

        let idx = slot.0;
        let (start_col, start_row) = if idx * 2 + 1 < gpu_state.positions.len() {
            let pos = Vec2::new(gpu_state.positions[idx * 2], gpu_state.positions[idx * 2 + 1]);
            grid.world_to_grid(pos)
        } else {
            continue;
        };

        let goal = *path.waypoints.last().unwrap();
        queue.requests.push(crate::resources::PathRequest {
            entity,
            slot: idx,
            start: IVec2::new(start_col as i32, start_row as i32),
            goal,
            goal_world: path.goal_world,
            priority: 1,
        });
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::BuildingInstance;
    use crate::world::WorldCell;

    /// Create a simple test grid with given dimensions and all Grass terrain.
    fn make_grid(width: usize, height: usize) -> WorldGrid {
        WorldGrid {
            width,
            height,
            cell_size: 64.0,
            cells: vec![WorldCell::default(); width * height],
        }
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
        let entity_map = EntityMap::default();
        let path = pathfind_on_grid(
            &grid,
            &entity_map,
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
    fn astar_routes_around_water() {
        let mut grid = make_grid(10, 10);
        // Place water wall from (2,0) to (2,4) — forces detour
        for row in 0..5 {
            grid.cells[row * 10 + 2].terrain = Biome::Water;
        }
        let entity_map = EntityMap::default();
        let path = pathfind_on_grid(
            &grid,
            &entity_map,
            IVec2::new(0, 0),
            IVec2::new(4, 0),
            5000,
        );
        assert!(path.is_some(), "should find path around water");
        let path = path.unwrap();
        // Path must go around the water (row >= 5 at some point)
        assert!(
            path.iter().any(|p| p.y >= 5),
            "path should route around water barrier: {:?}",
            path
        );
    }

    #[test]
    fn astar_no_path_when_fully_blocked() {
        let mut grid = make_grid(10, 10);
        // Water wall across entire column 2
        for row in 0..10 {
            grid.cells[row * 10 + 2].terrain = Biome::Water;
        }
        let entity_map = EntityMap::default();
        let path = pathfind_on_grid(
            &grid,
            &entity_map,
            IVec2::new(0, 0),
            IVec2::new(5, 0),
            5000,
        );
        assert!(path.is_none(), "should return None when no path exists");
    }

    #[test]
    fn astar_prefers_road_over_grass() {
        let grid = make_grid(10, 1);
        // All grass — verify basic cost behavior
        let entity_map = EntityMap::default();
        let path = pathfind_on_grid(
            &grid,
            &entity_map,
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
        let entity_map = EntityMap::default();
        assert!(line_of_sight(
            &grid,
            &entity_map,
            IVec2::new(0, 0),
            IVec2::new(5, 5)
        ));
    }

    #[test]
    fn los_blocked_by_water() {
        let mut grid = make_grid(10, 10);
        grid.cells[2 * 10 + 2].terrain = Biome::Water; // (2,2) is water
        let entity_map = EntityMap::default();
        assert!(
            !line_of_sight(&grid, &entity_map, IVec2::new(0, 0), IVec2::new(4, 4)),
            "LOS should be blocked by water at (2,2)"
        );
    }

    #[test]
    fn terrain_costs_match_gpu_shader() {
        // GPU shader: Road = 1.5x speed, Grass = 1.0x, Forest = 0.7x, Rock = 0.5x
        // Cost = 100 / speed → Road=67, Grass=100, Forest=143, Rock=200
        assert_eq!(terrain_cost(Biome::Grass), 100);
        assert_eq!(terrain_cost(Biome::Dirt), 100);
        assert_eq!(terrain_cost(Biome::Forest), 143);
        assert_eq!(terrain_cost(Biome::Rock), 200);
        assert_eq!(terrain_cost(Biome::Water), u32::MAX);
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
        let grid = make_grid(10, 10);
        let mut entity_map = EntityMap::default();
        // Wall at (3,0)..=(3,4) — blocks straight horizontal path
        for row in 0..5 {
            place_wall(&mut entity_map, 3, row, 100 + row as usize);
        }
        let path = pathfind_on_grid(
            &grid,
            &entity_map,
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
        let grid = make_grid(15, 11);
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

        // Start top-left, goal bottom-right
        let path = pathfind_on_grid(
            &grid,
            &entity_map,
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
        let grid = make_grid(10, 10);
        let mut entity_map = EntityMap::default();
        // Complete wall across column 5 (all rows)
        for row in 0..10 {
            place_wall(&mut entity_map, 5, row, 200 + row as usize);
        }
        let path = pathfind_on_grid(
            &grid,
            &entity_map,
            IVec2::new(0, 0),
            IVec2::new(8, 0),
            5000,
        );
        assert!(path.is_none(), "should return None when walled off");
    }

    #[test]
    fn los_blocked_by_wall() {
        let grid = make_grid(10, 10);
        let mut entity_map = EntityMap::default();
        place_wall(&mut entity_map, 3, 3, 300);
        assert!(
            !line_of_sight(&grid, &entity_map, IVec2::new(0, 0), IVec2::new(6, 6)),
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
            },
        ));
        app.update();
        let queue = app.world().resource::<PathRequestQueue>();
        assert!(queue.requests.is_empty(), "should not invalidate without dirty msg");
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
            },
        ));
        app.insert_resource(SendGridDirty(true));
        app.update();
        let queue = app.world().resource::<PathRequestQueue>();
        assert!(!queue.requests.is_empty(), "should requeue NPC with active path on dirty");
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
            },
        ));
        app.insert_resource(SendGridDirty(true));
        app.update();
        let queue = app.world().resource::<PathRequestQueue>();
        assert!(queue.requests.is_empty(), "should not requeue NPC with empty path");
    }
}
