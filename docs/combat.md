# Combat System

## Overview

Ten Bevy systems handle the complete combat loop: cooldown management, GPU-targeted attacks (with building fallback), damage application, death detection, XP-on-kill grant, cleanup with slot recycling, waypoint slot sync, building tower auto-attack (fountains), and building damage processing. Nine run chained in `Step::Combat`; `building_damage_system` runs in `Step::Behavior`.

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
  sync_waypoint_slots
  (alloc/free NPC slots,
   dirty-flag gated)
        │
        ▼
  building_tower_system
  (fountains via fire_towers,
   read combat_targets[slot])
```

attack_system fires projectiles via `PROJ_GPU_UPDATE_QUEUE` when in range, or applies point-blank damage for melee. The projectile system ([projectiles.md](projectiles.md)) handles movement, collision detection, hit readback, and slot recycling.

## Components

| Component | Type | Purpose |
|-----------|------|---------|
| Health | `f32` | Current HP (default 100.0) |
| Dead | marker | Inserted when health <= 0 |
| LastHitBy | `i32` | NPC slot index of last attacker (-1 = no attacker). Inserted by damage_system, read by xp_grant_system. |
| Faction | `struct(i32)` | Faction ID (-1=Neutral, 0=Player, 1+=AI settlements). NPCs attack different factions. Neutral (-1) is treated as same-faction by GPU combat targeting and projectile collision. |
| BaseAttackType | enum | `Melee` or `Ranged` — keys into `CombatConfig.attacks` HashMap. Crossbow units use `Ranged` but stats resolve from `CombatConfig.crossbow_attack` (overridden by Job in `resolve_combat_stats`). |
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
- **Skips** NPCs with `Activity::Returning`, `Activity::GoingToRest`, or `Activity::Resting` (prevents combat while heading home, going to bed, or sleeping)
- **Validates GPU target** before engaging — rejects self-targets (`ti == i`), non-NPC slots (`NpcEntityMap` lookup), same-faction or neutral targets (`GpuReadState.factions`), and dead targets (`GpuReadState.health <= 0`). Invalid targets clear `CombatState` and skip.
- If target is valid (not -1), passes validation, and in bounds:
  - Sets `CombatState::Fighting { origin }` (stores current position)
  - **In range**: sets `SetTarget` to own position (stand ground — stops GPU movement, NPC holds position while shooting). Projectile dodge from GPU shader provides evasion.
  - **In range + cooldown ready**: resets `AttackTimer`, fires projectile or applies point-blank damage
  - **Out of range**: pushes `GpuUpdate::SetTarget` to chase
- If no NPC target: sets `CombatState::None`, then checks for opportunistic building attack:
  - Only **archers**, **crossbows**, and **raiders** attempt building attacks (farmers/miners/fighters skip)
  - Queries `BuildingSpatialGrid` via `find_nearest_enemy_building()` for enemy buildings within `CachedStats.range`
  - Non-targetable buildings skipped: Fountain, Camp, GoldMine, Bed
  - **Raiders**: only target ArcherHome, CrossbowHome, Waypoint (leave FarmerHome/MinerHome alone for farm raiding)
  - **Archers/Crossbows**: target any enemy building type (except non-targetable)
  - "Enemy" = building faction != NPC faction (uses `BuildingRef.faction` field)
  - If found and cooldown ready: stand ground (SetTarget to own pos), fire projectile toward building position, reset cooldown
  - Building damage is projectile-based: `process_proj_hits` checks active projectiles against `BuildingSpatialGrid` and sends `BuildingDamageMsg` on collision (see [projectiles.md](projectiles.md))
  - NPCs don't chase buildings — pure attack of opportunity when nearby with nothing better to do

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
- Increments `FactionStats.inc_kills()` for the killer's faction
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
  4. Release `AssignedFarm` via `BuildingOccupancy.release()` if farmer had one
  5. Release `WorkPosition` via `BuildingOccupancy.release()` if miner was at a mine
  6. Remove from `RaidQueue` if Raider
  7. Update stats: `PopulationStats` (dec_alive, inc_dead, dec_working if `Working` or `MiningAtMine`), `FactionStats` (dec_alive, inc_dead), `KillStats`
  8. Remove from `NpcsByTownCache`
  9. Deselect if `SelectedNpc` matches dying NPC (clears inspector panel)
  10. `SlotAllocator.free(idx)` — recycle slot for future spawns

### 7. sync_waypoint_slots (combat.rs)

- Gated by `DirtyFlags.waypoint_slots` — skips entirely when no waypoints built/destroyed/loaded
- Scans `WorldData.waypoints` for slot mismatches:
  - **Alive post, no slot** (`position.x > -9000 && npc_slot == None`): allocates `SlotAllocator` index, emits GPU updates (SetPosition, SetTarget, SetSpeed=0, SetHealth=999, SetSpriteFrame col=-1). Sprite col=-1 makes the slot invisible to NPC renderer. SetTarget=position causes GPU to immediately mark settled.
  - **Tombstoned post, has slot** (`position.x < -9000 && npc_slot == Some`): emits `HideNpc`, frees slot
- Faction set in a second pass (borrow split: `iter_mut` on waypoints prevents reading towns simultaneously)
- Clears `dirty.waypoint_slots` after sync

### 8. building_tower_system (combat.rs)

Generalized tower system for any building kind that auto-shoots. Uses a shared `fire_towers()` helper parameterized by `BuildingKind`, `TowerStats`, and a `(Vec2, i32)` slice of (position, faction).

- **TowerState** resource: holds per-kind `TowerKindState` with `timers: Vec<f32>` and `attack_enabled: Vec<bool>`
- **TowerStats** struct in `constants.rs`: `range`, `damage`, `cooldown`, `proj_speed`, `proj_lifetime`
- State length auto-syncs with building count each tick
- **Fountains**: `FOUNTAIN_TOWER` (range=400, damage=15, cooldown=1.5s, proj_speed=350, proj_lifetime=1.5s). Always-on — `attack_enabled` refreshed from `Town.sprite_type == 0` every tick (camps excluded). Strong enough to defend spawn area.
- GPU integration: `allocate_building_slot` sets `npc_flags = 3` (bit 0 = combat scan, bit 1 = tower) for fountain buildings. Shader skips movement for towers but runs combat targeting → `combat_targets[slot]` populated by GPU.
- `fire_towers()` loop: for each enabled building, reads `GpuReadState.combat_targets[slot]` (O(1) GPU targeting), validates range, fires `ProjGpuUpdate::Spawn` with `shooter: -1`
- DRY: adding a new tower building kind requires a `TowerStats` const, a `TowerKindState` field in `TowerState`, and a block in `building_tower_system`. `Building::is_tower()` and the shader handle the rest.

### 9. building_damage_system (combat.rs, Step::Behavior)
- Reads `BuildingDamageMsg` events via `MessageReader`
- Decrements `BuildingHpState` by `msg.amount` for the target building kind + index
- Looks up position/town via `WorldData::building_pos_town()` (single dispatch method for all building kinds)
- Sets `DirtyFlags.buildings_need_healing` when a building survives damage (hp > 0)
- Syncs HP to GPU: looks up NPC slot via `BuildingSlotMap.get_slot()`, writes `GpuUpdate::SetHealth` with new HP
- Skips already-dead buildings (HP <= 0)
- When HP reaches 0:
  1. Captures linked NPC slot from `SpawnerState` by position match **before** destroy (tombstoning changes position)
  2. Calls `destroy_building()` shared helper (grid clear + WorldData tombstone + spawner tombstone + HP zero + combat log + free building NPC slot)
  3. Kills linked NPC via `GpuUpdate::HideNpc` + `SetHealth(0.0)`
- Profiled under `"building_damage"` scope

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
| CPU → GPU | Stand ground | `GpuUpdate::SetTarget` to own position when in attack range (stops movement, allows proj dodge) |
| CPU → GPU | Chase target | `GpuUpdate::SetTarget` when out of attack range |
| CPU → GPU | Fire projectile | `ProjGpuUpdate::Spawn` via `PROJ_GPU_UPDATE_QUEUE` (attack_system + building_tower_system + building attack fallback) |
| CPU → GPU | Guard post slots | `sync_waypoint_slots` allocates NPC slots for waypoints, sets position/faction/speed=0/health=999/sprite=-1 |
| CPU → GPU | Tower flags | `allocate_building_slot` sets `npc_flags = 3` (bits 0+1) for tower buildings (fountains). Shader runs combat targeting for these slots. |
| CPU → GPU | Building HP sync | `building_damage_system` writes `GpuUpdate::SetHealth` to sync building GPU slot HP after damage |
| GPU | Building collision | Buildings occupy NPC GPU slots (speed=0, hidden sprite). Projectile compute shader detects hits via NPC spatial grid. `process_proj_hits` routes building slot hits to `BuildingDamageMsg` via `BuildingSlotMap` lookup. |

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
