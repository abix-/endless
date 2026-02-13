# Game State Reference

## Overview

All game state lives in Bevy Resources — singleton structs accessible by any system via `Res<T>` (read) or `ResMut<T>` (write). There is no external API surface. Systems communicate through [messages](messages.md) and shared resources.

Defined in: `rust/src/resources.rs`, `rust/src/world.rs`

## NPC Identity

| Resource | Type | Writers | Readers |
|----------|------|---------|---------|
| NpcEntityMap | `HashMap<usize, Entity>` | spawn_npc_system, death_cleanup_system | damage_system (slot → entity lookup) |
| SlotAllocator | `{ next, free: Vec }` | spawn_npc_system (alloc), death_cleanup_system (free) | GPU compute dispatch, UI, tests |

`SlotAllocator` uses LIFO free list — most recently freed slot is reused first. `next` is the high-water mark. Two query methods: `count()` returns high-water mark (for GPU dispatch bounds), `alive()` returns `next - free.len()` (for UI display). Single source of truth for all NPC counting. See [spawn.md](spawn.md).

## NPC UI Caches

Pre-computed per-NPC data for UI queries, indexed by slot.

| Resource | Per-NPC Data | Writers | Readers |
|----------|-------------|---------|---------|
| NpcMetaCache | name, level, xp, trait_id, town_id, job | spawn_npc_system, xp_grant_system | UI queries |
| NpcLogCache | `VecDeque<NpcLogEntry>` (100 cap, circular) | behavior/decision systems | UI queries |
| NpcsByTownCache | `Vec<Vec<usize>>` — NPC slots grouped by town | spawn/death systems | UI queries |

`NpcLogCache.push(idx, day, hour, minute, message)` adds timestamped entries. Oldest evicted at capacity.

NPC state is derived at query time via `derive_npc_state()` which checks ECS components (Dead, InCombat, Resting, etc.), not cached.

## Population & Kill Stats

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| PopulationStats | `HashMap<(job, town), PopStats>` — alive, working, dead | spawn/death/state systems | UI |
| KillStats | guard_kills, villager_kills | death_cleanup_system | UI |
| FactionStats | `Vec<FactionStat>` — alive, dead, kills per faction | spawn/death systems | UI |

`FactionStats` — one entry per settlement (player towns + AI towns + raider camps). Methods: `inc_alive()`, `dec_alive()`, `inc_dead()`, `inc_kills()`.

## World Layout

Static world data, immutable after initialization.

| Resource | Data | Purpose |
|----------|------|---------|
| WorldData | towns, farms, beds, guard_posts, huts, barracks, tents | All building positions and metadata |
| SpawnerState | `Vec<SpawnerEntry>` — one per Hut/Barracks/Tent | Building→NPC links + respawn timers |
| BuildingOccupancy | private `HashMap<(i32,i32), i32>` — position → worker count | Building assignment (claim/release/is_occupied/count/clear) |
| FarmStates | `Vec<FarmGrowthState>` + `Vec<f32>` progress | Per-farm growth tracking |
| TownGrids | `Vec<TownGrid>` — one per town (villager + camp) | Per-town building slot unlock tracking |

### WorldData Structs

| Struct | Fields |
|--------|--------|
| Town | name, center (Vec2), faction, sprite_type (0=fountain, 1=tent) |
| Farm | position (Vec2), town_idx |
| Bed | position (Vec2), town_idx |
| GuardPost | position (Vec2), town_idx, patrol_order |
| Hut | position (Vec2), town_idx |
| Barracks | position (Vec2), town_idx |
| Tent | position (Vec2), town_idx |

Helper functions: `find_nearest_location()`, `find_location_within_radius()`, `find_nearest_free()` (generic via `Worksite` trait), `find_within_radius()`, `find_by_pos()`.

### World Grid

250x250 cell grid covering the entire 8000x8000 world (32px per cell). Each cell has a terrain biome and optional building.

| Resource | Data | Purpose |
|----------|------|---------|
| WorldGrid | `Vec<WorldCell>` (width × height), cell_size | World-wide terrain + building grid |
| WorldGenConfig | world dimensions, num_towns, spacing, per-town NPC counts | Procedural generation parameters |

**WorldCell** fields: `terrain: Biome` (Grass/Forest/Water/Rock/Dirt), `building: Option<Building>`.

**Building** variants: `Fountain { town_idx }`, `Farm { town_idx }`, `Bed { town_idx }`, `GuardPost { town_idx, patrol_order }`, `Camp { town_idx }`, `Hut { town_idx }`, `Barracks { town_idx }`, `Tent { town_idx }`.

**WorldGrid** helpers: `cell(col, row)`, `cell_mut(col, row)`, `world_to_grid(pos) -> (col, row)`, `grid_to_world(col, row) -> Vec2`.

**WorldGenConfig** defaults: 8000x8000 world, 400px margin, 2 towns, 1200px min distance, 32px grid spacing, 3500px camp distance, 2 farmers / 2 guards / 0 raiders per town (testing defaults).

**`generate_world()`**: Takes config and populates WorldGrid, WorldData, and TownGrids. Places towns randomly with min distance constraint, finds camp positions furthest from all towns (16 directions), assigns terrain via simplex noise with Dirt override near settlements. Villager towns get 1 fountain, 2 farms, N Huts + N Barracks (spiral-placed), then 4 guard posts on the outer ring. Raider camps get a Camp center + N Tents (spiral-placed from slider). Both town types get a TownGrid with expandable building slots. Building positions are generated via `spiral_slots()` — a spiral outward from center that skips occupied cells. Guard posts are placed after spawner buildings so they're always on the perimeter.

### Town Building Grid

Per-town slot tracking for the building system. Each town (villager and raider camp) has a `TownGrid` with a `HashSet<(i32, i32)>` of unlocked (row, col) slots and a `town_data_idx` linking to its `WorldData.towns` entry. Initial base grid is 6x6 (rows/cols -2 to +3), expandable to 100x100 by unlocking adjacent slots.

| Struct | Fields |
|--------|--------|
| TownGrid | town_data_idx: usize, unlocked: `HashSet<(i32, i32)>` |
| TownGrids | grids: `Vec<TownGrid>` (one per town — villager + camp) |
| TownSlotInfo | grid_idx, town_data_idx, row, col, slot_state |
| SlotState | Unlocked, Locked |
| BuildMenuContext | grid_idx, town_data_idx, slot, slot_world_pos, is_locked, has_building, is_fountain |

Coordinate helpers: `town_grid_to_world(center, row, col)`, `world_to_town_grid(center, world_pos)`, `get_adjacent_locked_slots(grid)`, `find_town_slot(world_pos, towns, grids)`.

Building placement: `place_building()` validates cell empty, places on WorldGrid, pushes to WorldData + FarmStates. `remove_building()` tombstones position to (-99999, -99999) in WorldData, clears grid cell. Tombstone deletion preserves parallel Vec indices (FarmStates). Fountains and camps cannot be removed.

Building costs (from constants.rs): Farm=1, GuardPost=1, Hut=1, Barracks=1, Tent=1, SlotUnlock=1 food.

## Food & Economy

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| FoodStorage | `Vec<i32>` — food count per town/camp | economy systems (arrival, eating) | economy systems, UI |
| FoodEvents | delivered: `Vec<FoodDelivered>`, consumed: `Vec<FoodConsumed>` | behavior systems | UI (poll and drain) |

`FoodStorage.init(count)` initializes per-town counters. Villager towns and raider camps share the same indexing.

## Raider Camps

| Resource | Data | Purpose |
|----------|------|---------|
| CampState | max_pop, respawn_timers, forage_timers per camp | Camp respawn/forage scheduling |
| RaidQueue | `HashMap<faction, Vec<(Entity, slot)>>` | Raiders waiting to form raid group |

`CampState::faction_to_camp(faction)` maps faction ID to camp index (faction 1 = camp 0).

## Game Time

| Resource | Fields | Default |
|----------|--------|---------|
| GameTime | total_seconds, seconds_per_hour, start_hour, time_scale, paused, last_hour, hour_ticked | 0.0s, 5.0s/hr, 6am, 1.0x, false |

Derived methods: `day()`, `hour()`, `minute()`, `is_daytime()` (6am–8pm), `total_hours()`.

`hour_ticked` is true for one frame when the game hour changes — used by economy/respawn systems.

## Game Config

| Resource | Fields | Default |
|----------|--------|---------|
| GameConfig | farmers_per_town, guards_per_town, raiders_per_camp, spawn_interval_hours, food_per_work_hour | 10, 30, 15, 4, 1 |

Pushed via `GAME_CONFIG_STAGING` static. Drained by `drain_game_config` system.

## GPU State

| Resource | Data | Status |
|----------|------|--------|
| GpuReadState | positions, combat_targets, health, factions, npc_count | Populated via staging buffer readback each frame |
| NpcSpriteTexture | handle (char atlas), world_handle (world atlas) | Shared with instanced renderer for dual atlas bind group |
| ProjSlotAllocator | next, free list, max (50,000) | Active — allocates projectile slots |

`GpuReadState` is populated each frame by staging buffer readback. Used by combat systems, position sync, and test assertions.

## Stats & Upgrades

| Resource | Data | Defined In | Purpose |
|----------|------|------------|---------|
| CombatConfig | `HashMap<Job, JobStats>` + `HashMap<BaseAttackType, AttackTypeStats>` + heal_rate + heal_radius | `systems/stats.rs` | All NPC base stats — resolved via `resolve_combat_stats()` |
| TownUpgrades | `Vec<[u8; 12]>` per town | `systems/stats.rs` | Per-town upgrade levels, indexed by `UpgradeType` enum |
| UpgradeQueue | `Vec<(usize, usize)>` — (town_idx, upgrade_index) | `systems/stats.rs` | Pending upgrade purchases from UI, drained by `process_upgrades_system` |
| AutoUpgrade | `Vec<[bool; 12]>` per town | `resources.rs` | Per-upgrade auto-buy flags; `auto_upgrade_system` queues affordable upgrades each game hour |

`CombatConfig::default()` initializes from hardcoded values (guard/raider damage=15, speeds=100, max_health=100, heal_rate=5, heal_radius=150). `resolve_combat_stats()` combines job base × upgrade mult × trait mult × level mult → `CachedStats` component.

`TownUpgrades` is indexed by town, each entry is a fixed-size array of 12 upgrade levels (`UpgradeType` enum: GuardHealth, GuardAttack, GuardRange, GuardSize, AttackSpeed, MoveSpeed, AlertRadius, FarmYield, FarmerHp, HealingRate, FoodEfficiency, FountainRadius). `UpgradeQueue` decouples the UI from stat re-resolution — `left_panel.rs` pushes `(town, upgrade)` tuples, `process_upgrades_system` validates food cost (`10 * 2^level`), increments level, deducts food, and re-resolves `CachedStats` for affected NPCs. `auto_upgrade_system` runs once per game hour before `process_upgrades_system`, queuing any auto-enabled upgrades the town can afford.

**Upgrade percentages** (`UPGRADE_PCT` array in `systems/stats.rs`):

| Index | Upgrade | % per level | Type |
|-------|---------|-------------|------|
| 0 | GuardHealth | +10% | Multiplicative |
| 1 | GuardAttack | +10% | Multiplicative |
| 2 | GuardRange | +5% | Multiplicative |
| 3 | GuardSize | +5% | Multiplicative |
| 4 | AttackSpeed | -8% cooldown | Reciprocal: `1/(1+level*0.08)` |
| 5 | MoveSpeed | +5% | Multiplicative |
| 6 | AlertRadius | +10% | Multiplicative |
| 7 | FarmYield | +15% | Multiplicative |
| 8 | FarmerHp | +20% | Multiplicative |
| 9 | HealingRate | +20% | Multiplicative |
| 10 | FoodEfficiency | +10% | Multiplicative |
| 11 | FountainRadius | +24px flat | Flat: `base_radius + level * 24.0` |

**Upgrade applicability by job** — not all upgrades flow through `resolve_combat_stats()`:

| Upgrade | Applies to | Notes |
|---------|-----------|-------|
| GuardHealth, GuardAttack, GuardRange, GuardSize, AlertRadius | Guard only | Combat resolver |
| AttackSpeed, MoveSpeed | All combatants (Guard, Raider, Fighter) | Combat resolver |
| FarmerHp | Farmer only | Combat resolver |
| FarmYield | `farm_growth_system` reads directly | Not combat resolver |
| HealingRate, FountainRadius | `healing_system` reads directly | Not combat resolver |
| FoodEfficiency | `decision_system` eat logic | Not combat resolver |

## Town Policies

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| TownPolicies | `Vec<PolicySet>` — per-town behavior configuration (16 slots default) | left_panel (UI) | decision_system, behavior systems |

`PolicySet` fields: `eat_food` (bool), `guard_aggressive` (bool), `guard_leash` (bool), `farmer_fight_back` (bool), `prioritize_healing` (bool), `farmer_flee_hp` (f32, 0.0-1.0), `guard_flee_hp` (f32), `recovery_hp` (f32), `farmer_schedule` (WorkSchedule enum), `guard_schedule` (WorkSchedule enum), `farmer_off_duty` (OffDutyBehavior enum), `guard_off_duty` (OffDutyBehavior enum).

`WorkSchedule`: Both (default), DayOnly, NightOnly. `OffDutyBehavior`: GoToBed (default), StayAtFountain, WanderTown.

Defaults: eat_food=true, guard_aggressive=false, guard_leash=true, farmer_fight_back=false, prioritize_healing=true, farmer_flee_hp=0.30, guard_flee_hp=0.15, recovery_hp=0.80.

Replaces per-entity `FleeThreshold`/`WoundedThreshold` components for standard NPCs. Raiders use hardcoded flee threshold (0.50). Per-entity overrides still possible via `FleeThreshold` component (e.g., boss NPCs).

`PolicySet`, `WorkSchedule`, and `OffDutyBehavior` all derive `serde::Serialize + Deserialize`. Settings path: `Documents\Endless\settings.json`.

## Selection

| Resource | Data | Purpose |
|----------|------|---------|
| SelectedNpc | `i32` (-1 = none) | Currently selected NPC for inspector panel |
| SelectedBuilding | `{ col, row, active }` (default inactive) | Currently selected building grid cell for building inspector |
| FollowSelected | `bool` (default false) | When true, camera tracks selected NPC position each frame |

## Test Framework

| Resource | Data | Purpose |
|----------|------|---------|
| AppState | TestMenu \| Running | Gates game systems; menu vs active test |
| TestState | test_name, phase, total_phases, phase_name, results, counters, flags | Shared state for active test |
| TestRegistry | `Vec<TestEntry>` (name, description, phase_count, time_scale) | All registered tests |
| RunAllState | active, queue, results | Sequential test execution state |

`TestState` is reset between tests via `cleanup_test_world` (OnExit Running). `test_is("name")` run condition gates per-test systems.

## UI State

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| UiState | build_menu_open, pause_menu_open, left_panel_open, left_panel_tab (LeftPanelTab enum) | ui_toggle_system (keyboard), top_bar (buttons), left_panel tabs, pause_menu | All panel systems |
| CombatLog | `VecDeque<CombatLogEntry>` (max 200) | death_cleanup, spawn_npc, decision_system, arrival_system, build_menu_system | bottom_panel_system |
| BuildMenuContext | grid_idx, town_data_idx, slot, slot_world_pos, screen_pos, is_locked, has_building, is_fountain | slot_right_click_system | build_menu_system |
| UpgradeQueue | `Vec<(usize, usize)>` — (town_idx, upgrade_index) | left_panel upgrades (UI), auto_upgrade_system | process_upgrades_system |
| GuardPostState | timers: `Vec<f32>`, attack_enabled: `Vec<bool>` | guard_post_attack_system (auto-sync length), build_menu (toggle) | guard_post_attack_system |
| SpawnerState | `Vec<SpawnerEntry>` — building_kind (0=Hut, 1=Barracks, 2=Tent), town_idx, position, npc_slot, respawn_timer | game_startup, build_menu (push on build), spawner_respawn_system | spawner_respawn_system, game_hud (counts) |
| UserSettings | world_size, towns, farmers, guards, raiders, ai_towns, raider_camps, ai_interval, scroll_speed, log_kills/spawns/raids/harvests/levelups/npc_activity/ai, debug_enemy_info/coordinates/all_npcs, policy (PolicySet) | main_menu (save on Play), bottom_panel (save on filter change), right_panel (save policies on tab leave), pause_menu (save on close) | main_menu (load on init), bottom_panel (load on init), game_startup (load policies), pause_menu settings, camera_pan_system. **Loaded from disk at app startup** via `insert_resource(load_settings())` in `build_app()` — persists across app restarts without waiting for UI init. |

`UiState` tracks which panels are open. All default to false. `LeftPanelTab` enum: Roster (default), Upgrades, Policies, Patrols, Squads. `toggle_left_tab()` method: if panel shows that tab → close, otherwise open to that tab. Reset on game cleanup.

`CombatLog` is a ring buffer of global events with 6 kinds: Kill, Spawn, Raid, Harvest, LevelUp, Ai. Each entry has day/hour/minute timestamps and a message string. `push()` evicts oldest when at capacity. AI entries (purple in HUD) log build/unlock/upgrade actions.

`PolicySet` is serializable (`serde::Serialize + Deserialize`) and persisted as part of `UserSettings`. Loaded into `TownPolicies` on game startup, saved when leaving the Policies tab in the left panel.

## Squads

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| SquadState | `squads: Vec<Squad>` (10), `selected: i32`, `placing_target: bool` | left_panel (recruit/dismiss), click_to_select (target placement), game_escape (cancel placement) | decision_system, squad_overlay_system, squad_cleanup_system |

`Squad` fields: `members: Vec<usize>` (NPC slot indices), `target: Option<Vec2>` (world position or None).

`SquadId(i32)` component (0-9) added to guards when recruited into a squad. Removed on dismiss. Guards with `SquadId` walk to squad target instead of patrolling (see [behavior.md](behavior.md#squads)).

`placing_target`: when true, next left-click on the map sets the selected squad's target. Cancelled by ESC or right-click.

## AI Players

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| AiPlayerConfig | `decision_interval: f32` (real seconds between AI ticks, default 5.0) | main_menu (from settings) | ai_decision_system |
| AiPlayerState | `players: Vec<AiPlayer>` — one per non-player settlement | game_startup (populate), game_cleanup (reset) | ai_decision_system |

`AiPlayer` fields: `town_data_idx` (WorldData.towns index), `grid_idx` (TownGrids index), `kind` (Builder or Raider), `personality` (Aggressive, Balanced, or Economic — randomly assigned at game start). `AiKind` determined by `Town.sprite_type`: 0 (fountain) = Builder, 1 (tent) = Raider.

Personality drives build order, upgrade priority, food reserve, and town policies:
- **Aggressive**: military first (barracks → guard posts → economy), zero food reserve, combat upgrades prioritized
- **Balanced**: economy and military in tandem (farm → house → barracks → guard post), 10 food reserve
- **Economic**: farms first with minimal military, 30 food reserve, FarmYield/FarmerHp upgrades prioritized

Slot selection: economy buildings (farms, houses, barracks) prefer inner slots (closest to center). Guard posts prefer outer slots (farthest from center) with minimum Manhattan distance of 5 between posts. Raider tents cluster around camp center (inner slots).

Both unlock slots when full (sets terrain to Dirt) and buy upgrades with surplus food. Combat log shows personality tag: `"Town [Balanced] built farm"`.

## Debug Resources

| Resource | Key Fields | Updated By |
|----------|-----------|------------|
| CombatDebug | attackers_queried, targets_found, attacks_made, chases_started, sample positions/distances | cooldown/attack systems |
| HealthDebug | damage_processed, deaths, despawned, entity_count, healing stats | damage/death systems |
| SystemTimings | per-system EMA-smoothed ms (internal Mutex, `Res` not `ResMut`), frame_ms, enabled toggle (F5) | all systems via `timings.scope("name")` RAII guard |

## Control Resources

| Resource | Data | Purpose |
|----------|------|---------|
| ResetFlag | `bool` | When true, `reset_bevy_system` clears all state |
| DeltaTime | `f32` | Frame delta in seconds |
| RespawnTimers | `HashMap<clan_id, hours>` | Per-clan respawn cooldowns |

## Known Issues

- **Health dual ownership**: CPU-authoritative but synced to GPU via messages. Could diverge if sync fails.
- **No external API**: All state is internal Bevy Resources. No query interface for external tools or UI frameworks.

## Rating: 8/10

Resources are well-organized by domain with clear ownership. UI caches avoid repeated ECS queries. SlotAllocator is single source of truth for NPC counting — `count()` for GPU dispatch, `alive()` for UI. No redundant count resources.
