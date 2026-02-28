# Spawn System

## Overview

NPCs are created through `SpawnNpcMsg` messages processed by `spawn_npc_system`. Slot allocation uses Bevy's `GpuSlotPool` resource (unified for NPCs + buildings), which reuses dead entity indices before allocating new ones. Job determines the component template at spawn time. All GPU writes go through `GpuUpdateMsg` messages — see [messages.md](messages.md).

The core spawn logic lives in `materialize_npc()` — a shared helper used by both fresh spawns and save-load. This ensures a single source of truth for entity creation, GPU init, and tracking cache registration.

## Data Flow

```
SpawnNpcMsg (via MessageWriter)          NpcSaveData (from save file)
    │                                        │
    ▼ (Step::Spawn)                          ▼ (load_game_system)
spawn_npc_system                         spawn_npcs_from_save
    │                                        │
    └──────────┐                ┌────────────┘
               ▼                ▼
          materialize_npc(overrides)
               │
               ├─ Emit GPU updates: SetPosition, SetTarget,
               │   SetSpeed, SetFaction, SetHealth, SetSpriteFrame, SetFlags
               │
               ├─ Spawn ECS entity with full component set
               │   (nested tuple bundles + conditional inserts)
               │
               ├─ Register slot→entity index in EntityMap
               │
               ├─ Initialize NpcMetaCache (name, level, trait)
               │
               └─ Add to NpcsByTownCache
```

Fresh spawns pass `NpcSpawnOverrides::default()` (all None — uses generated values). Save-load fills overrides with restored state (health, energy, activity, personality, name, level, equipment, etc.). `FactionStats.inc_alive()` is called only in `spawn_npc_system` (save-load restores FactionStats from the save file directly).

## Slot Allocation

`GpuSlotPool` (NPCs + buildings, max=MAX_ENTITIES=200K) wraps a `SlotPool` inner type (defined in `resources.rs`):

```rust
pub struct SlotPool {
    pub next: usize,      // High-water mark
    pub max: usize,       // Capacity cap
    pub free: Vec<usize>, // Recycled slots
}
```

`alloc()` pops from the free list first, falls back to incrementing `next` (capped at `max`). `free()` pushes onto the free list. LIFO reuse — most recently freed slot is allocated first. `GpuSlotPool` implements `Deref`/`DerefMut` to `SlotPool`.

NPC slots: allocated in `spawn_npc_system`, recycled in `death_system`.
Building slots: allocated in `place_building_instance`, recycled in `death_system` (building branch).
Both share the same `GpuSlotPool` — each entity's slot IS its GPU buffer index (no offset arithmetic).

GPU dispatch count comes from `GpuSlotPool.count()` (the high-water mark `next`). Dead entity slots within this range are hidden via sentinel position (-9999) and culled by the renderer.

## Spawn Parameters

`SpawnNpcMsg` fields:

| Field | Type | Notes |
|-------|------|-------|
| slot_idx | usize | Pre-allocated via GpuSlotPool |
| x, y | f32 | Spawn position |
| job | i32 | 0=Farmer, 1=Archer, 2=Raider, 3=Fighter, 4=Miner, 5=Crossbow |
| faction | i32 | 0=Player, 1+=AI settlements |
| town_idx | i32 | Town association (-1 = none) |
| home_x, home_y | f32 | Home position |
| work_x, work_y | f32 | Farm position (-1 = none, farmers only) |
| starting_post | i32 | Patrol start index (-1 = none, archers only) |
| attack_type | i32 | 0=melee, 1=ranged (fighters only) |

## materialize_npc

**ECS entity**: NPC entities are spawned with a full component set via nested tuple bundles (to stay under Bevy's 15-element tuple limit). Required components are always inserted; optional components (`PatrolRoute`, `WorkPosition`, `SquadId`, `EquippedWeapon/Helmet/Armor`, `LeashRange`, `Stealer`, `HasEnergy`) are conditionally inserted via `ecmds.insert()`. Buildings retain full ECS components (`GpuSlot`, `Position`, `Health`, `Faction`, `TownId`, `Building`).

**Required NPC components** (always inserted): `GpuSlot`, `Job`, `Faction`, `TownId`, `NpcFlags`, `Activity`, `Position`, `Home`, `Health`, `Energy`, `Speed`, `CombatState`, `CachedStats`, `BaseAttackType`, `AttackTimer`, `Personality`, `CarriedGold`.

**EntityMap registration**: `register_npc(slot, entity, job, faction, town_idx)` creates a lightweight `NpcEntry` (6 fields: slot, entity, job, faction, town_idx, dead) and adds the slot to `npc_by_town` secondary index. Debug assertion prevents duplicate slots.

Stats are resolved from `CombatConfig` resource via `resolve_combat_stats(job, attack_type, town_idx, level, personality, &config, &upgrades)`. The resolver applies job base stats × upgrade multipliers × trait multipliers × level multipliers. See `systems/stats.rs`. New NPCs spawn at level 0 (`level_from_xp(0) == 0`).

Job-specific optional components:

| Job | Optional Components |
|-----|------------|
| Archer | HasEnergy, PatrolRoute, EquippedWeapon, EquippedHelmet, `Activity::OnDuty { ticks_waiting: 0 }` |
| Crossbow | HasEnergy, PatrolRoute, EquippedWeapon, EquippedHelmet, `Activity::OnDuty { ticks_waiting: 0 }` |
| Farmer | HasEnergy |
| Miner | HasEnergy |
| Raider | HasEnergy, Stealer, LeashRange(400), EquippedWeapon |
| Fighter | HasEnergy, PatrolRoute, `Activity::OnDuty { ticks_waiting: 0 }` |

GPU writes (all jobs): `SetPosition`, `SetTarget` (spawn position; save-restore path may set work position for farmers), `SetSpeed(100)`, `SetFaction`, `SetHealth(100)`, `SetSpriteFrame` (job-based sprite from constants.rs), `SetFlags` (bit 0 = 1 for military jobs via `job.is_military()`, 0 for farmers/miners — controls GPU combat scan tier). Fresh spawns start with `NpcWorkState { occupied_slot: None, work_target: None }` — behavior system assigns work later. Save/restore path may restore explicit `work_target`. Colors and equipment sprites are derived from ECS component data by `build_visual_upload` (queries `EquippedWeapon/Helmet/Armor` components).

Sprite assignments: Farmer=(1,6), Archer=(0,11), Crossbow=(0,0) (placeholder, purple tint), Raider=(0,6), Fighter=(7,0), Miner=(1,6) (brown tint differentiates)

### Personality Generation

Deterministic based on slot index (reproducible). Each NPC gets 0-2 traits from [Brave, Tough, Swift, Focused] with 30% chance each, magnitude 0.5-1.5.

### Name Generation

Deterministic: adjective + job noun. Adjective cycles through a 10-word list, noun cycles through a 5-word job-specific list. Slot index determines both.

**Town index convention**: NPCs and buildings both use direct WorldData town indices. Villager towns are at even indices (0, 2, 4...), raider towns at odd indices (1, 3, 5...). `build_patrol_route()` is `pub(crate)` and uses `EntityMap::iter_kind_for_town(Waypoint, town_idx)` to filter waypoints directly (no `÷2` conversion).

## Building Spawners

All NPC population is building-driven: each **FarmerHome** supports 1 farmer, each **ArcherHome** supports 1 archer, each **CrossbowHome** supports 1 crossbowman, each **FighterHome** supports 1 fighter, each **MinerHome** supports 1 miner, and each **Tent** supports 1 raider. No NPCs are spawned directly at world gen — homes are placed with `respawn_timer: 0.0` and `spawner_respawn_system` spawns their NPCs on the first hour tick. Menu sliders control how many FarmerHomes/ArcherHomes/MinerHomes/Tents world gen places.

When an NPC dies, `spawner_respawn_system` (hourly, Step::Behavior) detects the death via `EntityMap` lookup, starts a 12-hour respawn timer, and spawns a replacement when it expires. New spawner buildings placed at runtime start with `respawn_timer: 0.0` — the system spawns the NPC on the next hourly tick. Raider grids only allow Tent placement; villager grids allow Farm/Waypoint/FarmerHome/ArcherHome/CrossbowHome/FighterHome/MinerHome.

Destroying a spawner building tombstones the `SpawnerEntry` (position.x = -99999). The linked NPC survives but won't respawn if killed.

All spawners set home to building position (FarmerHome/ArcherHome/CrossbowHome/FighterHome/MinerHome/Tent). All spawner types set faction from `world_data.towns[town_idx].faction` (player towns = 0, AI settlements = unique 1+).

## Known Issues

- **No spawn validation**: Doesn't verify town_idx is valid or that waypoints exist. Bad input silently creates an archer with no patrol route.
- **One-frame GPU delay**: GPU writes go through message collection → populate_buffer_writes → extract → upload. NPC won't render until the frame after Bevy processes the spawn.
