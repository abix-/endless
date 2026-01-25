#[compute]
#version 450

layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;

// Input buffers
layout(set = 0, binding = 0, std430) buffer PositionBuffer {
    vec2 positions[];
};

layout(set = 0, binding = 1, std430) restrict readonly buffer VelocityBuffer {
    vec2 velocities[];
};

layout(set = 0, binding = 2, std430) restrict readonly buffer GridCounts {
    int grid_counts[];
};

layout(set = 0, binding = 3, std430) restrict readonly buffer GridData {
    int grid_data[];
};

// Output: MultiMesh buffer (Transform2D + Color per instance)
// Format: [a.x, b.x, 0, origin.x, a.y, b.y, 0, origin.y, r, g, b, a] = 12 floats
layout(set = 0, binding = 4, std430) restrict writeonly buffer MultiMeshBuffer {
    float multimesh_data[];
};

layout(push_constant) uniform PushConstants {
    uint npc_count;
    float separation_radius;
    float separation_strength;
    float delta;
    uint grid_width;
    uint grid_height;
    float cell_size;
    uint max_per_cell;
    float world_size;
    float damping;
} params;

void main() {
    uint i = gl_GlobalInvocationID.x;
    if (i >= params.npc_count) return;

    vec2 pos = positions[i];
    vec2 vel = velocities[i];

    // Compute separation force
    vec2 sep = vec2(0.0);
    float sep_radius_sq = params.separation_radius * params.separation_radius;

    int cx = clamp(int(pos.x / params.cell_size), 0, int(params.grid_width) - 1);
    int cy = clamp(int(pos.y / params.cell_size), 0, int(params.grid_height) - 1);

    // Check 3x3 neighboring cells
    for (int dy = -1; dy <= 1; dy++) {
        int ny = cy + dy;
        if (ny < 0 || ny >= int(params.grid_height)) continue;

        for (int dx = -1; dx <= 1; dx++) {
            int nx = cx + dx;
            if (nx < 0 || nx >= int(params.grid_width)) continue;

            int cell_idx = ny * int(params.grid_width) + nx;
            int cell_count = grid_counts[cell_idx];
            int cell_base = cell_idx * int(params.max_per_cell);

            for (int n = 0; n < cell_count; n++) {
                uint j = uint(grid_data[cell_base + n]);
                if (j == i) continue;

                vec2 other_pos = positions[j];
                vec2 diff = pos - other_pos;
                float dist_sq = dot(diff, diff);

                if (dist_sq < sep_radius_sq && dist_sq > 0.01) {
                    float dist = sqrt(dist_sq);
                    float overlap = params.separation_radius - dist;
                    float inv_dist = 1.0 / dist;
                    sep += diff * inv_dist * overlap * 0.5;
                }
            }
        }
    }

    // Normalize and apply strength
    float sep_len = length(sep);
    if (sep_len > 0.0) {
        sep = (sep / sep_len) * params.separation_strength;
    }

    // Apply velocity + separation
    pos += (vel + sep) * params.delta;

    // Wrap world edges
    if (pos.x < 0.0) pos.x += params.world_size;
    if (pos.x > params.world_size) pos.x -= params.world_size;
    if (pos.y < 0.0) pos.y += params.world_size;
    if (pos.y > params.world_size) pos.y -= params.world_size;

    // Update position buffer for next frame
    positions[i] = pos;

    // Write to MultiMesh buffer (12 floats per instance)
    uint base = i * 12;
    // Transform2D: [a.x, b.x, 0, origin.x, a.y, b.y, 0, origin.y]
    multimesh_data[base + 0] = 1.0;    // a.x (scale x)
    multimesh_data[base + 1] = 0.0;    // b.x
    multimesh_data[base + 2] = 0.0;    // padding
    multimesh_data[base + 3] = pos.x;  // origin.x
    multimesh_data[base + 4] = 0.0;    // a.y
    multimesh_data[base + 5] = 1.0;    // b.y (scale y)
    multimesh_data[base + 6] = 0.0;    // padding
    multimesh_data[base + 7] = pos.y;  // origin.y
    // Color: green
    multimesh_data[base + 8] = 0.2;    // r
    multimesh_data[base + 9] = 0.8;    // g
    multimesh_data[base + 10] = 0.2;   // b
    multimesh_data[base + 11] = 1.0;   // a
}
