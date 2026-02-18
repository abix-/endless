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
| NpcMetaCache | name, level, xp, town_id, job | spawn_npc_system, xp_grant_system, inspector rename | UI queries |
| NpcLogCache | `VecDeque<NpcLogEntry>` (100 cap, circular, lazy init) | behavior/decision systems | UI queries |
| NpcsByTownCache | `Vec<Vec<usize>>` — NPC slots grouped by town | spawn/death systems | UI queries |

`NpcLogCache.push(idx, day, hour, minute, message)` adds timestamped entries. Oldest evicted at capacity.

NPC state is derived at query time via `derive_npc_state()` which checks ECS components (Dead, InCombat, Resting, etc.), not cached. Trait display reads from `Personality` component via `trait_summary()` at query time (not cached in meta). NPC rename edits `NpcMetaCache` directly from inspector UI.

## Population & Kill Stats

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| PopulationStats | `HashMap<(job, town), PopStats>` — alive, working, dead | spawn/death/state systems | UI |
| KillStats | archer_kills, villager_kills | death_cleanup_system | UI |
| FactionStats | `Vec<FactionStat>` — alive, dead, kills per faction | spawn/death/xp_grant systems | UI |

`FactionStats` — one entry per settlement (player towns + AI towns + raider camps). Methods: `inc_alive()`, `dec_alive()`, `inc_dead()`, `inc_kills()`.

## World Layout

Static world data, immutable after initialization.

| Resource | Data | Purpose |
|----------|------|---------|
| WorldData | towns, farms, beds, waypoints, unit_homes (BTreeMap\<BuildingKind, Vec\<UnitHome\>\>), miner_homes, gold_mines | All building positions and metadata |
| SpawnerState | `Vec<SpawnerEntry>` — one per unit-home spawner or MinerHome | Building→NPC links + respawn timers |
| BuildingOccupancy | private `HashMap<(i32,i32), i32>` — position → worker count | Building assignment (claim/release/is_occupied/count/clear) |
| FarmStates | `Vec<FarmGrowthState>` + `Vec<f32>` progress + `Vec<Vec2>` positions | Per-farm growth tracking; methods: `push_farm()`, `harvest()`, `tombstone()` (resets all 3 vecs, marks position offscreen) |
| MineStates | `Vec<f32>` gold + `Vec<f32>` max_gold + `Vec<Vec2>` positions | Per-mine gold tracking |
| BuildingSpatialGrid | 256px cell grid of `BuildingRef` entries (farms, waypoints, towns, gold mines, archer homes, crossbow homes, fighter homes, farmer homes, tents, miner homes, beds) | O(1) spatial queries for building find functions + enemy building targeting; rebuilt by `rebuild_building_grid_system` only when `DirtyFlags.building_grid` is set |
| BuildingSlotMap | bidirectional `HashMap<(BuildingKind, usize), usize>` | Maps buildings ↔ NPC GPU slots; buildings occupy invisible NPC slots (speed=0, sprite hidden) for GPU projectile collision; allocated at startup/load, freed on building destroy |
| BuildingHpState | Named `Vec<f32>` for waypoints/miner_homes/farms/towns/beds/gold_mines + `unit_home_hps: BTreeMap<BuildingKind, Vec<f32>>` for all unit-home kinds — custom `Serialize + Deserialize` flattens BTreeMap keys using registry `save_key` for save-format compatibility | Tracks current HP for all buildings; `hps(kind)`/`hps_mut(kind)` delegate to `BUILDING_REGISTRY` fn pointers (no per-kind match); `push_for()` delegates to registry; `iter_damaged()` loops over `BUILDING_REGISTRY` to find all damaged buildings; initialized on game startup, pushed on build, zeroed on destroy |
| DirtyFlags | `building_grid`, `patrols`, `patrol_perimeter`, `healing_zones`, `waypoint_slots`, `squads`, `mining`, `buildings_need_healing` (all bool), `patrol_swap: Option<(usize, usize)>` | Centralized dirty flags for gated rebuild systems; all default `true` so first frame rebuilds (except `buildings_need_healing` = false); `buildings_need_healing` set by `building_damage_system` on hits, cleared by `healing_system` when no damaged buildings remain; `waypoint_slots` triggers NPC slot alloc/free in `sync_waypoint_slots`; `squads` gates `squad_cleanup_system` (set by death/spawn/UI); `mining` gates `mining_policy_system`; `patrol_perimeter` gates `sync_patrol_perimeter_system`; `patrol_swap` queues patrol order swap from UI; `mark_building_changed(kind)` helper sets the right combo of flags for build/destroy events |
| TownGrids | `Vec<TownGrid>` — one per town (villager + camp) | Per-town building slot unlock tracking |
| GameAudio | `music_volume: f32`, `sfx_volume: f32`, `music_speed: f32`, `tracks: Vec<Handle<AudioSource>>`, `last_track: Option<usize>`, `loop_current: bool`, `play_next: Option<usize>` | Runtime audio state; tracks loaded at Startup, jukebox picks random no-repeat track; `loop_current` repeats same track on finish; `play_next` set by UI for explicit track selection; volume + speed synced from UserSettings |

### WorldData Structs

| Struct | Fields |
|--------|--------|
| Town | name, center (Vec2), faction, sprite_type (0=fountain, 1=tent) |
| Farm | position (Vec2), town_idx |
| Bed | position (Vec2), town_idx |
| Waypoint | position (Vec2), town_idx, patrol_order, npc_slot (Option\<usize\>) |
| UnitHome | position (Vec2), town_idx — shared struct for FarmerHome, ArcherHome, CrossbowHome, FighterHome, Tent (stored in `unit_homes` BTreeMap keyed by BuildingKind) |
| MinerHome | position (Vec2), town_idx, assigned_mine (Option\<usize\>), manual_mine (bool) |
| GoldMine | position (Vec2) |

Helper functions: `building_pos_town(kind, index)` → `Option<(Vec2, u32)>` delegates to `BUILDING_REGISTRY` fn pointer (no per-kind match), `building_len(kind)` delegates to registry, `building_counts(town_idx)` → `HashMap<BuildingKind, usize>` via registry loop (replaced `TownBuildingCounts` struct), `find_nearest_location()`, `find_location_within_radius()`, `find_nearest_free()`, `find_within_radius()`, `find_by_pos()`. The first four use `BuildingSpatialGrid` for O(1) cell lookups instead of linear scans. `find_by_pos` still uses the `Worksite` trait directly on slices.

### World Grid

250x250 cell grid covering the entire 8000x8000 world (32px per cell). Each cell has a terrain biome and optional building.

| Resource | Data | Purpose |
|----------|------|---------|
| WorldGrid | `Vec<WorldCell>` (width × height), cell_size | World-wide terrain + building grid |
| WorldGenConfig | world dimensions, num_towns, spacing, per-town NPC counts | Procedural generation parameters |

**WorldCell** fields: `terrain: Biome` (Grass/Forest/Water/Rock/Dirt), `building: Option<Building>`.

**Building** variants: `Fountain { town_idx }`, `Farm { town_idx }`, `Bed { town_idx }`, `Waypoint { town_idx, patrol_order }`, `Camp { town_idx }`, `GoldMine`, `MinerHome { town_idx }`, `Home { kind: BuildingKind, town_idx }` (generic variant for all unit-home buildings — FarmerHome, ArcherHome, CrossbowHome, FighterHome, Tent). Save-compatible via `BuildingSerde` proxy enum that preserves legacy variant names.

**WorldGrid** helpers: `cell(col, row)`, `cell_mut(col, row)`, `world_to_grid(pos) -> (col, row)`, `grid_to_world(col, row) -> Vec2`.

**WorldGenConfig** defaults: 8000x8000 world, 400px margin, 2 towns, 1200px min distance, 32px grid spacing, 3500px camp distance, 2 farmers / 2 archers / 0 raiders per town (testing defaults), 2 gold mines per town.

**`generate_world()`**: Takes config and populates WorldGrid, WorldData, TownGrids, and MineStates. Places towns randomly with min distance constraint, finds camp positions furthest from all towns (16 directions), assigns terrain via simplex noise with Dirt override near settlements. Villager towns get 1 fountain, 2 farms, N FarmerHomes + N ArcherHomes (spiral-placed), then 4 waypoints on the outer ring. Raider camps get a Camp center + N Tents (spiral-placed from slider). Both town types get a TownGrid with expandable building slots. Gold mines placed in wilderness between settlements (min 300px from any town, min 400px between mines, `gold_mines_per_town × total_towns` count). Building positions are generated via `spiral_slots()` — a spiral outward from center that skips occupied cells. Guard posts are placed after spawner buildings so they're always on the perimeter.

### Town Building Grid

Per-town slot tracking for the building system. Each town (villager and raider camp) has a `TownGrid` with an `area_level: i32` controlling the buildable radius and a `town_data_idx` linking to its `WorldData.towns` entry. Initial base grid is 6x6 (rows/cols -2 to +3), expandable via `expand_town_build_area()` which increments `area_level` (max 50x50 extent).

| Struct | Fields |
|--------|--------|
| TownGrid | town_data_idx: usize, area_level: i32 |
| TownGrids | grids: `Vec<TownGrid>` (one per town — villager + camp) |
| BuildMenuContext | town_data_idx: `Option<usize>`, selected_build: `Option<BuildingKind>`, destroy_mode: bool, hover_world_pos: Vec2, ghost_sprites: `HashMap<BuildingKind, Handle<Image>>` |
| DestroyRequest | `Option<(usize, usize)>` — (grid_col, grid_row), set by inspector, processed by `process_destroy_system` |

Coordinate helpers: `town_grid_to_world(center, row, col)`, `world_to_town_grid(center, world_pos)`, `build_bounds(grid) -> (min_row, max_row, min_col, max_col)`, `is_slot_buildable(grid, row, col)`, `find_town_slot(world_pos, towns, grids)`.

Building placement: `place_building()` validates cell empty, places on WorldGrid, pushes to WorldData + FarmStates. `build_and_pay()` additionally deducts food, registers spawner, pushes HP entry, allocates building GPU slot, and marks DirtyFlags — shared by both player UI and AI. `place_waypoint_at_world_pos()` is the waypoint-specific variant (no spawner, auto-assigns patrol_order). `remove_building()` tombstones position to (-99999, -99999) in WorldData, clears grid cell. `destroy_building()` shared helper consolidates all destroy side effects: `remove_building()` + spawner tombstone + HP zero + GPU slot free + combat log — used by click-destroy, inspector-destroy, `building_damage_system` (HP→0), and waypoint pruning. `is_alive(pos)` checks tombstone status (single source of truth for `pos.x > -9000.0`). `empty_slots(tg, center, grid)` scans a town grid for buildable cells. Tombstone deletion preserves parallel Vec indices (FarmStates, BuildingHpState). Fountains, camps, and gold mines cannot be destroyed.

Building costs: `building_cost(kind)` in `constants.rs`. Flat costs (no difficulty scaling): Farm=2, FarmerHome=2, MinerHome=4, ArcherHome=4, CrossbowHome=8, Waypoint=1, Tent=3. All properties defined in `BUILDING_REGISTRY`.

## Food & Economy

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| FoodStorage | `Vec<i32>` — food count per town/camp | economy systems (arrival, eating) | economy systems, UI |
| GoldStorage | `Vec<i32>` — gold count per town/camp | mining delivery (arrival_system) | UI (top bar) |
| MiningPolicy | `discovered_mines: Vec<Vec<usize>>`, `mine_enabled: Vec<bool>` | mining_policy_system | UI (policies tab, mine inspector) |
| FoodEvents | delivered: `Vec<FoodDelivered>`, consumed: `Vec<FoodConsumed>` | behavior systems | UI (poll and drain) |

`FoodStorage.init(count)` initializes per-town counters. Villager towns and raider camps share the same indexing.

## Raider Camps

| Resource | Data | Purpose |
|----------|------|---------|
| CampState | max_pop, respawn_timers, forage_timers per camp | Camp respawn/forage scheduling |

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
| Difficulty | Easy, Normal, Hard | Normal |
| GameConfig | farmers_per_town, archers_per_town, raiders_per_camp, spawn_interval_hours, food_per_work_hour | 10, 30, 15, 4, 1 |

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
| TownUpgrades | `Vec<[u8; 16]>` per town | `systems/stats.rs` | Per-town upgrade levels, indexed by `UpgradeType` enum. `town_levels(idx)` accessor. |
| UpgradeQueue | `Vec<(usize, usize)>` — (town_idx, upgrade_index) | `systems/stats.rs` | Pending upgrade purchases from UI, drained by `process_upgrades_system` |
| AutoUpgrade | `Vec<[bool; 16]>` per town | `resources.rs` | Per-upgrade auto-buy flags; `auto_upgrade_system` queues affordable upgrades each game hour; persisted per-player-town in `UserSettings.auto_upgrades` |

`CombatConfig::default()` initializes from hardcoded values (archer/raider damage=15, fighter damage=22.5, speeds=100, max_health=100, melee range=50/proj_speed=200, ranged range=100/proj_speed=100, heal_rate=5, heal_radius=150). Per-job `attack_override` in `NPC_REGISTRY` can override attack type defaults. `resolve_combat_stats()` combines job base × upgrade mult × trait mult × level mult → `CachedStats` component.

`UPGRADE_REGISTRY` is the single source of truth for all upgrade metadata — an `[UpgradeNode; 16]` const array in `stats.rs`. Each `UpgradeNode` has: `label`, `short`, `tooltip`, `category`, `cost: &[(ResourceKind, i32)]` (multi-resource), `prereqs: &[(UpgradePrereq)]`. `ResourceKind { Food, Gold }` is extensible for future resource types. `UpgradePrereq { upgrade: usize, min_level: u8 }` defines dependency edges forming a tech tree.

`TownUpgrades` is indexed by town, each entry is a fixed-size array of 16 upgrade levels (`UpgradeType` enum — Military: MilitaryHp, MilitaryAttack, MilitaryRange, AttackSpeed, MilitaryMoveSpeed, AlertRadius, Dodge; Farmer: FarmYield, FarmerHp, FarmerMoveSpeed; Miner: MinerHp, MinerMoveSpeed, GoldYield; Town: HealingRate, FountainRadius, TownArea). Shared helpers gate all purchase paths: `upgrade_unlocked(levels, idx)` checks prereqs, `upgrade_available(levels, idx, food, gold)` checks prereqs + multi-resource affordability, `deduct_upgrade_cost(idx, level, &mut food, &mut gold)` deducts from correct storages. `UpgradeQueue` decouples the UI from stat re-resolution — `left_panel.rs` pushes `(town, upgrade)` tuples, `process_upgrades_system` validates via `upgrade_available()`, deducts via `deduct_upgrade_cost()`, increments level, and re-resolves `CachedStats` for affected NPCs. `auto_upgrade_system` runs once per game hour, queuing auto-enabled upgrades that pass `upgrade_available()`. AI upgrade scoring in `ai_decision_system` also gates on `upgrade_available()`.

`UPGRADE_RENDER_ORDER` defines the UI tree layout — `&[(&str, &[(usize, u8)])]` where each entry is a branch label with ordered `(upgrade_index, depth)` pairs. Depth controls indentation in the upgrade panel. `branch_total()` sums levels per category. `upgrade_effect_summary()` returns `(now_text, next_text)` for UI display (handles multiplicative, reciprocal, unlock, flat, and discrete types).

**Upgrade percentages** (`UPGRADE_PCT` array in `systems/stats.rs`):

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
| UiState | build_menu_open, pause_menu_open, left_panel_open, left_panel_tab (LeftPanelTab enum), pending_faction_select (Option\<i32\>) | ui_toggle_system (keyboard), top_bar (buttons), left_panel tabs, pause_menu, click_to_select_system (fountain double-click) | All panel systems |
| CombatLog | `VecDeque<CombatLogEntry>` (max 200) | death_cleanup, spawn_npc, decision_system, arrival_system, build_menu_system | bottom_panel_system |
| BuildMenuContext | town_data_idx, selected_build (`Option<BuildingKind>`), destroy_mode (bool), hover_world_pos, ghost_sprites (`HashMap<BuildingKind, Handle<Image>>`) | build_menu_system (init_sprite_cache populates ghost_sprites), build_ghost_system | build_place_click_system, draw_slot_indicators |
| DestroyRequest | `Option<(usize, usize)>` (grid_col, grid_row) | bottom_panel_system (inspector destroy button) | process_destroy_system |
| UpgradeQueue | `Vec<(usize, usize)>` — (town_idx, upgrade_index) | left_panel upgrades (UI), auto_upgrade_system | process_upgrades_system |
| TurretState | `waypoint: TurretKindState`, `town: TurretKindState` — each has `timers: Vec<f32>`, `attack_enabled: Vec<bool>` | building_turret_system (auto-sync + refresh) | building_turret_system |
| SpawnerState | `Vec<SpawnerEntry>` — building (Building enum), town_idx, position, npc_slot, respawn_timer. `is_population_spawner()` checks registry for spawner def. | game_startup, build_menu (push on build), spawner_respawn_system | spawner_respawn_system, game_hud (counts), ai_player (reserve calc) |
| UserSettings | world_size, towns, farmers, archers, raiders, ai_towns, raider_camps, ai_interval, npc_interval, scroll_speed, ui_scale (f32, default 1.2), difficulty (Difficulty, default Normal), log_kills/spawns/raids/harvests/levelups/npc_activity/ai, debug_coordinates/all_npcs, policy (PolicySet) | main_menu (save on Play), bottom_panel (save on filter change), right_panel (save policies on tab leave), pause_menu (save on close) | main_menu (load on init), bottom_panel (load on init), game_startup (load policies), pause_menu settings, camera_pan_system, apply_ui_scale. **Loaded from disk at app startup** via `insert_resource(load_settings())` in `build_app()` — persists across app restarts without waiting for UI init. |

`UiState` tracks which panels are open. All default to false. `LeftPanelTab` enum: Roster (default), Upgrades, Policies, Patrols, Squads, Factions, Help. `toggle_left_tab()` method: if panel shows that tab → close, otherwise open to that tab. `pending_faction_select`: set by double-clicking a fountain in `click_to_select_system`, consumed by `factions_content` to pre-select the matching faction. Reset on game cleanup.

`CombatLog` is a ring buffer of global events with 7 kinds: Kill, Spawn, Raid, Harvest, LevelUp, Ai, BuildingDamage. Each entry has day/hour/minute timestamps, a `faction: i32` (-1=global, 0=player, 1+=AI), a message string, and an optional `location: Option<Vec2>` (world position for camera-pan button). `push()` evicts oldest when at capacity; `push_at()` accepts an explicit location. Raid entries for wave-started events include the target position as location. AI entries (purple in HUD) log build/unlock/upgrade actions; Raid entries (orange) log migration arrivals, camp settlements, and wave start/end. Combat log UI has "All"/"Mine" faction filter dropdown — "Mine" shows player (0) and global (-1) events only. Entries with a location show a clickable ">>" button that pans the camera to the target position.

`PolicySet` is serializable (`serde::Serialize + Deserialize`) and persisted as part of `UserSettings`. Loaded into `TownPolicies` on game startup, saved when leaving the Policies tab in the left panel.

## Squads

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| SquadState | `squads: Vec<Squad>` (first 10 player-reserved, AI appended after), `selected: i32`, `placing_target: bool` | left_panel, click_to_select, game_escape, squad_cleanup_system, ai_squad_commander_system | decision_system, squad_overlay_system, squad_cleanup_system, ai_squad_commander_system |

`SquadOwner` enum: `Player` (default) or `Town(usize)` (town_data_idx). Determines which town's military units get recruited into the squad.

`Squad` fields: `members: Vec<usize>` (NPC slot indices), `target: Option<Vec2>` (world position or None), `target_size: usize` (desired member count, 0 = manual mode — no auto-recruit/dismiss), `patrol_enabled: bool`, `rest_when_tired: bool`, `owner: SquadOwner`, `wave_active: bool`, `wave_start_count: usize`, `wave_min_start: usize`, `wave_retreat_below_pct: usize`.

`SquadId(i32)` component added to military units (archers, crossbows, fighters, raiders) when recruited into a squad. Removed on dismiss. Units with `SquadId` walk to squad target instead of patrolling (see [behavior.md](behavior.md#squads)).

`SquadUnit` marker component applied to all military NPCs (archers, crossbows, fighters, raiders) at spawn. Used by `squad_cleanup_system` and `ai_squad_commander_system` for recruitment queries instead of per-job component filters.

`placing_target`: when true, next left-click on the map sets the selected squad's target. Cancelled by ESC or right-click.

`npc_matches_owner(owner, npc_town_id, player_town)`: helper for owner-safe recruitment in `squad_cleanup_system`. Player squads recruit from player-town `SquadUnit` NPCs; `Town(tdi)` squads recruit from units with matching `TownId`.

UI filtering: left panel and squad overlay only show `is_player()` squads. Hotkeys 1-0 map to indices 0-9 (always player-reserved).

## AI Players

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| AiPlayerConfig | `decision_interval: f32` (real seconds between AI ticks, default 5.0) | main_menu (from settings) | ai_decision_system |
| AiPlayerState | `players: Vec<AiPlayer>` — one per non-player settlement | game_startup (populate), game_cleanup (reset) | ai_decision_system |
| NpcDecisionConfig | `interval: f32` (seconds between Tier 3 decisions, default 2.0) | main_menu (from settings) | decision_system |

`AiPlayer` fields: `town_data_idx` (WorldData.towns index), `grid_idx` (TownGrids index), `kind` (Builder or Raider), `personality` (Aggressive, Balanced, or Economic — randomly assigned at game start), `active` (bool — `ai_decision_system` skips inactive players; used by migration system to defer AI until camp settles), `squad_indices: Vec<usize>` (indices into SquadState.squads), `squad_cmd: HashMap<usize, AiSquadCmdState>` (per-squad command state with independent cooldown + target identity). `AiKind` determined by `Town.sprite_type`: 0 (fountain) = Builder, 1 (tent) = Raider.

Personality drives build order, upgrade priority, food reserve, town policies, and **squad behavior**:
- **Aggressive**: military first (archer homes → waypoints → economy), zero food reserve, combat upgrades prioritized, miner homes = 1/3 of farmer homes. 3 squads: reserve (25% defense), 2 attack squads (55/45 split of remainder). Retargets every 15s, attacks nearest enemy anything.
- **Balanced**: economy and military in tandem (farm → farmer home → archer home → waypoint), 10 food reserve, miner homes = 1/2 of farmer homes. 2 squads: reserve (45% defense), 1 attack (remainder) targets military first. Retargets every 25s.
- **Economic**: farms first with minimal military, 30 food reserve, FarmYield/FarmerHp upgrades prioritized, miner homes = 2/3 of farmer homes. 2 squads: reserve (65% defense), 1 attack (remainder) targeting enemy farms only. Retargets every 40s.

Slot selection: economy buildings (farms, farmer homes, archer homes) prefer inner slots (closest to center). Guard posts prefer outer slots (farthest from center) with minimum Manhattan distance of 5 between posts. Raider tents cluster around camp center (inner slots).

Both unlock slots when full (sets terrain to Dirt) and buy upgrades with surplus food. Combat log shows personality tag: `"Town [Balanced] built farm"`.

## Migration

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| MigrationState | `active: Option<MigrationGroup>`, `check_timer: f32` | migration_spawn_system, migration_settle_system | migration_attach_system, migration_settle_system, save/load |

`MigrationGroup` fields: `town_data_idx` (index into WorldData.towns for the camp-to-be), `grid_idx` (TownGrids index), `member_slots: Vec<usize>` (NPC slot indices of migrating raiders).

`Migrating` component: marker on NPC entities that are part of an active migration group. Attached by `migration_attach_system`, removed by `migration_settle_system` on settlement. Persisted in save via `MigrationSave.member_slots` and re-attached on load.

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
| BuildingHpRender | `{ positions: Vec<Vec2>, health_pcts: Vec<f32> }` | Damaged building positions + HP fractions; extracted to render world for GPU instanced HP bars (atlas_id=5.0 bar-only mode) |

## Known Issues

- **Health dual ownership**: CPU-authoritative but synced to GPU via messages. Could diverge if sync fails.
- **No external API**: All state is internal Bevy Resources. No query interface for external tools or UI frameworks.
