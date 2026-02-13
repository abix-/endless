// =============================================================================
// NPC Compute Shader - GPU-Accelerated Physics
// =============================================================================
// 3-pass dispatch per frame:
//   Mode 0: Clear spatial grid
//   Mode 1: Build spatial grid (insert NPCs into cells)
//   Mode 2: Separation + movement + combat targeting via grid

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
    proj_max_per_cell: u32,
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

// Projectile spatial grid + data (read only — built by projectile compute modes 0+1)
@group(0) @binding(11) var<storage, read> proj_grid_counts: array<i32>;
@group(0) @binding(12) var<storage, read> proj_grid_data: array<i32>;
@group(0) @binding(13) var<storage, read> proj_positions: array<vec2<f32>>;
@group(0) @binding(14) var<storage, read> proj_velocities: array<vec2<f32>>;
@group(0) @binding(15) var<storage, read> proj_factions: array<i32>;

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
    // MODE 2: Separation + Movement + Combat Targeting
    // =========================================================================
    if (i >= params.count) { return; }

    var pos = positions[i];
    let goal = goals[i];
    let speed = speeds[i];
    var settled = arrivals[i];
    var my_backoff = backoff[i];

    // Skip dead/hidden NPCs
    if (pos.x < -9000.0) {
        combat_targets[i] = -1;
        return;
    }

    let to_goal = goal - pos;
    let dist_to_goal = length(to_goal);
    let wants_goal = settled == 0;

    // Grid constants (shared by separation + combat targeting)
    let gw = i32(params.grid_width);
    let gh = i32(params.grid_height);
    let mpc = i32(params.max_per_cell);

    // --- STEP 2: Separation + dodge (single grid scan) ---
    // Separation pushes away from overlapping neighbors.
    // Dodge steers sideways around approaching moving NPCs.
    // Same-faction NPCs repel more strongly to prevent convoy clumping.
    var avoidance = vec2<f32>(0.0, 0.0);
    var dodge = vec2<f32>(0.0, 0.0);
    let sep_radius_sq = params.separation_radius * params.separation_radius;
    let approach_radius = params.separation_radius * 2.0;
    let approach_radius_sq = approach_radius * approach_radius;
    let my_faction = factions[i];

    // Pre-compute goal direction for dodge (only if moving toward goal)
    let is_moving = wants_goal && dist_to_goal > params.arrival_threshold;
    var my_dir = vec2<f32>(0.0, 0.0);
    if (is_moving) {
        my_dir = normalize(to_goal);
    }

    let cx = clamp(i32(pos.x / params.cell_size), 0, gw - 1);
    let cy = clamp(i32(pos.y / params.cell_size), 0, gh - 1);

    for (var dy: i32 = -1; dy <= 1; dy++) {
        let ny = cy + dy;
        if (ny < 0 || ny >= gh) { continue; }

        for (var dx: i32 = -1; dx <= 1; dx++) {
            let nx = cx + dx;
            if (nx < 0 || nx >= gw) { continue; }

            let cell_idx = ny * gw + nx;
            let cell_count = min(atomicLoad(&grid_counts[cell_idx]), mpc);
            let cell_base = cell_idx * mpc;

            for (var n: i32 = 0; n < cell_count; n++) {
                let j = grid_data[cell_base + n];
                if (j == i32(i)) { continue; }
                if (j < 0 || u32(j) >= params.count) { continue; }

                let other_pos = positions[j];
                var diff = pos - other_pos;
                let dist_sq = dot(diff, diff);
                let neighbor_settled = arrivals[j];

                // --- Separation force ---
                if (dist_sq < sep_radius_sq) {
                    var push_strength = 1.0;
                    if (settled == 0 && neighbor_settled == 1) {
                        push_strength = 0.2;  // I'm moving, they're settled: barely block me
                    } else if (settled == 1 && neighbor_settled == 0) {
                        push_strength = 2.0;  // I'm settled, they're moving: shove me aside
                    }

                    // Same-faction boost: spread out when heading to same area
                    if (factions[j] == my_faction) {
                        push_strength *= 1.5;
                    }

                    if (dist_sq < 0.0001) {
                        let angle = f32(i) * 2.399 + f32(j) * 0.7;
                        diff = vec2<f32>(cos(angle), sin(angle));
                        avoidance += diff * params.separation_radius * push_strength;
                    } else {
                        let dist = sqrt(dist_sq);
                        let overlap = params.separation_radius - dist;
                        avoidance += diff * (overlap / dist) * push_strength;
                    }
                }

                // --- Dodge: steer sideways around approaching movers ---
                if (is_moving && neighbor_settled == 0 && dist_sq < approach_radius_sq && dist_sq > 0.0001) {
                    let dist2 = sqrt(dist_sq);
                    let to_other = -diff / dist2;
                    let i_approach = dot(my_dir, to_other);

                    if (i_approach > 0.3) {
                        let other_goal = goals[j];
                        let ot = other_goal - other_pos;
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

                            if (i < u32(j)) {
                                dodge += perp * dodge_strength;
                            } else {
                                dodge -= perp * dodge_strength;
                            }
                        }
                    }
                }
            }
        }
    }

    avoidance *= params.separation_strength;

    // Normalize dodge direction, scale to fraction of separation strength
    let dodge_len = length(dodge);
    if (dodge_len > 0.0) {
        dodge = (dodge / dodge_len) * params.separation_strength * 0.7;
    }
    avoidance += dodge;

    // Clamp total avoidance so it can't wildly overpower movement
    let avoidance_mag = length(avoidance);
    let max_avoidance = speed * 1.5;
    if (avoidance_mag > max_avoidance) {
        avoidance = (avoidance / avoidance_mag) * max_avoidance;
    }

    // --- Projectile dodge: strafe away from incoming arrows (spatial grid) ---
    var proj_dodge = vec2<f32>(0.0, 0.0);
    let dodge_radius = 60.0;
    let dodge_radius_sq = dodge_radius * dodge_radius;
    let pmpc = i32(params.proj_max_per_cell);

    for (var pdy: i32 = -1; pdy <= 1; pdy++) {
        let pny = cy + pdy;
        if (pny < 0 || pny >= gh) { continue; }
        for (var pdx: i32 = -1; pdx <= 1; pdx++) {
            let pnx = cx + pdx;
            if (pnx < 0 || pnx >= gw) { continue; }
            let pcell = pny * gw + pnx;
            let pcount = min(proj_grid_counts[pcell], pmpc);
            let pbase = pcell * pmpc;
            for (var pn: i32 = 0; pn < pcount; pn++) {
                let pi = proj_grid_data[pbase + pn];
                if (pi < 0) { continue; }
                if (proj_factions[pi] == my_faction) { continue; }

                let pp = proj_positions[pi];
                let pv = proj_velocities[pi];
                let to_me = pos - pp;
                let dist_sq = dot(to_me, to_me);
                if (dist_sq > dodge_radius_sq || dist_sq < 1.0) { continue; }

                // Is projectile heading toward me?
                let pspd_sq = dot(pv, pv);
                if (pspd_sq < 1.0) { continue; }
                let pdir = pv / sqrt(pspd_sq);
                let approach = dot(pdir, to_me / sqrt(dist_sq));
                if (approach < 0.3) { continue; }

                // Dodge perpendicular to projectile direction
                let pperp = vec2<f32>(-pdir.y, pdir.x);
                let pside = dot(to_me, pperp);
                let ddir = select(-1.0, 1.0, pside >= 0.0);
                let urgency = 1.0 - sqrt(dist_sq) / dodge_radius;
                proj_dodge += pperp * ddir * urgency;
            }
        }
    }
    let pdlen = length(proj_dodge);
    if (pdlen > 0.0) {
        proj_dodge = (proj_dodge / pdlen) * speed * 1.5;
    }

    // --- STEP 3: Movement toward goal + lateral steering when blocked ---
    // Instead of slowing down when blocked, steer sideways to route around.
    var movement = vec2<f32>(0.0, 0.0);
    if (is_moving) {
        // Full-speed forward movement (no backoff persistence penalty)
        movement = my_dir * speed;

        // Lateral steering: when avoidance pushes us away from goal, steer sideways
        if (avoidance_mag > 0.1) {
            let push_dir = avoidance / avoidance_mag;
            let alignment = dot(push_dir, my_dir);

            if (alignment < -0.3) {
                // Blocked: steer perpendicular to goal, in the direction avoidance is already pushing
                let perp = vec2<f32>(-my_dir.y, my_dir.x);
                let side = dot(avoidance, perp);  // Which side has more space?
                let lateral_dir = select(-1.0, 1.0, side >= 0.0);
                movement += perp * lateral_dir * speed * 0.6;

                my_backoff += 1;
            } else {
                my_backoff = max(0, my_backoff - 3);
            }
        } else {
            my_backoff = max(0, my_backoff - 3);
        }
        my_backoff = min(my_backoff, 30);
    } else if (wants_goal && dist_to_goal <= params.arrival_threshold) {
        settled = 1;
    }

    // --- STEP 4: Apply movement + avoidance ---
    pos += (movement + avoidance + proj_dodge) * params.delta;

    positions[i] = pos;
    arrivals[i] = settled;
    backoff[i] = my_backoff;

    // --- Combat targeting via spatial grid ---
    // Uses wider search radius than separation (combat_range >> separation_radius)
    // my_faction already computed above for separation

    if (healths[i] <= 0.0) {
        combat_targets[i] = -1;
        return;
    }

    let range_sq = params.combat_range * params.combat_range;
    let search_r = i32(ceil(params.combat_range / params.cell_size)) + 1;

    // Use post-movement position for combat targeting
    let my_cx = i32(pos.x / params.cell_size);
    let my_cy = i32(pos.y / params.cell_size);

    var best_dist_sq = range_sq;
    var best_target: i32 = -1;

    for (var dy3: i32 = -search_r; dy3 <= search_r; dy3++) {
        for (var dx3: i32 = -search_r; dx3 <= search_r; dx3++) {
            let nx3 = my_cx + dx3;
            let ny3 = my_cy + dy3;

            if (nx3 < 0 || nx3 >= gw || ny3 < 0 || ny3 >= gh) { continue; }

            let cell_idx3 = ny3 * gw + nx3;
            let count3 = min(atomicLoad(&grid_counts[cell_idx3]), mpc);

            for (var n3: i32 = 0; n3 < count3; n3++) {
                let other3 = grid_data[cell_idx3 * mpc + n3];

                if (other3 < 0 || other3 == i32(i)) { continue; }
                if (u32(other3) >= params.count) { continue; }

                // Same faction = ally, skip
                if (factions[other3] == my_faction) { continue; }

                // Dead targets not worth targeting
                if (healths[other3] <= 0.0) { continue; }

                let other_pos3 = positions[other3];
                let diff3 = pos - other_pos3;
                let dist_sq3 = dot(diff3, diff3);

                if (dist_sq3 < best_dist_sq) {
                    best_dist_sq = dist_sq3;
                    best_target = other3;
                }
            }
        }
    }

    combat_targets[i] = best_target;
}
