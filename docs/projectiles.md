# Projectile System

## Overview

GPU-accelerated projectiles with WGSL compute shader for movement and spatial grid collision detection. Supports up to 50,000 simultaneous projectiles with slot reuse via `ProjSlotAllocator`. Runs as a render graph node after NPC compute, sharing the NPC spatial grid for collision queries.

## Data Flow

```
attack_system fires projectile
    → ProjGpuUpdate::Spawn pushed to PROJ_GPU_UPDATE_QUEUE
    → populate_proj_buffer_writes drains to ProjBufferWrites
    → extract_proj_data (Extract<Res<T>>, zero-clone) writes dirty slots to GPU
    → ProjectileComputeNode dispatches projectile_compute.wgsl (3 modes)
        ├─ Mode 0: Clear projectile spatial grid
        ├─ Mode 1: Build projectile spatial grid (for NPC dodge next frame)
        └─ Mode 2: Decrement lifetime → move → collision via NPC spatial grid → write hit buffer
```

## Render Graph Order

```
NpcComputeNode → ProjectileComputeNode → CameraDriverLabel → Transparent2d
```

Projectile compute runs after NPC compute because it reads the NPC spatial grid (built by NPC compute modes 0+1) and NPC positions/factions/healths. NPC compute reads the projectile spatial grid (built by projectile compute in the previous frame) for projectile dodge — 1-frame latency is acceptable.

## Fire Path

Projectiles originate from Bevy's `attack_system`. The flow:

1. `attack_system` pushes `ProjGpuUpdate::Spawn` to `PROJ_GPU_UPDATE_QUEUE`
2. `populate_proj_buffer_writes` (PostUpdate) drains queue into `ProjBufferWrites` flat arrays
3. `extract_proj_data` (ExtractSchedule) reads `ProjBufferWrites` via `Extract<Res<T>>` (zero-clone), writes dirty slots to GPU, builds projectile instance buffer
4. `ProjectileComputeNode` dispatches shader

Spawn data includes: position, velocity, damage, faction, shooter index, lifetime.

- **Melee**: speed=500, lifetime=0.5s (from `AttackStats::melee()`)
- **Ranged**: speed=200, lifetime=3.0s (from `AttackStats::ranged()`)

## GPU Dispatch

`projectile_compute.wgsl` — 64 threads per workgroup. 3 dispatches per frame with different `mode` uniform values, mirroring the NPC compute pattern.

### Mode 0: Clear Projectile Grid
One thread per grid cell. Atomically clears `proj_grid_counts[cell]` to 0. Dispatches `ceil(grid_cells / 64)` workgroups.

### Mode 1: Build Projectile Grid
One thread per projectile. Computes cell from `floor(pos / cell_size)`, atomically increments `proj_grid_counts[cell]`, writes projectile index into `proj_grid_data[cell * max_per_cell + slot]`. Skips inactive/hidden projectiles. The resulting grid is read by NPC compute (mode 2) in the next frame for projectile dodge.

### Mode 2: Movement + Collision
For each active projectile:
1. **Lifetime**: `lifetime -= delta`. If <= 0, deactivate, hide at (-9999, -9999), and write `proj_hits[i] = (-2, 0)` (expired sentinel for CPU slot recycling).
2. **Movement**: `pos += velocity * delta`
3. **Collision**: Skip if already hit. Compute grid cell, scan 3x3 neighborhood of NPC spatial grid:
   - Skip same faction or neutral faction -1 (no friendly fire)
   - Skip dead NPCs (`health <= 0`)
   - Oriented rectangle collision (long along velocity, thin perpendicular)
   - If hit: write `hit = ivec2(npc_idx, 0)`, deactivate, hide.

## Hit Processing

`ReadbackComplete` observers write hit results and positions directly to `Res<ProjHitState>` and `Res<ProjPositionState>` (Bevy async readback, no manual staging). `process_proj_hits` handles two phases:

**NPC + Building hits** (from GPU hit buffer):
```
for slot in 0..min(proj_alloc.next, hit_state.len()):
    skip if proj_writes.active[slot] == 0 (inactive, stale in readback)
    if hit.x >= 0 and hit.y == 0 (collision):
        if BuildingSlotMap.is_building(hit.x):
            look up (kind, index) via BuildingSlotMap.get_building()
            push BuildingDamageMsg { kind, index, amount, attacker_faction }
        else:
            push DamageMsg { npc_index: hit.x, amount: damage }
        recycle slot via ProjSlotAllocator
        send ProjGpuUpdate::Deactivate to GPU
    if hit.x == -2 (expired sentinel):
        recycle slot via ProjSlotAllocator
        send ProjGpuUpdate::Deactivate to GPU
```

Buildings occupy NPC GPU slots (speed=0, sprite hidden via col=-1). The projectile compute shader detects building hits through the NPC spatial grid — no separate CPU building collision pass needed. `BuildingSlotMap` routes hits to `BuildingDamageMsg` vs `DamageMsg`.

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

### Projectile Spatial Grid (built by modes 0+1, read by NPC compute for dodge)

| Binding | Name | Type | R/W | Purpose |
|---------|------|------|-----|---------|
| 14 | proj_grid_counts | atomic\<i32\>[] | RW | Projectiles per grid cell (atomically written by modes 0+1) |
| 15 | proj_grid_data | i32[] | RW | Projectile indices per cell (written by mode 1) |

### Uniform Params

| Binding | Field | Default | Purpose |
|---------|-------|---------|---------|
| 13 | proj_count | 0 | Active projectile count (from ProjSlotAllocator.next) |
| | npc_count | 0 | NPC count for bounds checking |
| | delta | 0.016 | Frame delta time |
| | hit_half_length | 10.0 | Oriented rectangle half-length along velocity (px) |
| | hit_half_width | 5.0 | Oriented rectangle half-width perpendicular to velocity (px) |
| | grid_width | 256 | Spatial grid columns (same as NPC grid) |
| | grid_height | 256 | Spatial grid rows |
| | cell_size | 128.0 | Pixels per grid cell |
| | max_per_cell | 48 | Max entries per grid cell |
| | mode | 0 | Dispatch mode (0=clear grid, 1=build grid, 2=movement+collision) |

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
- **Hit buffer must init to [-1, 0]**: `setup_readback_buffers` initializes proj hit `ShaderStorageBuffer` with `[-1, 0]` per slot. GPU zeroes would falsely indicate "hit NPC 0".
