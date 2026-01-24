#[compute]
#version 450

layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;

// Input buffers
layout(set = 0, binding = 0, std430) restrict readonly buffer PositionBuffer {
    vec2 positions[];
};

layout(set = 0, binding = 1, std430) restrict readonly buffer SizeBuffer {
    float sizes[];
};

layout(set = 0, binding = 2, std430) restrict readonly buffer HealthBuffer {
    float healths[];
};

layout(set = 0, binding = 3, std430) restrict writeonly buffer OutputBuffer {
    vec2 separation_velocities[];
};

layout(set = 0, binding = 4, std430) restrict readonly buffer StateBuffer {
    int npc_states[];
};

layout(set = 0, binding = 5, std430) restrict readonly buffer TargetBuffer {
    vec2 npc_targets[];
};

// Grid buffers
layout(set = 0, binding = 6, std430) restrict readonly buffer GridCounts {
    int grid_counts[];
};

layout(set = 0, binding = 7, std430) restrict readonly buffer GridData {
    int grid_data[];
};

layout(push_constant) uniform PushConstants {
    uint npc_count;
    float separation_radius;
    float separation_strength;
    uint stationary_mask;
    uint grid_width;
    uint grid_height;
    float cell_size;
    uint max_per_cell;
} params;

void main() {
    uint i = gl_GlobalInvocationID.x;

    if (i >= params.npc_count) return;

    if (healths[i] <= 0.0) {
        separation_velocities[i] = vec2(0.0);
        return;
    }

    vec2 my_pos = positions[i];
    float my_size = max(sizes[i], 1.0);
    float my_radius = params.separation_radius * my_size;

    // Movement direction for TCP dodge
    vec2 my_target = npc_targets[i];
    vec2 to_target = my_target - my_pos;
    float to_target_len = length(to_target);
    vec2 my_dir = vec2(0.0);
    if (to_target_len > 0.001) {
        my_dir = to_target / to_target_len;
    }

    vec2 sep = vec2(0.0);
    vec2 dodge = vec2(0.0);

    // Compute my grid cell
    int cx = clamp(int(my_pos.x / params.cell_size), 0, int(params.grid_width) - 1);
    int cy = clamp(int(my_pos.y / params.cell_size), 0, int(params.grid_height) - 1);

    // Check 3x3 adjacent cells
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
                if (healths[j] <= 0.0) continue;

                vec2 other_pos = positions[j];
                vec2 diff = my_pos - other_pos;
                float dist_sq = dot(diff, diff);
                if (dist_sq <= 0.0) continue;

                float other_size = max(sizes[j], 1.0);
                float combined_radius = (my_radius + params.separation_radius * other_size) * 0.5;
                float combined_radius_sq = combined_radius * combined_radius;

                // Separation force
                if (dist_sq < combined_radius_sq) {
                    int other_state = npc_states[j];
                    bool other_stationary = (params.stationary_mask & (1u << uint(other_state))) != 0u;

                    float push_strength = other_size / my_size;
                    if (other_stationary) {
                        push_strength *= 3.0;
                    }

                    float inv_dist = inversesqrt(dist_sq);
                    float factor = inv_dist * inv_dist * push_strength;
                    sep += diff * factor;
                }

                // TCP-like collision avoidance
                float approach_radius_sq = combined_radius_sq * 4.0;
                if (dist_sq < approach_radius_sq) {
                    int other_state = npc_states[j];
                    bool other_stationary = (params.stationary_mask & (1u << uint(other_state))) != 0u;

                    if (!other_stationary) {
                        float dist = sqrt(dist_sq);
                        vec2 to_other = -diff / dist;

                        float i_approach = dot(my_dir, to_other);
                        if (i_approach > 0.3) {
                            vec2 other_target = npc_targets[j];
                            vec2 ot = other_target - other_pos;
                            float ot_len = length(ot);

                            if (ot_len > 0.001) {
                                vec2 other_dir = ot / ot_len;
                                float they_approach = -dot(other_dir, to_other);

                                vec2 perp = vec2(-my_dir.y, my_dir.x);
                                float dodge_strength = 0.4;

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
        }
    }

    // Normalize and apply strength
    vec2 final_vel = vec2(0.0);

    float sep_len_sq = dot(sep, sep);
    if (sep_len_sq > 0.0) {
        float sep_len = sqrt(sep_len_sq);
        final_vel = (sep / sep_len) * params.separation_strength;
    }

    float dodge_len_sq = dot(dodge, dodge);
    if (dodge_len_sq > 0.0) {
        float dodge_len = sqrt(dodge_len_sq);
        float dodge_mag = params.separation_strength * 0.7;
        final_vel += (dodge / dodge_len) * dodge_mag;
    }

    separation_velocities[i] = final_vel;
}
