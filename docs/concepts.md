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

## GPU Readback Avoidance

Reading data back from GPU to CPU is expensive. Avoid it whenever possible.

**The problem:**
```
CPU → GPU: Upload positions (fast, ~1ms)
GPU: Run compute shader (fast, ~0.1ms)
GPU → CPU: Read positions back (SLOW, ~5-10ms)
```

The GPU→CPU transfer stalls the pipeline — CPU waits for GPU to finish, then copies data over PCIe.

**Solution: Cache on CPU**

Instead of reading positions back from GPU, maintain a CPU-side copy:

```rust
// BAD: Read 480KB back from GPU every frame
let positions = gpu.read_buffer(position_buffer);
build_multimesh(positions);

// GOOD: Cache positions on CPU, only upload to GPU
cpu_positions[i] += velocity * delta;  // Update CPU copy
gpu.write_buffer(position_buffer, &cpu_positions);  // Upload
build_multimesh(&cpu_positions);  // Use CPU copy for MultiMesh
```

This eliminated a 480KB/frame readback in Endless, saving ~5ms/frame.

**When you must read back:**
- Keep buffers small (read only what's needed)
- Read asynchronously if possible (don't block on result)
- Batch reads (one large read beats many small ones)

Used in: Rust `NpcBenchmark` (positions cached in `CpuPositions` resource)

---

## Debug Mode Overhead

Debug metrics can cost more than the actual simulation. Disable or throttle them.

**The trap:** You add O(n²) validation to verify NPCs are properly separated:

```gdscript
func _get_min_separation() -> float:
    var min_dist = INF
    for i in npc_count:
        for j in range(i + 1, npc_count):
            var d = positions[i].distance_to(positions[j])
            min_dist = min(min_dist, d)
    return min_dist
```

With 5,000 NPCs, that's 12.5 million distance checks per frame. Your 140fps simulation drops to 15fps — but the simulation itself is fine, only the *measurement* is slow.

**Solutions:**

1. **Disable by default:** Make expensive metrics opt-in
   ```gdscript
   var metrics_enabled := false  # Off by default

   if metrics_enabled:
       min_separation = _get_min_separation()  # O(n²)
   ```

2. **Throttle:** Run expensive checks once per second, not every frame
   ```gdscript
   var metric_timer := 0.0

   func _process(delta):
       metric_timer += delta
       if metric_timer >= 1.0:
           metric_timer = 0.0
           _update_expensive_metrics()  # Only once/second
   ```

3. **Sample:** Check 100 random pairs instead of all pairs
   ```gdscript
   for _i in 100:
       var a = randi() % npc_count
       var b = randi() % npc_count
       min_dist = min(min_dist, positions[a].distance_to(positions[b]))
   ```

**Rule of thumb:** If your metric is O(n²) or worse, it needs a toggle.

Used in: `ecs_test.gd` (metrics checkbox), debug stats throttling

---

## Asymmetric Push

Moving NPCs should push through settled ones, not get blocked.

**The problem:** With symmetric separation forces, a moving NPC approaching a group gets pushed back as hard as they push forward. They can't enter the crowd.

**Solution:** Asymmetric push strengths based on movement state:

```glsl
float push_strength = 1.0;
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

Used in: `npc_compute.glsl` (separation shader)

---

## TCP Dodge

When two moving NPCs approach each other, dodge sideways instead of stopping.

Named after TCP congestion avoidance — when packets collide, back off and try a different path.

**The problem:** Two NPCs walking toward each other with symmetric separation forces will push directly against each other, creating a standoff or oscillation.

**Solution:** Detect approaching collision and add perpendicular dodge:

```glsl
vec2 to_neighbor = neighbor_pos - my_pos;
float approach_speed = dot(my_velocity, normalize(to_neighbor));

if (approach_speed > 0) {  // We're closing in
    // Dodge perpendicular to approach direction
    vec2 perp = vec2(-to_neighbor.y, to_neighbor.x);

    // Consistent side: lower index dodges right
    float side = (my_index < neighbor_index) ? 1.0 : -1.0;

    dodge += normalize(perp) * side * approach_speed;
}
```

**Key details:**
- Only dodge around other *moving* NPCs (settled ones use asymmetric push)
- Consistent side selection prevents both NPCs dodging the same way
- Dodge strength scales with approach speed (faster approach = harder dodge)

Used in: `npc_compute.glsl` (TCP-style collision avoidance)

---

## State Machines in ECS

Bevy ECS represents states as marker components, not enums.

**Traditional state machine:**
```rust
enum GuardState { Patrolling, OnDuty, Resting, GoingToRest }

struct Guard {
    state: GuardState,
    // ... other fields
}
```

**ECS state machine:** States are separate components. An entity has exactly one state component at a time.

```rust
// Marker components (no data, just tags)
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

**Systems per state:**
```rust
// Only runs for guards in Patrolling state
fn patrol_system(guards: Query<&mut Guard, With<Patrolling>>) { ... }

// Only runs for guards in OnDuty state
fn on_duty_system(guards: Query<(&mut Guard, &mut OnDuty)>) { ... }
```

Used in: Rust guard behavior (Patrolling → OnDuty → GoingToRest → Resting → Patrolling)

---

## World Data Resources

Static world layout (buildings, locations) stored as ECS Resources, not entities.

**Entities vs Resources:**
- **Entity:** Dynamic, many instances, has lifecycle (spawn/despawn). NPCs, projectiles.
- **Resource:** Singleton, shared state, lives forever. World layout, config, occupancy tracking.

```rust
// Resource: one instance, globally accessible
#[derive(Resource, Default)]
pub struct WorldData {
    pub towns: Vec<Town>,
    pub farms: Vec<Farm>,
    pub beds: Vec<Bed>,
    pub guard_posts: Vec<GuardPost>,
}

// Individual building data (not an entity, just a struct)
pub struct GuardPost {
    pub position: Vector2,
    pub town_idx: u32,
    pub patrol_order: u32,  // 0-3 for clockwise perimeter
}
```

**Accessing in systems:**
```rust
fn patrol_system(
    world: Res<WorldData>,  // Read-only access to world
    guards: Query<(&Guard, &mut Target)>,
) {
    for (guard, mut target) in guards.iter_mut() {
        let post = &world.guard_posts[guard.current_post as usize];
        target.0 = post.position;
    }
}
```

**Occupancy tracking:** Mutable resources track which NPCs occupy which buildings:

```rust
#[derive(Resource, Default)]
pub struct BedOccupancy {
    pub occupant_npc: Vec<i32>,  // -1 = free, >= 0 = NPC index
}
```

**GDScript → Rust:** World data initialized once from GDScript via static Mutex:

```rust
static WORLD_DATA: LazyLock<Mutex<WorldData>> = LazyLock::new(|| ...);

// Called from GDScript at scene load
fn set_world_data(towns: Array, farms: Array, ...) {
    let mut world = WORLD_DATA.lock().unwrap();
    world.towns = parse_towns(towns);
    // ...
}
```

Used in: Rust `WorldData`, `BedOccupancy`, `FarmOccupancy` resources

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
| GPU Readback | Pipeline stalls | Cache on CPU, upload only |
| Debug Overhead | Metrics kill perf | Disable/throttle expensive checks |
| Asymmetric Push | Can't enter crowds | Moving NPCs push through settled |
| TCP Dodge | Head-on collisions | Perpendicular dodge on approach |
| ECS States | State machine in ECS | Marker components per state |
| World Resources | Static world data | Singleton Resources, not Entities |
