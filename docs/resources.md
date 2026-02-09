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
| NpcMetaCache | name, level, xp, trait_id, town_id, job | spawn_npc_system | UI queries |
| NpcEnergyCache | `f32` energy level | energy_system | UI queries |
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

**WorldGenConfig** defaults: 8000x8000 world, 400px margin, 2 towns, 1200px min distance, 34px grid spacing, 1100px camp distance, 5 farmers / 2 guards / 5 raiders per town.

**`generate_world()`**: Pure function that takes config and populates both WorldGrid and WorldData. Places towns randomly with min distance constraint, finds camp positions furthest from all towns (16 directions), assigns terrain via simplex noise with Dirt override near settlements, and places buildings per town (1 fountain, 2 farms, 4 beds, 4 guard posts at grid corners).

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

## Selection

| Resource | Data | Purpose |
|----------|------|---------|
| SelectedNpc | `i32` (-1 = none) | Currently selected NPC for inspector panel |

## Test Framework

| Resource | Data | Purpose |
|----------|------|---------|
| AppState | TestMenu \| Running | Gates game systems; menu vs active test |
| TestState | test_name, phase, total_phases, phase_name, results, counters, flags | Shared state for active test |
| TestRegistry | `Vec<TestEntry>` (name, description, phase_count, time_scale) | All registered tests |
| RunAllState | active, queue, results | Sequential test execution state |

`TestState` is reset between tests via `cleanup_test_world` (OnExit Running). `test_is("name")` run condition gates per-test systems.

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
