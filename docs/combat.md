# Combat System

## Overview

Six Bevy systems handle the complete combat loop: cooldown management, GPU-targeted attacks (NPC and building targets), unified damage application (NPCs + buildings), unified death processing (mark dead + XP grant + building destruction + NPC cleanup + despawn), and building tower auto-attack (fountains). All six run chained in `Step::Combat`.

## Data Flow

```
GPU combat_target_buffer (from compute shader)
        │  (returns NPC index OR building index)
        ▼
  attack_system
  (range check,
   cooldown check,
   fire projectile or
   point-blank DamageMsg)
        │
        ├── target < npc_count → NPC target (existing flow)
        ├── target >= npc_count → building target (GPU-targeted, real damage projectile)
        │
        ├── In range + cooldown ready → fire projectile → GPU projectile system
        └── Out of range → MovementIntents.submit(Combat) → resolve_movement_system
                                                      │
                                                      ▼
DamageMsg (from process_proj_hits)             GPU movement
        │  (unified: entity_idx routes NPC vs building)
        ▼
  damage_system
  ├─ NPC (entity_idx < npc_count):
  │   apply to Health, sync GPU, insert LastHitBy, SetDamageFlash
  └─ Building (entity_idx >= npc_count):
      apply to Health, BldSetHealth, BldSetDamageFlash
      insert LastHitBy (for loot attribution)
        │
        ▼
  death_system (health.rs) — unified, two phases per frame
  ├─ Phase 1: mark dead — SINGLE writer of Dead component
  │   Query Without<Dead> where health <= 0 → insert Dead
  └─ Phase 2: process dead (With<Dead> from previous frame)
      ├─ Building branch:
      │   ├─ destroy_building (grid clear, wall auto-tile)
      │   ├─ Fountain → deactivate AI, endless respawn queue
      │   ├─ Loot to attacker (LastHitBy → Activity::Returning)
      │   ├─ BldHide + BldSetHealth(0), BuildingSlots.free(idx)
      │   └─ remove_by_slot (slot_to_entity + instances + by_kind)
      └─ NPC branch:
          ├─ XP grant (LastHitBy → 100 XP, level-up, stat re-resolve)
          ├─ NPC kill loot (npc_def loot_drop → Activity::Returning)
          ├─ despawn entity, HideNpc → GPU (-9999)
          ├─ Release AssignedFarm/WorkPosition
          ├─ Update FactionStats, KillStats, PopulationStats
          └─ SlotAllocator.free(idx)
        │
        ▼
  building_tower_system
  (fountains via GPU combat_targets,
   reads readback at npc_count + bld_slot)
```

attack_system emits `ProjGpuUpdateMsg` when in range, or applies point-blank damage for melee. The projectile system ([projectiles.md](projectiles.md)) handles movement, collision detection, hit readback, and slot recycling.

## Components

| Component | Type | Purpose |
|-----------|------|---------|
| Health | `f32` | Current HP (default 100.0) |
| Dead | marker | Inserted when health <= 0 |
| LastHitBy | `i32` | NPC slot index of last attacker (-1 = no attacker). Inserted by damage_system, read by death_system for XP grant + loot attribution. |
| Faction | `struct(i32)` | Faction ID (-1=Neutral, 0=Player, 1+=AI settlements). NPCs attack different factions. Neutral (-1) is treated as same-faction by GPU combat targeting and projectile collision. |
| BaseAttackType | enum | `Melee` or `Ranged` — keys into `CombatConfig.attacks` HashMap. Crossbow units use `Ranged` but stats resolve from `CombatConfig.crossbow_attack` (overridden by Job in `resolve_combat_stats`). |
| CachedStats | struct | `damage, range, cooldown, projectile_speed, projectile_lifetime, max_health, speed` — resolved from `CombatConfig` via `resolve_combat_stats()` |
| AttackTimer | `f32` | Seconds until next attack allowed |
| CombatState | enum | `None`, `Fighting { origin: Vec2 }`, `Fleeing` — orthogonal to Activity enum (see [behavior.md](behavior.md)) |
| ManualTarget | enum | Player target: `Npc(usize)` (attack NPC slot), `Building(Vec2)` (attack building), `Position(Vec2)` (ground move). Inserted by right-click on DirectControl NPCs. `Npc` variant overrides GPU auto-targeting; others fall through. |

## System Pipeline

Execution order is **chained** — each system completes before the next starts.

### 1. cooldown_system (combat.rs)
- Decrements `AttackTimer` by `time.delta_secs()` each frame
- When timer reaches 0, attack is available
- Updates `CombatDebug` with sample timer and entity count

### 2. attack_system (combat.rs)
- **Manual target override**: if NPC has `ManualTarget::Npc(slot)`, uses that slot as target instead of GPU `combat_targets[i]`. Auto-clears `ManualTarget` when target's GPU health <= 0 (dead). `ManualTarget::Building` and `ManualTarget::Position` variants fall through to GPU auto-targeting. See [behavior.md](behavior.md#squads) for how `ManualTarget` is set.
- **Hold fire**: if NPC's squad has `hold_fire == true` and no `ManualTarget`, target is set to -1 (skip auto-engage). Reads `SquadState` via `SquadId`.
- Falls back to `GpuReadState.combat_targets` for NPCs without manual target or hold-fire.
- **Skips** NPCs with `Activity::Returning`, `Activity::GoingToRest`, or `Activity::Resting` (prevents combat while heading home, going to bed, or sleeping)
- **Unified GPU targeting**: `combat_targets[i]` can return NPC indices (`< npc_count`) or building indices (`>= npc_count`). One code path for all target types.
- **Building targets** (`target >= npc_count`):
  - Only **archers**, **crossbows**, and **raiders** attack buildings (farmers/miners/fighters skip)
  - Validates via `BuildingEntityMap.get_instance(bld_slot)` — checks faction (skip same-faction)
  - Gets building position from `BuildingInstance.position`
  - In range + cooldown ready: fires projectile with **real damage** (GPU projectile collision handles hit detection against buildings in the unified entity grid)
  - Point-blank: emits `DamageMsg` directly
  - Out of range but within close chase radius (range + 120px): chases building (`SetTarget` to building position)
  - Beyond close chase radius: ignores building (prevents cross-map pursuit of distant enemy buildings)
- **NPC targets** (`target < npc_count`):
  - Validates via `NpcEntityMap` lookup, faction check, health check (same as before)
  - Sets `CombatState::Fighting { origin }` (stores current position)
  - **In range**: submits `MovementIntents` at `Combat` priority to own position (stand ground — stops GPU movement, NPC holds position while shooting). Projectile dodge from GPU shader provides evasion.
  - **In range + cooldown ready**: resets `AttackTimer`, fires projectile or applies point-blank damage
  - **Out of range**: submits `MovementIntents` at `Combat` priority to chase target

### 3. damage_system (health.rs)
- Drains unified `DamageMsg` events from Bevy MessageReader
- Routes by `entity_idx`: `< npc_count` → NPC, `>= npc_count` → building
- **NPC damage** (`entity_idx < npc_count`):
  - O(1) entity lookup via `NpcEntityMap[entity_idx]`
  - Subtracts damage: `health.0 = (health.0 - amount).max(0.0)`
  - Pushes `GpuUpdate::SetHealth` + `GpuUpdate::SetDamageFlash` (intensity 1.0)
  - If `attacker >= 0`: inserts `LastHitBy(attacker)` via `get_entity()` guard
- **Building damage** (`entity_idx >= npc_count`):
  - `bld_slot = entity_idx - npc_count`
  - O(1) lookup via `BuildingEntityMap.get_instance(bld_slot)` + `get_entity(bld_slot)`
  - Skips indestructible buildings (GoldMine, Road) and already-dead buildings
  - Subtracts damage, pushes `GpuUpdate::BldSetHealth` + `GpuUpdate::BldSetDamageFlash`
  - If HP > 0: sets `BuildingHealState.needs_healing` for healing_system
  - Inserts `LastHitBy(attacker)` on buildings (same as NPCs) for death_system loot attribution

### 4. death_system (health.rs)

Unified death handler — replaces the old `death_system` + `xp_grant_system` + `building_death_system` + `death_cleanup_system` pipeline. Uses `ParamSet` to resolve query conflict between mark-dead (reads `&Health` on `Without<Dead>`) and killer/loot access (writes `&mut Health` on `Without<Dead>`). Uses `DeathResources` SystemParam (16 fields) merging CleanupResources + WorldState unique fields + BuildingDeathExtra fields.

**Phase 1: Mark dead** (deferred — takes effect next frame)
- Queries all entities `Without<Dead>` where `health.0 <= 0.0`
- Inserts `Dead` marker via deferred commands (same 1-frame delay as before)

**Phase 2: Process dead** (entities `With<Dead>` from previous frame)

For each dead entity:

**Building branch** (detected via `Building` component):
- Looks up instance data (kind, position, town_idx) from `BuildingEntityMap.get_instance(idx)`, copies fields before mutation
- Calls `destroy_building()` for grid cleanup (grid cell clear + wall auto-tile + combat log — no entity lifecycle)
- Emits `mark_building_changed(kind)` dirty signals
- **Fountain death**: deactivates AI player for that town. In endless mode, queues replacement AI (`PendingAiSpawn`) scaled to player strength.
- **Building loot**: `BuildingDef::loot_drop()` returns `cost / 2` as food. Uses `LastHitBy` to find attacker, looks up attacker entity via `params.p1()`. Attacker set to `Activity::Returning { loot }`, targets home. DC keep-fighting override skips disengage + home target when `dc_no_return`.
- `remove_by_slot(idx)` (clears `slot_to_entity` + `instances` + `by_kind`), `BldHide + BldSetHealth(0)`, `BuildingSlots.free(idx)`

**NPC branch:**
- **XP grant**: if `LastHitBy` present, looks up killer entity via `params.p1()`. Grants 100 XP, increments `FactionStats.inc_kills()`. Checks for level-up: `level_from_xp(new_xp) > level_from_xp(old_xp)`. On level-up: re-resolves `CachedStats`, updates `Speed`, rescales HP proportionally, sends GPU updates, emits `CombatEventKind::LevelUp`.
- **Loot on kill**: reads `npc_def(dead_job).loot_drop`, picks one deterministically via `xp % len`. Sets killer to `Activity::Returning { loot }`, clears `CombatState::None`. DC keep-fighting override applies.
- Despawn entity, `HideNpc` → GPU (-9999), release AssignedFarm/WorkPosition
- Update stats: `PopulationStats`, `FactionStats`, `KillStats`
- Remove from `NpcsByTownCache`, deselect if SelectedNpc matches
- `SlotAllocator.free(idx)` — recycle slot

XP formula: `level = floor(sqrt(xp / 100))`, level multiplier = `1.0 + level * 0.01`

### 5. building_tower_system (combat.rs)

Tower auto-attack using GPU spatial grid targeting. Towers are in the unified entity buffer (at index `npc_count + bld_slot`) with `ENTITY_FLAG_BUILDING | ENTITY_FLAG_COMBAT`. The GPU compute shader MODE 2 runs the same combat targeting scan for towers as for NPC combatants — finding the nearest enemy NPC via the spatial grid.

- **TowerState** resource: holds per-kind `TowerKindState` with `timers: Vec<f32>` and `attack_enabled: Vec<bool>`
- **TowerStats** struct in `constants.rs`: `range`, `damage`, `cooldown`, `proj_speed`, `proj_lifetime`
- State length auto-syncs with building count each tick
- **Fountains**: `FOUNTAIN_TOWER` (range=400, damage=15, cooldown=1.5s, proj_speed=350, proj_lifetime=1.5s). Always-on — `attack_enabled` refreshed from `is_alive(town.center)` every tick (all alive town centers shoot). Strong enough to defend spawn area.
- **GPU-side targeting**: Reads `GpuReadState.combat_targets[npc_count + bld_slot]` from readback buffer. The GPU found the nearest enemy via the spatial grid (same O(1) grid lookup as NPC targeting). `combat_range` = 400.0 to cover `FOUNTAIN_TOWER.range`. Only targets with `target < npc_count` are valid (towers only shoot NPCs, not other buildings).
- Tower loop: for each enabled building, look up building slot via `BuildingEntityMap.iter_kind_for_town(Fountain, town_idx).next()` (debug_assert one per town), read GPU target, emit `ProjGpuUpdateMsg(ProjGpuUpdate::Spawn)` with `shooter: -1`
- DRY: adding a new tower building kind requires a `TowerStats` const, a `TowerKindState` field in `TowerState`, and a block in `building_tower_system`. Building flags in `world.rs` + extract mapping in `npc_render.rs` handle the GPU side.


## Slot Recycling

```
NPC Spawn:  SlotAllocator.alloc()  ──▶ pop free list (or next++)
                                              ▲
NPC Death:  death_system  ───────────────────────┘
            SlotAllocator.free(idx)

Building:   BuildingSlots.alloc()  ──▶ pop free list (or next++)
                                              ▲
Bld Death:  death_system  ───────────────────────┘
            BuildingSlots.free(idx)
```

NPCs and buildings use separate slot allocators (`SlotAllocator` max=100K, `BuildingSlots` max=5K) backed by a shared `SlotPool` inner type. This keeps NPC and building slot domains isolated.

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
| CPU → GPU | Stand ground | `MovementIntents.submit(Combat)` to own position when in attack range → `resolve_movement_system` emits `SetTarget` (stops movement, allows proj dodge) |
| CPU → GPU | Chase target | `MovementIntents.submit(Combat)` when out of attack range → `resolve_movement_system` emits `SetTarget` |
| CPU → GPU | Fire projectile | `ProjGpuUpdateMsg(ProjGpuUpdate::Spawn)` (attack_system + building_tower_system) |
| CPU → GPU | Guard post slots | `sync_waypoint_slots` allocates NPC slots for waypoints, sets position/faction/speed=0/health=999/sprite=-1 |
| CPU → GPU | Building HP sync | `damage_system` writes entity `Health` + `GpuUpdate::BldSetHealth` to sync building HP in `BuildingGpuState` |
| CPU → GPU | Building damage flash | `damage_system` writes `GpuUpdate::BldSetDamageFlash` (intensity 1.0, decays at 5.0/s) |
| GPU → CPU | Tower targeting | `building_tower_system` reads `GpuReadState.combat_targets[npc_count + bld_slot]` — GPU spatial grid targeting (same as NPC targeting) |
| GPU → CPU | Projectile hits | `process_proj_hits`: unified `DamageMsg` for all hits (entity_idx routes NPC vs building in damage_system) |
| GPU → CPU | Building targeting | `attack_system` reads `combat_targets[i]` — GPU returns building indices (`>= npc_count`) when buildings are nearest enemy |

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
- **KillStats naming can be misread**: `archer_kills` tracks raiders killed by guards/defenders, while `villager_kills` tracks villagers killed by raiders.
