# Combat System

## Overview

Six Bevy systems handle the complete combat loop: cooldown management, GPU-targeted attacks (NPC and building targets), unified damage application (NPCs + buildings), unified death processing (mark dead + XP grant + building destruction + NPC cleanup + despawn), and building tower auto-attack (fountains + player-built towers). All six run chained in `Step::Combat`.

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
        ├── target has no building instance → NPC target (existing flow)
        ├── target has building instance → building target (GPU-targeted, real damage projectile)
        │
        ├── In range + cooldown ready → fire projectile → GPU projectile system
        └── Out of range → MovementIntents.submit(Combat) → resolve_movement_system
                                                      │
                                                      ▼
DamageMsg (from process_proj_hits)             GPU movement
        │  (target: EntityUid — resolved to slot via entity_map.slot_for_uid(), then routes via EntityMap.get_instance() presence check)
        ▼
  damage_system
  ├─ Building (slot in EntityMap):
  │   apply to Health, SetHealth, SetDamageFlash
  │   insert LastHitBy (for loot attribution)
  └─ NPC (slot in EntityMap):
      apply to Health, sync GPU, insert LastHitBy, SetDamageFlash
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
      │   ├─ Hide + SetHealth(0), GpuSlotPool.free(idx)
      │   └─ remove_by_slot (slot_to_entity + instances + by_kind)
      └─ NPC branch:
          ├─ XP grant — NPC killer (LastHitBy → 100 XP, level-up, stat re-resolve)
          ├─ XP grant — tower killer (LastHitBy → BuildingInstance.xp += 100, .kills++)
          ├─ Loot — NPC killer (npc_def loot_drop → Activity::Returning)
          ├─ Loot — tower killer (npc_def loot_drop → FoodStorage/GoldStorage + flash)
          ├─ despawn entity, HideNpc → GPU (-9999)
          ├─ Release NpcWorkState via WorkIntentMsg(Release { uid }) → resolved by resolve_work_targets
          ├─ Update FactionStats, KillStats, PopulationStats
          └─ GpuSlotPool.free(idx)
        │
        ▼
  building_tower_system
  (fountains + towers via GPU combat_targets,
   reads readback at bld_slot — unified namespace)
```

attack_system emits `ProjGpuUpdateMsg` when in range, or applies point-blank damage for melee. The projectile system ([projectiles.md](projectiles.md)) handles movement, collision detection, hit readback, and slot recycling.

## Components

| Component | Type | Purpose |
|-----------|------|---------|
| Health | `f32` | Current HP (per-type: Farmer 60, Crossbow 70, Archer/Miner 80, Raider 120, Fighter 150) |
| Dead | marker | Inserted when health <= 0 |
| LastHitBy | `i32` | NPC slot index of last attacker (-1 = no attacker). Inserted by damage_system, read by death_system for XP grant + loot attribution. |
| Faction | `struct(i32)` | Faction ID (0=Neutral, 1=Player, 2+=AI settlements). NPCs attack different factions. Neutral (0) is treated as same-faction by GPU combat targeting and projectile collision. GPU shaders also treat -1 as non-hostile (dead/empty slot sentinel). |
| BaseAttackType | enum | `Melee` or `Ranged` — keys into `CombatConfig.attacks` HashMap. Crossbow units use `Ranged` but stats resolve from `CombatConfig.crossbow_attack` (overridden by Job in `resolve_combat_stats`). |
| CachedStats | struct | `damage, range, cooldown, projectile_speed, projectile_lifetime, max_health, speed` — resolved from `CombatConfig` via `resolve_combat_stats()` |
| AttackTimer | `f32` | Seconds until next attack allowed |
| CombatState | enum | `None`, `Fighting { origin: Vec2 }`, `Fleeing` — orthogonal to Activity enum (see [behavior.md](behavior.md)) |
| ManualTarget | enum | Player target: `Npc(usize)` (attack NPC slot), `Building(Vec2)` (attack building), `Position(Vec2)` (ground move). Inserted by right-click on DirectControl NPCs. `Npc` variant overrides GPU auto-targeting; others fall through. |

## System Pipeline

Execution order is **chained** — each system completes before the next starts.

### 1. cooldown_system (combat.rs)
- Query-first: `(&GpuSlot, &mut AttackTimer)` with `(Without<Building>, Without<Dead>)` — no EntityMap lookup
- Decrements `AttackTimer` by `time.delta_secs()` each frame
- When timer reaches 0, attack is available
- Updates `CombatDebug` with sample timer and entity count

### 2. attack_system (combat.rs)
- **Query-first iteration**: uses a read-only ECS query `(Entity, &GpuSlot, &Job, &Faction, &CachedStats, &Activity, Option<&SquadId>, Option<&ManualTarget>)` with `Without<Building>, Without<Dead>` for the outer NPC loop. `AttackQueries` SystemParam holds only mutable queries (`&mut CombatState`, `&mut AttackTimer`). `EntityMap` retained for building target resolution.
- **Manual target override**: if NPC has `ManualTarget::Npc(slot)`, uses that slot as target instead of GPU `combat_targets[i]`. Auto-clears `ManualTarget` when target's GPU health <= 0 (dead). `ManualTarget::Building` and `ManualTarget::Position` variants fall through to GPU auto-targeting. `ManualTarget` is matched by reference (no clone per-NPC per-frame). See [behavior.md](behavior.md#squads) for how `ManualTarget` is set.
- **Hold fire**: if NPC's squad has `hold_fire == true` and no `ManualTarget`, target is set to -1 (skip auto-engage). Reads `SquadState` via `SquadId`.
- Falls back to `GpuReadState.combat_targets` for NPCs without manual target or hold-fire.
- **Skips** NPCs with `Activity::Returning`, `Activity::GoingToRest`, or `Activity::Resting` (prevents combat while heading home, going to bed, or sleeping)
- **Unified GPU targeting**: `combat_targets[i]` returns a unified entity slot. Building vs NPC is determined by `entity_map.get_instance()` presence check. One code path for all target types.
- **Building targets** (target has instance in `EntityMap`):
  - **Roads skipped**: `BuildingKind::Road` targets are ignored (roads are untargetable — also filtered via `ENTITY_FLAG_UNTARGETABLE` in GPU compute shader)
  - Only **archers**, **crossbows**, and **raiders** attack buildings (farmers/miners/fighters skip)
  - Validates via `entity_map.get_instance(target)` — checks faction (skip same-faction)
  - Gets building position from `BuildingInstance.position`
  - In range + cooldown ready: fires projectile with **real damage** (GPU projectile collision handles hit detection against buildings in the unified entity grid)
  - Point-blank: emits `DamageMsg` directly
  - Out of range but within close chase radius (range + 120px): chases building (`SetTarget` to building position)
  - Beyond close chase radius: ignores building (prevents cross-map pursuit of distant enemy buildings)
- **NPC targets** (target has no building instance):
  - Validates via `entity_map.get_npc()` lookup; **faction check uses ECS faction** from EntityMap (not GPU readback, which can be stale/-1 on throttled frames); liveness check via ECS (`EntityMap.get_npc().dead`)
  - Sets `CombatState::Fighting { origin }` (stores current position)
  - **In range**: submits `MovementIntents` at `Combat` priority to own position (stand ground — stops GPU movement, NPC holds position while shooting). Projectile dodge from GPU shader provides evasion.
  - **In range + cooldown ready**: resets `AttackTimer`, fires projectile or applies point-blank damage
  - **Out of range**: submits `MovementIntents` at `Combat` priority to chase target

### 3. damage_system (health.rs)
- Drains unified `DamageMsg` events from Bevy MessageReader
- Resolves `event.target` (EntityUid) to slot via `entity_map.slot_for_uid()` — skips if UID no longer valid
- Routes by slot: checks `entity_map.get_instance(idx)` — if found, it's a building; otherwise, it's an NPC (both share one `EntityMap`)
- **Building damage** (slot has instance in `EntityMap`):
  - O(1) lookup via `entity_map.get_instance(idx)` + `entity_map.entities.get(&idx)`
  - Skips indestructible buildings (GoldMine, Road) and already-dead buildings
  - Subtracts damage, pushes `GpuUpdate::SetHealth` + `GpuUpdate::SetDamageFlash`
  - If HP > 0: sets `BuildingHealState.needs_healing` for healing_system
  - Inserts `LastHitBy(attacker)` on buildings for death_system loot attribution
- **NPC damage** (entity_idx not in building instances):
  - O(1) entity lookup via `entity_map.entities[&entity_idx]`
  - Subtracts damage: `health.0 = (health.0 - amount).max(0.0)`
  - Pushes `GpuUpdate::SetHealth` + `GpuUpdate::SetDamageFlash` (intensity 1.0)
  - If `attacker >= 0`: inserts `LastHitBy(attacker)` via `get_entity()` guard

### 4. death_system (health.rs)

Unified death handler — replaces the old `death_system` + `xp_grant_system` + `building_death_system` + `death_cleanup_system` pipeline. Uses `ParamSet` to resolve query conflict between mark-dead (reads `&Health` on `Without<Dead>`) and killer/loot access (writes `&mut Health` on `Without<Dead>`). Uses `DeathResources` SystemParam (16 fields) merging CleanupResources + WorldState unique fields + BuildingDeathExtra fields.

**Phase 1: Mark dead** (deferred — takes effect next frame)
- Queries all entities `Without<Dead>` where `health.0 <= 0.0`
- Inserts `Dead` marker via deferred commands (same 1-frame delay as before)

**Phase 2: Process dead** (entities `With<Dead>` from previous frame)

For each dead entity:

**Building branch** (detected via `Building` component):
- Looks up instance data (kind, position, town_idx) from `entity_map.get_instance(idx)`, copies fields before mutation
- **Orphaned NPC home reset**: if building has `npc_uid`, the linked NPC's `Home` component is set to `(-1, -1)` — NPC becomes homeless (shown as "Homeless" in inspector). Prevents NPCs from walking to a destroyed building to rest.
- Calls `destroy_building()` for grid cleanup (grid cell clear + wall auto-tile + combat log — no entity lifecycle)
- Emits `mark_building_changed(kind)` dirty signals
- **Fountain death**: deactivates AI player for that town. In endless mode, queues replacement AI (`PendingAiSpawn`) scaled to player strength.
- **Building loot**: `BuildingDef::loot_drop()` returns `cost / 2` as food. Uses `LastHitBy` to find attacker, looks up attacker entity via `params.p1()`. Attacker set to `Activity::Returning { loot }`, targets home. DC keep-fighting override skips disengage + home target when `dc_no_return`.
- `remove_by_slot(idx)` (clears `entities` + `instances` + `by_kind`), `GpuSlotPool.free(idx)` (allocator queues GPU hide cleanup — position=-9999, health=0, speed=0, flags=0)

**NPC branch:**
- **XP grant (NPC killer)**: if `LastHitBy` present and killer is NPC (via `entity_map.get_npc`), grants 100 XP, increments `FactionStats.inc_kills()`. Checks for level-up: `level_from_xp(new_xp) > level_from_xp(old_xp)`. On level-up: re-resolves `CachedStats`, updates `Speed`, rescales HP proportionally, sends GPU updates, emits `CombatEventKind::LevelUp`.
- **Loot on kill (NPC killer)**: reads `npc_def(dead_job).loot_drop`, picks one deterministically via `xp % len`. Sets killer to `Activity::Returning { loot }`, clears `CombatState::None`. DC keep-fighting override applies. Equipment loot: if `npc_def.equipment_drop_rate > 0`, rolls deterministic check — on success, `roll_loot_item()` generates a `LootItem` pushed to killer's `CarriedLoot.equipment`.
- **Equipment drop on death**: victim's `NpcEquipment` items (via `all_items()`) and `CarriedLoot.equipment` each transfer to killer at 50% per-item (deterministic hash roll). NPC killers receive items in `CarriedLoot.equipment` (delivered to `TownInventory` on return home). Tower/fountain killers deposit directly to `TownInventory`.
- **XP grant (tower/fountain killer)**: if killer slot is a Fountain or Tower building (via `entity_map.get_instance`), grants 100 XP to `BuildingInstance.xp`, increments `BuildingInstance.kills` and `FactionStats.inc_kills()`. Same `level_from_xp()` formula as NPCs. Level-up emits `CombatEventKind::LevelUp` to combat log.
- **Loot on kill (tower killer)**: same `npc_def(dead_job).loot_drop` table, deposited directly to `FoodStorage`/`GoldStorage` for the tower's town (towers can't carry). `SetDamageFlash` on tower for visual feedback. Loot event logged to combat log. Equipment from victim's `NpcEquipment` and `CarriedLoot.equipment` deposited to `TownInventory` at 50% per item.
- Despawn entity, `GpuSlotPool.free(idx)` (allocator queues GPU hide cleanup), release AssignedFarm/WorkPosition
- Update stats: `PopulationStats`, `FactionStats`, `KillStats`
- Remove from `EntityMap.npc_by_town` (via `unregister_npc`), deselect if SelectedNpc matches
- `GpuSlotPool.free(idx)` — recycle slot

XP formula: `level = floor(sqrt(xp / 100))`, level multiplier = `1.0 + level * 0.01`

### 5. building_tower_system (combat.rs)

Tower auto-attack using GPU spatial grid targeting. Towers are in the unified entity buffer at their unified slot with `ENTITY_FLAG_BUILDING | ENTITY_FLAG_COMBAT`. The GPU compute shader MODE 2 runs the same combat targeting scan for towers as for NPC combatants — finding the nearest enemy NPC via the spatial grid.

- **TowerState** resource: `town: TowerKindState` (Vec-indexed by town for fountains) + `tower_cooldowns: HashMap<usize, f32>` (slot-indexed for player-built towers)
- **TowerStats** struct in `constants.rs`: `range`, `damage`, `cooldown`, `proj_speed`, `proj_lifetime`, `hp_regen`, `max_hp`
- **fire_projectile()** helper: shared projectile spawn function used by both `attack_system` (NPC ranged attacks) and `building_tower_system` (tower auto-attack). Takes raw `(src, target_pos, damage, proj_speed, lifetime, faction, shooter, sfx_writer)` — returns false when dist <= 1.0 (melee range, caller handles DamageMsg). Emits `PlaySfxMsg::ArrowShoot` with shooter position on successful fire. Eliminates duplication of ProjGpuUpdate::Spawn + SFX boilerplate across all 4 call sites.
- **Fountains**: `FOUNTAIN_TOWER` (range=400, damage=15, cooldown=1.5s, proj_speed=350, proj_lifetime=1.5s). Always-on — `attack_enabled` refreshed from `is_alive(town.center)` every tick. Lookup via `EntityMap.iter_kind_for_town(Fountain, town_idx)`.
- **Player-built Towers**: base `TOWER_STATS` (range=250, damage=10, cooldown=2.0s, proj_speed=300, proj_lifetime=1.2s, max_hp=1000). Buildable on TownGrid, 50 food cost, 1000 HP. Iterates via `EntityMap.iter_kind(Tower)`, per-slot cooldown in `tower_cooldowns` HashMap. Stale entries cleaned up via `retain()`.
- **Per-tower upgrades**: each tower has its own `upgrade_levels: Vec<u8>` and `auto_upgrade: bool` on `BuildingInstance`. `resolve_tower_instance_stats(level, upgrade_levels) -> TowerStats` applies XP level bonus (+1%/level to range/damage/max_hp) and per-stat upgrade multipliers from `TOWER_UPGRADES` (7 stats: HP, Attack, Range, AtkSpd, ProjSpd, ProjLife, HpRegen). Tower inspector shows resolved stats, per-stat upgrade buttons with cost/effect, and auto-buy checkbox. `auto_tower_upgrade_system` runs each game-hour for towers with `auto_upgrade = true`, buys cheapest affordable upgrade.
- **Tower HP regen**: towers with `hp_regen > 0` (from HpRegen upgrades, +2.0 HP/s per level) heal each frame in `building_tower_system`, capped at resolved `max_hp`.
- **GPU-side targeting**: Reads `GpuReadState.combat_targets[bld_slot]` from readback buffer (building slot IS the GPU index — unified namespace, no offset). Only NPC targets are valid (towers skip building targets via `EntityMap` check). Target re-validated via ECS: must exist in EntityMap, not dead, and enemy faction (per [authority.md](authority.md) — `combat_targets` is candidate-only).
- **Projectile spawn**: Both fountain and tower loops call `fire_projectile()` with `shooter: bld_slot` (building's unified entity slot — enables GPU self-collision skip).


## Slot Recycling

```
NPC Spawn:  GpuSlotPool.alloc()  ──▶ pop free list (or next++)
                                              ▲
NPC Death:  death_system  ───────────────────────┘
            GpuSlotPool.free(idx)

Building:   GpuSlotPool.alloc()  ──▶ pop free list (or next++)
                                              ▲
Bld Death:  death_system  ───────────────────────┘
            GpuSlotPool.free(idx)
```

NPCs and buildings share a unified slot allocator (`GpuSlotPool`, max=MAX_ENTITIES=200K) backed by a `SlotPool` inner type. Each entity's slot IS its GPU buffer index — no offset arithmetic needed.

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
| CPU → GPU | Building HP sync | `damage_system` writes entity `Health` + `GpuUpdate::SetHealth` to sync building HP in `EntityGpuState` |
| CPU → GPU | Building damage flash | `damage_system` writes `GpuUpdate::SetDamageFlash` (intensity 1.0, decays at 5.0/s) |
| GPU → CPU | Tower targeting | `building_tower_system` reads `GpuReadState.combat_targets[bld_slot]` — unified slot IS GPU index (same as NPC targeting) |
| GPU → CPU | Projectile hits | `process_proj_hits`: unified `DamageMsg` for all hits (EntityUid target resolved to slot in damage_system) |
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

- **No generational indices on slots**: Stale slot references could silently alias. Mitigated by DamageMsg using EntityUid (stable identity) instead of raw slots — damage_system resolves UID→slot, skipping if the UID is no longer valid. Chained execution within Step::Combat provides additional safety.
- **No friendly fire**: Faction check prevents same-faction damage. No way to enable it selectively.
- **CombatState::Fighting blocks behavior decisions**: While fighting, decision_system skips the NPC. However, Activity is preserved through combat — when combat ends (`CombatState::None`), the NPC resumes its previous activity.
- **KillStats faction-attributed**: `archer_kills` counts enemy NPCs killed by player faction (killer_faction == FACTION_PLAYER, victim != FACTION_PLAYER); `villager_kills` counts player NPCs killed by enemies (killer_faction != FACTION_PLAYER, victim == FACTION_PLAYER). Attribution uses `last_hit_by` slot → faction lookup via EntityMap (NPC or building).
