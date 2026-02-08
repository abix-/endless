// =============================================================================
// NPC Compute Shader - GPU-Accelerated Physics for Thousands of NPCs
// =============================================================================
//
// WGSL port of npc_compute.glsl. Runs once per NPC per frame, in parallel.
// Each invocation: reads neighbors, calculates forces, writes position.
//
// Key concepts:
// - Spatial Grid: World divided into cells. Each NPC only checks 3x3 neighborhood.
// - Separation: Boids-style force pushing NPCs apart when too close.
// - TCP Backoff: When blocked, NPCs slow down and eventually give up.

// =============================================================================
// GPU BUFFERS
// =============================================================================

// Binding 0: Position buffer (read/write - GPU owns authoritative positions)
@group(0) @binding(0) var<storage, read_write> positions: array<vec2<f32>>;

// Binding 1: Target buffer (read-only - set by CPU when set_target() called)
@group(0) @binding(1) var<storage, read> targets: array<vec2<f32>>;

// Binding 2: Color buffer (read-only - alpha > 0 means seeking)
@group(0) @binding(2) var<storage, read> colors: array<vec4<f32>>;

// Binding 3: Speed buffer (read-only - movement speed in pixels/second)
@group(0) @binding(3) var<storage, read> speeds: array<f32>;

// Binding 4: Grid counts (read/write - how many NPCs in each cell)
@group(0) @binding(4) var<storage, read_write> grid_counts: array<atomic<i32>>;

// Binding 5: Grid data (read/write - which NPCs are in each cell)
@group(0) @binding(5) var<storage, read_write> grid_data: array<i32>;

// Binding 6: Arrival flags (read/write - 1 = arrived or gave up)
@group(0) @binding(6) var<storage, read_write> arrivals: array<i32>;

// Binding 7: Backoff counters (read/write - TCP-style collision avoidance)
@group(0) @binding(7) var<storage, read_write> backoff: array<i32>;

// Binding 8: Faction buffer (read-only - 0=Villager, 1=Raider)
@group(0) @binding(8) var<storage, read> factions: array<i32>;

// Binding 9: Health buffer (read-only - current HP)
@group(0) @binding(9) var<storage, read> healths: array<f32>;

// Binding 10: Combat target buffer (write - output for CPU)
@group(0) @binding(10) var<storage, read_write> combat_targets: array<i32>;

// =============================================================================
// UNIFORM PARAMETERS
// =============================================================================

struct Params {
    count: u32,              // grid_cells (mode 0) or npc_count (mode 1,2)
    separation_radius: f32,  // Minimum distance between NPCs (20px)
    separation_strength: f32,// How hard NPCs push apart (100.0)
    delta: f32,              // Frame delta time in seconds
    grid_width: u32,         // Spatial grid width (128)
    grid_height: u32,        // Spatial grid height (128)
    cell_size: f32,          // Size of each grid cell (64px)
    max_per_cell: u32,       // Max NPCs per cell (48)
    arrival_threshold: f32,  // Distance to count as "arrived" (8px)
    mode: u32,               // 0=clear grid, 1=insert NPCs, 2=main logic
}

@group(0) @binding(11) var<uniform> params: Params;

// Non-atomic read of grid counts (for mode 2 after grid is built)
@group(0) @binding(12) var<storage, read> grid_counts_read: array<i32>;

// =============================================================================
// MAIN SHADER LOGIC
// =============================================================================

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let i = global_id.x;

    // =========================================================================
    // MODE 0: CLEAR GRID - One thread per cell, reset counts to 0
    // =========================================================================
    if (params.mode == 0u) {
        if (i >= params.count) { return; }
        atomicStore(&grid_counts[i], 0);
        return;
    }

    // =========================================================================
    // MODE 1: INSERT NPCs - One thread per NPC, atomically insert into grid
    // =========================================================================
    if (params.mode == 1u) {
        if (i >= params.count) { return; }

        let pos = positions[i];

        // Skip dead/hidden NPCs (position at -9999)
        if (pos.x < -9000.0) { return; }

        // Calculate cell
        let cx = clamp(i32(pos.x / params.cell_size), 0, i32(params.grid_width) - 1);
        let cy = clamp(i32(pos.y / params.cell_size), 0, i32(params.grid_height) - 1);
        let cell_idx = cy * i32(params.grid_width) + cx;

        // Atomically grab a slot in this cell
        let slot = atomicAdd(&grid_counts[cell_idx], 1);

        // Write NPC index if slot available
        if (slot < i32(params.max_per_cell)) {
            grid_data[cell_idx * i32(params.max_per_cell) + slot] = i32(i);
        }
        return;
    }

    // =========================================================================
    // MODE 2: MAIN NPC LOGIC
    // =========================================================================
    if (i >= params.count) { return; }

    // =========================================================================
    // STEP 1: READ CURRENT STATE
    // =========================================================================

    var pos = positions[i];
    let target = targets[i];
    let speed = speeds[i];
    var settled = arrivals[i];
    var my_backoff = backoff[i];

    let to_target = target - pos;
    let dist_to_target = length(to_target);

    let wants_target = settled == 0;

    // =========================================================================
    // STEP 2: CALCULATE AVOIDANCE FORCE (push away from neighbors)
    // =========================================================================

    var avoidance = vec2<f32>(0.0, 0.0);
    let sep_radius_sq = params.separation_radius * params.separation_radius;

    // Calculate which grid cell we're in
    let cx = clamp(i32(pos.x / params.cell_size), 0, i32(params.grid_width) - 1);
    let cy = clamp(i32(pos.y / params.cell_size), 0, i32(params.grid_height) - 1);

    // Check 3x3 neighborhood of cells
    for (var dy: i32 = -1; dy <= 1; dy++) {
        let ny = cy + dy;
        if (ny < 0 || ny >= i32(params.grid_height)) { continue; }

        for (var dx: i32 = -1; dx <= 1; dx++) {
            let nx = cx + dx;
            if (nx < 0 || nx >= i32(params.grid_width)) { continue; }

            let cell_idx = ny * i32(params.grid_width) + nx;
            let cell_count = grid_counts_read[cell_idx];
            let cell_base = cell_idx * i32(params.max_per_cell);

            for (var n: i32 = 0; n < cell_count; n++) {
                let j = u32(grid_data[cell_base + n]);
                if (j == i) { continue; }

                let other_pos = positions[j];
                var diff = pos - other_pos;
                let dist_sq = dot(diff, diff);

                if (dist_sq < sep_radius_sq) {
                    let neighbor_settled = arrivals[j];

                    var push_strength = 1.0;
                    if (settled == 0 && neighbor_settled == 1) {
                        push_strength = 0.2;
                    } else if (settled == 1 && neighbor_settled == 0) {
                        push_strength = 2.0;
                    }

                    if (dist_sq < 0.0001) {
                        // NPCs exactly on top of each other
                        let angle = f32(i) * 2.399 + f32(j) * 0.7;
                        diff = vec2<f32>(cos(angle), sin(angle));
                        avoidance += diff * params.separation_radius * push_strength;
                    } else {
                        let dist = sqrt(dist_sq);
                        let overlap = params.separation_radius - dist;
                        avoidance += diff * (overlap / dist) * push_strength;
                    }
                }
            }
        }
    }

    avoidance *= params.separation_strength;

    // =========================================================================
    // STEP 2b: TCP-STYLE DODGE
    // =========================================================================

    var dodge = vec2<f32>(0.0, 0.0);
    if (wants_target && dist_to_target > params.arrival_threshold) {
        let my_dir = normalize(to_target);

        for (var dy: i32 = -1; dy <= 1; dy++) {
            let ny = cy + dy;
            if (ny < 0 || ny >= i32(params.grid_height)) { continue; }

            for (var dx: i32 = -1; dx <= 1; dx++) {
                let nx = cx + dx;
                if (nx < 0 || nx >= i32(params.grid_width)) { continue; }

                let cell_idx = ny * i32(params.grid_width) + nx;
                let cell_count = grid_counts_read[cell_idx];
                let cell_base = cell_idx * i32(params.max_per_cell);

                for (var n: i32 = 0; n < cell_count; n++) {
                    let j = u32(grid_data[cell_base + n]);
                    if (j == i) { continue; }

                    let neighbor_settled = arrivals[j];
                    if (neighbor_settled == 1) { continue; }

                    let other_pos = positions[j];
                    let diff = pos - other_pos;
                    let dist_sq = dot(diff, diff);

                    let approach_radius = params.separation_radius * 2.0;
                    if (dist_sq > approach_radius * approach_radius) { continue; }
                    if (dist_sq < 0.0001) { continue; }

                    let dist = sqrt(dist_sq);
                    let to_other = -diff / dist;

                    let i_approach = dot(my_dir, to_other);
                    if (i_approach > 0.3) {
                        let other_target = targets[j];
                        let ot = other_target - other_pos;
                        let ot_len = length(ot);

                        if (ot_len > 0.001) {
                            let other_dir = ot / ot_len;
                            let they_approach = -dot(other_dir, to_other);

                            let perp = vec2<f32>(-my_dir.y, my_dir.x);
                            var dodge_strength = 0.4;

                            if (they_approach > 0.3) {
                                dodge_strength = 0.5;
                            } else if (they_approach < -0.3) {
                                dodge_strength = 0.3;
                            }

                            if (i < j) {
                                dodge += perp * dodge_strength;
                            } else {
                                dodge -= perp * dodge_strength;
                            }
                        }
                    }
                }
            }
        }

        let dodge_len = length(dodge);
        if (dodge_len > 0.0) {
            dodge = (dodge / dodge_len) * params.separation_strength * 0.7;
        }
    }

    avoidance += dodge;

    // =========================================================================
    // STEP 3: CALCULATE MOVEMENT TOWARD TARGET
    // =========================================================================

    var movement = vec2<f32>(0.0, 0.0);
    if (wants_target && dist_to_target > params.arrival_threshold) {
        let persistence = 1.0 / f32(1 + my_backoff);
        movement = normalize(to_target) * speed * persistence;
    }

    // =========================================================================
    // STEP 4: DETECT BLOCKING AND UPDATE BACKOFF
    // =========================================================================

    let avoidance_mag = length(avoidance);

    if (wants_target && dist_to_target > params.arrival_threshold) {
        if (avoidance_mag > 0.1) {
            let goal_dir = normalize(to_target);
            let push_dir = normalize(avoidance);
            let alignment = dot(push_dir, goal_dir);

            if (alignment < -0.3) {
                my_backoff += 2;
            } else if (alignment > 0.3) {
                my_backoff = max(0, my_backoff - 2);
            }
        } else {
            my_backoff = max(0, my_backoff - 1);
        }

        my_backoff = min(my_backoff, 200);
    } else if (wants_target && dist_to_target <= params.arrival_threshold) {
        settled = 1;
    }

    // =========================================================================
    // STEP 5: APPLY MOVEMENT
    // =========================================================================

    pos += (movement + avoidance) * params.delta;

    // =========================================================================
    // STEP 5b: COMBAT TARGETING
    // =========================================================================

    var best_target: i32 = -1;
    let detect_range = 300.0;
    var best_dist_sq = detect_range * detect_range;

    let my_health = healths[i];
    if (my_health > 0.0) {
        let my_faction = factions[i];

        for (var dy: i32 = -1; dy <= 1; dy++) {
            let ny = cy + dy;
            if (ny < 0 || ny >= i32(params.grid_height)) { continue; }

            for (var dx: i32 = -1; dx <= 1; dx++) {
                let nx = cx + dx;
                if (nx < 0 || nx >= i32(params.grid_width)) { continue; }

                let cell_idx = ny * i32(params.grid_width) + nx;
                let cell_count = grid_counts_read[cell_idx];
                let cell_base = cell_idx * i32(params.max_per_cell);

                for (var n: i32 = 0; n < cell_count; n++) {
                    let j = u32(grid_data[cell_base + n]);
                    if (j == i) { continue; }

                    let other_faction = factions[j];
                    if (other_faction == my_faction) { continue; }

                    let other_health = healths[j];
                    if (other_health <= 0.0) { continue; }

                    let other_pos = positions[j];
                    let diff = pos - other_pos;
                    let dist_sq = dot(diff, diff);

                    if (dist_sq < best_dist_sq) {
                        best_dist_sq = dist_sq;
                        best_target = i32(j);
                    }
                }
            }
        }
    }

    combat_targets[i] = best_target;

    // =========================================================================
    // STEP 6: WRITE OUTPUT
    // =========================================================================

    positions[i] = pos;
    arrivals[i] = settled;
    backoff[i] = my_backoff;
}
