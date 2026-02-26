# Game State Reference

## Overview

All game state lives in Bevy Resources — singleton structs accessible by any system via `Res<T>` (read) or `ResMut<T>` (write). There is no external API surface. Systems communicate through [messages](messages.md) and shared resources.

Defined in: `rust/src/resources.rs`, `rust/src/world.rs`

## NPC Identity

| Resource | Type | Writers | Readers |
|----------|------|---------|---------|
| NpcEntityMap | `HashMap<usize, Entity>` | spawn_npc_system, death_system | damage_system (slot → entity lookup) |
| SlotAllocator | `SlotPool` wrapper (max=100K) | spawn_npc_system (alloc), death_system (free) | GPU compute dispatch, UI, tests |
| BuildingSlots | `SlotPool` wrapper (max=5K) | place_building_instance (alloc), death_system (free) | Building GPU state, rendering |

Both `SlotAllocator` and `BuildingSlots` wrap a shared `SlotPool` inner type with LIFO free list. `next` is the high-water mark. Two query methods: `count()` returns high-water mark, `alive()` returns `next - free.len()`. Separate allocators prevent slot collisions between NPCs and buildings. See [spawn.md](spawn.md).

## NPC UI Caches

Pre-computed per-NPC data for UI queries, indexed by slot.

| Resource | Per-NPC Data | Writers | Readers |
|----------|-------------|---------|---------|
| NpcMetaCache | name, level, xp, town_id, job | spawn_npc_system, death_system (XP grant), inspector rename | UI queries |
| NpcLogCache | `VecDeque<NpcLogEntry>` (100 cap, circular, lazy init) | behavior/decision systems | UI queries |
| NpcsByTownCache | `Vec<Vec<usize>>` — NPC slots grouped by town | spawn/death systems | UI queries |

`NpcLogCache.push(idx, day, hour, minute, message)` adds timestamped entries. Oldest evicted at capacity.

NPC state is derived at query time via `derive_npc_state()` which checks ECS components (Dead, InCombat, Resting, etc.), not cached. Trait display reads from `Personality` component via `trait_summary()` at query time (not cached in meta). NPC rename edits `NpcMetaCache` directly from inspector UI.

## Population & Kill Stats

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| PopulationStats | `HashMap<(job, town), PopStats>` — alive, working, dead | spawn/death/state systems | UI |
| KillStats | archer_kills, villager_kills | death_system | UI |
| FactionStats | `Vec<FactionStat>` — alive, dead, kills per faction | spawn/death systems | UI |

`FactionStats` — one entry per settlement (player towns + AI towns + raider towns). Methods: `inc_alive()`, `dec_alive()`, `inc_dead()`, `inc_kills()`.

## World Layout

Static world data, immutable after initialization.

| Resource | Data | Purpose |
|----------|------|---------|
| WorldData | towns: `Vec<Town>` | Town center positions, factions, names |
| BuildingOccupancy | private `HashMap<(i32,i32), i32>` — position → worker count | Building assignment (claim/release/is_occupied/count/clear) |
| MineStates | `Vec<f32>` gold + `Vec<f32>` max_gold + `Vec<Vec2>` positions | Per-mine gold tracking |
| BuildingEntityMap | `BuildingInstance` storage + 256px spatial grid + by_kind/by_entity/by_grid_cell indexes | Sole source of truth for all building instance data (no WorldData.buildings); stores `BuildingInstance` (kind, position, town_idx, slot, entity, faction, patrol_order, assigned_mine, manual_mine, wall_level, npc_slot, respawn_timer, growth_ready, growth_progress); methods: `add_instance`/`remove_instance`/`get_instance[_mut]`/`find_by_position`/`find_farm_at[_mut]`/`find_mine_at[_mut]`/`iter_kind`/`iter_kind_for_town`/`iter_growable[_mut]`/`count_for_town`/`building_counts`/`gold_mine_index`/`for_each_nearby` (spatial)/`iter_instances`/`iter_instances_mut`; GPU slot maps: `insert`/`get_slot`/`get_building`/`get_entity`/`get_entity_by_building` |
| Dirty signaling | Concern-specific Bevy messages | `BuildingGridDirtyMsg`, `PatrolsDirtyMsg`, `PatrolPerimeterDirtyMsg`, `HealingZonesDirtyMsg`, `SquadsDirtyMsg`, `MiningDirtyMsg`, `AiSquadsDirtyMsg`, `PatrolSwapMsg`; `DirtyWriters<'w>` bundles writers and `emit_all()` covers startup/reset. See [messages.md](messages.md#dirty-signal-messages). |
| BuildingHealState | `needs_healing: bool` | Persistent flag (not a message): set by `building_damage_system` on hits, cleared by `healing_system` when no damaged buildings remain |
| TownGrids | `Vec<TownGrid>` — one per town (villager + raider) | Per-town building slot unlock tracking |
| GameAudio | `music_volume: f32`, `sfx_volume: f32`, `music_speed: f32`, `tracks: Vec<Handle<AudioSource>>`, `last_track: Option<usize>`, `loop_current: bool`, `play_next: Option<usize>` | Runtime audio state; tracks loaded at Startup, jukebox picks random no-repeat track; `loop_current` repeats same track on finish; `play_next` set by UI for explicit track selection; volume + speed synced from UserSettings |

### WorldData Structs

| Struct | Fields |
|--------|--------|
| Town | name, center (Vec2), faction, sprite_type (0=fountain, 1=tent) |

`WorldData` contains only towns. All building instance data lives in `BuildingEntityMap` — there is no `buildings` BTreeMap. `PlacedBuilding` remains in `save.rs` for backward-compatible deserialization of legacy save files.

Spatial queries (`find_nearest_location`, `find_location_within_radius`, `find_nearest_free`, `find_within_radius`, `find_nearest_enemy_building`) all use `BuildingEntityMap.for_each_nearby()` for O(1) cell lookups. Building counts use `BuildingEntityMap.count_for_town()` / `building_counts()`.

### World Grid

250x250 cell grid covering the entire 8000x8000 world (32px per cell). Each cell has a terrain biome and optional building.

| Resource | Data | Purpose |
|----------|------|---------|
| WorldGrid | `Vec<WorldCell>` (width × height), cell_size | World-wide terrain + building grid |
| WorldGenConfig | world dimensions, num_towns, spacing, npc_counts: BTreeMap\<Job, usize\> | Procedural generation parameters |

**WorldCell** fields: `terrain: Biome` (Grass/Forest/Water/Rock/Dirt), `building: Option<GridBuilding>` where `GridBuilding = (BuildingKind, u32)` (kind + town_idx). Save-compatible via `LegacyBuilding` deserialization in save.rs that converts the legacy enum format to tuples.

**WorldGrid** helpers: `cell(col, row)`, `cell_mut(col, row)`, `world_to_grid(pos) -> (col, row)`, `grid_to_world(col, row) -> Vec2`.

**WorldGenConfig** defaults: 8000x8000 world, 400px margin, 2 towns, 1200px min distance, 32px grid spacing, 3500px raider distance, npc_counts populated from NPC_REGISTRY default_count (Farmer:2, Archer:4, Raider:1, rest:0), 2 gold mines per town.

**`generate_world()`**: Takes config and populates WorldGrid, WorldData, TownGrids, and MineStates. Places towns randomly with min distance constraint, finds raider town positions furthest from all towns (16 directions), assigns terrain via simplex noise with Dirt override near settlements. Villager towns get 1 fountain, N farms, then homes for each village NPC type from NPC_REGISTRY (spiral-placed via `npc_counts` map), then 4 waypoints on the outer ring. Raider towns get a fountain center + homes for each raider NPC type from NPC_REGISTRY (spiral-placed via `npc_counts` map). Both town types get a TownGrid with expandable building slots. Gold mines placed in wilderness between settlements (min 300px from any town, min 400px between mines, `gold_mines_per_town × total_towns` count). Building positions are generated via `spiral_slots()` — a spiral outward from center that skips occupied cells. Guard posts are placed after spawner buildings so they're always on the perimeter.

### Town Building Grid

Per-town slot tracking for the building system. Each town (villager and raider) has a `TownGrid` with an `area_level: i32` controlling the buildable radius and a `town_data_idx` linking to its `WorldData.towns` entry. Initial base grid is 6x6 (rows/cols -2 to +3), expandable via `expand_town_build_area()` which increments `area_level` (max 50x50 extent).

| Struct | Fields |
|--------|--------|
| TownGrid | town_data_idx: usize, area_level: i32 |
| TownGrids | grids: `Vec<TownGrid>` (one per town — villager + raider) |
| BuildMenuContext | town_data_idx: `Option<usize>`, selected_build: `Option<BuildingKind>`, destroy_mode: bool, hover_world_pos: Vec2, ghost_sprites: `HashMap<BuildingKind, Handle<Image>>` |
| DestroyRequest | `Option<(usize, usize)>` — (grid_col, grid_row), set by inspector, processed by `process_destroy_system` |

Coordinate helpers: `town_grid_to_world(center, row, col)`, `world_to_town_grid(center, world_pos)`, `build_bounds(grid) -> (min_row, max_row, min_col, max_col)`, `is_slot_buildable(grid, row, col)`, `find_town_slot(world_pos, towns, grids)`.

Building placement: `place_building()` is the single entry point for all runtime building placement (player UI and AI, town-grid and wilderness). Takes `world_pos`, validates cell (exists, empty, not water), rejects foreign territory, deducts food, places on WorldGrid, creates `BuildingInstance` in `BuildingEntityMap`, auto-assigns waypoint `patrol_order`, pushes FarmStates for farms, registers spawner, spawns building entity (with `Building` marker + `Health` + `NpcIndex` + `Faction` + `TownId`), allocates building GPU slot, and marks DirtyFlags. `destroy_building()` shared helper consolidates all destroy side effects: grid cell clear + spawner tombstone + insert `Dead` on building entity + combat log — used by click-destroy, inspector-destroy, `building_damage_system` (HP→0), and waypoint pruning. `is_alive(pos)` checks tombstone status (single source of truth for `pos.x > -9000.0`). `empty_slots(tg, center, grid)` scans a town grid for buildable cells. Fountains and gold mines cannot be destroyed.

Building costs: `building_cost(kind)` in `constants.rs`. Flat costs (no difficulty scaling): Farm=2, FarmerHome=2, MinerHome=4, ArcherHome=4, CrossbowHome=8, Waypoint=1, Tent=3. All properties defined in `BUILDING_REGISTRY`.

## Food & Economy

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| FoodStorage | `Vec<i32>` — food count per town | economy systems (arrival, eating) | economy systems, UI |
| GoldStorage | `Vec<i32>` — gold count per town | mining delivery (arrival_system) | UI (top bar) |
| MiningPolicy | `discovered_mines: Vec<Vec<usize>>`, `mine_enabled: HashMap<usize, bool>` (keyed by GPU slot) | mining_policy_system | UI (policies tab, mine inspector) |
| Food flow signals | `FoodStorage` + system-local logic | economy systems | economy systems, UI |

`FoodStorage.init(count)` initializes per-town counters. Villager towns and raider towns share the same indexing.

## Raider Towns

| Resource | Data | Purpose |
|----------|------|---------|
| RaiderState | max_pop, respawn_timers, forage_timers per raider town | Raider town respawn/forage scheduling |

`RaiderState::faction_to_idx(faction)` maps faction ID to raider index (faction 1 = index 0).

## Game Time

| Resource | Fields | Default |
|----------|--------|---------|
| GameTime | total_seconds, seconds_per_hour, start_hour, time_scale, paused, last_hour, hour_ticked | 0.0s, 5.0s/hr, 6am, 1.0x, false |

Derived methods: `day()`, `hour()`, `minute()`, `is_daytime()` (6am–8pm), `total_hours()`.

`hour_ticked` is true for one frame when the game hour changes — used by economy/respawn systems.

## Game Config

| Resource | Fields | Default |
|----------|--------|---------|
| Difficulty | Easy, Normal, Hard | Normal |
| GameConfig | npc_counts: BTreeMap\<Job, i32\>, spawn_interval_hours, food_per_work_hour | from NPC_REGISTRY defaults, 4, 1 |

Pushed via `GAME_CONFIG_STAGING` static. Drained by `drain_game_config` system.

## GPU State

| Resource | Data | Status |
|----------|------|--------|
| GpuReadState | positions, combat_targets, health, factions, threat_counts, npc_count | Populated via GPU readback observers (mixed cadence; see below) |
| BuildingGpuState | positions, factions, healths, sprite_indices, flash_values, flags + dirty tracking | CPU-side GPU state for buildings; populated by `Bld*` GpuUpdate variants; read by building rendering + healing system |
| NpcSpriteTexture | handle (char atlas), world_handle (world atlas), extras_handle (extras atlas), building_handle (building atlas) | Shared with instanced renderer for texture bind group |
| ProjSlotAllocator | next, free list, max (50,000) | Active — allocates projectile slots |

`GpuReadState` is populated by `ReadbackComplete` observers. Positions/combat targets/health are always-on; `factions` is throttled to every 60 frames and `threat_counts` to every 30 frames. Used by combat systems (including `building_tower_system` for CPU-side tower targeting), behavior/AI threat logic, position sync, and test assertions.

`BuildingGpuState` holds building visual data on the CPU side — not uploaded to GPU compute. Building rendering reads from this state to build `BuildingBodyInstances` (instance buffer path). Building healing reads positions from this state.

## Stats & Upgrades

| Resource | Data | Defined In | Purpose |
|----------|------|------------|---------|
| CombatConfig | `HashMap<Job, JobStats>` + `HashMap<BaseAttackType, AttackTypeStats>` + heal_rate + heal_radius | `systems/stats.rs` | All NPC base stats — resolved via `resolve_combat_stats()` |
| TownUpgrades | `Vec<Vec<u8>>` per town (dynamic width = `upgrade_count()`) | `systems/stats.rs` | Per-town upgrade levels, indexed by dynamic registry layout |
| UpgradeMsg | Message `{ town_idx, upgrade_idx }` | `systems/stats.rs` | Upgrade purchase request from UI/auto/AI, consumed by `process_upgrades_system` |
| AutoUpgrade | `Vec<Vec<bool>>` per town (dynamic width = `upgrade_count()`) | `resources.rs` | Per-upgrade auto-buy flags; `auto_upgrade_system` emits `UpgradeMsg` each game hour for affordable enabled upgrades |

`CombatConfig::default()` initializes from hardcoded values (archer/raider damage=15, fighter damage=22.5, speeds=100, max_health=100, melee range=50/proj_speed=200, ranged range=100/proj_speed=100, heal_rate=5, heal_radius=150). Per-job `attack_override` in `NPC_REGISTRY` can override attack type defaults. `resolve_combat_stats()` combines job base × upgrade mult × trait mult × level mult → `CachedStats` component.

`UPGRADES` is the single source of truth for upgrade metadata — a global `LazyLock<UpgradeRegistry>` built from `NPC_REGISTRY` + `TOWN_UPGRADES` at startup. `UpgradeRegistry` contains dynamic `nodes`, UI `branches`, and an `(category, stat_kind) -> index` map. `UpgradeNode` includes: `label`, `short`, `tooltip`, `category`, `stat_kind`, `pct`, `cost`, `display`, `prereqs: Vec<(usize, u8)>`, and flags (`is_combat_stat`, `invalidates_healing`, `triggers_expansion`, `custom_cost`).

`TownUpgrades` stores dynamic per-town level vectors sized to `upgrade_count()`, and save/load uses decode helpers that pad older saves when new upgrades are added. Shared helpers gate all purchase paths: `upgrade_unlocked(levels, idx)` (prereqs), `upgrade_available(levels, idx, food, gold)` (prereqs + affordability), `deduct_upgrade_cost(...)`, `missing_prereqs(...)`, and `format_upgrade_cost(...)`. `UpgradeMsg` decouples writers from processing: UI, auto-upgrade, and AI emit messages; `process_upgrades_system` validates, deducts, increments, and re-resolves affected stats.

UI tree layout is driven by `UPGRADES.branches` (generated during registry build), not a hardcoded render-order array. `branch_total()` sums category levels, and `upgrade_effect_summary()` formats current/next effects (percentage, cooldown reduction, unlock, flat, discrete).

**Current default upgrade layout** (as built from current registries):

| Index | Upgrade | Category | % per level | Type |
|-------|---------|----------|-------------|------|
| 0 | MilitaryHp | Military | +10% | Multiplicative |
| 1 | MilitaryAttack | Military | +10% | Multiplicative |
| 2 | MilitaryRange | Military | +5% | Multiplicative |
| 3 | AttackSpeed | Military | -8% cooldown | Reciprocal: `1/(1+level*0.08)` |
| 4 | MilitaryMoveSpeed | Military | +5% | Multiplicative |
| 5 | AlertRadius | Military | +10% | Multiplicative |
| 6 | Dodge | Military | unlock | Unlock: projectile dodging |
| 7 | FarmYield | Farmer | +15% | Multiplicative |
| 8 | FarmerHp | Farmer | +20% | Multiplicative |
| 9 | FarmerMoveSpeed | Farmer | +5% | Multiplicative |
| 10 | MinerHp | Miner | +20% | Multiplicative |
| 11 | MinerMoveSpeed | Miner | +5% | Multiplicative |
| 12 | GoldYield | Miner | +15% | Multiplicative |
| 13 | HealingRate | Town | +20% | Multiplicative |
| 14 | FountainRadius | Town | +24px flat | Flat: `base_radius + level * 24.0` |
| 15 | TownArea | Town | +1 radius | Discrete: custom slot-based cost via `expansion_cost()` |
**Upgrade applicability by job** — not all upgrades flow through `resolve_combat_stats()`:

| Upgrade | Applies to | Notes |
|---------|-----------|-------|
| MilitaryHp, MilitaryAttack, MilitaryRange, MilitaryMoveSpeed, AlertRadius | Archer, Raider, Fighter | Combat resolver |
| AttackSpeed | Archer, Raider, Fighter | Combat resolver (reciprocal) |
| Dodge | Archer, Raider, Fighter | Unlock flag checked by `dodge_unlocked()` |
| FarmerHp, FarmerMoveSpeed | Farmer | Combat resolver |
| MinerHp, MinerMoveSpeed | Miner | Combat resolver |
| FarmYield | `farm_growth_system` reads directly | Not combat resolver |
| GoldYield | `decision_system` mining extraction | Not combat resolver |
| HealingRate, FountainRadius | `healing_system` reads directly | Not combat resolver |
| TownArea | `world.rs` build area expansion | Not combat resolver; custom cost via `expansion_cost()` |

## Town Policies

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| TownPolicies | `Vec<PolicySet>` — per-town behavior configuration (16 slots default) | left_panel (UI) | decision_system, behavior systems |

`PolicySet` fields: `eat_food` (bool), `archer_aggressive` (bool), `archer_leash` (bool), `farmer_fight_back` (bool), `prioritize_healing` (bool), `farmer_flee_hp` (f32, 0.0-1.0), `archer_flee_hp` (f32), `recovery_hp` (f32), `farmer_schedule` (WorkSchedule enum), `archer_schedule` (WorkSchedule enum), `farmer_off_duty` (OffDutyBehavior enum), `archer_off_duty` (OffDutyBehavior enum).

`WorkSchedule`: Both (default), DayOnly, NightOnly. `OffDutyBehavior`: GoToBed (default), StayAtFountain, WanderTown.

Defaults: eat_food=true, archer_aggressive=false, archer_leash=true, farmer_fight_back=false, prioritize_healing=true, farmer_flee_hp=0.30, archer_flee_hp=0.15, recovery_hp=0.80.

Replaces per-entity `FleeThreshold`/`WoundedThreshold` components for standard NPCs. Raiders use hardcoded flee threshold (0.50). Per-entity overrides still possible via `FleeThreshold` component (e.g., boss NPCs).

`PolicySet`, `WorkSchedule`, and `OffDutyBehavior` all derive `serde::Serialize + Deserialize`. Settings path: `Documents\Endless\settings.json`.

## Selection

| Resource | Data | Purpose |
|----------|------|---------|
| SelectedNpc | `i32` (-1 = none) | Currently selected NPC for inspector panel |
| SelectedBuilding | `{ col, row, kind, slot, active }` (default inactive) | Currently selected building — kind + GPU slot for direct BuildingEntityMap lookup |
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
| CombatLog | `entries: VecDeque<CombatLogEntry>` (max 200) + `priority_entries: VecDeque<CombatLogEntry>` (max 200, Raid/Ai events) | `drain_combat_log` system (collects `CombatLogMsg` messages from 18+ writer systems) | combat_log_system (via `iter_all()`), building inspector |
| BuildMenuContext | town_data_idx, selected_build (`Option<BuildingKind>`), destroy_mode (bool), hover_world_pos, ghost_sprites (`HashMap<BuildingKind, Handle<Image>>`) | build_menu_system (init_sprite_cache populates ghost_sprites), build_ghost_system | build_place_click_system, draw_slot_indicators |
| DestroyRequest | `Option<(usize, usize)>` (grid_col, grid_row) | bottom_panel_system (inspector destroy button) | process_destroy_system |
| UpgradeMsg | Message `{ town_idx, upgrade_idx }` | left_panel upgrades (UI), auto_upgrade_system, ai_player | process_upgrades_system |
| TowerState | `town: TowerKindState` where `TowerKindState = { timers: Vec<f32>, attack_enabled: Vec<bool> }` | building_tower_system (cooldown + fire) | building_tower_system |
| UserSettings | world_size, towns, farmers, archers, raiders, ai_towns, raider_towns, ai_interval, npc_interval, scroll_speed, ui_scale (f32, default 1.2), difficulty (Difficulty, default Normal), log_kills/spawns/raids/harvests/levelups/npc_activity/ai, debug_coordinates/all_npcs, policy (PolicySet), upgrade_expanded (Vec\<String\> — expanded branch labels) | main_menu (save on Play), bottom_panel (save on filter change), right_panel (save policies on tab leave), pause_menu (save on close), upgrade_content (save on expand/collapse) | main_menu (load on init), bottom_panel (load on init), game_startup (load policies), pause_menu settings, camera_pan_system, apply_ui_scale. **Loaded from disk at app startup** via `insert_resource(load_settings())` in `build_app()` — persists across app restarts without waiting for UI init. |

`UiState` tracks which panels are open. All default to false. `LeftPanelTab` enum: Roster (default), Upgrades, Policies, Patrols, Squads, Factions, Help. `toggle_left_tab()` method: if panel shows that tab → close, otherwise open to that tab. Faction pre-select now uses `SelectFactionMsg`: produced by fountain double-click and inspector faction links, consumed in `left_panel_system`/`factions_content` via `MessageReader<SelectFactionMsg>`.

`CombatLog` has two ring buffers: `entries` (max 200) for normal events and `priority_entries` (max 200) for Raid/Ai events — this prevents high-frequency combat events from pushing out important strategic entries. 7 event kinds: Kill, Spawn, Raid, Harvest, LevelUp, Ai, BuildingDamage. Each entry has day/hour/minute timestamps, a `faction: i32` (-1=global, 0=player, 1+=AI), a message string, and an optional `location: Option<Vec2>` (world position for camera-pan button). `push()` evicts oldest when at capacity; `push_at()` routes to the correct buffer by kind. `iter_all()` chains both buffers for display. Raid entries for wave-started events include the target position as location. AI entries (purple in HUD) log build/unlock/upgrade actions; Raid entries (orange) log migration arrivals, town settlements, and wave start/end. Combat log UI has "All"/"Mine" faction filter dropdown — "Mine" shows player (0) and global (-1) events only. Entries with a location show a clickable ">>" button that pans the camera to the target position.

`PolicySet` is serializable (`serde::Serialize + Deserialize`) and persisted as part of `UserSettings`. Loaded into `TownPolicies` on game startup, saved when leaving the Policies tab in the left panel.

## Squads

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| SquadState | `squads: Vec<Squad>` (first 10 player-reserved, AI appended after), `selected: i32`, `placing_target: bool`, `drag_start: Option<Vec2>`, `box_selecting: bool` | left_panel, click_to_select, box_select_system, game_escape, squad_cleanup_system, ai_squad_commander_system | decision_system, attack_system, squad_overlay_system, squad_cleanup_system, ai_squad_commander_system |

`SquadOwner` enum: `Player` (default) or `Town(usize)` (town_data_idx). Determines which town's military units get recruited into the squad.

`Squad` fields: `members: Vec<usize>` (NPC slot indices), `target: Option<Vec2>` (world position or None), `target_size: usize` (desired member count, 0 = manual mode — no auto-recruit/dismiss), `patrol_enabled: bool`, `rest_when_tired: bool`, `owner: SquadOwner`, `wave_active: bool`, `wave_start_count: usize`, `wave_min_start: usize`, `wave_retreat_below_pct: usize`, `hold_fire: bool` (when true, members only attack ManualTarget — no auto-engage).

`SquadId(i32)` component added to military units (archers, crossbows, fighters, raiders) when recruited into a squad. Removed on dismiss. Units with `SquadId` walk to squad target instead of patrolling (see [behavior.md](behavior.md#squads)).

`SquadUnit` marker component applied to all military NPCs (archers, crossbows, fighters, raiders) at spawn. Used by `squad_cleanup_system` and `ai_squad_commander_system` for recruitment queries instead of per-job component filters.

`placing_target`: when true, next right-click on the map sets the selected squad's target. Cancelled by ESC.

`drag_start` / `box_selecting`: box-select drag state. `drag_start` is set on left-click press (world-space position), `box_selecting` becomes true when the drag exceeds 5px threshold. On mouse release while `box_selecting`, all player military NPCs inside the AABB are assigned to the currently selected squad. Cleared by ESC or mouse release.

`ManualTarget` enum component: per-NPC target for DirectControl units. Variants: `Npc(usize)` (attack NPC slot), `Building(Vec2)` (attack building position), `Position(Vec2)` (ground move). Inserted by right-click commands on DirectControl NPCs. `Npc` variant overrides GPU auto-targeting in `attack_system`, auto-cleared when target dies. `Building`/`Position` variants fall through to GPU auto-targeting in combat. Crosshair overlay in `squad_overlay_system` renders for `Npc`/`Building` variants on DirectControl NPCs.

`npc_matches_owner(owner, npc_town_id, player_town)`: helper for owner-safe recruitment in `squad_cleanup_system`. Player squads recruit from player-town `SquadUnit` NPCs; `Town(tdi)` squads recruit from units with matching `TownId`.

UI filtering: left panel and squad overlay only show `is_player()` squads. Hotkeys 1-0 map to indices 0-9 (always player-reserved).

## AI Players

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| AiPlayerConfig | `decision_interval: f32` (real seconds between AI ticks, default 5.0) | main_menu (from settings) | ai_decision_system |
| AiPlayerState | `players: Vec<AiPlayer>` — one per non-player settlement | game_startup (populate), game_cleanup (reset) | ai_decision_system |
| NpcDecisionConfig | `interval: f32` (seconds between Tier 3 decisions, default 2.0) | main_menu (from settings) | decision_system |

`AiPlayer` fields: `town_data_idx` (WorldData.towns index), `grid_idx` (TownGrids index), `kind` (Builder or Raider), `personality` (Aggressive, Balanced, or Economic — randomly assigned at game start), `active` (bool — `ai_decision_system` skips inactive players; used by migration system to defer AI until town settles), `squad_indices: Vec<usize>` (indices into SquadState.squads), `squad_cmd: HashMap<usize, AiSquadCmdState>` (per-squad command state with independent cooldown + target identity). `AiKind` determined by `Town.sprite_type`: 0 (fountain) = Builder, 1 (tent) = Raider.

Personality drives build order, upgrade priority, food reserve, town policies, and **squad behavior**:
- **Aggressive**: military first (archer homes → waypoints → economy), zero food reserve, combat upgrades prioritized, miner homes = 1/3 of farmer homes. 3 squads: reserve (25% defense), 2 attack squads (55/45 split of remainder). Retargets every 15s, attacks nearest enemy anything.
- **Balanced**: economy and military in tandem (farm → farmer home → archer home → waypoint), 10 food reserve, miner homes = 1/2 of farmer homes. 2 squads: reserve (45% defense), 1 attack (remainder) targets military first. Retargets every 25s.
- **Economic**: farms first with minimal military, 30 food reserve, FarmYield/FarmerHp upgrades prioritized, miner homes = 2/3 of farmer homes. 2 squads: reserve (65% defense), 1 attack (remainder) targeting enemy farms only. Retargets every 40s.

Slot selection: economy buildings (farms, farmer homes, archer homes) prefer inner slots (closest to center). Guard posts prefer outer slots (farthest from center) with minimum Manhattan distance of 5 between posts. Raider tents cluster around town center (inner slots).

Both unlock slots when full (sets terrain to Dirt) and buy upgrades with surplus food. Combat log shows personality tag: `"Town [Balanced] built farm"`.

## Migration

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| MigrationState | `active: Option<MigrationGroup>`, `check_timer: f32` | migration_spawn_system, migration_settle_system | migration_attach_system, migration_settle_system, save/load |

`MigrationGroup` fields: `town_data_idx` (index into WorldData.towns for the raider town-to-be), `grid_idx` (TownGrids index), `member_slots: Vec<usize>` (NPC slot indices of migrating raiders), `boat_slot: Option<usize>` (NPC GPU slot for boat entity), `boat_pos: Vec2` (current boat position), `settle_target: Vec2` (destination chosen by `pick_settle_site`), `faction: i32`.

`Migrating` component: marker on NPC entities that are part of an active migration group. Attached by `migration_attach_system`, cleared by `migration_settle_system` on settlement. Persisted in save via `MigrationSave.member_slots` and re-attached on load.

## Movement Intent Resolution

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| MovementIntents | `HashMap<Entity, MovementIntent>` — sparse per-NPC intent slots, highest priority wins | decision_system, attack_system, death_system, click_to_select_system | resolve_movement_system (drains → single `SetTarget` per NPC) |

`MovementPriority` enum: `Wander(0) < JobRoute(1) < Squad(2) < Combat(3) < Survival(4) < ManualTarget(5) < DirectControl(6)`. Multiple `submit()` calls per NPC per frame keep the highest priority. Cleared every frame by `resolve_movement_system` via `drain()`. The resolver is the sole emitter of `GpuUpdate::SetTarget` and the sole recorder of `NpcTargetThrashDebug`. Change detection skips writes when the new target is within 1px of the current GPU target.

Systems that write intents no longer need `MessageWriter<GpuUpdateMsg>` for movement — they call `intents.submit(entity, target, priority, source)`. One-time init targets (spawn, boat migration) still write `SetTarget` directly.

## Debug Resources

| Resource | Key Fields | Updated By |
|----------|-----------|------------|
| CombatDebug | attackers_queried, targets_found, attacks_made, chases_started, sample positions/distances | cooldown/attack systems |
| HealthDebug | damage_processed, deaths, despawned, entity_count, healing stats | damage/death systems |
| SystemTimings | per-system EMA-smoothed ms (internal Mutex, `Res` not `ResMut`), frame_ms, enabled toggle (F5) | all systems via `timings.scope("name")` RAII guard |

## Control Resources

| Resource | Data | Purpose |
|----------|------|---------|
| Startup/reset sync | `emit_all()` + init systems | Startup/load consistency for dirty-driven systems |
| DeltaTime | `f32` | Frame delta in seconds |
| BuildingHpRender | `{ positions: Vec<Vec2>, health_pcts: Vec<f32> }` | Damaged building positions + HP fractions; extracted to render world for GPU instanced HP bars (atlas_id=5.0 bar-only mode) |

## Building HP — Entity Health as Source of Truth

Buildings are ECS entities with `Building` marker component + `Health` component (same as NPCs). There is no separate HP store — entity `Health` is the single source of truth. Lookup chain: `BuildingEntityMap.get_entity_by_building(kind, idx)` → entity → `Health`. Buildings are NOT in `NpcEntityMap` — they have their own `BuildingEntityMap` resource for all identity needs (slot ↔ entity ↔ building kind/index). Buildings use a separate slot allocator (`BuildingSlots`, max=5K) from NPCs (`SlotAllocator`, max=100K), and a separate CPU-side GPU state (`BuildingGpuState`) for rendering. Save/load serializes building HP as `HashMap<String, Vec<f32>>` (identical JSON format as the old `BuildingHpState`). `sync_building_hp_render` queries building entities to extract damaged building positions + HP fractions for GPU HP bar rendering (reads positions from `BuildingGpuState`).

## Known Issues

- **Health dual ownership**: CPU-authoritative but synced to GPU via messages. Could diverge if sync fails.
- **No external API**: All state is internal Bevy Resources. No query interface for external tools or UI frameworks.
