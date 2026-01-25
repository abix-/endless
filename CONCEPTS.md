# Core Concepts

Foundational knowledge for understanding the Endless codebase.

## Data-Oriented Design (DOD)

**Traditional OOP:** Each NPC is an object with properties.
```gdscript
class NPC:
    var position: Vector2
    var velocity: Vector2
    var health: float
```

**DOD:** Properties are parallel arrays. NPC #5's data is at index 5 in every array.
```gdscript
var positions: PackedVector2Array   # positions[5] = NPC #5's position
var velocities: PackedVector2Array  # velocities[5] = NPC #5's velocity
var healths: PackedFloat32Array     # healths[5] = NPC #5's health
```

**Why DOD?**
- Cache-friendly: updating all positions reads contiguous memory
- SIMD-friendly: CPU can process 4-8 floats simultaneously
- GPU-friendly: upload entire array in one call
- No object overhead: no vtables, no GC pressure

Used in: `npc_manager.gd` (30+ parallel arrays), Rust POC

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

```gdscript
# Get cell from position
var cell_x = int(position.x / CELL_SIZE)
var cell_y = int(position.y / CELL_SIZE)

# Check 3x3 neighborhood (9 cells max)
for dy in range(-1, 2):
    for dx in range(-1, 2):
        for npc_idx in grid[cell_x + dx][cell_y + dy]:
            # Only check these NPCs, not all 10,000
```

Cell size should be ≥ largest interaction radius. Endless uses 64px cells.

Used in: `npc_grid.gd`, `separation_compute.glsl`, Rust `SpatialGrid`

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
1. Find neighbors within `SEPARATION_RADIUS`
2. For each neighbor, calculate direction away from them
3. Scale by overlap amount (closer = stronger push)
4. Sum all forces, apply to velocity

```glsl
vec2 diff = my_pos - neighbor_pos;
float dist = length(diff);
float overlap = SEPARATION_RADIUS - dist;
separation_force += normalize(diff) * overlap;
```

This is one of the three classic "boid" behaviors (separation, alignment, cohesion). Endless only uses separation.

Used in: `npc_navigation.gd`, `separation_compute.glsl`, `gpu_separation.gd`

---

## Compute Shaders

Run code on the GPU in parallel. Instead of one CPU core processing 10,000 NPCs sequentially, 1,000+ GPU cores process them simultaneously.

**CPU (sequential):**
```
for i in 10000:
    process(npc[i])  # one at a time
```

**GPU (parallel):**
```glsl
// This runs 10,000 times simultaneously
void main() {
    uint i = gl_GlobalInvocationID.x;  // each invocation gets unique ID
    process(npc[i]);
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

Used in: `gpu_separation.gd`, `separation_compute.glsl`, Rust `GpuCompute`

---

## MultiMesh Rendering

Draw thousands of identical meshes in one draw call.

**Without MultiMesh:** 10,000 NPCs = 10,000 draw calls. GPU stalls on each call.

**With MultiMesh:** 10,000 NPCs = 1 draw call. GPU renders all instances together.

```gdscript
# One-time setup
multimesh.instance_count = 10000
multimesh.mesh = quad_mesh

# Per-frame: upload all transforms at once
var buffer: PackedFloat32Array  # 12 floats per instance
for i in npc_count:
    buffer[i * 12 + 3] = positions[i].x   # origin.x
    buffer[i * 12 + 7] = positions[i].y   # origin.y
RenderingServer.multimesh_set_buffer(multimesh_rid, buffer)
```

The `set_buffer()` call uploads all transforms in one GPU transfer. Per-instance calls (`set_instance_transform()`) are 50x slower.

Used in: `npc_renderer.gd`, Rust `NpcBenchmark`

---

## Staggered Processing

Don't update everything every frame. Spread work across frames.

**Without stagger:** Update 10,000 NPCs every frame. CPU spike.

**With stagger:** Update 1,250 NPCs per frame (1/8 each frame). Smooth load.

```gdscript
var scan_offset = 0
const SCAN_FRACTION = 8

func _process(delta):
    var start = scan_offset * npc_count / SCAN_FRACTION
    var end = (scan_offset + 1) * npc_count / SCAN_FRACTION

    for i in range(start, end):
        update_combat(i)  # only update this slice

    scan_offset = (scan_offset + 1) % SCAN_FRACTION
```

Trade-off: reactions are delayed by up to `SCAN_FRACTION` frames. Tune per-system.

Used in: `npc_combat.gd` (1/8), `npc_navigation.gd` (1/4 for separation)

---

## LOD Intervals (Level of Detail)

Update frequency based on importance/activity.

| State | Update Interval | Why |
|-------|-----------------|-----|
| Fighting | 2 frames | Need fast reactions |
| Moving | 5 frames | Moderate precision |
| Idle | 30 frames | Nothing happening |

Distance also affects LOD — off-screen NPCs update less often.

```gdscript
var interval = BASE_INTERVAL
if states[i] == State.FIGHTING:
    interval = 2
elif states[i] == State.IDLE:
    interval = 30

if frame_count % interval == (i % interval):  # spread across frames
    update_logic(i)
```

Used in: `npc_navigation.gd`

---

## ECS (Entity Component System)

Bevy's architecture (used in Rust POC). Alternative to DOD arrays.

- **Entity:** Just an ID (integer)
- **Component:** Data attached to entity (Position, Velocity, Health)
- **System:** Function that processes entities with specific components

```rust
// Components
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

Used in: Rust POC (though current impl uses simple arrays, not full Bevy ECS)

---

## Summary

| Concept | Problem | Solution |
|---------|---------|----------|
| DOD | Object overhead, cache misses | Parallel arrays |
| Spatial Grid | O(n²) neighbor search | O(n×k) cell lookup |
| Separation | NPC overlap | Push forces from neighbors |
| Compute Shader | CPU bottleneck | GPU parallelism |
| MultiMesh | Draw call overhead | Batched rendering |
| Stagger | Frame spikes | Spread work across frames |
| LOD Intervals | Wasted updates | Update based on importance |
| ECS | DOD ergonomics | Entity/Component/System pattern |
