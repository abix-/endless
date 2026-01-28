# Projectile System

## Overview

GPU-accelerated projectiles with compute shader movement and spatial grid collision detection. Supports up to 50,000 simultaneous projectiles with slot reuse via a free list. MultiMesh rendering is dynamically sized to active count — zero cost when idle.

## Data Flow

```
attack_system → PROJECTILE_FIRE_QUEUE → process() drains queue:
├─ Allocate slot (FREE_PROJ_SLOTS or proj_count++)
├─ Calculate velocity from speed + direction
├─ upload_projectile(idx, pos, vel, damage, faction, shooter, lifetime)
└─ Supports melee (speed=500, lifetime=0.5s) and ranged (speed=200, lifetime=3.0s)

process() each frame (if proj_count > 0):
├─ Dispatch projectile_compute.glsl
│   ├─ Decrement lifetime → deactivate expired
│   ├─ Move by velocity
│   └─ Collision via spatial grid → write hit buffer
├─ Read hit buffer
│   ├─ Push DamageMsg to DAMAGE_QUEUE (Bevy processes next frame)
│   └─ Push proj_idx to FREE_PROJ_SLOTS
├─ Read positions + active flags
├─ Resize MultiMesh to proj_count (if changed)
└─ Build + upload MultiMesh buffer
```

## Fire Path

All projectiles originate from Bevy's `attack_system` via `PROJECTILE_FIRE_QUEUE`. No GDScript fire API.

`process()` drains `PROJECTILE_FIRE_QUEUE` each frame:

1. Try `FREE_PROJ_SLOTS.pop()` for a recycled slot
2. If none, use `gpu.proj_count` and increment
3. If at `MAX_PROJECTILES` (50,000), skip
4. Calculate velocity: `normalize(to - from) * msg.speed`
5. `upload_projectile()` writes GPU buffers with per-projectile `lifetime`
6. Melee: speed=500, lifetime=0.5s. Ranged: speed=200, lifetime=3.0s

## GPU Dispatch

`projectile_compute.glsl` — 64 threads per workgroup, `ceil(proj_count / 64)` dispatches.

For each active projectile:
1. **Lifetime**: `lifetime -= delta`. If <= 0, set active = 0, position = (-9999, -9999), return.
2. **Movement**: `pos += velocity * delta`
3. **Collision**: Compute grid cell, scan 3x3 neighborhood:
   - Skip if already hit (`hit.x >= 0`)
   - Skip same faction (no friendly fire)
   - Skip dead NPCs (`health <= 0`)
   - If distance < 10px: write `hit = ivec2(npc_idx, 0)`, deactivate, hide at (-9999, -9999)

## Hit Processing

After dispatch, CPU reads `proj_hit_buffer` back:

```rust
for each projectile with hit.x >= 0 and hit.y == 0 (unprocessed):
    push DamageMsg { npc_index: hit.x, amount: damage }  → DAMAGE_QUEUE
    push proj_idx → FREE_PROJ_SLOTS
    mark hit.y = 1 (processed)
```

Damage is processed by Bevy's `damage_system` in the **next frame's** Combat phase.

## Rendering

- MultiMesh starts at **0 instances** (allocated empty at init)
- Each frame, if `proj_count != current_instance_count`, reallocate via `multimesh_allocate_data_ex()`
- `build_proj_multimesh(proj_count)` builds a `PackedFloat32Array`:
  - 12 floats per instance (PROJ_FLOATS_PER_INSTANCE)
  - Transform2D with velocity-based rotation: `angle = atan2(vy, vx)`
  - Inactive projectiles get position (-9999, -9999)
  - Active projectiles get rotated transform at current position
  - Color channel encodes faction (blue=guard, red=raider) via `proj_factions` cache
- Uploaded via `multimesh_set_buffer()`

## Slot Lifecycle

```
PROJECTILE_FIRE_QUEUE ── allocate slot ──▶ ACTIVE on GPU
                         ▲                   │
                         │            hit or expire
                         │                   │
                         │                   ▼
              FREE_PROJ_SLOTS ◀──── return slot
```

Slots are `usize` indices. `proj_count` only grows (represents high-water mark). Free slots are reused first. No generational indices — the GPU hit buffer uses `(npc_idx, processed_flag)` and hits are processed same-frame.

## Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| MAX_PROJECTILES | 50,000 | Pool capacity (~3.2 MB VRAM) |
| PROJECTILE_HIT_RADIUS | 10.0 | Collision detection radius (px) |
| Melee speed | 500.0 | AttackStats::melee() projectile speed |
| Ranged speed | 200.0 | AttackStats::ranged() projectile speed |
| Melee lifetime | 0.5s | AttackStats::melee() projectile lifetime |
| Ranged lifetime | 3.0s | AttackStats::ranged() projectile lifetime |
| PROJ_FLOATS_PER_INSTANCE | 12 | Transform2D (8) + color (4) |

## Known Issues / Limitations

- **proj_count never shrinks**: High-water mark means the MultiMesh stays at peak size even after all projectiles expire. Would need a compaction pass to reclaim.
- **Hit damage is one frame delayed**: Hits are read back and pushed to DAMAGE_QUEUE, which Bevy processes next frame. Not noticeable at 140fps but technically imprecise.
- **No projectile-projectile collision**: Projectiles pass through each other.
- **Fixed hit radius**: 10px hardcoded in shader, not configurable per projectile type.
- **Faction color is cached CPU-side**: `proj_factions` vec mirrors GPU faction buffer. Could be eliminated if color were computed in shader.

## Rating: 8/10

Clean GPU-accelerated system with proper slot reuse. Dynamic MultiMesh sizing (the fix from this session) eliminated the 50K-instance performance regression. Zero cost when idle. Main improvements: shrinking proj_count on compaction, and configurable hit radius.
