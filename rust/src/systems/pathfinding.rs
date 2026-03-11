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
// HPA* (HIERARCHICAL PATHFINDING A*)
// ============================================================================
//
// Divides the grid into chunks and precomputes paths between chunk-boundary
// entrance nodes. Queries search the small abstract graph (~500-1000 nodes)
// instead of the full grid (~62,500 cells). 5-10× faster per request.

use hashbrown::HashMap;

const HPA_CHUNK_SIZE: usize = 16;
/// Minimum terrain cost (road=67) — used for admissible heuristic on abstract graph.
const HPA_MIN_COST: u32 = 67;

/// Cached edge between two entrance nodes within the same chunk.
#[derive(Clone, Debug)]
struct HpaEdge {
    target: usize,     // node index
    cost: u32,         // total path cost
    path: Vec<IVec2>,  // cached grid-level path (excludes start, includes target)
}

/// An entrance node at a chunk boundary.
#[derive(Clone, Debug)]
struct HpaNode {
    pos: IVec2,
    chunk: (usize, usize), // (chunk_col, chunk_row)
    edges: Vec<HpaEdge>,
}

/// Precomputed hierarchical pathfinding cache.
pub struct HpaCache {
    nodes: Vec<HpaNode>,
    pos_to_node: HashMap<IVec2, usize>,
    chunk_nodes: HashMap<(usize, usize), Vec<usize>>,
}

impl HpaCache {
    /// Build the HPA* cache from a cost grid.
    pub fn build(costs: &[u16], width: usize, height: usize) -> Self {
        let mut cache = HpaCache {
            nodes: Vec::new(),
            pos_to_node: HashMap::new(),
            chunk_nodes: HashMap::new(),
        };

        let cols = width.div_ceil(HPA_CHUNK_SIZE);
        let rows = height.div_ceil(HPA_CHUNK_SIZE);
        let all_chunks: hashbrown::HashSet<(usize, usize)> =
            (0..cols).flat_map(|cc| (0..rows).map(move |cr| (cc, cr))).collect();
        cache.build_chunks(costs, width, height, &all_chunks);

        cache
    }

    /// Scan borders and compute intra-chunk edges for a subset of chunks.
    /// Shared by `build()` (all chunks) and `rebuild_chunks()` (affected only).
    fn build_chunks(
        &mut self,
        costs: &[u16],
        width: usize,
        height: usize,
        chunks: &hashbrown::HashSet<(usize, usize)>,
    ) {
        let cols = width.div_ceil(HPA_CHUNK_SIZE);
        let rows = height.div_ceil(HPA_CHUNK_SIZE);

        // Horizontal borders — scan if either side is in `chunks`
        for cr in 0..rows.saturating_sub(1) {
            let border_row = (cr + 1) * HPA_CHUNK_SIZE;
            if border_row >= height { continue; }
            for cc in 0..cols {
                if !chunks.contains(&(cc, cr)) && !chunks.contains(&(cc, cr + 1)) { continue; }
                let col_start = cc * HPA_CHUNK_SIZE;
                let col_end = ((cc + 1) * HPA_CHUNK_SIZE).min(width);
                self.scan_border_horizontal(costs, width, height, border_row, col_start, col_end, cr, cc);
            }
        }
        // Vertical borders — scan if either side is in `chunks`
        for cc in 0..cols.saturating_sub(1) {
            let border_col = (cc + 1) * HPA_CHUNK_SIZE;
            if border_col >= width { continue; }
            for cr in 0..rows {
                if !chunks.contains(&(cc, cr)) && !chunks.contains(&(cc + 1, cr)) { continue; }
                let row_start = cr * HPA_CHUNK_SIZE;
                let row_end = ((cr + 1) * HPA_CHUNK_SIZE).min(height);
                self.scan_border_vertical(costs, width, height, border_col, row_start, row_end, cc, cr);
            }
        }
        // Intra-chunk edges for all chunks in the set that have entrance nodes
        let chunk_keys: Vec<(usize, usize)> = chunks.iter()
            .copied()
            .filter(|k| self.chunk_nodes.contains_key(k))
            .collect();
        for chunk_key in chunk_keys {
            self.compute_intra_chunk_edges(costs, width, height, chunk_key);
        }
    }

    /// Scan a horizontal border between chunk (cc, cr) and (cc, cr+1).
    fn scan_border_horizontal(
        &mut self, costs: &[u16], width: usize, _height: usize,
        border_row: usize, col_start: usize, col_end: usize,
        cr: usize, cc: usize,
    ) {
        let mut run_start: Option<usize> = None;
        for col in col_start..col_end {
            let above = costs[(border_row - 1) * width + col];
            let below = costs[border_row * width + col];
            if above > 0 && below > 0 {
                if run_start.is_none() { run_start = Some(col); }
            } else {
                if let Some(start) = run_start.take() {
                    let mid = (start + col - 1) / 2;
                    self.add_border_pair(
                        IVec2::new(mid as i32, border_row as i32 - 1), (cc, cr),
                        IVec2::new(mid as i32, border_row as i32), (cc, cr + 1),
                        costs, width,
                    );
                }
            }
        }
        if let Some(start) = run_start {
            let mid = (start + col_end - 1) / 2;
            self.add_border_pair(
                IVec2::new(mid as i32, border_row as i32 - 1), (cc, cr),
                IVec2::new(mid as i32, border_row as i32), (cc, cr + 1),
                costs, width,
            );
        }
    }

    /// Scan a vertical border between chunk (cc, cr) and (cc+1, cr).
    fn scan_border_vertical(
        &mut self, costs: &[u16], width: usize, _height: usize,
        border_col: usize, row_start: usize, row_end: usize,
        cc: usize, cr: usize,
    ) {
        let mut run_start: Option<usize> = None;
        for row in row_start..row_end {
            let left = costs[row * width + border_col - 1];
            let right = costs[row * width + border_col];
            if left > 0 && right > 0 {
                if run_start.is_none() { run_start = Some(row); }
            } else {
                if let Some(start) = run_start.take() {
                    let mid = (start + row - 1) / 2;
                    self.add_border_pair(
                        IVec2::new(border_col as i32 - 1, mid as i32), (cc, cr),
                        IVec2::new(border_col as i32, mid as i32), (cc + 1, cr),
                        costs, width,
                    );
                }
            }
        }
        if let Some(start) = run_start {
            let mid = (start + row_end - 1) / 2;
            self.add_border_pair(
                IVec2::new(border_col as i32 - 1, mid as i32), (cc, cr),
                IVec2::new(border_col as i32, mid as i32), (cc + 1, cr),
                costs, width,
            );
        }
    }

    /// Add a pair of entrance nodes on opposite sides of a border and connect them.
    fn add_border_pair(
        &mut self,
        pos_a: IVec2, chunk_a: (usize, usize),
        pos_b: IVec2, chunk_b: (usize, usize),
        costs: &[u16], width: usize,
    ) {
        let idx_a = self.get_or_add_node(pos_a, chunk_a);
        let idx_b = self.get_or_add_node(pos_b, chunk_b);
        // Cross-border edge: one step between adjacent cells
        let cross_cost = costs[pos_b.y as usize * width + pos_b.x as usize] as u32;
        self.nodes[idx_a].edges.push(HpaEdge { target: idx_b, cost: cross_cost, path: vec![pos_b] });
        let back_cost = costs[pos_a.y as usize * width + pos_a.x as usize] as u32;
        self.nodes[idx_b].edges.push(HpaEdge { target: idx_a, cost: back_cost, path: vec![pos_a] });
    }

    fn get_or_add_node(&mut self, pos: IVec2, chunk: (usize, usize)) -> usize {
        if let Some(&idx) = self.pos_to_node.get(&pos) {
            return idx;
        }
        let idx = self.nodes.len();
        self.nodes.push(HpaNode { pos, chunk, edges: Vec::new() });
        self.pos_to_node.insert(pos, idx);
        self.chunk_nodes.entry(chunk).or_default().push(idx);
        idx
    }

    /// Precompute paths between all entrance pairs within a chunk using A*.
    fn compute_intra_chunk_edges(
        &mut self, costs: &[u16], width: usize, height: usize,
        chunk_key: (usize, usize),
    ) {
        let node_indices: Vec<usize> = self.chunk_nodes.get(&chunk_key)
            .cloned()
            .unwrap_or_default();
        if node_indices.len() < 2 { return; }

        let (cc, cr) = chunk_key;
        let min_col = (cc * HPA_CHUNK_SIZE) as i32;
        let min_row = (cr * HPA_CHUNK_SIZE) as i32;
        let max_col = (((cc + 1) * HPA_CHUNK_SIZE).min(width)) as i32;
        let max_row = (((cr + 1) * HPA_CHUNK_SIZE).min(height)) as i32;

        // For each pair of nodes in this chunk, find intra-chunk path
        for i in 0..node_indices.len() {
            let start_pos = self.nodes[node_indices[i]].pos;
            for j in (i + 1)..node_indices.len() {
                let goal_pos = self.nodes[node_indices[j]].pos;

                // Chunk-bounded A*
                let result = pathfinding::prelude::astar(
                    &start_pos,
                    |&pos| {
                        let mut result = Vec::with_capacity(4);
                        for d in NEIGHBOR_DIRS {
                            let np = pos + d;
                            if np.x >= min_col && np.x < max_col && np.y >= min_row && np.y < max_row {
                                if let Some(c) = cost_at(costs, width, height, np) {
                                    result.push((np, c));
                                }
                            }
                        }
                        result
                    },
                    |&pos| heuristic(pos, goal_pos),
                    |&pos| pos == goal_pos,
                );

                if let Some((path, cost)) = result {
                    let ni = node_indices[i];
                    let nj = node_indices[j];
                    // Forward edge (skip start)
                    let fwd_path: Vec<IVec2> = path[1..].to_vec();
                    self.nodes[ni].edges.push(HpaEdge { target: nj, cost, path: fwd_path });
                    // Reverse edge
                    let rev_path: Vec<IVec2> = path[..path.len()-1].iter().rev().copied().collect();
                    self.nodes[nj].edges.push(HpaEdge { target: ni, cost, path: rev_path });
                }
            }
        }
    }

    /// Rebuild specific chunks (after building placement/destruction).
    /// Incremental: removes nodes in affected chunks + neighbors, then re-scans borders
    /// and recomputes intra-chunk edges for just those chunks.
    pub fn rebuild_chunks(&mut self, costs: &[u16], width: usize, height: usize, dirty_cells: &[usize]) {
        let dirty: hashbrown::HashSet<(usize, usize)> = dirty_cells.iter()
            .map(|&i| ((i % width) / HPA_CHUNK_SIZE, (i / width) / HPA_CHUNK_SIZE))
            .collect();
        if dirty.is_empty() { return; }
        // Expand to neighbor chunks — border entrances are shared between adjacent chunks
        let cols = width.div_ceil(HPA_CHUNK_SIZE);
        let rows = height.div_ceil(HPA_CHUNK_SIZE);
        let affected: hashbrown::HashSet<(usize, usize)> = dirty.iter()
            .flat_map(|&(cc, cr)| [
                Some((cc, cr)),
                cc.checked_sub(1).map(|c| (c, cr)),
                (cc + 1 < cols).then_some((cc + 1, cr)),
                cr.checked_sub(1).map(|r| (cc, r)),
                (cr + 1 < rows).then_some((cc, cr + 1)),
            ].into_iter().flatten())
            .collect();
        self.remove_chunk_nodes(&affected);
        self.build_chunks(costs, width, height, &affected);
    }

    /// Remove all nodes belonging to the given chunks. Compacts the node array
    /// and remaps edge targets in surviving nodes.
    fn remove_chunk_nodes(&mut self, chunks: &hashbrown::HashSet<(usize, usize)>) {
        // Build remap: old index → new index (None if removed)
        let mut remap: Vec<Option<usize>> = Vec::with_capacity(self.nodes.len());
        let mut new_idx = 0usize;
        for node in &self.nodes {
            if chunks.contains(&node.chunk) {
                remap.push(None);
            } else {
                remap.push(Some(new_idx));
                new_idx += 1;
            }
        }
        // Compact nodes, remap + filter edges pointing to removed nodes
        let mut new_nodes = Vec::with_capacity(new_idx);
        for (i, mut node) in self.nodes.drain(..).enumerate() {
            if remap[i].is_none() { continue; }
            node.edges.retain_mut(|e| {
                if let Some(t) = remap[e.target] { e.target = t; true } else { false }
            });
            new_nodes.push(node);
        }
        self.nodes = new_nodes;
        // Rebuild indexes from compacted nodes
        self.pos_to_node.clear();
        self.chunk_nodes.clear();
        for (i, node) in self.nodes.iter().enumerate() {
            self.pos_to_node.insert(node.pos, i);
            self.chunk_nodes.entry(node.chunk).or_default().push(i);
        }
    }
}

/// HPA* pathfinding query. Falls back to chunk-local A* for same-chunk paths.
pub fn pathfind_hpa(
    grid: &WorldGrid,
    start: IVec2,
    goal: IVec2,
) -> Option<Vec<IVec2>> {
    let cache = grid.hpa_cache.as_ref()?;
    let costs = &grid.pathfind_costs;

    // Check goal is passable
    cost_at(costs, grid.width, grid.height, goal)?;

    let start_chunk = (start.x as usize / HPA_CHUNK_SIZE, start.y as usize / HPA_CHUNK_SIZE);
    let goal_chunk = (goal.x as usize / HPA_CHUNK_SIZE, goal.y as usize / HPA_CHUNK_SIZE);

    // Same chunk: use direct chunk-bounded A* (fast, small search space)
    if start_chunk == goal_chunk {
        let min_col = (start_chunk.0 * HPA_CHUNK_SIZE) as i32;
        let min_row = (start_chunk.1 * HPA_CHUNK_SIZE) as i32;
        let max_col = (((start_chunk.0 + 1) * HPA_CHUNK_SIZE).min(grid.width)) as i32;
        let max_row = (((start_chunk.1 + 1) * HPA_CHUNK_SIZE).min(grid.height)) as i32;
        return pathfinding::prelude::astar(
            &start,
            |&pos| {
                let mut result = Vec::with_capacity(4);
                for d in NEIGHBOR_DIRS {
                    let np = pos + d;
                    if np.x >= min_col && np.x < max_col && np.y >= min_row && np.y < max_row {
                        if let Some(c) = cost_at(costs, grid.width, grid.height, np) {
                            result.push((np, c));
                        }
                    }
                }
                result
            },
            |&pos| heuristic(pos, goal),
            |&pos| pos == goal,
        )
        .map(|(path, _)| path);
    }

    // Different chunks: search the abstract graph
    // Step 1: Connect start/goal to their chunk's entrance nodes via temp edges
    let chunk_entrance_nodes = |chunk: (usize, usize)| -> Vec<usize> {
        cache.chunk_nodes.get(&chunk).cloned().unwrap_or_default()
    };

    let start_entrances = chunk_entrance_nodes(start_chunk);
    let goal_entrances = chunk_entrance_nodes(goal_chunk);
    if start_entrances.is_empty() || goal_entrances.is_empty() {
        return None; // chunk has no entrances — isolated
    }

    // Compute temporary edges from start to its chunk's entrance nodes
    let (sc_min_col, sc_min_row, sc_max_col, sc_max_row) = chunk_bounds(start_chunk, grid.width, grid.height);
    let mut start_edges: Vec<(usize, u32, Vec<IVec2>)> = Vec::new();
    for &nid in &start_entrances {
        let entrance_pos = cache.nodes[nid].pos;
        let result = pathfinding::prelude::astar(
            &start,
            |&pos| {
                let mut r = Vec::with_capacity(4);
                for d in NEIGHBOR_DIRS {
                    let np = pos + d;
                    if np.x >= sc_min_col && np.x < sc_max_col && np.y >= sc_min_row && np.y < sc_max_row {
                        if let Some(c) = cost_at(costs, grid.width, grid.height, np) {
                            r.push((np, c));
                        }
                    }
                }
                r
            },
            |&pos| heuristic(pos, entrance_pos),
            |&pos| pos == entrance_pos,
        );
        if let Some((path, cost)) = result {
            start_edges.push((nid, cost, path[1..].to_vec()));
        }
    }

    // Compute temporary edges from goal chunk entrances to goal
    let (gc_min_col, gc_min_row, gc_max_col, gc_max_row) = chunk_bounds(goal_chunk, grid.width, grid.height);
    let mut goal_edges: HashMap<usize, (u32, Vec<IVec2>)> = HashMap::new();
    for &nid in &goal_entrances {
        let entrance_pos = cache.nodes[nid].pos;
        let result = pathfinding::prelude::astar(
            &entrance_pos,
            |&pos| {
                let mut r = Vec::with_capacity(4);
                for d in NEIGHBOR_DIRS {
                    let np = pos + d;
                    if np.x >= gc_min_col && np.x < gc_max_col && np.y >= gc_min_row && np.y < gc_max_row {
                        if let Some(c) = cost_at(costs, grid.width, grid.height, np) {
                            r.push((np, c));
                        }
                    }
                }
                r
            },
            |&pos| heuristic(pos, goal),
            |&pos| pos == goal,
        );
        if let Some((path, cost)) = result {
            goal_edges.insert(nid, (cost, path[1..].to_vec()));
        }
    }

    // Step 2: A* on abstract graph
    // Node IDs: 0..N-1 are real nodes, N = virtual start, N+1 = virtual goal
    let n = cache.nodes.len();
    let v_start = n;
    let v_goal = n + 1;

    let abstract_result = pathfinding::prelude::astar(
        &v_start,
        |&node_id| {
            let mut successors: Vec<(usize, u32)> = Vec::new();
            if node_id == v_start {
                for &(nid, cost, _) in &start_edges {
                    successors.push((nid, cost));
                }
            } else if node_id == v_goal {
                // goal has no outgoing edges
            } else if node_id < n {
                // Real node — emit cached edges
                for edge in &cache.nodes[node_id].edges {
                    successors.push((edge.target, edge.cost));
                }
                // If this node is in the goal chunk, try reaching virtual goal
                if cache.nodes[node_id].chunk == goal_chunk {
                    if let Some(&(cost, _)) = goal_edges.get(&node_id) {
                        successors.push((v_goal, cost));
                    }
                }
            }
            successors
        },
        |&node_id| {
            if node_id == v_goal { return 0; }
            let pos = if node_id == v_start { start }
                      else if node_id < n { cache.nodes[node_id].pos }
                      else { goal };
            heuristic(pos, goal) * HPA_MIN_COST
        },
        |&node_id| node_id == v_goal,
    );

    let (abstract_path, _total_cost) = abstract_result?;

    // Step 3: Stitch cached paths into full grid-level path
    let mut full_path = vec![start];
    for window in abstract_path.windows(2) {
        let from_id = window[0];
        let to_id = window[1];

        if from_id == v_start {
            // start → first entrance node
            if let Some((_, _, path)) = start_edges.iter().find(|e| e.0 == to_id) {
                full_path.extend_from_slice(path);
            }
        } else if to_id == v_goal {
            // last entrance → goal
            if let Some((_, path)) = goal_edges.get(&from_id) {
                full_path.extend_from_slice(path);
            }
        } else if from_id < n && to_id < n {
            // cached edge between entrance nodes
            if let Some(edge) = cache.nodes[from_id].edges.iter().find(|e| e.target == to_id) {
                full_path.extend_from_slice(&edge.path);
            }
        }
    }

    Some(full_path)
}

/// Get chunk bounds as (min_col, min_row, max_col, max_row) in grid coords.
fn chunk_bounds(chunk: (usize, usize), width: usize, height: usize) -> (i32, i32, i32, i32) {
    let min_col = (chunk.0 * HPA_CHUNK_SIZE) as i32;
    let min_row = (chunk.1 * HPA_CHUNK_SIZE) as i32;
    let max_col = (((chunk.0 + 1) * HPA_CHUNK_SIZE).min(width)) as i32;
    let max_row = (((chunk.1 + 1) * HPA_CHUNK_SIZE).min(height)) as i32;
    (min_col, min_row, max_col, max_row)
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

/// When buildings change, re-queue paths whose remaining waypoints overlap dirty chunks.
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

    // Build set of dirty chunks from cells with building cost overrides
    let dirty_cells = grid.dirty_cost_cells();
    if dirty_cells.is_empty() {
        return;
    }
    let dirty_chunks: hashbrown::HashSet<(usize, usize)> = dirty_cells.iter()
        .map(|&i| ((i % grid.width) / HPA_CHUNK_SIZE, (i / grid.width) / HPA_CHUNK_SIZE))
        .collect();

    for (entity, slot, path) in path_q.iter() {
        if path.waypoints.is_empty() || path.current >= path.waypoints.len() {
            continue;
        }
        if path.path_cooldown > 0.0 {
            continue;
        }

        // Only invalidate if remaining waypoints cross a dirty chunk
        let dominated = path.waypoints[path.current..].iter().any(|wp| {
            let chunk = (wp.x as usize / HPA_CHUNK_SIZE, wp.y as usize / HPA_CHUNK_SIZE);
            dirty_chunks.contains(&chunk)
        });
        if !dominated {
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
            occupants: 0,
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
        // Mark cell (5,5) as dirty so the NPC's waypoint overlaps the dirty chunk
        let mut grid = app.world_mut().resource_mut::<WorldGrid>();
        let idx = 5 * grid.width + 5;
        grid.building_cost_cells.push(idx);
        drop(grid);
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
    fn invalidate_skips_paths_outside_dirty_chunks() {
        let mut app = setup_invalidate_app();
        let mut gpu = app.world_mut().resource_mut::<crate::resources::GpuReadState>();
        gpu.positions = vec![100.0, 100.0];
        // Dirty cell at (8,8) — but NPC path goes to (2,2), different chunk on larger grids
        // On a 10×10 grid with chunk_size=16, everything is chunk (0,0), so use a 40×40 grid
        let grid = make_grid(40, 40);
        app.insert_resource(grid);
        let mut grid = app.world_mut().resource_mut::<WorldGrid>();
        let idx = 30 * grid.width + 30; // cell (30,30) → chunk (1,1)
        grid.building_cost_cells.push(idx);
        drop(grid);
        app.world_mut().spawn((
            GpuSlot(0),
            NpcPath {
                waypoints: vec![IVec2::new(1, 1), IVec2::new(2, 2)], // chunk (0,0)
                current: 0,
                goal_world: Vec2::new(128.0, 128.0),
                ..default()
            },
        ));
        app.insert_resource(SendGridDirty(true));
        app.update();
        let queue = app.world().resource::<PathRequestQueue>();
        assert!(queue.is_empty(), "should not requeue path that doesn't cross dirty chunk");
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
