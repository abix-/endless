// =============================================================================
// NPC Compute Shader - GPU-Accelerated Physics for Thousands of NPCs
// =============================================================================
//
// This shader runs once per NPC per frame, in parallel across all GPU cores.
// Each invocation handles one NPC: reads neighbors, calculates forces, writes position.
//
// Key concepts:
// - Spatial Grid: World divided into cells. Each NPC only checks 3x3 neighborhood.
// - Separation: Boids-style force pushing NPCs apart when too close.
// - TCP Backoff: When blocked, NPCs slow down and eventually give up.
// - Zero-Copy Rendering: MultiMesh buffer written directly, no CPU copy needed.

#[compute]
#version 450

// Workgroup size: 64 NPCs processed in parallel per workgroup.
// Total workgroups = ceil(npc_count / 64).
layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;

// =============================================================================
// GPU BUFFERS - Bound from Rust via uniform set
// =============================================================================

// Binding 0: Position buffer (read/write - GPU owns authoritative positions)
// Layout: [x0, y0, x1, y1, ...] - 2 floats per NPC
// The GPU updates these each frame based on movement and separation forces.
layout(set = 0, binding = 0, std430) buffer PositionBuffer {
    vec2 positions[];
};

// Binding 1: Target buffer (read-only - set by CPU when set_target() called)
// Layout: [tx0, ty0, tx1, ty1, ...] - 2 floats per NPC
// NPCs move toward their target until arrival or blocked.
layout(set = 0, binding = 1, std430) restrict readonly buffer TargetBuffer {
    vec2 targets[];
};

// Binding 2: Color buffer (read-only - set at spawn, determines faction)
// Layout: [r0, g0, b0, a0, r1, ...] - 4 floats per NPC
// Alpha > 0 means "has target" (seeking movement). Alpha = 0 means idle.
layout(set = 0, binding = 2, std430) restrict readonly buffer ColorBuffer {
    vec4 colors[];
};

// Binding 3: Speed buffer (read-only - movement speed in pixels/second)
layout(set = 0, binding = 3, std430) restrict readonly buffer SpeedBuffer {
    float speeds[];
};

// Binding 4: Grid counts (read-only - how many NPCs in each cell)
// Rebuilt on CPU each frame and uploaded before dispatch.
layout(set = 0, binding = 4, std430) restrict readonly buffer GridCounts {
    int grid_counts[];
};

// Binding 5: Grid data (read-only - which NPCs are in each cell)
// Layout: flat array, cell_idx * max_per_cell + n gives NPC index
layout(set = 0, binding = 5, std430) restrict readonly buffer GridData {
    int grid_data[];
};

// Binding 6: MultiMesh output (write-only - direct to Godot renderer)
// Layout: 12 floats per NPC (Transform2D + Color)
// Transform2D: [a.x, b.x, 0, origin.x, a.y, b.y, 0, origin.y]
// Color: [r, g, b, a]
layout(set = 0, binding = 6, std430) restrict writeonly buffer MultiMeshBuffer {
    float multimesh_data[];
};

// Binding 7: Arrival flags (read/write - 1 = arrived or gave up)
// Once set to 1, NPC stops pursuing target until new target is set.
layout(set = 0, binding = 7, std430) buffer ArrivalBuffer {
    int arrivals[];
};

// Binding 8: Backoff counters (read/write - TCP-style collision avoidance)
// Incremented when blocked by neighbors, decremented when making progress.
// When backoff > threshold, NPC gives up (sets arrival = 1).
layout(set = 0, binding = 8, std430) buffer BackoffBuffer {
    int backoff[];
};

// =============================================================================
// COMBAT TARGETING BUFFERS
// =============================================================================

// Binding 9: Faction buffer (read-only - 0=Villager, 1=Raider)
// Set at spawn time. Villagers fight Raiders and vice versa.
layout(set = 0, binding = 9, std430) restrict readonly buffer FactionBuffer {
    int factions[];
};

// Binding 10: Health buffer (read-only - current HP)
// Updated when damage is applied. Dead NPCs (health <= 0) ignored for targeting.
layout(set = 0, binding = 10, std430) restrict readonly buffer HealthBuffer {
    float healths[];
};

// Binding 11: Combat target buffer (write-only - output for CPU)
// -1 = no target, >= 0 = NPC index of nearest enemy
layout(set = 0, binding = 11, std430) restrict writeonly buffer CombatTargetBuffer {
    int combat_targets[];
};

// =============================================================================
// PUSH CONSTANTS - Small, fast-changing parameters passed each frame
// =============================================================================

layout(push_constant) uniform PushConstants {
    uint npc_count;          // 0-4: Number of active NPCs
    float separation_radius; // 4-8: Minimum distance between NPCs (20px)
    float separation_strength; // 8-12: How hard NPCs push apart (100.0)
    float delta;             // 12-16: Frame delta time in seconds
    uint grid_width;         // 16-20: Spatial grid width (128)
    uint grid_height;        // 20-24: Spatial grid height (128)
    float cell_size;         // 24-28: Size of each grid cell (64px)
    uint max_per_cell;       // 28-32: Max NPCs per cell (48)
    float arrival_threshold; // 32-36: Distance to count as "arrived" (8px)
    float _pad1;             // 36-40: Padding for 48-byte alignment
    float _pad2;             // 40-44: (GPU requires specific alignment)
    float _pad3;             // 44-48:
} params;

// =============================================================================
// MAIN SHADER LOGIC
// =============================================================================

void main() {
    // Each invocation handles one NPC
    uint i = gl_GlobalInvocationID.x;
    if (i >= params.npc_count) return;  // Guard against out-of-bounds

    // =========================================================================
    // STEP 1: READ CURRENT STATE
    // =========================================================================

    vec2 pos = positions[i];       // Current position (GPU-owned)
    vec2 target = targets[i];      // Where we want to go (CPU-set)
    float speed = speeds[i];       // Movement speed (pixels/second)
    vec4 color = colors[i];        // Color + alpha (alpha > 0 = seeking)
    int settled = arrivals[i];     // 1 = reached target or gave up
    int my_backoff = backoff[i];   // Collision backoff counter

    vec2 to_target = target - pos;
    float dist_to_target = length(to_target);

    // Only pursue target if: has valid target (alpha > 0) AND hasn't settled
    bool wants_target = color.a > 0.0 && settled == 0;

    // =========================================================================
    // STEP 2: CALCULATE AVOIDANCE FORCE (push away from neighbors)
    // =========================================================================
    //
    // This is boids-style separation: if another NPC is within separation_radius,
    // we push away from them proportionally to how close they are.
    //
    // Uses spatial grid for O(1) neighbor lookup instead of O(n) full scan.
    // We check our cell plus 8 neighbors (3x3 area around us).

    vec2 avoidance = vec2(0.0);
    float sep_radius_sq = params.separation_radius * params.separation_radius;

    // Calculate which grid cell we're in
    int cx = clamp(int(pos.x / params.cell_size), 0, int(params.grid_width) - 1);
    int cy = clamp(int(pos.y / params.cell_size), 0, int(params.grid_height) - 1);

    // Check 3x3 neighborhood of cells
    for (int dy = -1; dy <= 1; dy++) {
        int ny = cy + dy;
        if (ny < 0 || ny >= int(params.grid_height)) continue;

        for (int dx = -1; dx <= 1; dx++) {
            int nx = cx + dx;
            if (nx < 0 || nx >= int(params.grid_width)) continue;

            // Get NPCs in this cell
            int cell_idx = ny * int(params.grid_width) + nx;
            int cell_count = grid_counts[cell_idx];
            int cell_base = cell_idx * int(params.max_per_cell);

            // Check each NPC in this cell
            for (int n = 0; n < cell_count; n++) {
                uint j = uint(grid_data[cell_base + n]);
                if (j == i) continue;  // Don't avoid ourselves

                vec2 other_pos = positions[j];
                vec2 diff = pos - other_pos;  // Vector FROM other TO us
                float dist_sq = dot(diff, diff);

                if (dist_sq < sep_radius_sq) {
                    // This neighbor is too close - push away
                    // Asymmetric push: moving NPCs shove through settled ones
                    int neighbor_settled = arrivals[j];

                    float push_strength = 1.0;
                    if (settled == 0 && neighbor_settled == 1) {
                        // I'm moving, they're settled: they barely block me
                        push_strength = 0.2;
                    } else if (settled == 1 && neighbor_settled == 0) {
                        // I'm settled, they're moving: they shove me aside
                        push_strength = 2.0;
                    }

                    if (dist_sq < 0.0001) {
                        // Special case: NPCs exactly on top of each other
                        // Use golden angle to spread them in unique directions
                        float angle = float(i) * 2.399 + float(j) * 0.7;
                        diff = vec2(cos(angle), sin(angle));
                        avoidance += diff * params.separation_radius * push_strength;
                    } else {
                        // Normal case: push proportional to overlap
                        float dist = sqrt(dist_sq);
                        float overlap = params.separation_radius - dist;
                        avoidance += diff * (overlap / dist) * push_strength;
                    }
                }
            }
        }
    }

    // Scale avoidance by strength parameter
    avoidance *= params.separation_strength;

    // =========================================================================
    // STEP 2b: TCP-STYLE DODGE (avoid collisions with other moving NPCs)
    // =========================================================================
    // When approaching another moving NPC, dodge sideways to pass smoothly.
    // Handles head-on, overtaking, and crossing paths.

    vec2 dodge = vec2(0.0);
    if (wants_target && dist_to_target > params.arrival_threshold) {
        vec2 my_dir = normalize(to_target);

        // Re-check neighbors for dodge calculation
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

                    // Only dodge around other MOVING NPCs
                    int neighbor_settled = arrivals[j];
                    if (neighbor_settled == 1) continue;

                    vec2 other_pos = positions[j];
                    vec2 diff = pos - other_pos;
                    float dist_sq = dot(diff, diff);

                    // Check within approach radius (2x separation)
                    float approach_radius = params.separation_radius * 2.0;
                    if (dist_sq > approach_radius * approach_radius) continue;
                    if (dist_sq < 0.0001) continue;

                    float dist = sqrt(dist_sq);
                    vec2 to_other = -diff / dist;

                    // Am I moving toward them?
                    float i_approach = dot(my_dir, to_other);
                    if (i_approach > 0.3) {
                        // Get their movement direction
                        vec2 other_target = targets[j];
                        vec2 ot = other_target - other_pos;
                        float ot_len = length(ot);

                        if (ot_len > 0.001) {
                            vec2 other_dir = ot / ot_len;
                            float they_approach = -dot(other_dir, to_other);

                            vec2 perp = vec2(-my_dir.y, my_dir.x);
                            float dodge_strength = 0.4;

                            if (they_approach > 0.3) {
                                // Head-on collision
                                dodge_strength = 0.5;
                            } else if (they_approach < -0.3) {
                                // Overtaking
                                dodge_strength = 0.3;
                            }

                            // Consistent dodge direction based on index
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

        // Normalize and scale dodge
        float dodge_len = length(dodge);
        if (dodge_len > 0.0) {
            dodge = (dodge / dodge_len) * params.separation_strength * 0.7;
        }
    }

    avoidance += dodge;

    // =========================================================================
    // STEP 3: CALCULATE MOVEMENT TOWARD TARGET
    // =========================================================================
    //
    // Movement is reduced by backoff counter (TCP-style exponential backoff).
    // persistence = 1 / (1 + backoff), so higher backoff = slower pursuit.

    vec2 movement = vec2(0.0);
    if (wants_target && dist_to_target > params.arrival_threshold) {
        float persistence = 1.0 / float(1 + my_backoff);
        movement = normalize(to_target) * speed * persistence;
    }

    // =========================================================================
    // STEP 4: DETECT BLOCKING AND UPDATE BACKOFF
    // =========================================================================
    //
    // Lifelike crowd behavior:
    // - Pushed AWAY from target = blocked (increment backoff)
    // - Pushed TOWARD target = making progress (decrement backoff)
    // - Pushed SIDEWAYS = jostling (no change)
    // - Not pushed = clear path (decrement backoff)
    // - High backoff = give up and settle

    float avoidance_mag = length(avoidance);

    if (wants_target && dist_to_target > params.arrival_threshold) {
        if (avoidance_mag > 0.1) {
            // Being pushed - check direction relative to goal
            vec2 goal_dir = normalize(to_target);
            vec2 push_dir = normalize(avoidance);
            float alignment = dot(push_dir, goal_dir);

            if (alignment < -0.3) {
                // Pushed strongly AWAY from target - blocked
                my_backoff += 2;
            } else if (alignment > 0.3) {
                // Pushed strongly TOWARD target - making progress
                my_backoff = max(0, my_backoff - 2);
            }
            // Sideways pushing = jostling in crowd, don't increment backoff
        } else {
            // Clear path - making progress
            my_backoff = max(0, my_backoff - 1);
        }

        // Give up after sustained blocking (~2 seconds at 60fps)
        if (my_backoff > 120) {
            settled = 1;
        }
    } else if (wants_target && dist_to_target <= params.arrival_threshold) {
        // Reached target!
        settled = 1;
    }

    // =========================================================================
    // STEP 5: APPLY MOVEMENT
    // =========================================================================

    pos += (movement + avoidance) * params.delta;

    // =========================================================================
    // STEP 5b: COMBAT TARGETING (find nearest enemy)
    // =========================================================================
    // Uses same spatial grid to find hostile NPCs within detection range.
    // Output: combat_targets[i] = index of nearest enemy, or -1 if none.

    int best_target = -1;
    float detect_range = 300.0;  // 2x attack range (150px)
    float best_dist_sq = detect_range * detect_range;

    // Only living NPCs can have targets
    float my_health = healths[i];
    if (my_health > 0.0) {
        int my_faction = factions[i];

        // Check 3x3 neighborhood for enemies
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

                    // Faction check: hostile if different
                    int other_faction = factions[j];
                    if (other_faction == my_faction) continue;

                    // Alive check
                    float other_health = healths[j];
                    if (other_health <= 0.0) continue;

                    // Distance check
                    vec2 other_pos = positions[j];
                    vec2 diff = pos - other_pos;
                    float dist_sq = dot(diff, diff);

                    if (dist_sq < best_dist_sq) {
                        best_dist_sq = dist_sq;
                        best_target = int(j);
                    }
                }
            }
        }
    }

    combat_targets[i] = best_target;

    // =========================================================================
    // STEP 6: WRITE OUTPUT
    // =========================================================================

    // Update our state buffers
    positions[i] = pos;
    arrivals[i] = settled;
    backoff[i] = my_backoff;

    // Write directly to MultiMesh buffer for rendering (zero-copy!)
    // Layout: 12 floats per instance
    // Transform2D: [a.x, b.x, 0, origin.x, a.y, b.y, 0, origin.y]
    uint base = i * 12;
    multimesh_data[base + 0] = 1.0;      // scale x (identity transform)
    multimesh_data[base + 1] = 0.0;      // shear
    multimesh_data[base + 2] = 0.0;      // unused
    multimesh_data[base + 3] = pos.x;    // position x
    multimesh_data[base + 4] = 0.0;      // shear
    multimesh_data[base + 5] = 1.0;      // scale y
    multimesh_data[base + 6] = 0.0;      // unused
    multimesh_data[base + 7] = pos.y;    // position y
    // Color from buffer (faction tinting)
    multimesh_data[base + 8] = color.r;
    multimesh_data[base + 9] = color.g;
    multimesh_data[base + 10] = color.b;
    multimesh_data[base + 11] = color.a;
}
