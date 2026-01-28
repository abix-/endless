# Combat System

## Overview

Five chained Bevy systems handle the complete combat loop: cooldown management, GPU-targeted attacks, damage application, death detection, and cleanup with slot recycling. All run sequentially in `Step::Combat`.

## Data Flow

```
GPU combat_target_buffer
        │
        ▼
  attack_system ──── DamageMsg ────▶ damage_system
  (range check,                      (apply to Health,
   cooldown check,                    sync GPU health)
   queue damage)                           │
        │                                  ▼
        │                           death_system
        │                           (health <= 0 → Dead)
        │                                  │
        │                                  ▼
        │                      death_cleanup_system
        │                      ├─ despawn entity
        │                      ├─ HideNpc → GPU (-9999)
        │                      └─ push to FREE_SLOTS
        │
        └── SetTarget (chase) ──▶ GPU_UPDATE_QUEUE
```

## Components

| Component | Type | Purpose |
|-----------|------|---------|
| Health | `f32` | Current HP (default 100.0) |
| Dead | marker | Inserted when health <= 0 |
| Faction | enum | Villager(0) or Raider(1) |
| AttackStats | struct | `range: f32, damage: f32, cooldown: f32` |
| AttackTimer | `f32` | Seconds until next attack allowed |
| InCombat | marker | Prevents behavior systems from overriding chase target |

## System Pipeline

Execution order is **chained** — each system completes before the next starts.

### 1. cooldown_system (combat.rs)
- Decrements `AttackTimer` by `FRAME_DELTA` each frame
- When timer reaches 0, attack is available

### 2. attack_system (combat.rs)
- Reads `GPU_READ_STATE.combat_targets` for each NPC with AttackStats
- If target is valid (not -1) and target is alive:
  - **In range**: queue `DamageMsg`, reset `AttackTimer`, mark `InCombat`
  - **Out of range**: push `SetTarget` to chase, mark `InCombat`
- If no target: remove `InCombat`

### 3. damage_system (health.rs)
- Drains `DamageMsg` events from Bevy MessageReader
- O(1) entity lookup via `NpcEntityMap[npc_index]`
- Subtracts damage: `health.0 = (health.0 - amount).max(0.0)`
- Pushes `GpuUpdate::SetHealth` to sync GPU health buffer

### 4. death_system (health.rs)
- Queries all NPCs with Health but `Without<Dead>`
- If `health.0 <= 0.0`: insert `Dead` marker component

### 5. death_cleanup_system (health.rs)
- Queries all entities `With<Dead>`
- For each dead entity:
  1. `commands.entity(entity).despawn()` — remove from Bevy ECS
  2. `npc_map.0.remove(&idx)` — remove from O(1) lookup
  3. `GpuUpdate::HideNpc { idx }` — position to (-9999, -9999) on GPU
  4. `FREE_SLOTS.lock().push(idx)` — recycle slot for future spawns

## Slot Recycling

```
Spawn: allocate_slot() ──▶ pop FREE_SLOTS (or npc_count++)
                                    ▲
Death: death_cleanup_system ────────┘
       push idx to FREE_SLOTS
```

Slots are raw `usize` indices without generational counters. This is safe because:
1. Combat systems are **chained** — damage is applied and death is processed in the same frame
2. Slot reuse only happens on the **next** spawn call, which writes fresh GPU data before the next dispatch
3. No cross-frame references exist to stale indices

## GPU Integration

| Direction | What | How |
|-----------|------|-----|
| GPU → CPU | Combat targets | `GPU_READ_STATE.combat_targets[]` read by attack_system |
| GPU → CPU | Positions | `GPU_READ_STATE.positions[]` read by attack_system for range check |
| CPU → GPU | Health sync | `GpuUpdate::SetHealth` after damage |
| CPU → GPU | Hide dead | `GpuUpdate::HideNpc` sets position to (-9999, -9999) |
| CPU → GPU | Chase target | `GpuUpdate::SetTarget` when out of attack range |

## Debug API

`get_combat_debug()` returns a Dictionary with:
- **Bevy combat stats**: attackers, targets_found, attacks, in_range, timer_ready, chases
- **CPU cache**: positions, factions, healths for NPC 0 and 1
- **Grid cells**: cell coordinates and counts for each NPC's position
- **GPU buffer direct reads**: faction and health read back from GPU buffers (not CPU cache)

## Known Issues / Limitations

- **No generational indices**: Stale references to recycled slots would silently alias. Currently safe due to chained execution, but would break if damage messages span frames.
- **Equal stats**: All NPCs have identical AttackStats (100 HP, 15 damage, 150 range, 1s cooldown). Per-NPC stats would need component variation.
- **No friendly fire**: Faction check prevents same-faction damage. No way to enable it selectively.
- **InCombat blocks all behavior**: Once in combat, NPCs can't rest, patrol, or work until the target dies or leaves range.
- **Clone per frame**: attack_system clones positions and combat_targets vecs from GPU_READ_STATE (~80KB at 10K NPCs). Negligible but not zero-copy.
- **Debug mutex overhead**: COMBAT_DEBUG and HEALTH_DEBUG lock every frame even in release builds.

## Rating: 8/10

Solid pipeline with correct execution ordering. The chained guarantee is the key safety property. O(1) entity lookup via NpcEntityMap is good. Main gap is no generational indices — fine today, risk grows with complexity.
