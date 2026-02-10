# Projectile System

## Overview

GPU-accelerated projectiles with WGSL compute shader for movement and spatial grid collision detection. Supports up to 50,000 simultaneous projectiles with slot reuse via `ProjSlotAllocator`. Runs as a render graph node after NPC compute, sharing the NPC spatial grid for collision queries.

## Data Flow

```
attack_system fires projectile
    → ProjGpuUpdate::Spawn pushed to PROJ_GPU_UPDATE_QUEUE
    → populate_proj_buffer_writes drains to ProjBufferWrites
    → ExtractResource clones to render world
    → write_proj_buffers uploads to GPU
    → ProjectileComputeNode dispatches projectile_compute.wgsl
        ├─ Decrement lifetime → deactivate expired
        ├─ Move by velocity × delta
        └─ Collision via NPC spatial grid → write hit buffer
```

## Render Graph Order

```
NpcComputeNode → ProjectileComputeNode → CameraDriverLabel → Transparent2d
```

Projectile compute runs after NPC compute because it reads the NPC spatial grid (built by NPC compute modes 0+1, not yet ported) and NPC positions/factions/healths.

## Fire Path

Projectiles originate from Bevy's `attack_system`. The flow:

1. `attack_system` pushes `ProjGpuUpdate::Spawn` to `PROJ_GPU_UPDATE_QUEUE`
2. `populate_proj_buffer_writes` (PostUpdate) drains queue into `ProjBufferWrites` flat arrays
3. `ExtractResource` clones to render world
4. `write_proj_buffers` uploads per-slot (spawn writes all fields, deactivate writes active+hits only)
5. `ProjectileComputeNode` dispatches shader

Spawn data includes: position, velocity, damage, faction, shooter index, lifetime.

- **Melee**: speed=500, lifetime=0.5s (from `AttackStats::melee()`)
- **Ranged**: speed=200, lifetime=3.0s (from `AttackStats::ranged()`)

## GPU Dispatch

`projectile_compute.wgsl` — 64 threads per workgroup, `ceil(proj_count / 64)` dispatches.

For each active projectile:
1. **Lifetime**: `lifetime -= delta`. If <= 0, deactivate, hide at (-9999, -9999), and write `proj_hits[i] = (-2, 0)` (expired sentinel for CPU slot recycling).
2. **Movement**: `pos += velocity * delta`
3. **Collision**: Skip if already hit. Compute grid cell, scan 3x3 neighborhood:
   - Skip same faction (no friendly fire)
   - Skip dead NPCs (`health <= 0`)
   - If distance² < hit_radius²: write `hit = ivec2(npc_idx, 0)`, deactivate, hide.

## Hit Processing

`readback_all` (unified NPC + projectile readback) reads hit results and positions from double-buffered GPU staging buffers to CPU statics (`PROJ_HIT_STATE`, `PROJ_POSITION_STATE`) via a single `device.poll()`. `process_proj_hits` then handles two cases:

```
for each projectile slot:
    if hit.x >= 0 and hit.y == 0 (collision):
        push DamageMsg { npc_index: hit.x, amount: damage }
        recycle slot via ProjSlotAllocator
        send ProjGpuUpdate::Deactivate to GPU
    if hit.x == -2 (expired sentinel):
        recycle slot via ProjSlotAllocator
        send ProjGpuUpdate::Deactivate to GPU
```

## GPU Buffers

### Projectile Buffers (ProjGpuBuffers)

| Binding | Name | Type | Per-Proj Size | R/W | Purpose |
|---------|------|------|--------------|-----|---------|
| 0 | proj_positions | vec2\<f32\> | 8B | RW | Current XY |
| 1 | proj_velocities | vec2\<f32\> | 8B | RW | Direction × speed |
| 2 | proj_damages | f32 | 4B | RW | Damage on hit |
| 3 | proj_factions | i32 | 4B | RW | Shooter's faction |
| 4 | proj_shooters | i32 | 4B | RW | Shooter NPC index |
| 5 | proj_lifetimes | f32 | 4B | RW | Seconds remaining |
| 6 | proj_active | i32 | 4B | RW | 1=active, 0=inactive |
| 7 | proj_hits | vec2\<i32\> | 8B | RW | (npc_idx, processed). Init -1. |

### Shared NPC Buffers (read-only)

| Binding | Name | Source |
|---------|------|--------|
| 8 | npc_positions | NpcGpuBuffers.positions |
| 9 | npc_factions | NpcGpuBuffers.factions |
| 10 | npc_healths | NpcGpuBuffers.healths |
| 11 | grid_counts | NpcGpuBuffers.grid_counts |
| 12 | grid_data | NpcGpuBuffers.grid_data |

### Uniform Params

| Binding | Field | Default | Purpose |
|---------|-------|---------|---------|
| 13 | proj_count | 0 | Active projectile count (from ProjSlotAllocator.next) |
| | npc_count | 0 | NPC count for bounds checking |
| | delta | 0.016 | Frame delta time |
| | hit_radius | 10.0 | Collision detection radius (px) |
| | grid_width | 128 | Spatial grid columns |
| | grid_height | 128 | Spatial grid rows |
| | cell_size | 64.0 | Pixels per grid cell |
| | max_per_cell | 48 | Max NPCs per grid cell |

## Slot Lifecycle

```
PROJ_GPU_UPDATE_QUEUE → ProjBufferWrites → GPU ──▶ ACTIVE
                         ▲                            │
                         │                    hit or expire
                         │                            │
                         │                            ▼
              ProjSlotAllocator ◀──── process_proj_hits frees slot
```

`ProjSlotAllocator` (Bevy Resource) manages slot indices with an internal free list, same pattern as NPC `SlotAllocator`. `proj_count` is the high-water mark from `ProjSlotAllocator.next`.

## Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| MAX_PROJECTILES | 50,000 | Pool capacity |
| HIT_RADIUS | 10.0 | Collision detection radius (px) |
| WORKGROUP_SIZE | 64 | GPU threads per workgroup |
| Melee speed | 500.0 | AttackStats::melee() projectile speed |
| Ranged speed | 200.0 | AttackStats::ranged() projectile speed |
| Melee lifetime | 0.5s | AttackStats::melee() projectile lifetime |
| Ranged lifetime | 3.0s | AttackStats::ranged() projectile lifetime |

## Known Issues

- **proj_count never shrinks**: High-water mark (`ProjSlotAllocator.next`). Freed slots are recycled via LIFO free list but don't reduce dispatch count.
- **No projectile-projectile collision**: Projectiles pass through each other.
- **Hit buffer must init to -1**: GPU default of 0 would falsely indicate "hit NPC 0".

## Rating: 7/10

Full end-to-end pipeline: compute shader moves projectiles, spatial grid collision detects hits, readback sends damage to ECS, instanced rendering draws faction-colored projectiles. Unified `readback_all` handles NPC + projectile readback with double-buffered ping-pong staging and a single `device.poll()`. Expired projectiles signal CPU via `-2` sentinel for slot recycling. Per-slot dirty tracking minimizes GPU uploads. Rendering reuses the NPC pipeline (same shader, quad, bind groups) with a separate instance buffer. Projectiles render above NPCs via sort key.
