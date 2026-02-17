// =============================================================================
// Projectile Compute Shader - Movement and Collision Detection
// =============================================================================
// Ported from projectile_compute.glsl.
// Uses NPC spatial grid (read-only) for O(1) collision queries.
// Single dispatch per frame — runs AFTER NPC compute builds the grid.

struct ProjParams {
    proj_count: u32,
    npc_count: u32,
    delta: f32,
    hit_half_length: f32,
    hit_half_width: f32,
    grid_width: u32,
    grid_height: u32,
    cell_size: f32,
    max_per_cell: u32,
    mode: u32,
}

// Projectile buffers (read_write)
@group(0) @binding(0) var<storage, read_write> proj_positions: array<vec2<f32>>;
@group(0) @binding(1) var<storage, read_write> proj_velocities: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read_write> proj_damages: array<f32>;
@group(0) @binding(3) var<storage, read_write> proj_factions: array<i32>;
@group(0) @binding(4) var<storage, read_write> proj_shooters: array<i32>;
@group(0) @binding(5) var<storage, read_write> proj_lifetimes: array<f32>;
@group(0) @binding(6) var<storage, read_write> proj_active: array<i32>;
@group(0) @binding(7) var<storage, read_write> proj_hits: array<vec2<i32>>;

// NPC buffers (read only — shared from NPC compute pipeline)
@group(0) @binding(8)  var<storage, read> npc_positions: array<vec2<f32>>;
@group(0) @binding(9)  var<storage, read> npc_factions: array<i32>;
@group(0) @binding(10) var<storage, read> npc_healths: array<f32>;

// Spatial grid (read only — built by NPC compute modes 0+1)
@group(0) @binding(11) var<storage, read> grid_counts: array<i32>;
@group(0) @binding(12) var<storage, read> grid_data: array<i32>;

// Uniform params
@group(0) @binding(13) var<uniform> params: ProjParams;

// Projectile spatial grid (read_write — built by modes 0+1, read by NPC compute)
@group(0) @binding(14) var<storage, read_write> proj_grid_counts: array<atomic<i32>>;
@group(0) @binding(15) var<storage, read_write> proj_grid_data: array<i32>;

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let i = global_id.x;

    // =========================================================================
    // MODE 0: Clear projectile spatial grid
    // =========================================================================
    if (params.mode == 0u) {
        let grid_cells = params.grid_width * params.grid_height;
        if (i >= grid_cells) { return; }
        atomicStore(&proj_grid_counts[i], 0);
        return;
    }

    // =========================================================================
    // MODE 1: Build projectile spatial grid (insert active projectiles)
    // =========================================================================
    if (params.mode == 1u) {
        if (i >= params.proj_count) { return; }
        if (proj_active[i] == 0) { return; }
        let p = proj_positions[i];
        if (p.x < -9000.0) { return; }
        let pcx = clamp(i32(p.x / params.cell_size), 0, i32(params.grid_width) - 1);
        let pcy = clamp(i32(p.y / params.cell_size), 0, i32(params.grid_height) - 1);
        let cell_idx = pcy * i32(params.grid_width) + pcx;
        let slot = atomicAdd(&proj_grid_counts[cell_idx], 1);
        if (slot < i32(params.max_per_cell)) {
            proj_grid_data[cell_idx * i32(params.max_per_cell) + slot] = i32(i);
        }
        return;
    }

    // =========================================================================
    // MODE 2: Movement + Collision Detection
    // =========================================================================
    if (i >= params.proj_count) { return; }
    if (proj_active[i] == 0) { return; }

    // Decrement lifetime
    var lifetime = proj_lifetimes[i] - params.delta;
    proj_lifetimes[i] = lifetime;

    if (lifetime <= 0.0) {
        // Expired — deactivate, hide, and signal CPU for slot recycling
        proj_active[i] = 0;
        proj_positions[i] = vec2<f32>(-9999.0, -9999.0);
        proj_hits[i] = vec2<i32>(-2, 0);  // -2 = expired sentinel
        return;
    }

    // Move projectile
    var pos = proj_positions[i];
    let vel = proj_velocities[i];
    pos += vel * params.delta;
    proj_positions[i] = pos;

    // Skip if already hit something
    if (proj_hits[i].x >= 0) { return; }

    // Oriented rectangle collision: arrow is long along velocity, thin perpendicular
    let my_faction = proj_factions[i];
    let speed_sq = dot(vel, vel);

    // Derive arrow axes from velocity (fallback to circle for near-zero velocity)
    var fwd: vec2<f32>;
    var perp: vec2<f32>;
    var use_oriented: bool = speed_sq > 0.001;
    if (use_oriented) {
        let inv_speed = 1.0 / sqrt(speed_sq);
        fwd = vel * inv_speed;
        perp = vec2<f32>(-fwd.y, fwd.x);
    }

    // Get grid cell
    let cx = i32(pos.x / params.cell_size);
    let cy = i32(pos.y / params.cell_size);

    // Bounds check
    if (cx < 0 || cx >= i32(params.grid_width)) { return; }
    if (cy < 0 || cy >= i32(params.grid_height)) { return; }

    // Check 3x3 neighborhood
    let gw = i32(params.grid_width);
    let gh = i32(params.grid_height);
    let mpc = i32(params.max_per_cell);
    let nc = i32(params.npc_count);

    for (var dy: i32 = -1; dy <= 1; dy++) {
        for (var dx: i32 = -1; dx <= 1; dx++) {
            let nx = cx + dx;
            let ny = cy + dy;

            if (nx < 0 || nx >= gw) { continue; }
            if (ny < 0 || ny >= gh) { continue; }

            let cell_idx = ny * gw + nx;
            let count = min(grid_counts[cell_idx], mpc);

            for (var n: i32 = 0; n < count; n++) {
                let npc_idx = grid_data[cell_idx * mpc + n];

                if (npc_idx < 0 || npc_idx >= nc) { continue; }

                // Skip same faction (no friendly fire)
                if (npc_factions[npc_idx] == my_faction || npc_factions[npc_idx] == -1) { continue; }

                // Skip dead NPCs
                if (npc_healths[npc_idx] <= 0.0) { continue; }

                let npc_pos = npc_positions[npc_idx];
                let diff = npc_pos - pos;

                var hit = false;
                if (use_oriented) {
                    // Project onto arrow axes
                    let along = abs(dot(diff, fwd));
                    let across = abs(dot(diff, perp));
                    hit = along < params.hit_half_length && across < params.hit_half_width;
                } else {
                    // Fallback: circle with average radius
                    let r = params.hit_half_length;
                    hit = dot(diff, diff) < r * r;
                }

                if (hit) {
                    // HIT — record target, deactivate, hide
                    proj_hits[i] = vec2<i32>(npc_idx, 0);  // 0 = not processed yet
                    proj_active[i] = 0;
                    proj_positions[i] = vec2<f32>(-9999.0, -9999.0);
                    return;
                }
            }
        }
    }
}
