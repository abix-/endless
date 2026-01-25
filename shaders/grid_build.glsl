// =============================================================================
// Grid Build Compute Shader - Spatial Grid Construction on GPU
// =============================================================================
//
// This shader builds the spatial grid entirely on GPU, eliminating the need
// to read positions back to CPU. Uses atomic operations for thread-safe insertion.
//
// Algorithm:
// 1. Clear grid counts (done via buffer fill before dispatch)
// 2. Each NPC atomically inserts itself into its cell
// 3. Main physics shader reads the grid normally

#[compute]
#version 450

layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;

// Position buffer (read-only - just need to know where each NPC is)
layout(set = 0, binding = 0, std430) restrict readonly buffer PositionBuffer {
    vec2 positions[];
};

// Grid counts (read/write - atomic increment)
layout(set = 0, binding = 1, std430) buffer GridCounts {
    int grid_counts[];
};

// Grid data (write-only - store NPC indices)
layout(set = 0, binding = 2, std430) restrict writeonly buffer GridData {
    int grid_data[];
};

layout(push_constant) uniform PushConstants {
    uint npc_count;      // Number of active NPCs
    uint grid_width;     // Grid width in cells (128)
    uint grid_height;    // Grid height in cells (128)
    float cell_size;     // Size of each cell (64px)
    uint max_per_cell;   // Max NPCs per cell (48)
} params;

void main() {
    uint i = gl_GlobalInvocationID.x;
    if (i >= params.npc_count) return;

    vec2 pos = positions[i];

    // Calculate cell coordinates
    int cx = clamp(int(pos.x / params.cell_size), 0, int(params.grid_width) - 1);
    int cy = clamp(int(pos.y / params.cell_size), 0, int(params.grid_height) - 1);
    int cell_idx = cy * int(params.grid_width) + cx;

    // Atomically get a slot in this cell
    int slot = atomicAdd(grid_counts[cell_idx], 1);

    // Write our index if there's room
    if (slot < int(params.max_per_cell)) {
        grid_data[cell_idx * int(params.max_per_cell) + slot] = int(i);
    }
    // If cell is full, we just don't get added (graceful degradation)
}
