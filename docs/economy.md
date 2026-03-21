# Economy System

## Overview

Economy systems handle time progression, food production, starvation, raider foraging, and AI decisions. All run in `Step::Behavior` and use `GameTime.hour_ticked` for hourly event gating. Defined in `rust/src/systems/economy.rs` and `rust/src/systems/ai_player.rs`.

## Data Flow

```
game_time_system (every frame)
    │
    ▼ sets hour_ticked = true when hour changes
    │
    ├─ construction_tick_system (every frame, uses game-time delta)
    │   └─ BuildingInstance: under_construction countdown, HP scales 0.01→full, arms spawner on complete
    │
    ├─ farm_growth_system (every frame, uses game-time delta, iter_instances_mut with Farm/GoldMine match)
    │   └─ BuildingInstance: Growing → Ready when progress >= 1.0 (skips under-construction buildings)
    │
    ├─ raider_forage_system (hourly)
    │   └─ Each raider town gains RAIDER_FORAGE_RATE food
    │
    ├─ spawner_respawn_system (hourly)
    │   └─ Detects dead NPCs linked to FarmerHome/ArcherHome/FighterHome/Tent/MinerHome/MasonHome, counts down 12h timer, spawns replacement
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
    ├─ farm_visual_system (cadenced, see performance.md)
    │   └─ BuildingInstance Growing→Ready: spawn FarmReadyMarker; Ready→Growing: despawn
    │
    ├─ ai_decision_system (real-time interval, default 5s)
    │   └─ Per AI settlement: build → unlock slots → buy upgrades (food-gated, personality-driven)
    │   └─ Uses AiBuildRes SystemParam bundle (8 mutable resources) to stay under Bevy's 16-param limit
    │
    └─ squad_cleanup_system (message-gated: MessageReader<SquadsDirtyMsg>)
        └─ Phase 1: remove dead UIDs from Squad.members (UID→slot resolution, dead/missing = remove)
        └─ Phase 2: keep Default Squad (0) as live pool of unsquadded player military (slot→UID on push)
        └─ Phase 3: dismiss excess if members > target_size (UID→slot for entity lookup, remove SquadId)
        └─ Phase 4: auto-recruit unsquadded military if members < target_size (slot→UID on push, insert SquadId)
```

## Systems

### game_time_system
- Advances `GameTime.total_seconds` based on `time.delta_secs() * time_scale`
- Sets `hour_ticked = true` when the hour changes (single source of truth for hourly events)
- Respects `paused` flag
- Other systems check `game_time.hour_ticked` instead of tracking their own timers

### construction_tick_system
- Runs every frame, ticks `ConstructionProgress` ECS component countdown on newly placed buildings
- All runtime-placed buildings (player + AI) start with `ConstructionProgress(BUILDING_CONSTRUCT_SECS)` (10s at 1x speed), `SpawnerState { respawn_timer: -1.0 }` (dormant), and `Health(0.01)`
- Each frame: `ConstructionProgress.0 -= game_time.delta()`, scales ECS Health proportionally (`progress * def.hp`), sends `SetHealth` GPU update
- On completion (`<= 0.0`): sets `ConstructionProgress(0.0)`, arms spawner (`SpawnerState.respawn_timer = 0.0`), sets full HP
- World-gen buildings skip construction (`ConstructionProgress(0.0)`)
- Growth system skips buildings with `ConstructionProgress.0 > 0.0`
- Rendering: `build_building_body_instances` overrides health field with construction progress fraction (0.0→0.999) so shader clips sprite bottom-to-top
- Save/load: construction progress persisted in `SpawnerSave` and `FarmGrowthSave`

### growth_system (unified farms + mines)
- Runs every frame, advances growth based on elapsed game time
- Queries ECS `(&GpuSlot, &Building, &TownId, &Position, &ConstructionProgress, &mut ProductionState)` — iterates only buildings with these components
- Skips tombstoned entries (`position.x < -9000`) — destroyed farms/mines don't regrow
- **ECS component**: `ProductionState { ready: bool, progress: f32 }` on each Farm/Mine entity
- **Hybrid growth model**:
  - Passive: `FARM_BASE_GROWTH_RATE` (0.08/hour) — ~12 game hours to full growth
  - Tended: `FARM_TENDED_GROWTH_RATE` (0.25/hour) — ~4 game hours with farmer working
- Farm transitions to `Ready` when progress >= 1.0
- Checks `entity_map.occupant_count(slot) >= 1` (slot-indexed occupancy on EntityMap) to determine if a farmer is tending
- **FarmYield upgrade**: growth rate multiplied by `1.0 + level * 0.15` per-town (precomputed per-town `farm_mults` Vec, indexed by `town_id.0`)

### raider_forage_system
- Runs when `game_time.hour_ticked` is true
- Each raider town (faction != FACTION_PLAYER and != FACTION_NEUTRAL) gains `RAIDER_FORAGE_RATE` (1) food per hour
- Passive income ensures raiders can survive even if they never steal

### spawner_respawn_system
- Runs when `game_time.hour_ticked` is true
- Iterates `EntityMap.spawner_slots()` pre-built index (maintained on add/remove_instance) instead of scanning all buildings. Spawner state lives in `SpawnerState` ECS component (`npc_slot: Option<usize>`, `respawn_timer: f32`), queried via `Query<(&mut SpawnerState, Option<&MinerHomeConfig>)>`.
- Sentinel values: `npc_slot = None` (no NPC alive), `respawn_timer = -1.0` (not respawning), `>= 0.0` (countdown active)
- If `npc_slot.is_some()` and NPC is dead (slot not in EntityMap): clears `npc_slot`, starts 12h respawn timer
- Timer decrements 1.0 per game hour; on expiry: allocates slot via `SlotAllocator`, emits `SpawnNpcMsg`, logs to `CombatLog`
- All spawner buildings (world gen and player-built) start with `SpawnerState { npc_slot: None, respawn_timer: 0.0 }` — the system spawns the first NPC on the next hourly tick. No separate initial spawn function.
- Tombstoned entries (position.x < -9000) are skipped (building was destroyed)
- Spawn mapping resolved by `world::resolve_spawner_npc()` (single source of truth, takes `&BuildingInstance`): FarmerHome → Farmer (nearest farm via `find_nearest_free` with kind-filtered spatial search as hint, no claim at spawn — farmer self-claims via behavior system), ArcherHome → Archer (nearest waypoint via `find_location_within_radius`), FighterHome → Fighter (nearest waypoint via `find_location_within_radius`), Tent → Raider (home = tent position), MinerHome → Miner (assigned mine from `MinerHomeConfig.assigned_mine` if set, otherwise nearest gold mine via `find_nearest_free`), MasonHome → Mason (home = mason home position, no initial worksite — mason self-selects damaged buildings via decision_system). All types look up faction from `world_data.towns[town_idx].faction`. Note: spawner_respawn_system does **not** pre-claim work slots — farmers self-claim via `find_farmer_farm_target()` in decision_system.

### starvation_system
- Query-first: `(&GpuSlot, &Energy, &CachedStats, &mut NpcFlags, &mut Health)` with `Without<Building>, Without<Dead>` — no `EntityMap` dependency
- Runs when `game_time.hour_ticked` is true
- NPCs with `energy <= 0` get `starving` flag set on `NpcFlags`
- Starving NPCs: speed set to `CachedStats.speed * STARVING_SPEED_MULT` (0.5) via `GpuUpdate::SetSpeed`
- **HP cap**: always clamps HP to `max_health * STARVING_HP_CAP` (50%) for all starving NPCs (handles both transition and save/load edge cases) via `GpuUpdate::SetHealth`
- When energy rises above 0 (eating or resting): `starving` is cleared, speed restored to `CachedStats.speed`

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

**Farm harvest** uses `ProductionState::harvest()` (single source of truth for Ready → Growing transition):
- `harvest(&mut self, combat_log, game_time, faction) -> i32` — resets `ready` to false, `progress` to 0.0, returns yield (farm=1 food, mine=MINE_EXTRACT_PER_CYCLE gold), logs to CombatLog. Returns 0 if not ready.
- Callers look up the building entity via `entity_map.entities.get(&slot)` and query `&mut ProductionState`.
- All callers enter `ActivityKind::ReturnLoot` and carry yield home — delivery happens via `arrival_system` proximity check. No caller instant-credits storage.
- Called from 5 sites: arrival_system (working farmer harvest), decision_system (farmer Work arrival), decision_system (raider steal), decision_system (miner Mine arrival), decision_system (Mine at_dest harvest)

**Farmer harvest** (visible food transport):
- Working farmer at farm: `decision_system` detects Ready farm at arrival (`at_destination`), calls `harvest()`, sends `WorkIntent::Release` (resolver releases occupancy + clears `NpcWorkState.worksite`), enters `ReturnLoot` targeting home. Farmer visibly carries food sprite home.
- Work arrival: checks worksite slot, sends `WorkIntent::Retarget` if occupied by another. If Ready, `harvest()` + `WorkIntent::Release` + `ReturnLoot`. If not Ready, stays `Work` (tending at worksite).
- On delivery at home: farmer goes `Idle` — decision system re-evaluates best target (may pick a different farm if one is Ready). Dynamic work→carry→deliver→re-evaluate cycle.

**Raider steal** (decision_system, Raid arrival):
- Uses `find_location_within_radius()` to find farm within FARM_ARRIVAL_RADIUS
- Only steals if farm is Ready — `harvest()` resets farm, enters `ReturnLoot`
- If farm not ready: find a different farm (excludes current position, skips tombstoned); if no other farm found, return home
- Logs "Stole food → ReturnLoot" vs "Farm not ready, seeking another" vs "No other farms, returning"

**Farm destruction**: Building removal from `EntityMap` handles cleanup. Tombstoned position (x < -9000) causes render pipeline to skip the crop sprite and `growth_system` to skip growth.

**Visual feedback**: `farm_visual_system` queries `(&GpuSlot, &ProductionState)` with `With<Building>` to watch for state transitions and spawns/despawns `FarmReadyMarker` entities (keyed by `farm_slot: usize` — building slot). Uses `Local<HashMap<usize, Entity>>` for O(1) farm-slot → marker lookup, validates the mapped marker still exists, and prunes stale entries before respawning. Cadenced (see [performance.md](performance.md#fixed-cadence-systems)). `!ready → ready` spawns a marker; `ready → !ready` (harvest) despawns it.

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
Starving marker cleared
Speed restored to CachedStats.speed
```

**Recovery paths:**
- **Eat**: consumes 1 food from town storage, instantly restores energy to 100. No travel required.
- **Rest**: walk home to spawner building (FarmerHome/ArcherHome), recover energy slowly (6 hours 0→100). Works even when starving.

**Constants:**
- `STARVING_HP_CAP`: 0.5 (50% of MaxHealth)
- `STARVING_SPEED_MULT`: 0.5 (50% of normal speed)

Starvation applies to **both villagers and raiders**. If raiders can't steal food and their town runs out, they'll starve and become easier to kill.

The HP cap is enforced by `starvation_system` (immediate clamp on every hourly tick) and `healing_system` (healing zone caps at 50% for starving NPCs). No other system can raise HP outside healing zones.

## Raider Attack System

Raiders use the AI squad commander's wave-based attack cycle (see [ai-player.md](ai-player.md#wave-based-attack-cycle)). Each raider town gets one squad containing all raiders. The `ai_squad_commander_system` picks the nearest enemy farm as target, gathers raiders until `wave_min_start` (RAID_GROUP_SIZE) are ready, then dispatches the wave. Wave ends when the target is destroyed or losses exceed `wave_retreat_below_pct` (30%).

Raiders without a squad assignment wander near their town. Group attacks use squad-driven waves.

**Constants:**
- `RAID_GROUP_SIZE`: 3 (minimum raiders to start a wave)

## Resources

| Resource | Purpose | Updated By |
|----------|---------|------------|
| GameTime | total_seconds, time_scale, paused, hour_ticked | game_time_system |
| FoodStore | ECS component `i32` per town entity — via `TownAccess.food()` | harvest, steal, forage, respawn |
| Dirty/resource signals | Message types + resources (see [messages.md](messages.md)) | Message drain + consumer systems |
| ProductionState | ECS component `{ ready: bool, progress: f32 }` on Farm/Mine entities | growth_system, harvest/steal |
| GoldStore | ECS component `i32` per town entity — via `TownAccess.gold()` | mining delivery, UI |
| MineStates | gold, max_gold, positions per mine | mine_regen_system, mining behavior |
| MinerProgressRender | positions + progress for active miners | sync_miner_progress_render → render world (ExtractResource) |
| EntityMap (occupancy) | `EntityMap.occupancy: DenseSlotMap<i16>` — slot-indexed claim/release/is_occupied/occupant_count methods | decision_system, death_cleanup |
| MiningPolicy | discovered_mines per town, mine_enabled per mine | mining_policy_system (dirty-flag gated) |
| RaiderState | max_pop, respawn_timers, forage_timers | raider_forage_system |
| SpawnerState | ECS component `{ npc_slot: Option<usize>, respawn_timer: f32 }` on spawner buildings | spawner_respawn_system, place_building |
| ConstructionProgress | ECS component `(f32)` seconds remaining on building entities | construction_tick_system, growth_system (skip guard) |
| PopulationStats | alive/working/dead per (job, town) | spawn, death, state transitions |

## Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| BUILDING_CONSTRUCT_SECS | 10.0 | Seconds (at 1x) for building construction |
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
- Only regenerates when mine has no occupant (`entity_map.occupant_count(slot) == 0` via slot-indexed occupancy)
- Rate: `MINE_REGEN_RATE` (2.0 gold/hour), capped at `MINE_MAX_GOLD` (200.0) per mine
- Uses `MineStates` resource — parallel Vecs of gold, max_gold, and positions per mine

### mining_policy_system
- Gated by `MessageReader<MiningDirtyMsg>` — only runs when mining topology/policy changes (radius slider, mine toggle, miner spawn/death)
- **Discovery**: scans `EntityMap.iter_kind(GoldMine)` within `PolicySet.mining_radius` of each faction-0 town center, populates `MiningPolicy.discovered_mines[town_idx]`
- **Distribution**: collects alive auto-assigned miners per town (skips `MinerHomeConfig.manual_mine == true`), round-robin assigns across enabled discovered mines via `MinerHomeConfig.assigned_mine`
- **Stale clearing**: if assigned mine falls outside radius or disabled, clears `assigned_mine` on auto-assigned miner homes
- **mine_enabled**: keyed by GPU slot (`HashMap<usize, bool>`) instead of sequential index, decoupled from WorldData ordering
- `MAX_MINE_OCCUPANCY` in `constants.rs` limits concurrent miners per mine; behavior system (`decision_system`) skips full mines

### squad_cleanup_system
- Message-gated via `MessageReader<SquadsDirtyMsg>` — skips entirely when no squad-relevant changes occurred
- Signal emitted by: `death_system` (any death), `spawn_npc_system` (archer spawn), left_panel UI (assign/dismiss), save load (`emit_all()`)
- **Phase 1**: retains only members whose slot is still in `EntityMap` (alive)
- **Phase 2**: keeps Default Squad (index 0) as live pool of unsquadded player military NPCs (query-first via `(&GpuSlot, &Job, &TownId, Option<&SquadId>)` with `Without<Building>, Without<Dead>`, inserts `SquadId(0)`)
- **Phase 3**: if `target_size > 0` and `members.len() > target_size`, dismisses excess (removes `SquadId` + `DirectControl` components, pops from members)
- **Phase 4**: if `target_size > 0` and `members.len() < target_size`, auto-recruits unsquadded player-faction military NPCs from the same query (inserts `SquadId`, pushes to members). Pool is shared across squads — earlier squad indices get priority.

## Dynamic Raider Town Migration

New raider towns spawn organically as the player grows. Three systems in `economy.rs` handle the lifecycle:

```
migration_spawn_system (hourly check)
        │
        ▼ check_timer >= RAIDER_SPAWN_CHECK_HOURS (12h)?
        │ player_alive >= VILLAGERS_PER_RAIDER * (raider_count + 1)? (alive towns only)
        │ no active migration? alive raider_towns < MAX_RAIDER_TOWNS (20)?
        │
        ▼ YES: spawn group at nearest map edge to settle target
        │
        ├─ Create Town entry (faction = next faction index, sprite_type = 1)
        ├─ Create TownGrid (with world-edge caps via recompute_world_caps), extend all per-town resources (food, gold, factions, raider_state, policies, faction_list)
        ├─ Create inactive AiPlayer (active: false, kind: Raider, random personality)
        ├─ Spawn N raiders via SpawnNpcMsg with Home = player town center
        │   Group size: MIGRATION_BASE_SIZE (3) + player_alive / difficulty.migration_scaling()
        │   (Easy=6, Normal=4, Hard=2), capped at 20
        ├─ pick_settle_site() selects target position (farthest from existing towns)
        ├─ Spawn boat as proper NPC entity via SpawnNpcMsg (Job::Boat = 6, ATLAS_BOAT sprite)
        └─ Store MigrationGroup in MigrationState resource
           Combat log: "A raider band approaches from the {direction}!"

migration_attach_system (after Step::Spawn, before Step::Combat)
        │
        ▼ if migration active: attach Migrating component to spawned entities
           (bridges 1-frame gap between SpawnNpcMsg and entity creation)

migration_settle_system (every frame, early-returns if no active migration)
        │
        ▼ BOAT phase: sail boat toward settle_target at BOAT_SPEED
        │ when boat reaches land → disembark NPCs (SpawnNpcMsg), despawn boat entity
        │ (entity despawn + unregister_npc + free slot)
        │
        ▼ ATTACH phase: insert Migrating component on spawned entities
        │ (bridges 1-frame gap between SpawnNpcMsg and entity creation)
        │
        ▼ SETTLE phase: read GPU positions for group members
        │ compute average position
        │
        ▼ within RAIDER_SETTLE_RADIUS (500px) of settle_target?
        │
        ▼ count == 0 (all dead)? → queue replacement, clear migration
        │
        ▼ YES: settle town
        ├─ Town center = settle_target (verified land cell, not NPC centroid)
        ├─ SpawnerState ECS component set on each tent entity
        ├─ stamp_dirt() around town
        ├─ Activate AiPlayer (active = true)
        ├─ Remove Migrating from members, update Home to settle_target
        ├─ Emit TerrainDirtyMsg + BuildingGridDirtyMsg for tilemap rebuild
        └─ Clear MigrationState.active
           Combat log: "A raider band has settled nearby!"

        ▼ WIPEOUT: count == 0 AND found > 0 (all spawned NPCs dead)
        ├─ Queue replacement PendingAiSpawn (same strength/resources, 4h delay)
        ├─ Combat log: "The migrating {kind} was wiped out!"
        └─ Clear MigrationState.active (unblock pipeline for next migration)
        │
        ▼ NOT SPAWNED YET: count == 0 AND found == 0 (SpawnNpcMsg not processed)
        └─ Early return — wait for spawn_npc_system to process messages next frame
```

**Movement**: Migration group spawns a boat as a proper NPC entity (`Job::Boat`) via `SpawnNpcMsg` at the map edge nearest to the settle target. The boat uses `ATLAS_BOAT` sprite (set via `NpcDef.atlas` field). The boat sails toward `settle_target` at `BOAT_SPEED` (150px/s) with CPU-driven position updates. When the boat reaches land (non-water terrain), NPCs disembark — spawned at the boat position with `Migrating` component. Boat entity is properly cleaned up: ECS entity despawned, `entity_map.unregister_npc()`, GPU slot freed. NPCs then walk toward `settle_target` using the existing `Home` component + `Action::Wander` behavior.

**Settlement site selection**: `pick_settle_site()` samples 100 random land positions and picks the one farthest from all existing towns — ensures new settlements spread across the map rather than clustering. The verified `settle_target` position is used for all placement (town center, buildings, dirt stamp, NPC homes, combat log) — never the NPC centroid `avg_pos` (which could be over water).

**Migration wipeout**: If all spawned NPCs in the group die before settling (`count == 0` and `found > 0` — distinguishes "all dead" from "not spawned yet"), the migration is cleared and a replacement `PendingAiSpawn` is queued with `ENDLESS_RESPAWN_DELAY_HOURS` (4h) delay. When `found == 0` (SpawnNpcMsg not yet processed by `spawn_npc_system`), the system waits rather than declaring wipeout. The replacement inherits the original group's `upgrade_levels`, `starting_food`, and `starting_gold`. This ensures the target number of AI towns is eventually reached — it's endless.

**Save/load**: `MigrationState` serialized as `Option<MigrationSave>` in `SaveData`. On load, `Migrating` component re-attached to saved member slot entities.

**AiPlayer.active**: New `bool` field. `ai_decision_system` skips inactive players. All existing AiPlayer creation sites set `active: true`. Migration creates with `active: false`, activated on settlement.

### Migration Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| RAIDER_SPAWN_CHECK_HOURS | 12.0 | Game hours between migration trigger checks |
| MAX_RAIDER_TOWNS | 20 | Maximum number of dynamic raider towns |
| RAIDER_SETTLE_RADIUS | 500.0px | Distance to settle_target that triggers settlement |
| BOAT_SPEED | 150.0 | Boat movement speed (px/s) |
| MIGRATION_BASE_SIZE | 3 | Base raiders per migration group |
| VILLAGERS_PER_RAIDER | 20 | Player alive NPCs per raider town threshold |
| ENDLESS_RESPAWN_DELAY_HOURS | 4.0 | Delay before replacement migration after wipeout or town defeat |

## Known Issues

- (none currently)
