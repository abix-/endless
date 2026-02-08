# Core Concepts

Foundational knowledge for understanding the Endless codebase.

## Data-Oriented Design (DOD)

**Traditional OOP:** Each NPC is an object with properties.
```rust
struct Npc {
    position: Vec2,
    velocity: Vec2,
    health: f32,
}
```

**DOD:** Properties are parallel arrays. NPC #5's data is at index 5 in every array.
```rust
positions: Vec<f32>,   // [x0, y0, x1, y1, ...] positions[5*2] = NPC #5's x
targets: Vec<f32>,     // [x0, y0, x1, y1, ...]
healths: Vec<f32>,     // healths[5] = NPC #5's health
```

**Why DOD?**
- Cache-friendly: updating all positions reads contiguous memory
- SIMD-friendly: CPU can process 4-8 floats simultaneously
- GPU-friendly: upload entire array in one call
- No object overhead: no vtables, no GC pressure

Used in: `NpcBufferWrites` (flat arrays for GPU upload), GPU storage buffers

---

## Spatial Grid

Divides the world into cells. Each cell tracks which NPCs are inside it.

```
┌───┬───┬───┬───┐
│ 2 │   │ 1 │   │  ← cell contains count of NPCs
├───┼───┼───┼───┤
│   │ 3 │   │   │
├───┼───┼───┼───┤
│ 1 │   │   │ 2 │
└───┴───┴───┴───┘
```

**Without grid:** To find neighbors, check every NPC. O(n²) for n NPCs.

**With grid:** Only check NPCs in same cell + adjacent cells. O(n × k) where k ≈ 10-50.

```wgsl
// Get cell from position
let cell_x = u32(pos.x / params.cell_size);
let cell_y = u32(pos.y / params.cell_size);

// Check 3x3 neighborhood (9 cells max)
for (var dy: i32 = -1; dy <= 1; dy++) {
    for (var dx: i32 = -1; dx <= 1; dx++) {
        // Only check NPCs in this cell, not all 16,384
    }
}
```

Cell size should be >= largest interaction radius. Endless uses 64px cells, 128x128 grid.

Used in: `npc_compute.wgsl` (buffers allocated, grid logic not yet ported)

---

## Separation Forces (Boids)

NPCs push each other apart to avoid overlapping. Each NPC feels a force away from nearby NPCs.

```
     NPC B
       ↑
       │ force (pushes A away from B)
       │
     NPC A ←───── NPC C
              force (pushes A away from C)
```

**Algorithm:**
1. Find neighbors within `separation_radius` (via spatial grid)
2. For each neighbor, calculate direction away from them
3. Scale by overlap amount (closer = stronger push)
4. Sum all forces, apply to velocity

```wgsl
let diff = my_pos - neighbor_pos;
let dist = length(diff);
let overlap = params.separation_radius - dist;
separation_force += normalize(diff) * overlap;
```

This is one of the three classic "boid" behaviors (separation, alignment, cohesion). Endless only uses separation.

Used in: Not yet ported to `npc_compute.wgsl`. Parameters allocated: `separation_radius=20`, `separation_strength=100`.

---

## Compute Shaders

Run code on the GPU in parallel. Instead of one CPU core processing 16,384 NPCs sequentially, 1,000+ GPU cores process them simultaneously.

**CPU (sequential):**
```
for i in 0..16384:
    process(npc[i])  // one at a time
```

**GPU (parallel):**
```wgsl
@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let i = global_id.x;  // each invocation gets unique ID
    process(npc[i]);       // 16,384 run simultaneously
}
```

**Workflow:**
1. Upload data to GPU buffers (CPU → GPU)
2. Dispatch shader (GPU runs in parallel)
3. Read results back (GPU → CPU) or render directly

**Limitations:**
- GPU→CPU readback is slow (avoid if possible)
- Branching (if/else) hurts performance
- Best for embarrassingly parallel work (same operation on many items)

Used in: `npc_compute.wgsl` via `NpcComputeNode` in Bevy render graph

---

## GPU Instanced Rendering

Draw thousands of sprites in one draw call. Each NPC is a textured quad with per-instance data.

**Without instancing:** 16,384 NPCs = 16,384 draw calls. GPU stalls on each call.

**With instancing:** 16,384 NPCs = 1 draw call. GPU renders all instances in parallel.

```rust
// One static quad (4 vertices, shared by all NPCs)
static QUAD_VERTICES: [QuadVertex; 4] = [ /* corners */ ];

// Per-instance data (unique per NPC, 32 bytes each)
pub struct NpcInstanceData {
    pub position: [f32; 2],  // world position
    pub sprite: [f32; 2],    // atlas cell (col, row)
    pub color: [f32; 4],     // tint color
}

// One draw call for all NPCs
pass.draw_indexed(0..6, 0, 0..instance_count);
```

The GPU uses `VertexStepMode::Instance` to advance instance data once per quad, not once per vertex.

Used in: `npc_render.rs` (RenderCommand + Transparent2d), `npc_render.wgsl`

---

## Staggered Processing

Don't update everything every frame. Spread work across frames.

**Without stagger:** Update 16,384 NPCs every frame. CPU spike.

**With stagger:** Update 2,048 NPCs per frame (1/8 each frame). Smooth load.

```rust
let start = scan_offset * npc_count / SCAN_FRACTION;
let end = (scan_offset + 1) * npc_count / SCAN_FRACTION;

for i in start..end {
    update_combat(i);  // only update this slice
}

scan_offset = (scan_offset + 1) % SCAN_FRACTION;
```

Trade-off: reactions are delayed by up to `SCAN_FRACTION` frames. Tune per-system.

Not currently used — GPU compute handles all NPCs every frame.

---

## LOD Intervals (Level of Detail)

Update frequency based on importance/activity.

| State | Update Interval | Why |
|-------|-----------------|-----|
| Fighting | 2 frames | Need fast reactions |
| Moving | 5 frames | Moderate precision |
| Idle | 30 frames | Nothing happening |

Distance also affects LOD — off-screen NPCs update less often.

Not currently used — GPU compute processes all NPCs uniformly.

---

## ECS (Entity Component System)

Bevy's architecture. Used throughout the codebase.

- **Entity:** Just an ID (integer)
- **Component:** Data attached to entity (Position, Health, Job)
- **System:** Function that processes entities with specific components

```rust
#[derive(Component)]
struct Position(Vec2);

#[derive(Component)]
struct Velocity(Vec2);

// System: runs on all entities with Position AND Velocity
fn movement_system(mut query: Query<(&mut Position, &Velocity)>) {
    for (mut pos, vel) in query.iter_mut() {
        pos.0 += vel.0 * DELTA;
    }
}
```

ECS gives you DOD benefits with nicer ergonomics. Bevy schedules systems in parallel automatically.

Used in: All game logic (spawn, combat, behavior, economy systems)

---

## GPU Readback Avoidance

Reading data back from GPU to CPU is expensive. Avoid it whenever possible.

**The problem:**
```
CPU → GPU: Upload positions (fast, ~1ms)
GPU: Run compute shader (fast, ~0.1ms)
GPU → CPU: Read positions back (SLOW, ~5-10ms)
```

The GPU→CPU transfer stalls the pipeline — CPU waits for GPU to finish, then copies data over PCIe.

**Solution: Keep data on GPU**

The render pipeline reads directly from GPU buffers (or from the same CPU-side arrays that were uploaded), avoiding readback. CPU-side systems that need position data use the pre-upload `NpcBufferWrites` arrays.

**When you must read back:**
- Keep buffers small (read only what's needed)
- Read asynchronously if possible (don't block on result)
- Batch reads (one large read beats many small ones)

Used in: Current design avoids readback entirely. `GpuReadState` exists for future readback but is unpopulated.

---

## Debug Mode Overhead

Debug metrics can cost more than the actual simulation. Disable or throttle them.

**The trap:** You add O(n²) validation to verify NPCs are properly separated:

```rust
fn get_min_separation(positions: &[f32], count: usize) -> f32 {
    let mut min_dist = f32::MAX;
    for i in 0..count {
        for j in (i+1)..count {
            let dx = positions[i*2] - positions[j*2];
            let dy = positions[i*2+1] - positions[j*2+1];
            min_dist = min_dist.min((dx*dx + dy*dy).sqrt());
        }
    }
    min_dist
}
```

With 5,000 NPCs, that's 12.5 million distance checks per frame. Your 140fps simulation drops to 15fps — but the simulation itself is fine, only the *measurement* is slow.

**Solutions:**
1. **Disable by default:** Make expensive metrics opt-in
2. **Throttle:** Run expensive checks once per second, not every frame
3. **Sample:** Check 100 random pairs instead of all pairs

**Rule of thumb:** If your metric is O(n²) or worse, it needs a toggle.

---

## Asymmetric Push

Moving NPCs should push through settled ones, not get blocked.

**The problem:** With symmetric separation forces, a moving NPC approaching a group gets pushed back as hard as they push forward. They can't enter the crowd.

**Solution:** Asymmetric push strengths based on movement state:

```wgsl
var push_strength = 1.0;
if (i_am_moving && neighbor_is_settled) {
    push_strength = 0.2;  // Settled NPCs barely block me
} else if (i_am_settled && neighbor_is_moving) {
    push_strength = 2.0;  // Moving NPCs shove me aside
}
avoidance += diff * overlap * push_strength;
```

| My State | Neighbor State | Push Strength | Result |
|----------|----------------|---------------|--------|
| Moving | Settled | 0.2 | I push through |
| Settled | Moving | 2.0 | They shove me |
| Moving | Moving | 1.0 | Equal contest |
| Settled | Settled | 1.0 | Stable formation |

This lets NPCs flow through crowds to reach their targets, then settle into formation.

Not yet ported to `npc_compute.wgsl`.

---

## TCP Dodge

When two moving NPCs approach each other, dodge sideways instead of stopping.

Named after TCP congestion avoidance — when packets collide, back off and try a different path.

**The problem:** Two NPCs walking toward each other with symmetric separation forces will push directly against each other, creating a standoff or oscillation.

**Solution:** Detect approaching collision and add perpendicular dodge:

```wgsl
let to_neighbor = neighbor_pos - my_pos;
let approach_speed = dot(my_velocity, normalize(to_neighbor));

if (approach_speed > 0.0) {  // We're closing in
    // Dodge perpendicular to approach direction
    let perp = vec2(-to_neighbor.y, to_neighbor.x);

    // Consistent side: lower index dodges right
    let side = select(-1.0, 1.0, my_index < neighbor_index);

    dodge += normalize(perp) * side * approach_speed;
}
```

**Key details:**
- Only dodge around other *moving* NPCs (settled ones use asymmetric push)
- Consistent side selection prevents both NPCs dodging the same way
- Dodge strength scales with approach speed (faster approach = harder dodge)

Not yet ported to `npc_compute.wgsl`.

---

## State Machines in ECS

Bevy ECS represents states as marker components, not enums.

**Traditional state machine:**
```rust
enum GuardState { Patrolling, OnDuty, Resting, GoingToRest }
```

**ECS state machine:** States are separate components. An entity has exactly one state component at a time.

```rust
#[derive(Component)]
struct Patrolling;

#[derive(Component)]
struct OnDuty { ticks_waiting: u32 }

#[derive(Component)]
struct Resting;

#[derive(Component)]
struct GoingToRest;
```

**Why markers?**
- Queries filter by component: `Query<&Guard, With<Patrolling>>` only matches patrolling guards
- State transitions = add/remove components
- Each state can have its own data (`OnDuty` has `ticks_waiting`, others don't need it)

**State transitions:**
```rust
fn transition_to_rest(
    mut commands: Commands,
    tired_guards: Query<Entity, (With<Guard>, With<Patrolling>)>,
    energy: Query<&Energy>,
) {
    for entity in tired_guards.iter() {
        if energy.get(entity).unwrap().0 < ENERGY_HUNGRY {
            commands.entity(entity)
                .remove::<Patrolling>()
                .insert(GoingToRest);
        }
    }
}
```

Used in: All NPC behavior (Patrolling, OnDuty, Working, Resting, GoingToRest, GoingToWork, Raiding, Returning, Recovering, Wandering)

---

## World Data Resources

Static world layout (buildings, locations) stored as ECS Resources, not entities.

**Entities vs Resources:**
- **Entity:** Dynamic, many instances, has lifecycle (spawn/despawn). NPCs, projectiles.
- **Resource:** Singleton, shared state, lives forever. World layout, config, occupancy tracking.

```rust
#[derive(Resource, Default)]
pub struct WorldData {
    pub towns: Vec<Town>,
    pub farms: Vec<Farm>,
    pub beds: Vec<Bed>,
    pub guard_posts: Vec<GuardPost>,
}
```

**Occupancy tracking:** Mutable resources track which NPCs occupy which buildings:

```rust
#[derive(Resource, Default)]
pub struct BedOccupancy {
    pub occupant_npc: Vec<i32>,  // -1 = free, >= 0 = NPC index
}
```

Used in: `WorldData`, `BedOccupancy`, `FarmOccupancy` resources

---

## Bevy Pipelined Rendering

Bevy runs the main world and render world in parallel, synchronized once per frame at the extract barrier.

```
Frame N:   Main World computes game logic
           ────── extract barrier ──────
           Render World processes frame N
Frame N+1: Main World computes next frame  ← runs in parallel with render
           ────── extract barrier ──────
           Render World processes frame N+1
```

**Extract:** Resources are cloned from main world to render world via `ExtractResourcePlugin`. This is the sync point — both worlds pause briefly, data is copied, then they resume in parallel.

**Consequence:** One-frame render latency. The GPU renders positions from the previous main world frame. At 140fps this is ~7ms of latency — invisible.

Used in: `GpuComputePlugin` (extract NpcBufferWrites, NpcGpuData, NpcComputeParams), `NpcRenderPlugin` (extract NpcBatch entity)

---

## Summary

| Concept | Problem | Solution |
|---------|---------|----------|
| DOD | Object overhead, cache misses | Parallel arrays |
| Spatial Grid | O(n²) neighbor search | O(n×k) cell lookup |
| Separation | NPC overlap | Push forces from neighbors |
| Compute Shader | CPU bottleneck | GPU parallelism |
| Instanced Rendering | Draw call overhead | One draw call for all instances |
| Stagger | Frame spikes | Spread work across frames |
| LOD Intervals | Wasted updates | Update based on importance |
| ECS | DOD ergonomics | Entity/Component/System pattern |
| GPU Readback | Pipeline stalls | Avoid readback, use CPU copies |
| Debug Overhead | Metrics kill perf | Disable/throttle expensive checks |
| Asymmetric Push | Can't enter crowds | Moving NPCs push through settled |
| TCP Dodge | Head-on collisions | Perpendicular dodge on approach |
| ECS States | State machine in ECS | Marker components per state |
| World Resources | Static world data | Singleton Resources, not Entities |
| Pipelined Rendering | CPU/GPU sync overhead | Parallel main + render worlds |
