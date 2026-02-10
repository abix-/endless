# Game State Reference

## Overview

All game state lives in Bevy Resources — singleton structs accessible by any system via `Res<T>` (read) or `ResMut<T>` (write). There is no external API surface. Systems communicate through [messages](messages.md) and shared resources.

Defined in: `rust/src/resources.rs`, `rust/src/world.rs`

## NPC Identity

| Resource | Type | Writers | Readers |
|----------|------|---------|---------|
| NpcCount | `usize` | spawn_npc_system, death_cleanup_system | UI, spawn throttling |
| NpcEntityMap | `HashMap<usize, Entity>` | spawn_npc_system, death_cleanup_system | damage_system (slot → entity lookup) |
| SlotAllocator | `{ next, free: Vec }` | spawn_npc_system (alloc), death_cleanup_system (free) | — |
| GpuDispatchCount | `usize` | spawn_npc_system | GPU compute dispatch sizing |

`SlotAllocator` uses LIFO free list — most recently freed slot is reused first. `next` is the high-water mark. See [spawn.md](spawn.md).

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

`FactionStats` index 0 = villagers (all towns share), 1+ = raider camps. Methods: `inc_alive()`, `dec_alive()`, `inc_dead()`, `inc_kills()`.

## World Layout

Static world data, immutable after initialization.

| Resource | Data | Purpose |
|----------|------|---------|
| WorldData | towns, farms, beds, guard_posts | All building positions and metadata |
| BedOccupancy | `HashMap<(i32,i32), i32>` — bed position → NPC index (-1 = free) | Bed assignment |
| FarmOccupancy | `HashMap<(i32,i32), i32>` — farm position → worker count | Farm assignment |
| FarmStates | `Vec<FarmGrowthState>` + `Vec<f32>` progress | Per-farm growth tracking |
| TownGrids | `Vec<TownGrid>` — one per villager town | Per-town building slot unlock tracking |

### WorldData Structs

| Struct | Fields |
|--------|--------|
| Town | name, center (Vec2), faction, sprite_type (0=fountain, 1=tent) |
| Farm | position (Vec2), town_idx |
| Bed | position (Vec2), town_idx |
| GuardPost | position (Vec2), town_idx, patrol_order |

Helper functions: `find_nearest_location()`, `find_location_within_radius()`, `find_farm_index_by_pos()`.

### World Grid

250x250 cell grid covering the entire 8000x8000 world (32px per cell). Each cell has a terrain biome and optional building.

| Resource | Data | Purpose |
|----------|------|---------|
| WorldGrid | `Vec<WorldCell>` (width × height), cell_size | World-wide terrain + building grid |
| WorldGenConfig | world dimensions, num_towns, spacing, per-town NPC counts | Procedural generation parameters |

**WorldCell** fields: `terrain: Biome` (Grass/Forest/Water/Rock/Dirt), `building: Option<Building>`.

**Building** variants: `Fountain { town_idx }`, `Farm { town_idx }`, `Bed { town_idx }`, `GuardPost { town_idx, patrol_order }`, `Camp { town_idx }`.

**WorldGrid** helpers: `cell(col, row)`, `cell_mut(col, row)`, `world_to_grid(pos) -> (col, row)`, `grid_to_world(col, row) -> Vec2`.

**WorldGenConfig** defaults: 8000x8000 world, 400px margin, 2 towns, 1200px min distance, 32px grid spacing, 1100px camp distance, 2 farmers / 2 guards / 0 raiders per town (testing defaults).

**`generate_world()`**: Pure function that takes config and populates both WorldGrid and WorldData. Places towns randomly with min distance constraint, finds camp positions furthest from all towns (16 directions), assigns terrain via simplex noise with Dirt override near settlements, and places buildings per town (1 fountain, 2 farms, 4 beds, 4 guard posts at grid corners).

### Town Building Grid

Per-town slot tracking for the building system. Each villager town has a `TownGrid` with a `HashSet<(i32, i32)>` of unlocked (row, col) slots. Initial base grid is 6x6 (rows/cols -2 to +3), expandable to 100x100 by unlocking adjacent slots.

| Struct | Fields |
|--------|--------|
| TownGrid | unlocked: `HashSet<(i32, i32)>` |
| TownGrids | grids: `Vec<TownGrid>` (one per villager town) |
| TownSlotInfo | grid_idx, town_data_idx, row, col, slot_state |
| SlotState | Unlocked, Locked |
| BuildMenuContext | grid_idx, town_data_idx, slot, slot_world_pos, is_locked, has_building, is_fountain |

Coordinate helpers: `town_grid_to_world(center, row, col)`, `world_to_town_grid(center, world_pos)`, `get_adjacent_locked_slots(grid)`, `find_town_slot(world_pos, towns, grids)`.

Building placement: `place_building()` validates cell empty, places on WorldGrid, pushes to WorldData + FarmStates. `remove_building()` tombstones position to (-99999, -99999) in WorldData, clears grid cell. Tombstone deletion preserves parallel Vec indices (FarmStates). Fountains and camps cannot be removed.

Building costs (from constants.rs): Farm=1, Bed=1, GuardPost=1, SlotUnlock=1 food (lowered for testing; real costs TBD).

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
| TownUpgrades | `Vec<[u8; 14]>` per town | `systems/stats.rs` | Per-town upgrade levels, indexed by `UpgradeType` enum |
| UpgradeQueue | `Vec<(usize, usize)>` — (town_idx, upgrade_index) | `systems/stats.rs` | Pending upgrade purchases from UI, drained by `process_upgrades_system` |

`CombatConfig::default()` initializes from hardcoded values (guard/raider damage=15, speeds=100, max_health=100, heal_rate=5, heal_radius=150). `resolve_combat_stats()` combines job base × upgrade mult × trait mult × level mult → `CachedStats` component.

`TownUpgrades` is indexed by town, each entry is a fixed-size array of 14 upgrade levels (`UpgradeType` enum). `UpgradeQueue` decouples the UI from stat re-resolution — `upgrade_menu.rs` pushes `(town, upgrade)` tuples, `process_upgrades_system` validates food cost (`10 * 2^level`), increments level, deducts food, and re-resolves `CachedStats` for affected NPCs.

## Town Policies

| Resource | Data | Writers | Readers |
|----------|------|---------|---------|
| TownPolicies | `Vec<PolicySet>` — per-town behavior configuration (16 slots default) | policies_panel (UI) | decision_system, behavior systems |

`PolicySet` fields: `eat_food` (bool), `guard_aggressive` (bool), `guard_leash` (bool), `farmer_fight_back` (bool), `prioritize_healing` (bool), `farmer_flee_hp` (f32, 0.0-1.0), `guard_flee_hp` (f32), `recovery_hp` (f32), `work_schedule` (WorkSchedule enum), `farmer_off_duty` (OffDutyBehavior enum), `guard_off_duty` (OffDutyBehavior enum).

`WorkSchedule`: Both (default), DayOnly, NightOnly. `OffDutyBehavior`: GoToBed (default), StayAtFountain, WanderTown.

Defaults: eat_food=true, guard_aggressive=false, guard_leash=true, farmer_fight_back=false, prioritize_healing=true, farmer_flee_hp=0.30, guard_flee_hp=0.15, recovery_hp=0.80.

Replaces per-entity `FleeThreshold`/`WoundedThreshold` components for standard NPCs. Raiders use hardcoded flee threshold (0.50). Per-entity overrides still possible via `FleeThreshold` component (e.g., boss NPCs).

## Selection

| Resource | Data | Purpose |
|----------|------|---------|
| SelectedNpc | `i32` (-1 = none) | Currently selected NPC for inspector panel |
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
| UiState | combat_log_open, build_menu_open, pause_menu_open, right_panel_open, right_panel_tab (RightPanelTab enum) | ui_toggle_system (keyboard), game_hud (buttons), right_panel tabs, pause_menu | All panel systems |
| CombatLog | `VecDeque<CombatLogEntry>` (max 200) | death_cleanup, spawn_npc, decision_system, arrival_system, build_menu_system, reassign_npc_system | combat_log panel |
| BuildMenuContext | grid_idx, town_data_idx, slot, slot_world_pos, screen_pos, is_locked, has_building, is_fountain | slot_right_click_system | build_menu_system |
| ReassignQueue | `Vec<(usize, i32)>` — (npc_slot, new_job) | right_panel roster (UI) | reassign_npc_system |
| UpgradeQueue | `Vec<(usize, usize)>` — (town_idx, upgrade_index) | right_panel upgrades (UI) | process_upgrades_system |
| GuardPostState | timers: `Vec<f32>`, attack_enabled: `Vec<bool>` | guard_post_attack_system (auto-sync length), build_menu (toggle) | guard_post_attack_system |
| UserSettings | world_size, towns, farmers, guards, raiders, scroll_speed, log_kills/spawns/raids/harvests/levelups | main_menu (save on Play), combat_log (save on filter change), pause_menu (save on close) | main_menu (load on init), combat_log (load on init), pause_menu settings, camera_pan_system |

`UiState` tracks which panels are open. `combat_log_open` defaults to true, all others false. `RightPanelTab` enum: Roster (default), Upgrades, Policies. `toggle_right_tab()` method: if panel shows that tab → close, otherwise open to that tab. Reset on game cleanup.

`CombatLog` is a ring buffer of global events with 5 kinds: Kill, Spawn, Raid, Harvest, LevelUp. Each entry has day/hour/minute timestamps and a message string. `push()` evicts oldest when at capacity.

## Debug Resources

| Resource | Key Fields | Updated By |
|----------|-----------|------------|
| CombatDebug | attackers_queried, targets_found, attacks_made, chases_started, sample positions/distances | cooldown/attack systems |
| HealthDebug | damage_processed, deaths, despawned, entity_count, healing stats | damage/death systems |
| PerfStats | queue_ms, dispatch_ms, bevy_ms, frame_ms (static Mutex, not Resource) | bevy_timer_end |
| BevyFrameTimer | start: `Option<Instant>` | frame timing systems |

## Control Resources

| Resource | Data | Purpose |
|----------|------|---------|
| ResetFlag | `bool` | When true, `reset_bevy_system` clears all state |
| DeltaTime | `f32` | Frame delta in seconds |
| RespawnTimers | `HashMap<clan_id, hours>` | Per-clan respawn cooldowns |

## Known Issues

- **Health dual ownership**: CPU-authoritative but synced to GPU via messages. Could diverge if sync fails.
- **NpcCount high-water mark**: SlotAllocator.next never shrinks. 1000 spawns + 999 deaths = count still 1000.
- **No external API**: All state is internal Bevy Resources. No query interface for external tools or UI frameworks.

## Rating: 7/10

Resources are well-organized by domain with clear ownership. UI caches avoid repeated ECS queries. Slot allocator with free list handles reuse efficiently. Weaknesses: GpuReadState is a dead struct, high-water mark dispatch count, and NPC state derivation at query time could be expensive with many NPCs.
