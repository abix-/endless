# Projectile System

## Overview

GPU-accelerated projectiles with WGSL compute shader for movement and spatial grid collision detection. Supports up to 50,000 simultaneous projectiles with slot reuse via `ProjSlotAllocator`. Runs as a render graph node after NPC compute, sharing the unified entity spatial grid for collision queries against both NPCs and buildings.

## Data Flow

```
attack_system fires projectile
    → ProjGpuUpdateMsg(ProjGpuUpdate::Spawn) emitted
    → populate_proj_buffer_writes reads messages to update ProjBufferWrites
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

1. `attack_system` emits `ProjGpuUpdateMsg(ProjGpuUpdate::Spawn)`
2. `populate_proj_buffer_writes` (PostUpdate) reads `ProjGpuUpdateMsg` and applies updates to `ProjBufferWrites` flat arrays
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
3. **Collision**: Skip if already hit. Compute grid cell, scan 3x3 neighborhood of entity spatial grid (contains both NPCs and buildings):
   - Skip shooter entity (`entity_idx == proj_shooters[i]`) — prevents self-collision with source
   - Skip same faction or neutral faction -1 (no friendly fire)
   - Skip dead entities (`health <= 0`)
   - Oriented rectangle collision (long along velocity, thin perpendicular)
   - If hit: write `hit = ivec2(entity_idx, 0)`, deactivate, hide. `entity_idx < npc_count` = NPC hit, `entity_idx >= npc_count` = building hit.

## Hit Processing

`ReadbackComplete` observers write hit results and positions directly to `Res<ProjHitState>` and `Res<ProjPositionState>` (Bevy async readback, no manual staging). `process_proj_hits` emits unified `DamageMsg` for all hits:

```
for slot in 0..min(proj_alloc.next, hit_state.len()):
    skip if proj_writes.active[slot] == 0 (inactive, stale in readback)
    if hit.x >= 0 and hit.y == 0 (collision):
        push DamageMsg { entity_idx: hit.x, amount: damage, attacker: shooter, attacker_faction }
        recycle slot via ProjSlotAllocator
        emit ProjGpuUpdateMsg(ProjGpuUpdate::Deactivate)
    if hit.x == -2 (expired sentinel):
        recycle slot via ProjSlotAllocator
        emit ProjGpuUpdateMsg(ProjGpuUpdate::Deactivate)
```

Entity buffer layout: `[0..npc_count]` = NPCs, `[npc_count..entity_count]` = buildings. The GPU collision scans the unified entity spatial grid, so projectiles hit both NPCs and buildings automatically via faction check (no friendly fire on same-faction buildings). `damage_system` routes by `entity_idx`: `< npc_count` → NPC damage, `>= npc_count` → building damage.

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

### Shared Entity Buffers (read-only — contains NPCs + buildings)

| Binding | Name | Source |
|---------|------|--------|
| 8 | entity_positions | EntityGpuBuffers.positions |
| 9 | entity_factions | EntityGpuBuffers.factions |
| 10 | entity_healths | EntityGpuBuffers.healths |
| 11 | grid_counts | EntityGpuBuffers.grid_counts |
| 12 | grid_data | EntityGpuBuffers.grid_data |

### Projectile Spatial Grid (built by modes 0+1, read by NPC compute for dodge)

| Binding | Name | Type | R/W | Purpose |
|---------|------|------|-----|---------|
| 14 | proj_grid_counts | atomic\<i32\>[] | RW | Projectiles per grid cell (atomically written by modes 0+1) |
| 15 | proj_grid_data | i32[] | RW | Projectile indices per cell (written by mode 1) |

### Uniform Params

| Binding | Field | Default | Purpose |
|---------|-------|---------|---------|
| 13 | proj_count | 0 | Active projectile count (from ProjSlotAllocator.next) |
| | npc_count | 0 | NPC count (legacy, kept for compat) |
| | delta | 0.016 | Frame delta time |
| | hit_half_length | 10.0 | Oriented rectangle half-length along velocity (px) |
| | hit_half_width | 5.0 | Oriented rectangle half-width perpendicular to velocity (px) |
| | grid_width | 256 | Spatial grid columns (same as NPC grid) |
| | grid_height | 256 | Spatial grid rows |
| | cell_size | 128.0 | Pixels per grid cell |
| | max_per_cell | 48 | Max entries per grid cell |
| | mode | 0 | Dispatch mode (0=clear grid, 1=build grid, 2=movement+collision) |
| | entity_count | 0 | Total entity count for collision bounds (npc_count + building_count) |

## Slot Lifecycle

```
ProjGpuUpdateMsg → ProjBufferWrites → GPU ──▶ ACTIVE
                         ▲                            │
                         │                    hit or expire
                         │                            │
                         │                            ▼
              ProjSlotAllocator ◀──── process_proj_hits frees slot
```

`ProjSlotAllocator` (Bevy Resource) manages slot indices with an internal free list, same pattern as NPC `SlotAllocator`. `proj_count` is the high-water mark from `ProjSlotAllocator.next`.

`ProjBufferWrites.active_set` tracks currently active projectile indices — maintained incrementally by `apply()` (push on Spawn, swap_remove on Deactivate). `extract_proj_data` iterates only `active_set` instead of scanning `0..proj_count`, avoiding O(high_water_mark) per frame.

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

- **No projectile-projectile collision**: Projectiles pass through each other.
- **Hit buffer must init to [-1, 0]**: `setup_readback_buffers` initializes proj hit `ShaderStorageBuffer` with `[-1, 0]` per slot. GPU zeroes would falsely indicate "hit NPC 0".
