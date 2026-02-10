# Combat System

## Overview

Eight chained Bevy systems handle the complete combat loop: cooldown management, GPU-targeted attacks, damage application, death detection, XP-on-kill grant, cleanup with slot recycling, and guard post turret auto-attack. All run sequentially in `Step::Combat`.

## Data Flow

```
GPU combat_target_buffer (from compute shader)
        │
        ▼
  attack_system
  (range check,
   cooldown check,
   fire projectile or
   point-blank damage)
        │
        ├── In range + cooldown ready → fire projectile → GPU projectile system
        │
        └── Out of range → SetTarget (chase) → GPU_UPDATE_QUEUE
                                                      │
                                                      ▼
DamageMsg (from process_proj_hits)             GPU movement
        │
        ▼
  damage_system
  (apply to Health,
   sync GPU health,
   insert LastHitBy)
        │
        ▼
  death_system
  (health <= 0 → Dead)
        │
        ▼
  xp_grant_system
  (Dead + LastHitBy →
   grant 100 XP to killer,
   level-up → re-resolve stats)
        │
        ▼
  death_cleanup_system
  ├─ despawn entity
  ├─ HideNpc → GPU (-9999)
  ├─ Release AssignedFarm, clear RaidQueue
  ├─ Update FactionStats, KillStats, PopulationStats
  └─ SlotAllocator.free(idx)
        │
        ▼
  guard_post_attack_system
  (scan enemies near posts,
   fire projectiles)
```

attack_system fires projectiles via `PROJ_GPU_UPDATE_QUEUE` when in range, or applies point-blank damage for melee. The projectile system ([projectiles.md](projectiles.md)) handles movement, collision detection, hit readback, and slot recycling.

## Components

| Component | Type | Purpose |
|-----------|------|---------|
| Health | `f32` | Current HP (default 100.0) |
| Dead | marker | Inserted when health <= 0 |
| LastHitBy | `i32` | NPC slot index of last attacker (-1 = no attacker). Inserted by damage_system, read by xp_grant_system. |
| Faction | `struct(i32)` | Faction ID (0=Villager, 1+=Raider camps). NPCs attack different factions. |
| BaseAttackType | enum | `Melee` or `Ranged` — keys into `CombatConfig.attacks` HashMap |
| CachedStats | struct | `damage, range, cooldown, projectile_speed, projectile_lifetime, max_health, speed` — resolved from `CombatConfig` via `resolve_combat_stats()` |
| AttackTimer | `f32` | Seconds until next attack allowed |
| CombatState | enum | `None`, `Fighting { origin: Vec2 }`, `Fleeing` — orthogonal to Activity enum (see [behavior.md](behavior.md)) |

## System Pipeline

Execution order is **chained** — each system completes before the next starts.

### 1. cooldown_system (combat.rs)
- Decrements `AttackTimer` by `time.delta_secs()` each frame
- When timer reaches 0, attack is available
- Updates `CombatDebug` with sample timer and entity count

### 2. attack_system (combat.rs)
- Reads `GpuReadState.combat_targets` for each NPC with CachedStats + BaseAttackType
- If target is valid (not -1) and in bounds:
  - Sets `CombatState::Fighting { origin }` (stores current position)
  - **In range + cooldown ready**: resets `AttackTimer`, fires projectile or applies point-blank damage
  - **Out of range**: pushes `GpuUpdate::SetTarget` to chase
- If no target: sets `CombatState::None` (Activity is preserved — e.g. Raiding NPC stays Raiding so decision_system can re-target farm)

### 3. damage_system (health.rs)
- Drains `DamageMsg` events from Bevy MessageReader
- O(1) entity lookup via `NpcEntityMap[npc_index]`
- Subtracts damage: `health.0 = (health.0 - amount).max(0.0)`
- Pushes `GpuUpdate::SetHealth` to sync GPU health buffer
- Pushes `GpuUpdate::SetDamageFlash` (intensity 1.0) for visual hit feedback
- If `DamageMsg.attacker >= 0`: inserts `LastHitBy(attacker)` on target entity (overwrites previous)

### 4. death_system (health.rs)
- Queries all NPCs with Health but `Without<Dead>`
- If `health.0 <= 0.0`: insert `Dead` marker component

### 5. xp_grant_system (stats.rs)
- Queries entities `With<Dead>` that have `Option<&LastHitBy>`
- If `LastHitBy` present, looks up killer entity via `NpcEntityMap`
- Grants 100 XP to killer's `NpcMetaCache` entry
- Checks for level-up: `level_from_xp(new_xp) > level_from_xp(old_xp)`
- On level-up: re-resolves `CachedStats`, updates `Speed` component, rescales HP proportionally (`hp * new_max / old_max`), sends `GpuUpdate::SetSpeed` and `GpuUpdate::SetHealth`, emits `CombatEventKind::LevelUp` to `CombatLog`
- XP formula: `level = floor(sqrt(xp / 100))`, level multiplier = `1.0 + level * 0.01`

### 6. death_cleanup_system (health.rs)
- Queries all entities `With<Dead>`
- For each dead entity:
  1. `commands.entity(entity).despawn()` — remove from Bevy ECS
  2. `npc_map.0.remove(&idx)` — remove from O(1) lookup
  3. `GpuUpdate::HideNpc { idx }` — full slot cleanup on GPU:
     - Position → (-9999, -9999)
     - Target → (-9999, -9999) — prevents zombie movement
     - Arrival → 1 — stops GPU from computing movement
     - Health → 0 — ensures click detection skips slot
  4. Release `AssignedFarm` occupancy if `Activity::Working`
  5. Remove from `RaidQueue` if Raider
  6. Update stats: `PopulationStats` (dec_alive, inc_dead, dec_working), `FactionStats` (dec_alive, inc_dead), `KillStats`
  7. Remove from `NpcsByTownCache`
  8. `SlotAllocator.free(idx)` — recycle slot for future spawns

### 7. guard_post_attack_system (combat.rs)
- Iterates `WorldData.guard_posts` with `GuardPostState` per-post timers and enabled flags
- State length auto-syncs with guard post count (handles runtime building)
- For each enabled post with cooldown ready: scans `GpuReadState.positions`+`factions` for nearest enemy (faction != 0) within `GUARD_POST_RANGE` (250px)
- Fires projectile via `PROJ_GPU_UPDATE_QUEUE` with `shooter: -1` (building, not NPC) and `faction: 0`
- Constants: range=250, damage=8, cooldown=3s, proj_speed=300, proj_lifetime=1.5s
- Turret toggle: `GuardPostState.attack_enabled[i]` toggled via build menu UI

## Slot Recycling

```
Spawn: SlotAllocator.alloc() ──▶ pop free list (or next++)
                                        ▲
Death: death_cleanup_system ────────────┘
       SlotAllocator.free(idx)
```

Slots are raw `usize` indices without generational counters. This is safe because:
1. Combat systems are **chained** — damage is applied and death is processed in the same frame
2. Slot reuse only happens on the **next** spawn call, which writes fresh GPU data before the next dispatch
3. No cross-frame references exist to stale indices

## GPU Integration

| Direction | What | How |
|-----------|------|-----|
| GPU → CPU | Combat targets | `GpuReadState.combat_targets[]` — populated via Bevy `ReadbackComplete` observer |
| GPU → CPU | Positions | `GpuReadState.positions[]` — populated via Bevy `ReadbackComplete` observer |
| GPU → CPU | Projectile hits | `ProjHitState` — populated via Bevy `ReadbackComplete` observer, includes expired sentinel (-2) |
| CPU → GPU | Health sync | `GpuUpdate::SetHealth` after damage |
| CPU → GPU | Hide dead | `GpuUpdate::HideNpc` resets position, target, arrival, health |
| CPU → GPU | Chase target | `GpuUpdate::SetTarget` when out of attack range |
| CPU → GPU | Fire projectile | `ProjGpuUpdate::Spawn` via `PROJ_GPU_UPDATE_QUEUE` (attack_system + guard_post_attack_system) |

## Debug

`CombatDebug` (Bevy Resource) updated each frame by cooldown_system and attack_system:
- attackers_queried, targets_found, attacks_made, chases_started, in_combat_added
- positions_len, combat_targets_len, bounds_failures
- sample_target_idx, sample_dist, sample_timer
- in_range_count, timer_ready_count

`HealthDebug` (Bevy Resource) updated by damage/death/healing systems:
- damage_processed, deaths_this_frame, despawned_this_frame, bevy_entity_count
- healing_npcs_checked, healing_in_zone_count, healing_healed_count

## Known Issues / Limitations

- **No generational indices**: Stale references to recycled slots would silently alias. Currently safe due to chained execution, but would break if damage messages span frames.
- **No friendly fire**: Faction check prevents same-faction damage. No way to enable it selectively.
- **CombatState::Fighting blocks behavior decisions**: While fighting, decision_system skips the NPC. However, Activity is preserved through combat — when combat ends (`CombatState::None`), the NPC resumes its previous activity.
- **KillStats naming inverted**: `guard_kills` tracks raiders killed (by guards), `villager_kills` tracks villagers killed (by raiders). The names describe the victim, not the killer.

## Rating: 8/10

Full combat loop: GPU targeting → attack → damage (with last-hit tracking) → death → XP grant → cleanup. Chained execution guarantees safety. O(1) entity lookup via NpcEntityMap. XP-on-kill grants 100 XP to last attacker with level-up stat re-resolution and proportional HP rescale. death_cleanup_system is thorough (releases farm occupancy, clears raid queue, updates all stat resources). Projectile slot recycling handles both collisions and expired projectiles via sentinel.
