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
4. `write_proj_buffers` uploads all buffers to GPU
5. `ProjectileComputeNode` dispatches shader

Spawn data includes: position, velocity, damage, faction, shooter index, lifetime.

- **Melee**: speed=500, lifetime=0.5s (from `AttackStats::melee()`)
- **Ranged**: speed=200, lifetime=3.0s (from `AttackStats::ranged()`)

## GPU Dispatch

`projectile_compute.wgsl` — 64 threads per workgroup, `ceil(proj_count / 64)` dispatches.

For each active projectile:
1. **Lifetime**: `lifetime -= delta`. If <= 0, deactivate and hide at (-9999, -9999).
2. **Movement**: `pos += velocity * delta`
3. **Collision**: Skip if already hit. Compute grid cell, scan 3x3 neighborhood:
   - Skip same faction (no friendly fire)
   - Skip dead NPCs (`health <= 0`)
   - If distance² < hit_radius²: write `hit = ivec2(npc_idx, 0)`, deactivate, hide.

## Hit Processing

Not yet implemented in Bevy. The shader writes hits to `proj_hits` buffer, but no GPU→CPU readback reads them back. When implemented:

```
for each projectile with hit.x >= 0 and hit.y == 0 (unprocessed):
    push DamageMsg { npc_index: hit.x, amount: damage }
    recycle slot via ProjSlotAllocator
    mark hit.y = 1 (processed)
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
              ProjSlotAllocator ◀──── return slot (not yet implemented)
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

- **No GPU→CPU hit readback**: Shader writes hits but CPU never reads them back. Projectile damage doesn't reach Bevy's combat pipeline.
- **No projectile rendering**: No instanced draw pipeline for projectiles (unlike NPCs which have `npc_render.rs`). Projectiles compute but are invisible.
- **Grid not yet built on GPU**: Projectile shader reads NPC spatial grid, but NPC compute doesn't build the grid yet (modes 0/1 not ported). Collision detection is non-functional.
- **proj_count never shrinks**: High-water mark. Slot recycling exists in `ProjSlotAllocator` but freed slots don't reduce dispatch count.
- **No projectile-projectile collision**: Projectiles pass through each other.
- **Hit buffer must init to -1**: GPU default of 0 would falsely indicate "hit NPC 0".

## Rating: 4/10

Pipeline compiles and dispatches. WGSL shader is fully ported with movement, lifetime, and grid-based collision logic. Buffers are allocated and uploaded. However: no hit readback (damage doesn't work), no rendering (invisible projectiles), and collision depends on the NPC spatial grid which isn't built yet. The plumbing is complete but the system is non-functional end-to-end.
