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
                    if (dist_sq < 0.0001) {
                        // Special case: NPCs exactly on top of each other
                        // Use golden angle to spread them in unique directions
                        float angle = float(i) * 2.399 + float(j) * 0.7;
                        diff = vec2(cos(angle), sin(angle));
                        avoidance += diff * params.separation_radius;
                    } else {
                        // Normal case: push proportional to overlap
                        float dist = sqrt(dist_sq);
                        float overlap = params.separation_radius - dist;
                        avoidance += diff * (overlap / dist);  // Normalize and scale
                    }
                }
            }
        }
    }

    // Scale avoidance by strength parameter
    avoidance *= params.separation_strength;

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
    // TCP-style collision avoidance:
    // - If we're being pushed away from our goal: increment backoff
    // - If we're making progress toward goal: decrement backoff
    // - If backoff exceeds threshold: give up (mark as settled)
    //
    // "Blocked" is detected by checking if avoidance force opposes goal direction.

    float avoidance_strength = length(avoidance);
    float cluster_radius = params.separation_radius * 6.0;  // ~120px for typical cluster

    if (wants_target && dist_to_target > params.arrival_threshold) {
        if (avoidance_strength > 0.5) {
            // We're being pushed by neighbors - check if blocked
            vec2 goal_dir = normalize(to_target);
            vec2 push_dir = normalize(avoidance);
            float blocked = dot(push_dir, goal_dir);

            if (blocked < -0.2) {
                // Pushed directly AWAY from target - definitely blocked
                // dot < -0.2 means angle > 101 degrees (pushing us back)
                my_backoff += 2;
            } else if (dist_to_target < cluster_radius) {
                // Close to target but being jostled - probably blocked
                // This catches NPCs stuck in the outer ring of a cluster
                my_backoff++;
            } else if (blocked > 0.3) {
                // Being pushed TOWARD target - making progress
                my_backoff = max(0, my_backoff - 2);
            }
        } else {
            // No significant avoidance - making progress
            my_backoff = max(0, my_backoff - 1);
        }

        // Give up after sustained blocking (60 frames = ~1 second at 60fps)
        if (my_backoff > 60) {
            settled = 1;
        }
    } else if (wants_target && dist_to_target <= params.arrival_threshold) {
        // Within arrival threshold - we made it!
        settled = 1;
    }

    // =========================================================================
    // STEP 5: APPLY MOVEMENT
    // =========================================================================

    pos += (movement + avoidance) * params.delta;

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
