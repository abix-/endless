#[compute]
#version 450

// Projectile compute shader - movement and collision detection
// Uses NPC spatial grid for O(1) collision queries

layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;

// Projectile data buffers
layout(set = 0, binding = 0, std430) buffer ProjPositions { vec2 proj_positions[]; };
layout(set = 0, binding = 1, std430) buffer ProjVelocities { vec2 proj_velocities[]; };
layout(set = 0, binding = 2, std430) buffer ProjDamages { float proj_damages[]; };
layout(set = 0, binding = 3, std430) buffer ProjFactions { int proj_factions[]; };
layout(set = 0, binding = 4, std430) buffer ProjShooters { int proj_shooters[]; };
layout(set = 0, binding = 5, std430) buffer ProjLifetimes { float proj_lifetimes[]; };
layout(set = 0, binding = 6, std430) buffer ProjActive { int proj_active[]; };
layout(set = 0, binding = 7, std430) buffer ProjHits { ivec2 proj_hits[]; };

// NPC data (read-only for collision)
layout(set = 0, binding = 8, std430) readonly buffer NpcPositions { vec2 npc_positions[]; };
layout(set = 0, binding = 9, std430) readonly buffer NpcFactions { int npc_factions[]; };
layout(set = 0, binding = 10, std430) readonly buffer NpcHealths { float npc_healths[]; };

// Spatial grid (reuse NPC grid)
layout(set = 0, binding = 11, std430) readonly buffer GridCounts { int grid_counts[]; };
layout(set = 0, binding = 12, std430) readonly buffer GridData { int grid_data[]; };

// Push constants
layout(push_constant) uniform Params {
    uint proj_count;
    uint npc_count;
    float delta;
    float hit_radius;
    uint grid_width;
    uint grid_height;
    float cell_size;
    uint max_per_cell;
};

void main() {
    uint i = gl_GlobalInvocationID.x;
    if (i >= proj_count) return;
    if (proj_active[i] == 0) return;

    // Decrement lifetime
    float lifetime = proj_lifetimes[i] - delta;
    proj_lifetimes[i] = lifetime;

    if (lifetime <= 0.0) {
        // Expired - deactivate and hide
        proj_active[i] = 0;
        proj_positions[i] = vec2(-9999.0, -9999.0);
        return;
    }

    // Move projectile
    vec2 pos = proj_positions[i];
    vec2 vel = proj_velocities[i];
    pos += vel * delta;
    proj_positions[i] = pos;

    // Skip if already hit something
    if (proj_hits[i].x >= 0) return;

    // Collision detection via spatial grid
    int my_faction = proj_factions[i];
    float hit_radius_sq = hit_radius * hit_radius;

    // Get grid cell
    int cx = int(pos.x / cell_size);
    int cy = int(pos.y / cell_size);

    // Bounds check
    if (cx < 0 || cx >= int(grid_width)) return;
    if (cy < 0 || cy >= int(grid_height)) return;

    // Check 3x3 neighborhood
    for (int dy = -1; dy <= 1; dy++) {
        for (int dx = -1; dx <= 1; dx++) {
            int nx = cx + dx;
            int ny = cy + dy;

            if (nx < 0 || nx >= int(grid_width)) continue;
            if (ny < 0 || ny >= int(grid_height)) continue;

            int cell_idx = ny * int(grid_width) + nx;
            int count = grid_counts[cell_idx];

            for (int n = 0; n < count && n < int(max_per_cell); n++) {
                int npc_idx = grid_data[cell_idx * int(max_per_cell) + n];

                if (npc_idx < 0 || npc_idx >= int(npc_count)) continue;

                // Skip same faction (no friendly fire)
                if (npc_factions[npc_idx] == my_faction) continue;

                // Skip dead NPCs
                if (npc_healths[npc_idx] <= 0.0) continue;

                // Distance check
                vec2 npc_pos = npc_positions[npc_idx];
                vec2 diff = pos - npc_pos;
                float dist_sq = dot(diff, diff);

                if (dist_sq < hit_radius_sq) {
                    // HIT! Record the hit
                    proj_hits[i] = ivec2(npc_idx, 0);  // 0 = not processed yet
                    proj_active[i] = 0;
                    proj_positions[i] = vec2(-9999.0, -9999.0);  // Hide
                    return;
                }
            }
        }
    }
}
