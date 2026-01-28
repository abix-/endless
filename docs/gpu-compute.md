# GPU Compute

## Overview

Two compute shaders run on the GPU each frame: `npc_compute.glsl` handles NPC movement, separation physics, and combat targeting; `projectile_compute.glsl` handles projectile movement and collision detection. Both use a spatial grid for O(1) neighbor lookups.

## Data Flow

```
CPU (process())                          GPU
│                                        │
├─ Build spatial grid (CPU)              │
├─ Upload grid buffers ──────────────────┤
├─ Dispatch npc_compute.glsl ───────────▶├─ Read positions, targets, speeds
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

Workgroup: 64 threads. Dispatched as `ceil(npc_count / 64)` workgroups.

| Step | What it does |
|------|-------------|
| 1 | Read current state (position, target, speed, arrival, backoff) |
| 2 | Avoidance force — scan 3x3 grid neighborhood, push away from NPCs within 20px. Asymmetric: moving NPCs shove harder through settled ones. Golden angle spreading for stack-ups. |
| 2b | TCP-style dodge — head-on, overtaking, crossing path avoidance |
| 3 | Movement toward target — `velocity = normalize(target - pos) * speed * (1 / (1 + backoff))` |
| 4 | Blocking detection — push away from target = blocked (backoff += 2), push toward = progress (backoff -= 2), backoff > 120 = give up (settled) |
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
| 2 | color_buffer | vec4 | 16B | R | RGBA color |
| 3 | speed_buffer | float | 4B | R | Movement speed |
| 4 | grid_counts_buffer | int[] | - | R | NPCs per grid cell |
| 5 | grid_data_buffer | int[] | - | R | NPC indices per cell |
| 6 | multimesh_buffer | float[16] | 64B | W | Direct render output (Transform2D + Color + CustomData) |
| 7 | arrival_buffer | int | 4B | RW | Settled/arrived flag |
| 8 | backoff_buffer | int | 4B | RW | Collision backoff counter |
| 9 | faction_buffer | int | 4B | R | 0=Villager, 1=Raider |
| 10 | health_buffer | float | 4B | R | Current HP |
| 11 | combat_target_buffer | int | 4B | W | Nearest enemy index or -1 |
| 12 | sprite_frame_buffer | vec2 | 8B | R | Sprite sheet column/row |

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

Built on CPU each frame from cached positions, uploaded to GPU before dispatch. NPCs are binned by `floor(pos / cell_size)`. The 3x3 neighborhood search covers a 300px radius (3 * 100px), which matches the combat detection range.

## CPU Cache Sync

After each dispatch, the CPU reads back:

| Buffer | When | Storage |
|--------|------|---------|
| position_buffer | Every frame | `gpu.positions[]` + `GPU_READ_STATE.positions` |
| combat_target_buffer | Every frame | `gpu.combat_targets[]` + `GPU_READ_STATE.combat_targets` |
| arrival_buffer | Every frame | Compared to `prev_arrivals[]`, deltas pushed to ARRIVAL_QUEUE |
| health_buffer | Cached on CPU | `gpu.healths[]` + `GPU_READ_STATE.health` (written by CPU, not read back from GPU) |
| proj_hit_buffer | Every frame (if proj_count > 0) | Parsed for hits, routed to DAMAGE_QUEUE |
| proj_position_buffer | Every frame (if proj_count > 0) | `gpu.proj_positions[]` |
| proj_active_buffer | Every frame (if proj_count > 0) | `gpu.proj_active[]` |

Note: health_buffer is CPU-authoritative — it's written to GPU but never read back. The GPU only reads it for targeting (skip dead NPCs).

## Known Issues / Limitations

- **Grid rebuilt every frame on CPU**: The spatial grid is built in Rust and uploaded. A GPU-side grid build would eliminate this transfer but adds complexity.
- **Health is CPU-authoritative**: The GPU reads health for targeting but never modifies it. If GPU-side damage were ever needed, this would require a readback.
- **Fixed grid dimensions**: 80x80 grid is hardcoded. Larger worlds need a bigger grid or dynamic sizing.
- **Max 64 NPCs per cell**: Exceeding this silently drops NPCs from neighbor queries. At 10K NPCs in 6,400 cells, average is ~1.5 per cell, so this is safe with margin.
- **Wasted multimesh_buffer write**: NPC shader writes Transform2D+Color+CustomData to binding 6, but the CPU rebuilds the MultiMesh from cached positions/colors/sprite_frames via `build_multimesh_from_cache()`. The GPU-written buffer is unused for rendering — wasted GPU work every frame.
- **Blocking sync**: `rd.sync()` stalls CPU until GPU completes. No async readback or double-buffering.
- **Two sequential dispatches**: NPC and projectile shaders run with a full sync between them. Could be pipelined.
- **Hit buffer init**: Must be initialized to -1. GPU default of 0 would falsely indicate "hit NPC 0".

## Rating: 8/10

Solid GPU compute achieving 10K NPCs @ 140fps. Spatial grid shared between both shaders is efficient. TCP-style backoff produces good crowd behavior. Main waste: the GPU writes a multimesh buffer that the CPU ignores and rebuilds. Fixing this (use GPU-written buffer directly) would eliminate a per-frame CPU rebuild. Blocking sync prevents CPU/GPU overlap.
