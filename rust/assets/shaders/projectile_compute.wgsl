// =============================================================================
// Projectile Compute Shader - Movement and Collision Detection
// =============================================================================
// Ported from projectile_compute.glsl.
// Uses NPC spatial grid (read-only) for O(1) collision queries.
// Single dispatch per frame — runs AFTER NPC compute builds the grid.

// PowerShell-style mental model:
// - `global_id.x` is the current index (`$i`) in a parallel loop.
// - Each pass is selected by `params.mode`, similar to `switch ($mode)`.
// - Early `return` behaves like a pipeline filter (`Where-Object`) that drops
//   work items that do not apply.
//
// Per-frame phases:
// - Mode 0: clear projectile grid counters.
// - Mode 1: insert active projectiles into projectile grid.
// - Mode 2: update lifetimes, move projectiles, and detect hits.
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
    entity_count: u32,
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

// Entity buffers (read only — shared from NPC compute pipeline, contains NPCs + buildings)
@group(0) @binding(8)  var<storage, read> entity_positions: array<vec2<f32>>;
@group(0) @binding(9)  var<storage, read> entity_factions: array<i32>;
@group(0) @binding(10) var<storage, read> entity_healths: array<f32>;

// Spatial grid (read only — built by NPC compute modes 0+1)
@group(0) @binding(11) var<storage, read> grid_counts: array<i32>;
@group(0) @binding(12) var<storage, read> grid_data: array<i32>;

// Uniform params
@group(0) @binding(13) var<uniform> params: ProjParams;

// Projectile spatial grid (read_write — built by modes 0+1, read by NPC compute)
@group(0) @binding(14) var<storage, read_write> proj_grid_counts: array<atomic<i32>>;
@group(0) @binding(15) var<storage, read_write> proj_grid_data: array<i32>;

// Per-entity hitbox half-sizes (read only — Minkowski sum with projectile hitbox)
@group(0) @binding(16) var<storage, read> entity_half_sizes: array<vec2<f32>>;

// Per-entity flags (read only — bit 2 = UNTARGETABLE, skip collision)
@group(0) @binding(17) var<storage, read> entity_flags: array<u32>;

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let i = global_id.x;
    // All projectile arrays are parallel arrays indexed by this same `i`.

    // MODE 0: Clear projectile spatial grid.
    // One invocation clears one cell counter with an atomic store.
    if (params.mode == 0u) {
        let grid_cells = params.grid_width * params.grid_height;
        if (i >= grid_cells) { return; }
        atomicStore(&proj_grid_counts[i], 0);
        return;
    }

    // MODE 1: Build projectile spatial grid.
    // Keep active/visible projectiles, map world position to a grid cell, then
    // atomically reserve a slot in that cell list.
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

    // MODE 2: Main gameplay path for projectiles.
    // Sequence: lifetime tick -> move -> local candidate scan -> hit test.
    if (i >= params.proj_count) { return; }
    if (proj_active[i] == 0) { return; }

    // Lifetime countdown by frame delta time.
    var lifetime = proj_lifetimes[i] - params.delta;
    proj_lifetimes[i] = lifetime;

    if (lifetime <= 0.0) {
        // Expired — deactivate, hide, and signal CPU for slot recycling
        proj_active[i] = 0;
        proj_positions[i] = vec2<f32>(-9999.0, -9999.0);
        proj_hits[i] = vec2<i32>(-2, 0);  // Sentinel consumed by CPU cleanup.
        return;
    }

    // Integrate position with simple Euler step.
    var pos = proj_positions[i];
    let vel = proj_velocities[i];
    pos += vel * params.delta;
    proj_positions[i] = pos;

    // Hit already recorded in a previous frame; do not re-hit.
    if (proj_hits[i].x >= 0) { return; }

    // Oriented rectangle collision: arrow is long along velocity, thin perpendicular
    let my_faction = proj_factions[i];
    let speed_sq = dot(vel, vel);

    // Derive local axes from velocity:
    // - forward axis along travel direction
    // - perpendicular axis for width checks
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

    // Broad phase: check current cell plus 8 neighboring cells.
    let gw = i32(params.grid_width);
    let gh = i32(params.grid_height);
    let mpc = i32(params.max_per_cell);
    let ec = i32(params.entity_count);

    for (var dy: i32 = -1; dy <= 1; dy++) {
        for (var dx: i32 = -1; dx <= 1; dx++) {
            let nx = cx + dx;
            let ny = cy + dy;

            if (nx < 0 || nx >= gw) { continue; }
            if (ny < 0 || ny >= gh) { continue; }

            let cell_idx = ny * gw + nx;
            let count = min(grid_counts[cell_idx], mpc);

            for (var n: i32 = 0; n < count; n++) {
                let entity_idx = grid_data[cell_idx * mpc + n];

                if (entity_idx < 0 || entity_idx >= ec) { continue; }

                // Ignore shooter to prevent immediate self-hit.
                if (entity_idx == proj_shooters[i]) { continue; }

                // Ignore same faction and neutral entities.
                if (entity_factions[entity_idx] == my_faction || entity_factions[entity_idx] == -1) { continue; }

                // Ignore dead entities.
                if (entity_healths[entity_idx] <= 0.0) { continue; }

                // Ignore untargetable entities (roads).
                if ((entity_flags[entity_idx] & 4u) != 0u) { continue; }

                let entity_pos = entity_positions[entity_idx];
                let diff = entity_pos - pos;

                // Minkowski-style expand: treat target center as point against an
                // expanded projectile box (projectile half-size + entity half-size).
                let ehs = entity_half_sizes[entity_idx];
                let total_half_len = params.hit_half_length + ehs.x;
                let total_half_wid = params.hit_half_width + ehs.y;

                var hit = false;
                if (use_oriented) {
                    // Narrow phase: project delta vector to forward/perp axes.
                    let along = abs(dot(diff, fwd));
                    let across = abs(dot(diff, perp));
                    hit = along < total_half_len && across < total_half_wid;
                } else {
                    // Fallback when direction is too small to orient reliably.
                    let r = total_half_len;
                    hit = dot(diff, diff) < r * r;
                }

                if (hit) {
                    // HIT — record target index (NPC or building), deactivate, hide
                    proj_hits[i] = vec2<i32>(entity_idx, 0);  // 0 = not processed yet
                    proj_active[i] = 0;
                    proj_positions[i] = vec2<f32>(-9999.0, -9999.0);
                    return;
                }
            }
        }
    }
}
