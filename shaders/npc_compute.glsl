#[compute]
#version 450

layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;

// Position buffer (read/write - GPU owns positions)
layout(set = 0, binding = 0, std430) buffer PositionBuffer {
    vec2 positions[];
};

// Target buffer (read - set by Bevy)
layout(set = 0, binding = 1, std430) restrict readonly buffer TargetBuffer {
    vec2 targets[];
};

// Color buffer (read - set by Bevy based on job)
layout(set = 0, binding = 2, std430) restrict readonly buffer ColorBuffer {
    vec4 colors[];
};

// Speed buffer (read - set by Bevy)
layout(set = 0, binding = 3, std430) restrict readonly buffer SpeedBuffer {
    float speeds[];
};

// Spatial grid
layout(set = 0, binding = 4, std430) restrict readonly buffer GridCounts {
    int grid_counts[];
};

layout(set = 0, binding = 5, std430) restrict readonly buffer GridData {
    int grid_data[];
};

// MultiMesh output (write - direct to rendering)
layout(set = 0, binding = 6, std430) restrict writeonly buffer MultiMeshBuffer {
    float multimesh_data[];
};

// Arrival flags (write - read by Bevy to detect arrivals)
layout(set = 0, binding = 7, std430) buffer ArrivalBuffer {
    int arrivals[];
};

// Backoff counter (TCP-style collision avoidance)
layout(set = 0, binding = 8, std430) buffer BackoffBuffer {
    int backoff[];
};

layout(push_constant) uniform PushConstants {
    uint npc_count;          // 0-4
    float separation_radius; // 4-8
    float separation_strength; // 8-12
    float delta;             // 12-16
    uint grid_width;         // 16-20
    uint grid_height;        // 20-24
    float cell_size;         // 24-28
    uint max_per_cell;       // 28-32
    float arrival_threshold; // 32-36
    float _pad1;             // 36-40
    float _pad2;             // 40-44
    float _pad3;             // 44-48
} params;

void main() {
    uint i = gl_GlobalInvocationID.x;
    if (i >= params.npc_count) return;

    // === READ STATE ===
    vec2 pos = positions[i];
    vec2 target = targets[i];
    float speed = speeds[i];
    vec4 color = colors[i];
    int settled = arrivals[i];  // 1 = given up trying to reach target
    int my_backoff = backoff[i];

    vec2 to_target = target - pos;
    float dist_to_target = length(to_target);
    bool wants_target = color.a > 0.0 && settled == 0;

    // === STEP 1: AVOIDANCE (push away from neighbors) ===
    vec2 avoidance = vec2(0.0);
    float sep_radius_sq = params.separation_radius * params.separation_radius;

    int cx = clamp(int(pos.x / params.cell_size), 0, int(params.grid_width) - 1);
    int cy = clamp(int(pos.y / params.cell_size), 0, int(params.grid_height) - 1);

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

                if (dist_sq < sep_radius_sq) {
                    if (dist_sq < 0.0001) {
                        float angle = float(i) * 2.399 + float(j) * 0.7;
                        diff = vec2(cos(angle), sin(angle));
                        avoidance += diff * params.separation_radius;
                    } else {
                        float dist = sqrt(dist_sq);
                        float overlap = params.separation_radius - dist;
                        avoidance += diff * (overlap / dist);
                    }
                }
            }
        }
    }
    avoidance *= params.separation_strength;

    // === STEP 2: MOVEMENT (toward target, reduced by backoff) ===
    vec2 movement = vec2(0.0);
    if (wants_target && dist_to_target > params.arrival_threshold) {
        float persistence = 1.0 / float(1 + my_backoff);
        movement = normalize(to_target) * speed * persistence;
    }

    // === STEP 3: DETECT BLOCKING ===
    if (wants_target && length(avoidance) > 0.0 && dist_to_target > params.arrival_threshold) {
        vec2 goal_dir = normalize(to_target);
        vec2 push_dir = normalize(avoidance);
        float blocked = dot(push_dir, goal_dir);

        if (blocked < -0.2) {
            my_backoff++;
            if (my_backoff > 120) {
                settled = 1;
            }
        } else if (blocked > 0.2) {
            my_backoff = max(0, my_backoff - 2);
        } else {
            my_backoff = max(0, my_backoff - 1);
        }
    } else if (wants_target && dist_to_target <= params.arrival_threshold) {
        settled = 1;
    } else {
        my_backoff = max(0, my_backoff - 1);
    }

    // === STEP 4: APPLY ===
    pos += (movement + avoidance) * params.delta;

    // === WRITE STATE ===
    positions[i] = pos;
    arrivals[i] = settled;
    backoff[i] = my_backoff;

    // Write to MultiMesh buffer (12 floats per instance)
    uint base = i * 12;
    // Transform2D: [a.x, b.x, 0, origin.x, a.y, b.y, 0, origin.y]
    multimesh_data[base + 0] = 1.0;      // scale x
    multimesh_data[base + 1] = 0.0;
    multimesh_data[base + 2] = 0.0;
    multimesh_data[base + 3] = pos.x;    // position x
    multimesh_data[base + 4] = 0.0;
    multimesh_data[base + 5] = 1.0;      // scale y
    multimesh_data[base + 6] = 0.0;
    multimesh_data[base + 7] = pos.y;    // position y
    // Color from buffer
    multimesh_data[base + 8] = color.r;
    multimesh_data[base + 9] = color.g;
    multimesh_data[base + 10] = color.b;
    multimesh_data[base + 11] = color.a;
}
