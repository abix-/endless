// =============================================================================
// NPC Compute Shader - GPU-Accelerated Physics
// =============================================================================
// 3-pass dispatch per frame:
//   Mode 0: Clear spatial grid
//   Mode 1: Build spatial grid (insert NPCs into cells)
//   Mode 2: Movement toward goals + combat targeting via grid

struct Params {
    count: u32,
    separation_radius: f32,
    separation_strength: f32,
    delta: f32,
    grid_width: u32,
    grid_height: u32,
    cell_size: f32,
    max_per_cell: u32,
    arrival_threshold: f32,
    mode: u32,
    combat_range: f32,
    _pad2: f32,
}

// Storage buffers matching Rust bind group layout
@group(0) @binding(0) var<storage, read_write> positions: array<vec2<f32>>;
@group(0) @binding(1) var<storage, read_write> goals: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read_write> speeds: array<f32>;
@group(0) @binding(3) var<storage, read_write> grid_counts: array<atomic<i32>>;
@group(0) @binding(4) var<storage, read_write> grid_data: array<i32>;
@group(0) @binding(5) var<storage, read_write> arrivals: array<i32>;
@group(0) @binding(6) var<storage, read_write> backoff: array<i32>;
@group(0) @binding(7) var<storage, read_write> factions: array<i32>;
@group(0) @binding(8) var<storage, read_write> healths: array<f32>;
@group(0) @binding(9) var<storage, read_write> combat_targets: array<i32>;
@group(0) @binding(10) var<uniform> params: Params;

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let i = global_id.x;

    // =========================================================================
    // MODE 0: Clear spatial grid
    // =========================================================================
    if (params.mode == 0u) {
        let grid_cells = params.grid_width * params.grid_height;
        if (i >= grid_cells) { return; }
        atomicStore(&grid_counts[i], 0);
        return;
    }

    // =========================================================================
    // MODE 1: Build spatial grid (insert NPCs into cells)
    // =========================================================================
    if (params.mode == 1u) {
        if (i >= params.count) { return; }
        let pos = positions[i];

        // Skip hidden/dead NPCs
        if (pos.x < -9000.0) { return; }

        let cx = i32(pos.x / params.cell_size);
        let cy = i32(pos.y / params.cell_size);
        let gw = i32(params.grid_width);
        let gh = i32(params.grid_height);

        if (cx < 0 || cx >= gw || cy < 0 || cy >= gh) { return; }

        let cell_idx = cy * gw + cx;
        let mpc = i32(params.max_per_cell);
        let slot = atomicAdd(&grid_counts[cell_idx], 1);

        if (slot < mpc) {
            grid_data[cell_idx * mpc + slot] = i32(i);
        }
        return;
    }

    // =========================================================================
    // MODE 2: Movement + combat targeting
    // =========================================================================
    if (i >= params.count) { return; }

    var pos = positions[i];
    let goal = goals[i];
    let speed = speeds[i];
    var settled = arrivals[i];

    // Skip dead/hidden NPCs
    if (pos.x < -9000.0) {
        combat_targets[i] = -1;
        return;
    }

    // --- Movement toward goal ---
    let to_goal = goal - pos;
    let dist_to_goal = length(to_goal);
    let wants_goal = settled == 0;

    var movement = vec2<f32>(0.0, 0.0);
    if (wants_goal && dist_to_goal > params.arrival_threshold) {
        movement = normalize(to_goal) * speed;
    } else if (wants_goal && dist_to_goal <= params.arrival_threshold) {
        settled = 1;
    }

    pos += movement * params.delta;

    positions[i] = pos;
    arrivals[i] = settled;

    // --- Combat targeting via spatial grid ---
    let my_faction = factions[i];

    // Dead NPCs can't target
    if (healths[i] <= 0.0) {
        combat_targets[i] = -1;
        return;
    }

    let gw = i32(params.grid_width);
    let gh = i32(params.grid_height);
    let mpc = i32(params.max_per_cell);
    let range_sq = params.combat_range * params.combat_range;
    let search_r = i32(ceil(params.combat_range / params.cell_size)) + 1;

    let my_cx = i32(pos.x / params.cell_size);
    let my_cy = i32(pos.y / params.cell_size);

    var best_dist_sq = range_sq;
    var best_target: i32 = -1;

    for (var dy: i32 = -search_r; dy <= search_r; dy++) {
        for (var dx: i32 = -search_r; dx <= search_r; dx++) {
            let nx = my_cx + dx;
            let ny = my_cy + dy;

            if (nx < 0 || nx >= gw || ny < 0 || ny >= gh) { continue; }

            let cell_idx = ny * gw + nx;
            let count = min(atomicLoad(&grid_counts[cell_idx]), mpc);

            for (var n: i32 = 0; n < count; n++) {
                let other = grid_data[cell_idx * mpc + n];

                if (other < 0 || other == i32(i)) { continue; }
                if (u32(other) >= params.count) { continue; }

                // Same faction = ally, skip
                if (factions[other] == my_faction) { continue; }

                // Dead targets not worth targeting
                if (healths[other] <= 0.0) { continue; }

                let other_pos = positions[other];
                let diff = pos - other_pos;
                let dist_sq = dot(diff, diff);

                if (dist_sq < best_dist_sq) {
                    best_dist_sq = dist_sq;
                    best_target = other;
                }
            }
        }
    }

    combat_targets[i] = best_target;
}
