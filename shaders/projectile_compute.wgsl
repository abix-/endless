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
    hit_radius: f32,
    grid_width: u32,
    grid_height: u32,
    cell_size: f32,
    max_per_cell: u32,
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

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let i = global_id.x;
    if (i >= params.proj_count) { return; }
    if (proj_active[i] == 0) { return; }

    // Decrement lifetime
    var lifetime = proj_lifetimes[i] - params.delta;
    proj_lifetimes[i] = lifetime;

    if (lifetime <= 0.0) {
        // Expired — deactivate and hide
        proj_active[i] = 0;
        proj_positions[i] = vec2<f32>(-9999.0, -9999.0);
        return;
    }

    // Move projectile
    var pos = proj_positions[i];
    let vel = proj_velocities[i];
    pos += vel * params.delta;
    proj_positions[i] = pos;

    // Skip if already hit something
    if (proj_hits[i].x >= 0) { return; }

    // Collision detection via spatial grid
    let my_faction = proj_factions[i];
    let hit_radius_sq = params.hit_radius * params.hit_radius;

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
                if (npc_factions[npc_idx] == my_faction) { continue; }

                // Skip dead NPCs
                if (npc_healths[npc_idx] <= 0.0) { continue; }

                // Distance check
                let npc_pos = npc_positions[npc_idx];
                let diff = pos - npc_pos;
                let dist_sq = dot(diff, diff);

                if (dist_sq < hit_radius_sq) {
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
