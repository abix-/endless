// =============================================================================
// NPC Compute Shader - GPU-Accelerated Physics
// =============================================================================
// Simplified version for initial testing. Full version has 3 modes.

struct Params {
    count: u32,
    separation_radius: f32,
    separation_strength: f32,
    delta: f32,
    grid_width: u32,
    grid_height: u32,
    cell_size: f32,
    max_per_cell: u32,
    arrival_threshold: f32,
    mode: u32,
    _pad1: f32,
    _pad2: f32,
}

// Storage buffers matching Rust bind group layout (all read_write for simplicity)
@group(0) @binding(0) var<storage, read_write> positions: array<vec2<f32>>;
@group(0) @binding(1) var<storage, read_write> goals: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read_write> speeds: array<f32>;
@group(0) @binding(3) var<storage, read_write> grid_counts: array<i32>;
@group(0) @binding(4) var<storage, read_write> grid_data: array<i32>;
@group(0) @binding(5) var<storage, read_write> arrivals: array<i32>;
@group(0) @binding(6) var<storage, read_write> backoff: array<i32>;
@group(0) @binding(7) var<storage, read_write> factions: array<i32>;
@group(0) @binding(8) var<storage, read_write> healths: array<f32>;
@group(0) @binding(9) var<storage, read_write> combat_targets: array<i32>;
@group(0) @binding(10) var<uniform> params: Params;

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let i = global_id.x;
    if (i >= params.count) { return; }

    // Read current state
    var pos = positions[i];
    let goal = goals[i];
    let speed = speeds[i];
    var settled = arrivals[i];

    // Skip dead/hidden NPCs
    if (pos.x < -9000.0) { return; }

    let to_goal = goal - pos;
    let dist_to_goal = length(to_goal);
    let wants_goal = settled == 0;

    // Simple movement toward goal (no separation for now)
    var movement = vec2<f32>(0.0, 0.0);
    if (wants_goal && dist_to_goal > params.arrival_threshold) {
        movement = normalize(to_goal) * speed;
    } else if (wants_goal && dist_to_goal <= params.arrival_threshold) {
        settled = 1;
    }

    // Apply movement
    pos += movement * params.delta;

    // Write output
    positions[i] = pos;
    arrivals[i] = settled;

    // Combat targeting placeholder
    combat_targets[i] = -1;
}
