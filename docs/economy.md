# Economy System

## Overview

Economy systems handle time progression, food production, starvation, raider foraging, and AI decisions. All run in `Step::Behavior` and use `GameTime.hour_ticked` for hourly event gating. Defined in `rust/src/systems/economy.rs` and `rust/src/systems/ai_player.rs`.

## Data Flow

```
game_time_system (every frame)
    │
    ▼ sets hour_ticked = true when hour changes
    │
    ├─ farm_growth_system (every frame, uses game-time delta)
    │   └─ FarmStates: Growing → Ready when progress >= 1.0
    │
    ├─ raider_forage_system (hourly)
    │   └─ Each raider town gains RAIDER_FORAGE_RATE food
    │
    ├─ spawner_respawn_system (hourly)
    │   └─ Detects dead NPCs linked to FarmerHome/ArcherHome/FighterHome/Tent/MinerHome, counts down 12h timer, spawns replacement
    │
    ├─ starvation_system (hourly)
    │   └─ NPCs with zero energy → Starving marker
    │
    ├─ mine_regen_system (every frame, uses game-time delta)
    │   └─ MineStates: gold slowly regenerates when mine is unoccupied
    │
    ├─ sync_miner_progress_render (every frame)
    │   └─ Populates MinerProgressRender from miners with MiningProgress (positions + progress for GPU bar rendering)
    │
    ├─ farm_visual_system (every frame)
    │   └─ FarmStates Growing→Ready: spawn FarmReadyMarker; Ready→Growing: despawn
    │
    ├─ ai_decision_system (real-time interval, default 5s)
    │   └─ Per AI settlement: build → unlock slots → buy upgrades (food-gated, personality-driven)
    │   └─ Uses AiBuildRes SystemParam bundle (8 mutable resources) to stay under Bevy's 16-param limit
    │
    └─ squad_cleanup_system (dirty-flag gated: DirtyFlags.squads)
        └─ Phase 1: remove dead slots from Squad.members
        └─ Phase 2: keep Default Squad (0) as live pool of unsquadded player archers
        └─ Phase 3: dismiss excess if members > target_size (remove SquadId)
        └─ Phase 4: auto-recruit unsquadded player archers if members < target_size (insert SquadId)
```

## Systems

### game_time_system
- Advances `GameTime.total_seconds` based on `time.delta_secs() * time_scale`
- Sets `hour_ticked = true` when the hour changes (single source of truth for hourly events)
- Respects `paused` flag
- Other systems check `game_time.hour_ticked` instead of tracking their own timers

### growth_system (unified farms + mines)
- Runs every frame, advances growth based on elapsed game time
- Skips tombstoned entries (`position.x < -9000`) — destroyed farms/mines don't regrow
- **GrowthStates resource**: tracks `Growing` vs `Ready` state, progress (0.0-1.0), and `GrowthKind` (Farm/Mine) per entry
- **Hybrid growth model**:
  - Passive: `FARM_BASE_GROWTH_RATE` (0.08/hour) — ~12 game hours to full growth
  - Tended: `FARM_TENDED_GROWTH_RATE` (0.25/hour) — ~4 game hours with farmer working
- Farm transitions to `Ready` when progress >= 1.0
- Checks `BuildingOccupancy.is_occupied()` to determine if a farmer is tending
- **FarmYield upgrade**: growth rate multiplied by `1.0 + level * 0.15` per-town (reads `TownUpgrades` via `farm.town_idx`)

### raider_forage_system
- Runs when `game_time.hour_ticked` is true
- Each raider town (faction > 0) gains `RAIDER_FORAGE_RATE` (1) food per hour
- Passive income ensures raiders can survive even if they never steal

### spawner_respawn_system
- Runs when `game_time.hour_ticked` is true
- Each `SpawnerEntry` in `SpawnerState` links a unit-home building (FarmerHome→farmer, ArcherHome→archer, FighterHome→fighter, Tent→raider) or MinerHome (miner) to an NPC slot
- If `npc_slot >= 0` and NPC is dead (not in `NpcEntityMap`): starts 12h respawn timer
- Timer decrements 1.0 per game hour; on expiry: allocates slot via `SlotAllocator`, emits `SpawnNpcMsg`, logs to `CombatLog`
- Newly-built spawners start with `respawn_timer: 0.0` — the `>= 0.0` check catches these, spawning an NPC on the next hourly tick
- Tombstoned entries (position.x < -9000) are skipped (building was destroyed)
- Spawn mapping resolved by `world::resolve_spawner_npc()` (single source of truth): FarmerHome → Farmer (nearest **free** farm via `find_nearest_free`), ArcherHome → Archer (nearest waypoint via `find_location_within_radius`), FighterHome → Fighter (nearest waypoint via `find_location_within_radius`), Tent → Raider (home = tent position), MinerHome → Miner (assigned mine from `MinerHome.assigned_mine` if set, otherwise nearest gold mine via `find_nearest_free`). All types look up faction from `world_data.towns[town_idx].faction`. Same function used by `game_startup_system` for initial NPC spawns.

### starvation_system
- Runs when `game_time.hour_ticked` is true
- NPCs with `energy <= 0` get `Starving` marker
- Starving NPCs: speed set to `CachedStats.speed * STARVING_SPEED_MULT` (0.5) via `GpuUpdate::SetSpeed`
- When energy rises above 0 (eating or resting): `Starving` removed, speed restored to `CachedStats.speed`

## Farm Growth

Farms have a growth cycle instead of infinite food:

```
     farm_growth_system advances progress
              │
              ▼
┌──────────┐      progress >= 1.0      ┌─────────┐
│ Growing  │ ────────────────────────▶ │  Ready  │ (shows food icon)
│progress++│                           │progress=1│
└──────────┘                           └────┬────┘
      ▲                                     │
      │            farmer/raider arrival    │
      └─────────────────────────────────────┘
              reset to Growing, progress=0

```

**Farm harvest** uses `GrowthStates::harvest()` (single source of truth for Ready → Growing transition):
- `harvest(idx, combat_log, game_time, faction) -> i32` — resets to Growing, returns yield (farm=1 food, mine=MINE_EXTRACT_PER_CYCLE gold), logs to CombatLog. Returns 0 if not Ready.
- All callers enter `Activity::Returning { loot }` and carry yield home — delivery happens via `arrival_system` proximity check. No caller instant-credits storage.
- Called from 5 sites: arrival_system (working farmer harvest), decision_system (farmer GoingToWork arrival), decision_system (raider steal), decision_system (miner Mining arrival), decision_system (MiningAtMine harvest)

**Farmer harvest** (visible food transport):
- Working farmer at farm: `arrival_system` detects Ready farm, calls `harvest()`, releases occupancy, removes `AssignedFarm`, enters `Returning { loot: [(Food, 1)] }` targeting home. Farmer visibly carries food sprite home.
- GoingToWork arrival: if farm already Ready, `harvest()` + immediate `Returning` (no claim/Working). If not Ready, claim farm + `Working` (tending).
- On delivery at home: farmer goes `GoingToWork` (back to farm), not Idle. Continuous work→carry→deliver→return cycle.

**Raider steal** (decision_system, Raiding arrival):
- Uses `find_location_within_radius()` to find farm within FARM_ARRIVAL_RADIUS
- Only steals if farm is Ready — `harvest()` resets farm, enters `Returning { loot: Food }`
- If farm not ready: find a different farm (excludes current position, skips tombstoned); if no other farm found, return home
- Logs "Stole food → Returning" vs "Farm not ready, seeking another" vs "No other farms, returning"

**Farm destruction**: `FarmStates::tombstone(farm_idx)` resets all 3 parallel vecs (positions, states, progress) — called by `remove_building()`. Tombstoned position (-99999) causes render pipeline to skip the crop sprite and `farm_growth_system` to skip growth.

**Visual feedback**: `farm_visual_system` watches `FarmStates` for state transitions and spawns/despawns `FarmReadyMarker` entities. Uses `Local<Vec<FarmGrowthState>>` to detect transitions without extra resources. Growing→Ready spawns a marker; Ready→Growing (harvest) despawns it.

## Starvation

Energy is the single survival resource. When energy hits zero, the NPC is starving:

```
energy_system drains energy while active
        │
        ▼  starvation_system (hourly)
energy <= 0?
        │
    YES ▼
┌────────────┐
│  Starving  │  - HP capped at 50%
│   marker   │  - Speed reduced to 50%
└────┬───────┘
     │ energy > 0 (eating or resting)
     ▼
Starving marker removed
Speed restored to CachedStats.speed
```

**Recovery paths:**
- **Eat**: consumes 1 food from town storage, instantly restores energy to 100. No travel required.
- **Rest**: walk home to spawner building (FarmerHome/ArcherHome), recover energy slowly (6 hours 0→100). Works even when starving.

**Constants:**
- `STARVING_HP_CAP`: 0.5 (50% of MaxHealth)
- `STARVING_SPEED_MULT`: 0.5 (50% of normal speed)

Starvation applies to **both villagers and raiders**. If raiders can't steal food and their town runs out, they'll starve and become easier to kill.

The HP cap is enforced by `healing_system` — starving NPCs can't heal above 50% MaxHealth even inside a healing aura.

## Raider Attack System

Raiders use the AI squad commander's wave-based attack cycle (see [ai-player.md](ai-player.md#wave-based-attack-cycle)). Each raider town gets one squad containing all raiders. The `ai_squad_commander_system` picks the nearest enemy farm as target, gathers raiders until `wave_min_start` (RAID_GROUP_SIZE) are ready, then dispatches the wave. Wave ends when the target is destroyed or losses exceed `wave_retreat_below_pct` (30%).

Raiders without a squad assignment wander near their town. The old `RaidQueue` group-formation system has been replaced by squad-driven waves.

**Constants:**
- `RAID_GROUP_SIZE`: 3 (minimum raiders to start a wave)

## Resources

| Resource | Purpose | Updated By |
|----------|---------|------------|
| GameTime | total_seconds, time_scale, paused, hour_ticked | game_time_system |
| FoodStorage | `Vec<i32>` — food count per town | harvest, steal, forage, respawn |
| FoodEvents | delivered/consumed event logs | arrival_system, decision_system |
| FarmStates | Growing/Ready state + progress per farm | farm_growth_system, harvest/steal |
| GoldStorage | `Vec<i32>` — gold count per town | mining delivery, UI |
| MineStates | gold, max_gold, positions per mine | mine_regen_system, mining behavior |
| MinerProgressRender | positions + progress for active miners | sync_miner_progress_render → render world (ExtractResource) |
| BuildingOccupancy | private map, methods: claim/release/is_occupied/count/clear | decision_system, death_cleanup, game_startup, spawner_respawn |
| MiningPolicy | discovered_mines per town, mine_enabled per mine | mining_policy_system (dirty-flag gated) |
| RaiderState | max_pop, respawn_timers, forage_timers | raider_forage_system |
| SpawnerState | `Vec<SpawnerEntry>` — building→NPC links + respawn timers | spawner_respawn_system, game_startup |
| PopulationStats | alive/working/dead per (job, town) | spawn, death, state transitions |

## Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| FARM_BASE_GROWTH_RATE | 0.08/hour | Passive growth (~12h to harvest) |
| FARM_TENDED_GROWTH_RATE | 0.25/hour | Tended growth (~4h to harvest) |
| RAIDER_FORAGE_RATE | 1 food/hour | Passive raider food income |
| STARVING_HP_CAP | 0.5 | 50% MaxHealth cap while starving |
| STARVING_SPEED_MULT | 0.5 | 50% speed while starving |
| RAID_GROUP_SIZE | 5 | Min raiders to form a raid group |

### Building Costs

Flat costs via `building_cost(kind)` in `constants.rs` (no difficulty scaling). All defined in `BUILDING_REGISTRY`:

| Building | Cost |
|----------|------|
| Farm | 2 |
| FarmerHome | 2 |
| MinerHome | 4 |
| ArcherHome | 4 |
| CrossbowHome | 8 |
| FighterHome | 5 |
| Waypoint | 1 |
| Tent | 3 |

Both player build menu and AI player use `building_cost()` for affordability checks.
| SPAWNER_RESPAWN_HOURS | 12.0 | Game hours before dead NPC respawns from building |
| MINE_MAX_GOLD | 200.0 | Maximum gold a mine can hold |
| MINE_REGEN_RATE | 2.0/hour | Gold regeneration rate (when unoccupied) |
| MINE_EXTRACT_PER_CYCLE | 5 | Base gold per mining cycle (scaled by GoldYield upgrade: `base * (1 + level * 0.15)`) |
| MINE_WORK_HOURS | 4.0 | Game hours per mining work cycle (progress bar 0→1) |
| MINE_MIN_SETTLEMENT_DIST | 300.0px | Minimum distance from mine to any town center |
| MINE_MIN_SPACING | 400.0px | Minimum distance between mines |

### mine_regen_system
- Runs every frame, advances mine gold based on elapsed game time
- Only regenerates when mine has no occupant (`BuildingOccupancy.is_occupied()` returns false)
- Rate: `MINE_REGEN_RATE` (2.0 gold/hour), capped at `MINE_MAX_GOLD` (200.0) per mine
- Uses `MineStates` resource — parallel Vecs of gold, max_gold, and positions per mine

### mining_policy_system
- Gated by `DirtyFlags.mining` — only runs when mining topology/policy changes (radius slider, mine toggle, miner spawn/death)
- **Discovery**: scans `WorldData.gold_mines` within `PolicySet.mining_radius` of each faction-0 town center, populates `MiningPolicy.discovered_mines[town_idx]`
- **Distribution**: collects alive auto-assigned miners per town (skips `MinerHome.manual_mine == true`), round-robin assigns across enabled discovered mines via `MinerHome.assigned_mine`
- **Stale clearing**: if assigned mine falls outside radius or disabled, clears `assigned_mine` on auto-assigned miners
- `MAX_MINE_OCCUPANCY` in `constants.rs` limits concurrent miners per mine; behavior system (`decision_system`) skips full mines

### squad_cleanup_system
- Dirty-flag gated via `DirtyFlags.squads` — skips entirely when no squad-relevant changes occurred
- Flag set by: `death_cleanup_system` (any death), `spawn_npc_system` (archer spawn), left_panel UI (assign/dismiss), save load (`DirtyFlags::default()`)
- **Phase 1**: retains only members whose slot is still in `NpcEntityMap` (alive)
- **Phase 2**: keeps Default Squad (index 0) as live pool of unsquadded player archers (inserts `SquadId(0)`)
- **Phase 3**: if `target_size > 0` and `members.len() > target_size`, dismisses excess (removes `SquadId` + `DirectControl` components, pops from members)
- **Phase 4**: if `target_size > 0` and `members.len() < target_size`, auto-recruits unsquadded player-faction archers (inserts `SquadId`, pushes to members). Pool is shared across squads — earlier squad indices get priority.

## Dynamic Raider Town Migration

New raider towns spawn organically as the player grows. Three systems in `economy.rs` handle the lifecycle:

```
migration_spawn_system (hourly check)
        │
        ▼ check_timer >= RAIDER_SPAWN_CHECK_HOURS (12h)?
        │ player_alive >= VILLAGERS_PER_RAIDER * (raider_count + 1)? (alive towns only)
        │ no active migration? alive raider_towns < MAX_RAIDER_TOWNS (20)?
        │
        ▼ YES: spawn group at random map edge
        │
        ├─ Create Town entry (faction = max+1, sprite_type = 1)
        ├─ Create TownGrid, extend all per-town resources (food, gold, factions, raider_state, policies)
        ├─ Create inactive AiPlayer (active: false, kind: Raider, random personality)
        ├─ Spawn N raiders via SpawnNpcMsg with Home = player town center
        │   Group size: MIGRATION_BASE_SIZE (3) + player_alive / difficulty.migration_scaling()
        │   (Easy=6, Normal=4, Hard=2), capped at 20
        └─ Store MigrationGroup in MigrationState resource
           Combat log: "A raider band approaches from the {direction}!"

migration_attach_system (after Step::Spawn, before Step::Combat)
        │
        ▼ if migration active: attach Migrating component to spawned entities
           (bridges 1-frame gap between SpawnNpcMsg and entity creation)

migration_settle_system (every frame, early-returns if no active migration)
        │
        ▼ read GPU positions for group members
        │ compute average position
        │
        ▼ within RAIDER_SETTLE_RADIUS (3000px, ~30s walk) of any town?
        │
        ▼ YES: settle town
        ├─ Update Town.center to average group position
        ├─ place_buildings() — town center + tents in spiral
        ├─ register_spawner() for each tent
        ├─ stamp_dirt() around town
        ├─ Activate AiPlayer (active = true)
        ├─ Remove Migrating from members, update Home to town center
        ├─ Force tilemap rebuild (TilemapSpawned = false)
        └─ Clear MigrationState.active
           Combat log: "A raider band has settled nearby!"
```

**Movement**: Raiders use the existing `Home` component + `Action::Wander` behavior. Setting `Home` to the player's town center makes them naturally walk there — no custom pathfinding needed.

**Save/load**: `MigrationState` serialized as `Option<MigrationSave>` in `SaveData`. On load, `Migrating` component re-attached to saved member slot entities.

**AiPlayer.active**: New `bool` field. `ai_decision_system` skips inactive players. All existing AiPlayer creation sites set `active: true`. Migration creates with `active: false`, activated on settlement.

### Migration Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| RAIDER_SPAWN_CHECK_HOURS | 12.0 | Game hours between migration trigger checks |
| MAX_RAIDER_TOWNS | 20 | Maximum number of dynamic raider towns |
| RAIDER_SETTLE_RADIUS | 3000.0px | Distance to any town that triggers settlement (~30s walk at 100px/s) |
| MIGRATION_BASE_SIZE | 3 | Base raiders per migration group |
| VILLAGERS_PER_RAIDER | 20 | Player alive NPCs per raider town threshold |

## Known Issues

None currently.
