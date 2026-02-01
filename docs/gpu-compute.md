# GPU Compute

## Overview

Two compute shaders run on the GPU each frame: `npc_compute.glsl` handles NPC movement, separation physics, and combat targeting; `projectile_compute.glsl` handles projectile movement and collision detection. Both use a spatial grid for O(1) neighbor lookups.

## Data Flow

```
CPU (process())                          GPU
│                                        │
├─ Dispatch npc_compute.glsl (mode 0) ──▶├─ Clear grid counts to 0
│                          (barrier)     │
├─ Dispatch npc_compute.glsl (mode 1) ──▶├─ Insert NPCs into grid (atomics)
│                          (barrier)     │
├─ Dispatch npc_compute.glsl (mode 2) ──▶├─ Read positions, targets, speeds
│                                        ├─ Separation physics (3x3 grid)
│                                        ├─ Movement toward target
│                                        ├─ Blocking detection + backoff
│                                        ├─ Combat targeting (nearest enemy)
│◀──────────────────── Read back ────────├─ Write positions, arrivals, combat_targets
│                                        │
├─ Dispatch projectile_compute.glsl ────▶├─ Decrement lifetime
│                                        ├─ Move by velocity
│                                        ├─ Collision check (3x3 grid)
│◀──────────────────── Read back ────────├─ Write hits, positions, active flags
```

## NPC Compute Shader (npc_compute.glsl)

Workgroup: 64 threads. Dispatched 3 times per frame with different modes:

### Mode 0: Clear Grid
Dispatched as `ceil(grid_cells / 64)` workgroups. Clears `grid_counts[i] = 0` for all cells.

### Mode 1: Insert NPCs
Dispatched as `ceil(npc_count / 64)` workgroups. Each NPC atomically inserts itself:
```glsl
int slot = atomicAdd(grid_counts[cell_idx], 1);
if (slot < MAX_PER_CELL) {
    grid_data[cell_idx * MAX_PER_CELL + slot] = npc_idx;
}
```
Skips hidden NPCs (position < -9000).

### Mode 2: Main Logic
Dispatched as `ceil(npc_count / 64)` workgroups.

| Step | What it does |
|------|-------------|
| 1 | Read current state (position, target, speed, arrival, backoff) |
| 2 | Avoidance force — scan 3x3 grid neighborhood, push away from NPCs within 20px. Asymmetric: moving NPCs shove harder through settled ones. Golden angle spreading for stack-ups. |
| 2b | TCP-style dodge — head-on, overtaking, crossing path avoidance |
| 3 | Movement toward target — `velocity = normalize(target - pos) * speed * (1 / (1 + backoff))` |
| 4 | Blocking detection — push away from target = blocked (backoff += 2), push toward = progress (backoff -= 2), backoff capped at 200 (no fake arrivals) |
| 5 | Apply movement — `pos += (movement + avoidance) * delta` |
| 5b | Combat targeting — find nearest hostile NPC within 300px using 3x3 grid. Skip dead (health <= 0) and same faction. |
| 6 | Write output — positions, arrivals, backoff, combat_targets, MultiMesh buffer |

## Projectile Compute Shader (projectile_compute.glsl)

Workgroup: 64 threads. Dispatched as `ceil(proj_count / 64)` workgroups.

| Step | What it does |
|------|-------------|
| 1 | Skip inactive projectiles |
| 2 | Decrement lifetime, deactivate if <= 0 |
| 3 | Move: `pos += velocity * delta` |
| 4 | Collision — scan 3x3 grid neighborhood, check distance < 10px, skip same faction, skip dead. Record hit as `ivec2(npc_idx, 0)`. Deactivate and hide at (-9999, -9999). |

## GPU Buffers

### NPC Buffers (npc_compute.glsl)

| Binding | Name | Type | Per-NPC Size | R/W | Purpose |
|---------|------|------|-------------|-----|---------|
| 0 | position_buffer | vec2 | 8B | RW | Current XY position |
| 1 | target_buffer | vec2 | 8B | R | Movement target |
| 2 | color_buffer | vec4 | 16B | R | RGBA color (visual only) |
| 3 | speed_buffer | float | 4B | R | Movement speed |
| 4 | grid_counts_buffer | int[] | - | RW | NPCs per grid cell (cleared/written by modes 0/1) |
| 5 | grid_data_buffer | int[] | - | RW | NPC indices per cell (written by mode 1) |
| 6 | (reserved) | - | - | - | Was multimesh_buffer, now unused |
| 7 | arrival_buffer | int | 4B | RW | Settled/arrived flag (0=moving, 1=arrived) |
| 8 | backoff_buffer | int | 4B | RW | Collision backoff counter |
| 9 | faction_buffer | int | 4B | R | 0=Villager, 1+=Raider camps |
| 10 | health_buffer | float | 4B | R | Current HP |
| 11 | combat_target_buffer | int | 4B | W | Nearest enemy index or -1 |

### Projectile Buffers (projectile_compute.glsl)

| Binding | Name | Type | Per-Proj Size | R/W | Purpose |
|---------|------|------|--------------|-----|---------|
| 0 | proj_position_buffer | vec2 | 8B | RW | Current XY |
| 1 | proj_velocity_buffer | vec2 | 8B | RW | Direction * speed |
| 2 | proj_damage_buffer | float | 4B | RW | Damage on hit |
| 3 | proj_faction_buffer | int | 4B | RW | Shooter's faction |
| 4 | proj_shooter_buffer | int | 4B | RW | Shooter NPC index |
| 5 | proj_lifetime_buffer | float | 4B | RW | Seconds remaining |
| 6 | proj_active_buffer | int | 4B | RW | 1=active, 0=inactive |
| 7 | proj_hit_buffer | ivec2 | 8B | RW | (npc_idx, processed). Init -1. |
| 8 | npc_position_buffer | - | - | R | Shared with NPC binding 0 |
| 9 | npc_faction_buffer | - | - | R | Shared with NPC binding 9 |
| 10 | npc_health_buffer | - | - | R | Shared with NPC binding 10 |
| 11 | grid_counts_buffer | - | - | R | Shared with NPC binding 4 |
| 12 | grid_data_buffer | - | - | R | Shared with NPC binding 5 |

## Spatial Grid

- **Cell size**: `GRID_CELL_SIZE` = 100px
- **Grid dimensions**: `GRID_SIZE` = 80x80 (covers 8000x8000 world)
- **Max per cell**: `MAX_PER_CELL` = 64
- **Total cells**: 6,400
- **Memory**: grid_counts = 25.6KB, grid_data = 1.6MB

Built on GPU each frame via atomic operations (mode 0 clears, mode 1 inserts). No CPU-side grid building or upload. NPCs are binned by `floor(pos / cell_size)`. The 3x3 neighborhood search covers a 300px radius (3 * 100px), which matches the combat detection range.

## CPU Cache Sync

After each dispatch, the CPU reads back:

| Buffer | When | Storage |
|--------|------|---------|
| position_buffer | Every frame | `gpu.positions[]` + `GPU_READ_STATE.positions` |
| combat_target_buffer | Every frame | `gpu.combat_targets[]` + `GPU_READ_STATE.combat_targets` |
| arrival_buffer | Every frame | Compared to `prev_arrivals[]`, deltas pushed to ARRIVAL_QUEUE, stats cached in PERF_STATS |
| backoff_buffer | Every frame | Stats (avg, max) cached in PERF_STATS during main sync |
| health_buffer | Cached on CPU | `gpu.healths[]` + `GPU_READ_STATE.health` (written by CPU, not read back from GPU) |
| proj_hit_buffer | Every frame (if proj_count > 0) | Parsed for hits, routed to DAMAGE_QUEUE |
| proj_position_buffer | Every frame (if proj_count > 0) | `gpu.proj_positions[]` |
| proj_active_buffer | Every frame (if proj_count > 0) | `gpu.proj_active[]` |

Note: health_buffer is CPU-authoritative — it's written to GPU but never read back. The GPU only reads it for targeting (skip dead NPCs).

## Known Issues / Limitations

- **Health is CPU-authoritative**: The GPU reads health for targeting but never modifies it. If GPU-side damage were ever needed, this would require a readback.
- **Fixed grid dimensions**: 80x80 grid is hardcoded. Larger worlds need a bigger grid or dynamic sizing.
- **Max 64 NPCs per cell**: Exceeding this silently drops NPCs from neighbor queries. At 10K NPCs in 6,400 cells, average is ~1.5 per cell, so this is safe with margin.
- **Blocking sync**: `rd.sync()` stalls CPU until GPU completes. No async readback or double-buffering.
- **Two sequential dispatches**: NPC and projectile shaders run with a full sync between them. Could be pipelined.
- **Hit buffer init**: Must be initialized to -1. GPU default of 0 would falsely indicate "hit NPC 0".

## Key Optimizations

- **Batched buffer uploads**: CPU caches track dirty ranges; one `buffer_update()` call per buffer type per frame instead of per-NPC (~670 → ~8 calls)
- **O(1) entity lookup**: `NpcEntityMap` (HashMap<usize, Entity>) for instant damage routing
- **Slot reuse**: `FREE_SLOTS` pool recycles dead NPC indices (infinite churn, no 10K cap)
- **Grid sizing**: 100px cells ensure 3×3 neighborhood covers 300px detection range
- **Single locks**: One Mutex per direction instead of 10+ scattered queues
- **Removed dead code**: sprite_frame_buffer (never read by shader), multimesh GPU writes (CPU rebuilds)

## Performance Lessons Learned

**GPU sync() is the bottleneck, not compute:**
- `RenderingDevice.sync()` blocks CPU waiting for GPU (~2.5ms per frame)
- `buffer_get_data()` also stalls pipeline for GPU→CPU transfer
- Godot's local RenderingDevice requires sync() between submits (can't pipeline)
- `buffer_get_data_async()` doesn't work with local RD (Godot issue #105256)

**GDScript O(n²) traps:**
- Calling `get_npc_position()` in nested loops crosses GDScript→Rust boundary 124,750 times for 500 NPCs
- Test assertions must run ONCE when triggered, not every frame after timer passes
- Debug metrics (min separation) must be throttled to 1/sec, not every frame
- `get_debug_stats()` uses cached values from main sync - safe to call anytime

**MultiMesh culling:**
- Godot auto-calculates AABB for canvas items — wrong for world-spanning MultiMesh
- NPCs disappear at close zoom without `canvas_item_set_custom_rect` on the canvas item
- Fix: set large custom rect (-100K to +100K) to disable culling

**What worked:**
- Build multimesh from cached positions on CPU (eliminates 480KB GPU readback)
- Throttle expensive operations to once per second
- Advance test_phase immediately to prevent repeated assertion runs

## Rating: 6/10

GPU compute works but hits fundamental Godot limitations. Blocking `rd.sync()` stalls CPU waiting for GPU (~2.5ms) with no workaround — `buffer_get_data_async()` doesn't work with local RenderingDevice (Godot issue #105256). Two sequential dispatches with full sync between them. Batched buffer uploads helped (0.1ms queue time) but the 9.5ms Godot overhead is outside ECS control. Spatial grid and TCP-style backoff are solid. The architecture is sound but performance ceiling is Godot-imposed.
