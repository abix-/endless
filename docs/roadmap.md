# Roadmap

Target: 20,000+ NPCs @ 60fps with pure Bevy ECS + WGSL compute + GPU instanced rendering.

## How to Maintain This Roadmap

This file has three sections:

- **Completed** = what's been built, grouped by system. Reference material for understanding what exists. All `[x]` checkboxes live here.
- **Stages** = what order we build things. Each stage has a "done when" sentence. Future work items live here, grouped by problem. Read top-down — the first unchecked stage is the current sprint.
- **Specs** = detailed implementation plans for complex features. Linked from stages.

Rules:
1. **Stages are the priority.** Read top-down. First unchecked stage is the current sprint.
2. **No duplication.** Each work item lives in exactly one place. Stages have future work. Completed has done work. Specs have implementation detail.
3. **Completed checkboxes are accomplishments.** Never delete them. When a stage is done, move its `[x]` items to Completed and add ✓ to the stage header.
4. **"Done when" sentences don't change** unless the game design changes. They define the goal, not the implementation.
5. **New features** go in the appropriate stage. If no stage fits, add to Stage 12.
6. **Godot lineage breadcrumbs** (like "Port config.gd → Bevy Resource") are intentional — they show where the design originated.

## Completed

### Spawning & Rendering
- [x] NPCs spawn with jobs (guard, farmer, raider, fighter, miner)
- [x] GPU instanced rendering via RenderCommand + Transparent2d (10,000+ @ 140fps)
- [x] Sprite frames, faction colors
- [x] Unified spawn API with job-as-template pattern
- [x] spawn_guard(), spawn_guard_at_post(), spawn_farmer() convenience APIs
- [x] Slot reuse for dead NPCs (SlotAllocator)

### GPU Compute
- [x] npc_compute.wgsl compute shader (3-mode dispatch: clear grid, build grid, movement+targeting)
- [x] wgpu compute pipeline via Bevy render graph
- [x] ECS→GPU buffer writes (NpcBufferWrites + per-field dirty flags)
- [x] GPU→ECS readback (positions + combat targets via Bevy async Readback + ReadbackComplete)
- [x] projectile_compute.wgsl compute shader
- [x] Multi-dispatch NpcComputeNode with 3 bind groups
- [x] atomicAdd for thread-safe grid cell insertion

### Instanced Rendering
- [x] RenderCommand + Transparent2d phase (single instanced draw call)
- [x] 2D camera setup, texture atlas loading (char + world sprites)
- [x] Sprite texture sampling with alpha discard and color tinting
- [x] TilemapChunk terrain + buildings (two layers on 250x250 grid, zero per-frame CPU cost)
- [x] FPS counter overlay (egui, bottom-left, EMA-smoothed)
- [x] Sleep indicator (sleep.png sprite texture on status layer, atlas_id=3.0, scale 16 with white tint)
- [x] Healing halo (heal.png sprite texture on healing layer, atlas_id=2.0, scale 20 with yellow tint)

### Movement & Physics
- [x] GPU compute shader for movement toward targets
- [x] set_target(npc_index, x, y) API for directing NPCs
- [x] Separation physics (boids-style, same-faction 1.5x boost, avoidance clamped to speed×1.5)
- [x] Spatial grid for O(1) neighbor lookups (256x256 cells, 128px each, 48 NPCs/cell max, covers 32,768px)
- [x] Arrival detection with persistent flag
- [x] Lateral steering for blocked NPCs (replaced TCP-style backoff — routes around obstacles at 60% speed)
- [x] Zero-distance fallback (golden angle when NPCs overlap exactly)
- [x] reset() function for scene reload

### Combat
- [x] GPU targeting (nearest enemy within 300px via spatial grid)
- [x] Projectile system (50k projectiles, GPU compute shader)
- [x] Melee attacks (range=150, speed=500, lifetime=0.5s)
- [x] Ranged attacks (range=300, speed=200, lifetime=3.0s)
- [x] Health, damage, death systems
- [x] O(1) entity lookup via NpcEntityMap
- [x] Projectile slot reuse (ProjSlotAllocator)
- [x] Damage flash effect (white overlay, CPU-side decay at 5.0/s)
- [x] Guards have no leash (fight anywhere)
- [x] Alert nearby allies when combat starts

### NPC Behaviors
- [x] Guards: patrol posts clockwise, rest when tired (energy < 50), resume when rested (energy > 80)
- [x] Farmers: work at assigned farm, rest when tired
- [x] Raiders: steal food from farms, flee when wounded, return to camp, recover
- [x] Energy system (drain while active, recover while resting)
- [x] Leash system (disengage if too far from home)
- [x] Flee system (exit combat below HP threshold)
- [x] Wounded rest + recovery system
- [x] 15-minute decision cycles (event-driven override on state changes)
- [x] Building arrival based on sprite size (not pixel coordinates)
- [x] Drift detection (working NPCs pushed off position walk back)
- [x] `rebuild_patrol_routes_system` — rebuilds all guards' patrol routes when WorldData changes (guard post added/removed/reordered)

### Economy
- [x] Food production (farmers generate food per hour)
- [x] Food theft (raiders steal and deliver to camp)
- [x] Respawning (dead NPCs respawn after cooldown via RespawnTimers)
- [x] Per-town food storage (FoodStorage resource)
- [x] GameTime resource (time_scale, pause, hourly tick events)
- [x] GameConfig resource (farmers/guards per town, spawn interval, food per hour)
- [x] PopulationStats resource (alive/working counts per job/clan)
- [x] economy_tick_system (unified hourly economy)
- [x] Miner job type (Job::Miner, brown tint, separate behavior from farmer)
- [x] MinerTarget resource (per-town desired miner count, DragValue UI)
- [x] job_reassign_system (converts idle farmers↔miners to match target)
- [x] Population caps per town (upgradeable)

### World Generation
- [x] Procedural town/farm/bed/guard_post placement (2 towns default, 1200px spacing, random layout)
- [x] Named towns from pool of Florida cities
- [x] WorldGrid (250x250 cells, 32px each, terrain biome + building per cell)
- [x] WorldGenConfig resource (world size, town count, spacing, NPC counts)
- [x] Building grid expansion (6x6 start, expandable to 100x100 via per-tile unlock)
- [x] Spiral building placement (`spiral_slots()` generates positions outward from center, auto-unlocks TownGrid slots)

### World Data
- [x] Towns, farms, beds, guard posts as Bevy resources
- [x] BuildingOccupancy resource (private map + claim/release/is_occupied/count API, replaces FarmOccupancy)
- [x] Worksite trait + generic `find_nearest_free()`/`find_within_radius()`/`find_by_pos()` helpers
- [x] Query APIs: get_town_center, get_camp_position, get_patrol_post
- [x] init_world, add_town/farm/bed/guard_post APIs

### UI Integration
- [x] Click to select NPC or building (click_to_select_system, nearest NPC within 20px, fallback to WorldGrid building cell)
- [x] Name generation ("Adjective Noun" by job)
- [x] NpcMetaCache resource (name, level, xp, trait, town_id, job per NPC)
- [x] NpcEnergyCache resource (energy per NPC)
- [x] NpcsByTownCache resource (per-town NPC lists)
- [x] PopulationStats, KillStats, SelectedNpc resources
- [x] bevy_egui start menu with world config sliders (ui/main_menu.rs)
- [x] Game HUD: population, time, food, kill stats, NPC inspector (ui/game_hud.rs)
- [x] Time controls: Space=pause, +/-=speed, ESC=menu (ui/mod.rs)
- [x] GameConfig + WorldGenConfig Bevy resources (replaces Godot config.gd)
- [x] roster_panel.rs (NPC list with sorting/filtering, select/follow)
- [x] build_menu.rs (right-click context menu: Farm/Bed/GuardPost build, Destroy, Unlock)
- [x] combat_log.rs (event feed with color-coded timestamps, Kill/Spawn/Raid/Harvest)
- [x] upgrade_menu.rs (14 upgrade rows with level/cost, spend food to purchase)
- [x] policies_panel.rs (behavior config with live policy controls wired to TownPolicies resource)
- [x] Keyboard toggles: R=roster, B=build, U=upgrades, P=policies, T=patrols, F=follow
- [x] Building inspector (click building → farm growth/occupancy, spawner NPC status/respawn timer, guard post patrol order/turret, fountain heal radius/food)
- [x] Patrols tab (T) — view and reorder guard post patrol routes, swap buttons mutate WorldData
- [x] Left panel (renamed from right_panel): Roster / Upgrades / Policies / Patrols tabs

### Building System
- [x] Runtime add/remove farm/bed/guard_post (place_building/remove_building with tombstone deletion)
- [x] Slot unlock system (spend food to unlock adjacent grid slots)
- [x] Slot indicators (green "+" empty, dim brackets locked, gold ring town center)
- [x] NPCs claim new buildings (existing decision system finds nearest bed/farm)
- [x] Right-click to build and destroy buildings (context menu)

### Visual Feedback
- [x] Camera uniform buffer (replaces hardcoded CAMERA_POS/VIEWPORT in npc_render.wgsl)
- [x] Camera pan (WASD) and zoom (scroll wheel toward cursor)
- [x] Click-to-select NPC wired to camera transform
- [x] Camera follow selected NPC (F key toggle, WASD cancels follow)
- [x] Target indicator overlay (yellow line + diamond marker to NPC's movement target, blue circle on NPC)
- [x] Multi-layer equipment rendering (see [rendering.md](rendering.md))
- [x] Guards spawn with weapon + helmet layers, raiders with weapon layer
- [x] Projectile instanced pipeline (same RenderCommand pattern as NPC renderer)
- [x] Separate InstanceData buffer for active projectiles
- [x] Health bars (3-color: green/yellow/red, show-when-damaged mode in fragment shader)
- [x] Damage flash in npc_render.wgsl (white overlay on hit, fade out over ~0.2s via CPU-side decay)
- [x] Sleep indicator on resting NPCs (SLEEP_SPRITE on status layer via sync_visual_sprites)
- [x] Healing indicator on healing NPCs (HEAL_SPRITE on healing layer via sync_visual_sprites)
- [x] Carried item icon (food sprite on returning raiders)
- [x] Farm growth state visible (Growing → Ready sprite change via farm-visual test)

### Events
- [x] Death events emitted to CombatLog (Kill kind, NPC name/job/level)
- [x] Spawn events emitted to CombatLog (Spawn kind, skips initial bulk spawn)
- [x] Raid dispatch events emitted to CombatLog (Raid kind, group size)
- [x] Harvest events emitted to CombatLog (Harvest kind, farm index)
- [x] Combat log panel displays events with color coding and filters

### Test Framework
- [x] `AppState` (TestMenu/Running) with bevy_egui menu listing tests with run buttons
- [x] `TestState` shared resource (phase, start, results, passed/failed, counters HashMap, flags HashMap)
- [x] Helper methods: `pass_phase()`, `fail_phase()`, `set_flag()`, `get_flag()`, `inc()`
- [x] `src/tests/mod.rs` with `register_tests(app)` — registers all test setup+tick systems
- [x] Each test file exports `setup` (OnEnter) + `tick` (Update, after Step::Behavior)
- [x] Test results displayed on screen (phase progress, pass/fail, elapsed time)
- [x] Return to test menu after test completes (or on cancel)
- [x] "Run All" button that runs tests sequentially, shows summary
- [x] Move existing Test12 from lib.rs into `src/tests/vertical_slice.rs`
- [x] Game systems gated on `AppState::Running` (don't tick during menu)
- [x] World cleanup on `OnExit(AppState::Running)` (despawn NPCs, reset resources)
- [x] HEALTH_DEBUG, COMBAT_DEBUG resources for diagnostics

### Tests (one file each in `src/tests/`, all pass)
- [x] `movement` — Movement & Arrival (3 phases): spawn, move toward target, arrive
- [x] `guard-patrol` — Guard Patrol Cycle (5 phases): OnDuty → Patrolling → OnDuty → rest → resume
- [x] `farmer-cycle` — Farmer Work Cycle (5 phases): GoingToWork → Working → tired → rest → resume
- [x] `raider-cycle` — Raider Raid Cycle (5 phases): dispatch → arrive → steal → return → deliver
- [x] `combat` — Combat Pipeline (6 phases): targeting → InCombat → projectile → damage → death → cleanup
- [x] `economy` — Farm Growth & Respawn (5 phases): Growing → Ready → harvest → forage → respawn
- [x] `energy` — Energy System (3 phases): start 100 → drain → reach threshold
- [x] `healing` — Healing Aura (3 phases): damaged in zone → heal → full HP
- [x] `spawning` — Spawn & Slot Reuse (4 phases): exist → kill → free slot → reuse slot
- [x] `projectiles` — Projectile Pipeline (4 phases): targeting → spawn proj → hit → free slot
- [x] `world-gen` — World Generation (6 phases): grid → towns → spacing → buildings → terrain → camps
- [x] `vertical-slice` — Full Core Loop (8 phases, time_scale=10)
- [x] `sleep-visual` — Sleep Icon (3 phases): energy > 0 → rest shows SLEEP_SPRITE → wake clears
- [x] `farm-visual` — Farm Ready Marker (3 phases): Growing → Ready spawns marker → harvest despawns
- [x] `heal-visual` — Heal Icon (3 phases): damaged → Healing shows HEAL_SPRITE → healed clears

### Data-Driven Stats
- [x] `CombatConfig` resource with per-job `JobStats` + per-attack-type `AttackTypeStats`
- [x] `systems/stats.rs` with `resolve_combat_stats()` function
- [x] `CachedStats` component on all NPCs — populated on spawn, invalidated on upgrade/level-up
- [x] `BaseAttackType` component (Melee/Ranged) replaces `AttackStats` on entities
- [x] `TownUpgrades` resource with per-town upgrade levels (activated in Stage 9)
- [x] `attack_system` reads `&CachedStats` instead of `&AttackStats`
- [x] `healing_system` reads `CombatConfig.heal_rate`/`heal_radius` instead of local constants
- [x] `MaxHealth` component removed — `CachedStats.max_health` is single source of truth
- [x] `Personality` (4 traits: Brave/Tough/Swift/Focused) wired into `resolve_combat_stats()`. Display `trait_id` uses separate 9-name list — unification in Stage 14
- [x] Init values match hardcoded values: guard/raider damage=15, speeds=100, max_health=100, heal_rate=5, heal_radius=150
- [x] Stage 8 parity checks verified stats matched hardcoded values (removed in Stage 9)

### Settings & Config
- [x] User settings persistence (serde JSON, scroll speed + world gen sliders)
- [x] Cross-platform settings path (USERPROFILE on Windows, HOME fallback on macOS/Linux)
- [x] Main menu DragValue widgets alongside sliders for typeable config inputs

### Guard Post Turrets
- [x] Guard post auto-attack (turret behavior, fires projectiles at enemies within 250px)
- [x] Guard post turret toggle (enable/disable via right-click build menu)

### Upgrades & XP
- [x] `UpgradeQueue` resource + `process_upgrades_system`: drains queue, validates food cost, increments `TownUpgrades`, re-resolves `CachedStats` for affected NPCs
- [x] `upgrade_cost(level) -> i32` = `10 * 2^level` (doubles each level, capped at 20)
- [x] Wire upgrade multipliers into `resolve_combat_stats()` via `UPGRADE_PCT` array
- [x] Enable `upgrade_menu.rs` buttons: click → push to `UpgradeQueue` → deduct food → increment level
- [x] Guard upgrades: health (+10%), attack (+10%), range (+5%), size (+5%), attack speed (-8%), move speed (+5%), alert radius (+10%)
- [x] Farm upgrades: yield (+15%), farmer HP (+20%), farmer cap (+2 flat)
- [x] Town upgrades: guard cap (+10 flat), healing rate (+20%), food efficiency (10%), fountain radius (+24px flat)
- [x] AttackSpeed upgrade uses reciprocal cooldown scaling: `1/(1+level*pct)` — asymptotic, never reaches zero
- [x] `farm_growth_system` applies FarmYield upgrade per-town via `TownUpgrades`
- [x] `healing_system` applies HealingRate + FountainRadius upgrades per-town
- [x] `level_from_xp(xp) -> i32` = `floor(sqrt(xp / 100))`, `level_multiplier = 1.0 + level * 0.01`
- [x] Wire level multiplier into `resolve_combat_stats()`
- [x] `xp_grant_system`: last-hit tracking via `DamageMsg.attacker` → `LastHitBy` component → grant 100 XP to killer on death
- [x] Level-up → `CombatLog` event (LevelUp kind, cyan color), rescale current HP proportionally to new max
- [x] `game_hud.rs` NPC inspector shows level, XP, XP-to-next-level
- [x] Fix `starvation_system` speed: uses `CachedStats.speed * STARVING_SPEED_MULT` instead of hardcoded 60.0

### Town Policies
- [x] `TownPolicies` resource with `PolicySet` per town (eat_food, flee thresholds, work schedule, off-duty behavior)
- [x] `WorkSchedule` enum (Both, DayOnly, NightOnly) gates work scoring in decision_system
- [x] `OffDutyBehavior` enum (GoToBed, StayAtFountain, WanderTown) drives idle behavior off-schedule
- [x] `policies_panel.rs` wired to `ResMut<TownPolicies>` — sliders/checkboxes directly mutate resource
- [x] Policy-driven flee: guards use `guard_flee_hp`, farmers use `farmer_flee_hp`, raiders hardcoded 0.50
- [x] `guard_aggressive` disables guard flee, `farmer_fight_back` disables farmer flee
- [x] `guard_leash` policy controls whether guards return to post after combat
- [x] `prioritize_healing` sends wounded NPCs to fountain before resuming work
- [x] Removed hardcoded `FleeThreshold`/`WoundedThreshold` from raider spawn — thresholds policy-driven

### Building Spawners
- [x] `Building::House { town_idx }` and `Building::Barracks { town_idx }` variants in `world.rs`
- [x] `House`/`Barracks` structs in `WorldData`, `BUILDING_TILES` extended 5→7
- [x] Wire `place_building()`/`remove_building()` for House/Barracks (same tombstone pattern)
- [x] World gen: `place_town_buildings()` places N Houses + N Barracks from config sliders
- [x] `SpawnerEntry` struct: `building_kind`, `town_idx`, `position`, `npc_slot` (-1=none), `respawn_timer`
- [x] `SpawnerState` resource: `Vec<SpawnerEntry>` — one entry per House/Barracks
- [x] `spawner_respawn_system` in `systems/economy.rs` (Step::Behavior, hourly): detects dead NPC via `NpcEntityMap`, starts 12h timer, spawns replacement when timer expires
- [x] House + Barracks buttons in `build_menu.rs` (push `SpawnerEntry` on build)
- [x] Sliders renamed to Houses/Barracks (kept for testing, control world gen building count)
- [x] HUD shows spawner counts: `Farmers: alive/houses` / `Guards: alive/barracks`
- [x] `game_startup_system` builds `SpawnerState` from world gen Houses/Barracks, spawns 1 NPC per entry (instant, no timer)
- [x] Replaced bulk farmer/guard spawn loops with spawner-based spawn — raider spawn loop kept
- [x] `.init_resource::<SpawnerState>()`, add `spawner_respawn_system` to Step::Behavior
- [x] Remove beds — NPCs rest at their spawner building (House/Barracks) instead of separate beds. Home = spawner position.

### GPU Performance
- [x] Replace hand-rolled readback with Bevy's `Readback` + `ReadbackComplete` (eliminates 9ms blocking `device.poll`)
- [x] Eliminate `GPU_READ_STATE`, `PROJ_HIT_STATE`, `PROJ_POSITION_STATE` static Mutexes (replaced by `ReadbackComplete` events → Bevy Resources)
- [x] Convert 4 readback compute buffers to `ShaderStorageBuffer` assets with `COPY_SRC`

### Continent World Generation
- [x] `WorldGenStyle` enum (Classic/Continents) in `WorldGenConfig`, selectable from main menu combo box
- [x] 3-octave fBm elevation noise (freq 0.0008/0.0016/0.0032) + square-bump edge falloff + power redistribution
- [x] Independent moisture noise (freq 0.003) for biome selection: dry→Rock, moderate→Grass, wet→Forest
- [x] Town/camp placement constrained to land cells in Continents mode (5000 max attempts)
- [x] `stamp_dirt()` clears terrain around settlements after placement
- [x] Setting persisted in UserSettings as `gen_style: u8`

### Architecture
- [x] Bevy Messages (MessageWriter/MessageReader) for all inter-system communication
- [x] All state as Bevy Resources (WorldData, Debug, KillStats, NpcMeta, FoodEvents, etc.)
- [x] GpuUpdateMsg batching via collect_gpu_updates

### In-Game Help
- [x] `HelpCatalog` resource (~35 entries, HashMap keyed by topic ID)
- [x] `help_tip()` helper: small "?" button with rich tooltip on hover
- [x] Top bar tips (getting started, food, gold, pop, farmers, guards, raiders)
- [x] Left panel tab tips (roster, upgrades, policies, patrols, squads, intel, profiler)
- [x] Build menu hover text (farm, house, barracks, guard post, tent)
- [x] NPC inspector tips (level/xp, trait, energy, state)

## Stages

**Stage 1: Standalone Bevy App ✓**

*Done when: Bevy app launches, spawns NPC entities with job components, and runs an Update loop.*

**Stage 2: GPU Compute ✓**

*Done when: compute shader dispatches movement, targeting, and spatial grid — positions read back to ECS every frame.*

**Stage 3: GPU Instanced Rendering ✓**

*Done when: 10,000+ NPCs render at 140fps via a single instanced draw call using the RenderCommand pattern.*

**Stage 4: Core Loop ✓**

*Done when: 5 farmers grow food, 5 raiders form a group and steal it, 2 guards intercept with combat, someone dies, slot recycles, replacement spawns. Validated by Test 12 (8 phases, 6.8s).*

**Stage 5: Test Framework ✓**

*Done when: every completed system has a dedicated test, tests are selectable from an in-game menu, and all tests pass.*

**Stage 6: Visual Feedback** ✓

*Done when: you can watch the core loop happen on screen and understand what's going on without reading logs.*

**Stage 7: Playable Game** ✓

*Done when: someone who isn't you can open it, understand what's happening, and make decisions that affect the outcome.*

**Stage 8: Data-Driven Stats** ✓ (see [spec](#stat-resolution--upgrades))

*Done when: all NPC stats resolve from `CombatConfig` resource via `resolve_combat_stats()`. Game plays identically — pure refactor, no behavior change. All existing tests pass.*

**Stage 9: Upgrades & XP** ✓ (see [spec](#stat-resolution--upgrades))

*Done when: player can spend food on upgrades in the UI, guards with upgrades visibly outperform unupgraded ones, and NPCs gain levels from kills.*

**Stage 10: Town Policies** ✓

*Done when: changing a policy slider immediately alters NPC behavior — raiders flee at the configured HP%, farmers sleep during night shift, off-duty guards wander to the fountain.*

**Stage 11: Building Spawners** ✓ (see [spec](#building-spawners))

*Done when: each House supports 1 farmer, each Barracks supports 1 guard. Killing the NPC triggers a 12-hour respawn timer on the building. Player builds more Houses/Barracks to grow population. Menu sliders for farmers/guards removed.*

**Stage 12: AI Players** (see [spec](#ai-players))

*Done when: the player is one town in a sea of hostile AI — enemy towns build farms/guards and grow their economy, raider camps build tents and send raids, all factions fight each other, and the AI decision speed is configurable from the main menu.*

- [x] `AiPlayerConfig` resource (decision interval in real seconds, configurable from main menu)
- [x] `AiPlayerState` resource with `Vec<AiPlayer>` — one per AI settlement
- [x] `AiKind::Raider` AI: builds tents, unlocks slots, buys AttackSpeed/MoveSpeed upgrades
- [x] `AiKind::Builder` AI: builds farms/houses/barracks/guard posts, buys all upgrade types
- [x] World gen: independent placement of player towns, AI towns, and raider camps (not paired)
- [x] Main menu sliders: AI Towns (0-10), Raider Camps (0-10), AI Speed (1-30s)
- [x] Fix faction hardcoding: `spawner_respawn_system` + `game_startup_system` use town faction instead of 0
- [x] Fix `NpcsByTownCache` initialization (resize to num_towns in `game_startup_system`)

**Stage 13: In-game Help** ✓

*Done when: a new player can hover any UI element and understand what it does and how to use it, without reading external docs.*

- [x] `HelpCatalog` resource with ~35 help entries (topic key → help text)
- [x] `help_tip()` helper renders "?" button with rich tooltip on hover
- [x] Top bar: "?" getting started tip + tips on Food, Gold, Pop, Farmers, Guards, Raiders
- [x] Left panel: context help tip at top of every tab (Roster, Upgrades, Policies, Patrols, Squads, Intel, Profiler)
- [x] Build menu: rich hover text on every build button (Farm, House, Barracks, Guard Post, Tent)
- [x] NPC inspector: tips on Level/XP, Trait, Energy, State

**Stage 13: Tension**

*Done when: a player who doesn't build or upgrade loses within 30 minutes — raids escalate, food runs out, town falls.*

- [ ] Raid escalation: wave size and stats increase every N game-hours
- [ ] Differentiate job base stats (raiders hit harder, guards are tankier, farmers are fragile)
- [ ] Food consumption: NPCs eat hourly, eating restores HP/energy
- [ ] Starvation effects: no food → HP drain, speed penalty, desertion
- [ ] Loss condition: all town NPCs dead + no spawners → game over screen
- [ ] Building costs rebalanced (everything=1 is not an economy)

**Stage 14: Performance**

*Done when: `NpcBufferWrites` ExtractResource clone drops from 18ms to <5ms, and `command_buffer_generation_tasks` drops from ~10ms to ~1ms at default zoom on a 250×250 world.*

GPU extract optimization (see [spec](#gpu-readback--extract-optimization)):
- [ ] Split `NpcBufferWrites` (1.9MB) into `NpcComputeWrites` (~460KB) + `NpcVisualData` (~1.4MB static)
- [ ] `NpcVisualData` bypasses ExtractResource via static Mutex (render world reads directly)

Chunked tilemap (see [spec](#chunked-tilemap)):
- [ ] Split single 250×250 TilemapChunk per layer into 32×32 tile chunks
- [ ] Bevy frustum-culls off-screen chunk entities — only visible chunks generate draw commands
- [ ] `sync_building_tilemap` updates only chunks whose grid region changed, not all 62K+ tiles

Entity sleeping:
- [ ] Entity sleeping (Factorio-style: NPCs outside camera radius sleep)

SystemParam bundle consolidation:
- [ ] Create `GameLog` bundle in `resources.rs`: `{ combat_log: ResMut<CombatLog>, game_time: Res<GameTime>, timings: Res<SystemTimings> }`. This triple appears in 8+ systems: `arrival_system`, `spawn_npc_system`, `death_cleanup_system`, `spawner_respawn_system`, `ai_decision_system`, `xp_grant_system`, `healing_system`, `process_upgrades_system`. Each system drops its 3 direct params in favor of `log: GameLog`.
- [ ] Move `FarmParams` and `EconomyParams` from `systems/behavior.rs` to `resources.rs` (they're `pub` but only imported by behavior.rs today). Update imports in behavior.rs.
- [ ] Adopt `FarmParams` + `EconomyParams` in `arrival_system` (13→8 params): replace direct `farm_states`, `world_data`, `food_storage`, `gold_storage`, `food_events` with the two bundles. Access via `farms.states`, `economy.food_storage`, etc.
- [ ] Do NOT refactor systems where `WorldData` mutability mismatches — `ai_decision_system` and `build_menu_system` need `ResMut<WorldData>` but `FarmParams` has `Res<WorldData>`. Leave those as-is.
- [ ] Do NOT nest bundles (e.g. `GameLog` inside `DecisionExtras`). Flat bundles only.
- [ ] Expected param count reductions: `arrival_system` 13→8, `spawn_npc_system` 15→13, `death_cleanup_system` 9→7, `spawner_respawn_system` 9→7, `ai_decision_system` 15→13, `xp_grant_system` 10→8, `healing_system` 10→8, `process_upgrades_system` 10→9. Pure refactor — no behavioral changes. Verify with `cargo check`.

**Stage 15: Combat Depth**

*Done when: two guards with different traits fight the same raider noticeably differently — one flees early, the other berserks at low HP.*

- [ ] Unify `TraitKind` (4 variants) and `trait_name()` (9 names) into single 9-trait Personality system
- [ ] All 9 traits affect both `resolve_combat_stats()` and `decision_system` behavior weights
- [ ] Trait combinations (multiple traits per NPC)
- [ ] Target switching (prefer non-fleeing enemies, prioritize low-HP targets)

**Stage 16: Walls & Defenses**

*Done when: player builds a stone wall perimeter with a gate, raiders path around it or attack through it, chokepoints make guard placement strategic.*

- [ ] Wall building type (straight segments on grid, connects to adjacent walls)
- [ ] Wall HP + raiders attack walls blocking their path to farms
- [ ] Gate building (walls with a passthrough that friendlies use, raiders must breach)
- [ ] Pathfinding update: raiders route around walls to find openings, attack walls when no path exists
- [ ] Guard towers (upgrade from guard post — elevated, +range, requires wall adjacency)

**Stage 17: Save/Load**

*Done when: player builds up a town for 20 minutes, quits, relaunches, and continues exactly where they left off — NPCs in the same positions, same HP, same upgrades, same food.*

- [ ] Serialize full game state (WorldData, SpawnerState, TownUpgrades, TownPolicies, FoodStorage, GameTime, NPC positions/states/stats)
- [ ] Save to JSON file, load from main menu
- [ ] Autosave every N game-hours
- [ ] Save slot selection (3 slots)

**Stage 18: Loot & Equipment**

*Done when: raider dies → drops loot bag → guard picks it up → item appears in town inventory → player equips it on a guard → guard's stats increase and sprite changes.*

- [ ] `LootItem` struct: slot (Weapon/Armor), stat bonus (damage% or armor%)
- [ ] Raider death → chance to drop `LootBag` entity at death position (30% base rate)
- [ ] Guards detect and collect nearby loot bags (priority above patrol, below combat)
- [ ] `TownInventory` resource, inventory UI tab
- [ ] `Equipment` component: weapon + armor slots, feeds into `resolve_combat_stats()`
- [ ] Equipped items reflected in NPC equipment sprite layers

**Stage 19: Tech Trees**

*Done when: player researches "Iron Working" which unlocks Barracks Lv2 and Guard damage upgrade tier 2 — visible tech tree with branching paths and resource costs.*

- [ ] `TechTree` resource with node graph (prereqs, cost, unlock effects)
- [ ] Research building (Library/Workshop — 1 per town, consumes food over time to research)
- [ ] Tech nodes unlock: new buildings, upgrade tiers, new unit types, passive bonuses
- [ ] 3 branches: Military (guards/combat), Agriculture (farms/food), Industry (walls/buildings)
- [ ] UI: tech tree viewer tab in left panel

**Stage 20: Economy Depth**

*Done when: player must choose between feeding NPCs and buying upgrades — food is a constraint, not a score.*

- [ ] HP regen tiers (1x idle, 3x sleeping, 10x fountain)
- [ ] FoodEfficiency upgrade wired into `decision_system` eat logic
- [ ] Economy pressure: upgrades cost more food, NPCs consume more as population grows

**Stage 21: Diplomacy**

*Done when: a raider camp sends a messenger offering a truce for 3 food/hour tribute — accepting stops raids, refusing triggers an immediate attack wave.*

- [ ] Camp reputation system (hostile → neutral → friendly, based on food tribute and combat history)
- [ ] Tribute offers: camps propose truces at reputation thresholds
- [ ] Trade routes between player towns (send food caravan from surplus town to deficit town)
- [ ] Allied camps stop raiding, may send fighters during large attacks
- [ ] Betrayal: allied camps can turn hostile if tribute stops or player is weak

**Stage 22: World Generation** ✓ (see [spec](#continent-world-generation))

*Done when: player selects "Continents" from main menu, sees landmasses with ocean, towns only on land, biome variety across continents.*

- [x] `WorldGenStyle` enum: Classic (current) / Continents (multi-octave fBm elevation + moisture noise with land/ocean)
- [x] 3-octave elevation fBm + square-bump edge falloff + independent moisture noise for biome selection
- [x] Town/camp placement constrained to land cells in Continents mode (5000 max attempts)
- [x] Main menu combo box to select generation style, persisted in UserSettings

**Stage 23: Resources & Jobs**

*Done when: player builds a lumber mill near Forest tiles, assigns a woodcutter, collects wood, and builds a stone wall using wood + stone instead of food — multi-resource economy with job specialization.*

- [x] Gold mines: wilderness resource nodes placed between towns, unowned (any faction), slow regen, mining_pct policy slider, AI personality allocation
- [ ] Resource types: wood (Forest biome), stone (Rock biome), iron (ore nodes, rare)
- [ ] Harvester buildings: lumber mill, quarry (same spawner pattern as House/Barracks, 1 worker each)
- [ ] Resource storage per town (like FoodStorage but for each type — gold already done via GoldStorage)
- [ ] Building costs use mixed resources (walls=stone, barracks=wood+stone, upgrades=food+iron, etc.)
- [ ] Crafting: blacksmith building consumes iron → produces weapons/armor (feeds into Stage 18 loot system)
- [ ] Villager job assignment UI (drag workers between roles — farming, woodcutting, mining, smithing, military)

**Stage 24: Armies & Marching**

*Done when: player recruits 15 guards into an army, gives a march order to a neighboring camp, and the army walks across the map as a formation — arriving ready to fight.*

- [ ] Army formation from existing squads (select squad → "Form Army" → army entity with member list)
- [ ] March orders: right-click map location → army walks as group (use existing movement system, group speed = slowest member)
- [ ] Unit types via tech tree unlocks: levy (cheap, weak), archer (ranged), men-at-arms (tanky, expensive)
- [ ] Army supply: marching armies consume food from origin town's storage, starve without supply
- [ ] Field battles: two armies in proximity → combat triggers (existing combat system handles it)

**Stage 25: Conquest**

*Done when: player marches an army to a raider camp, defeats defenders, and claims the town — camp converts to player-owned town with buildings intact, player now manages two towns.*

- [ ] Camp/town siege: army arrives at hostile settlement → attacks defenders + buildings
- [ ] Building HP: walls, barracks, houses have HP — attackers must breach defenses
- [ ] Town capture: all defenders dead + town center HP → 0 = captured → converts to player town
- [ ] AI expansion: AI players can attack each other and the player (not just raid — full conquest attempts)
- [ ] Victory condition: control all settlements on the map

**Stage 26: World Map**

*Done when: player conquers all towns on "County of Palm Beach", clicks "Next Region" on the world map, and starts a new county with harder AI and more camps — campaign progression.*

- [ ] World map screen: grid of regions (counties), each is a separate game map
- [ ] Region difficulty scaling (more camps, tougher AI, scarcer resources)
- [ ] Persistent bonuses between regions (tech carries over, starting resources from tribute)
- [ ] "Country" = set of regions. "World" = set of countries. Campaign arc.

**Stage 27: Tower Defense (Wintermaul Wars-inspired)**

*Done when: player builds towers in a maze layout to shape enemy pathing, towers have elemental types with rock-paper-scissors counters, income accrues with interest, and towers upgrade/evolve into advanced forms.*

Maze building:
- [ ] Open-field tower placement on a grid (towers block pathing, enemies path around them)
- [ ] Pathfinding recalculation on tower place/remove (A* or flow field on grid)
- [ ] Maze validation — path from spawn to goal must always exist (reject placements that fully block)
- [ ] Visual path preview (show calculated enemy route through current maze)

Elemental rock-paper-scissors:
- [ ] `Element` enum: Fire, Ice, Nature, Lightning, Arcane, Dark (6 elements)
- [ ] Element weakness matrix (Fire→Nature→Lightning→Ice→Fire, Arcane↔Dark)
- [ ] Creep waves carry an element — weak-element towers deal 2x, strong-element towers deal 0.5x
- [ ] Tower/creep element shown via tint or icon overlay

Income & interest:
- [ ] Per-wave gold income (base + bonus for no leaks)
- [ ] Interest on banked gold each wave (5% per round, capped)
- [ ] Leak penalty — lives lost per creep that reaches the goal

Sending creeps:
- [ ] Spend gold to send extra creeps into opponent's lane
- [ ] Send menu with creep tiers (cheap/fast, tanky, elemental, boss)
- [ ] Income bonus from sending (reward aggressive play)

Tower upgrades & evolution:
- [ ] Multi-tier upgrade path (Lv1 → Lv2 → Lv3, increasing stats + visual change)
- [ ] At max tier, evolve into specialized variants (e.g. Fire Lv3 → Inferno AoE or Sniper Flame)
- [ ] Evolved towers get unique abilities (slow, DoT, chain lightning, lifesteal)

Sound (bevy_audio) should be woven into stages as they're built — not deferred to a dedicated stage.

## Specs

### GPU Readback & Extract Optimization

Steps 1-6 (completed) replaced hand-rolled staging buffers with Bevy's async `Readback` + `ReadbackComplete` pattern — see [gpu-compute.md](gpu-compute.md) and [messages.md](messages.md). What remains is reducing the ExtractResource clone cost.

**Problem: ExtractResource clone (18ms)**

`NpcBufferWrites` is 1.9MB (15 Vecs × 16384 slots) cloned every frame via `ExtractResourcePlugin`. Only ~460KB is compute upload data (positions, targets, speeds, factions, healths, arrivals). The remaining ~1.4MB is render-only visual data (sprite_indices, colors, flash_values, 6 equipment/status sprite arrays) written by `sync_visual_sprites` and read by `prepare_npc_buffers`.

**Step 7 — Split `NpcBufferWrites` to reduce extract clone:**
- [ ] Create `NpcVisualData` struct with: `sprite_indices`, `colors`, `flash_values`, `armor_sprites`, `helmet_sprites`, `weapon_sprites`, `item_sprites`, `status_sprites`, `healing_sprites` (~1.4MB)
- [ ] Create `static NPC_VISUAL_DATA: Mutex<NpcVisualData>` (same pattern as existing `GPU_READ_STATE`)
- [ ] `sync_visual_sprites` writes to `NPC_VISUAL_DATA.lock()` instead of `NpcBufferWrites`
- [ ] `prepare_npc_buffers` (render world) reads from `NPC_VISUAL_DATA.lock()` instead of extracted `NpcBufferWrites`
- [ ] Remaining `NpcComputeWrites` keeps: positions, targets, speeds, factions, healths, arrivals + dirty indices (~50KB with sparse dirty data)
- [ ] Change `ExtractResourcePlugin::<NpcBufferWrites>` to `ExtractResourcePlugin::<NpcComputeWrites>`
- [ ] Update `write_npc_buffers` to read from `Res<NpcComputeWrites>`

**Files changed:**

| File | Changes |
|---|---|
| `rust/src/gpu.rs` | Split NpcBufferWrites → NpcComputeWrites + NpcVisualData |
| `rust/src/npc_render.rs` | `prepare_npc_buffers` reads visual data from `NPC_VISUAL_DATA` static |

## Performance

| Milestone | NPCs | FPS | Status |
|-----------|------|-----|--------|
| CPU Bevy | 5,000 | 60+ | ✓ |
| GPU physics | 10,000+ | 140 | ✓ |
| Full behaviors | 10,000+ | 140 | ✓ |
| Combat + projectiles | 10,000+ | 140 | ✓ |
| GPU spatial grid | 10,000+ | 140 | ✓ |
| Full game integration | 10,000 | 130 | ✓ |
| Max scale tested | 50,000 | TBD | ✓ buffers sized |
| Future (chunked tilemap) | 50,000+ | 60+ | Planned |

## Game Design Reference

### Personality Traits
40% of NPCs spawn with a trait. Effects:

| Trait | Effect |
|-------|--------|
| Brave | Never flees |
| Coward | Flees at +20% higher HP threshold |
| Efficient | +25% farm yield, -25% attack cooldown |
| Hardy | +25% max HP |
| Lazy | -20% farm yield, +20% attack cooldown |
| Strong | +25% damage |
| Swift | +25% move speed |
| Sharpshot | +25% attack range |
| Berserker | +50% damage below 50% HP |

### NPC States

| State | Jobs | Description |
|-------|------|-------------|
| Idle | All | Between decisions |
| Resting | All | At home/camp, recovering energy |
| Off Duty | All | At home/camp, awake |
| Fighting | Guard, Raider | In combat |
| Fleeing | All | Running from combat |
| Walking | Farmer, Guard | Moving to destination |
| Working | Farmer | At farm, producing food |
| On Duty | Guard | Stationed at post |
| Patrolling | Guard | Moving between posts |
| Raiding | Raider | Going to/at farm to steal |
| Returning | Raider | Heading back to camp |
| Wandering | Farmer, Guard | Off-duty wandering |

### Chunked Tilemap

The world tilemap is spawned as one giant `TilemapChunk` entity per layer (terrain + buildings). At 250×250 that's 62,500 tiles per layer, all processed every frame for draw command generation even when most are off-screen. At 1000×1000 it's 1M tiles. Bevy can only skip entities whose bounding box is fully off-screen — one entity = no culling.

**Fix:** split into 32×32 tile chunks (Factorio-style). 250×250 → 8×8 = 64 chunks/layer. 1000×1000 → 32×32 = 1,024 chunks/layer. At typical zoom, only ~4-6 chunks are visible, so draw command generation drops from O(all tiles) to O(visible tiles).

**File: `rust/src/render.rs`**

Constants:
```rust
const CHUNK_SIZE: usize = 32;
```

Components — add grid origin to `BuildingChunk` (for sync):
```rust
#[derive(Component)]
pub struct BuildingChunk {
    pub origin_x: usize,
    pub origin_y: usize,
    pub chunk_w: usize,  // may be < 32 for edge chunks
    pub chunk_h: usize,
}
```

`spawn_world_tilemap` — replace single chunk spawn with nested loop:
```
for chunk_y in (0..grid.height).step_by(CHUNK_SIZE)
  for chunk_x in (0..grid.width).step_by(CHUNK_SIZE)
    cw = min(CHUNK_SIZE, grid.width - chunk_x)
    ch = min(CHUNK_SIZE, grid.height - chunk_y)
    // Extract tile data: iterate ly in 0..ch, lx in 0..cw
    //   grid index = (chunk_y + ly) * grid.width + (chunk_x + lx)
    // Terrain: TileData::from_tileset_index(cell.terrain.tileset_index(gi))
    // Building: cell.building.map(|b| TileData::from_tileset_index(b.tileset_index()))
    // Transform center = ((chunk_x + cw/2) * cell_size, (chunk_y + ch/2) * cell_size, z)
    // Spawn TilemapChunk with chunk_size = UVec2(cw, ch)
    // Building chunks get BuildingChunk { origin_x, origin_y, chunk_w, chunk_h }
```

`sync_building_tilemap` — each chunk re-reads only its sub-region:
```rust
fn sync_building_tilemap(
    grid: Res<WorldGrid>,
    mut chunks: Query<(&mut TilemapChunkTileData, &BuildingChunk)>,
) {
    if !grid.is_changed() || grid.width == 0 { return; }
    for (mut tile_data, chunk) in chunks.iter_mut() {
        for ly in 0..chunk.chunk_h {
            for lx in 0..chunk.chunk_w {
                let gi = (chunk.origin_y + ly) * grid.width + (chunk.origin_x + lx);
                let li = ly * chunk.chunk_w + lx;
                tile_data.0[li] = grid.cells[gi].building.as_ref()
                    .map(|b| TileData::from_tileset_index(b.tileset_index()));
            }
        }
    }
}
```

Cleanup (`ui/mod.rs:500`): already queries `Entity, With<TilemapChunk>` and despawns all — works unchanged with multiple chunks.

`spawn_chunk` helper: can be removed or inlined — no longer needed as a separate function since the loop body handles everything.

**Tileset handles:** `build_tileset()` returns a `Handle<Image>`. Clone it for each chunk — Bevy ref-counts texture assets, so all chunks share the same GPU texture.

**Verification:**
1. Build and run, pan camera — no gaps or offset errors at chunk boundaries
2. Place a building — appears correctly (sync still works)
3. Zoom out fully — all chunks visible, slight FPS drop expected vs close zoom
4. Tracy: `command_buffer_generation_tasks` should drop from ~10ms to ~1ms at default zoom
5. New game / restart — chunks despawn and respawn correctly

### Continent World Generation

Add a selectable world generation style: **Classic** (current single-noise behavior) and **Continents** (multi-layer noise producing landmasses surrounded by ocean). Both styles available from the main menu, persisted in settings.

**`WorldGenStyle` enum** (`world.rs`):

```rust
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldGenStyle {
    #[default]
    Classic,
    Continents,
}
```

Add `pub gen_style: WorldGenStyle` to `WorldGenConfig`. Default = Classic (no behavior change for existing players).

**`generate_world()` branching** (`world.rs`):

Classic mode (existing flow, unchanged):
1. Init grid
2. Place town centers (random with min distance)
3. Find camp positions
4. `generate_terrain()` — single Simplex noise + Dirt near towns/camps
5. Place buildings

Continents mode (new flow):
1. Init grid
2. `generate_terrain_continents()` — multi-layer noise, no Dirt yet
3. Place town centers — **constrained to land cells** (reject Water positions)
4. Find camp positions — also constrained to land
5. `stamp_dirt()` — overwrite terrain near towns/camps with Dirt
6. Place buildings

**`generate_terrain_continents()`** (`world.rs`, new function):

Uses two Simplex noise layers with different seeds + an edge distance falloff.

```rust
fn generate_terrain_continents(grid: &mut WorldGrid) {
    use noise::{NoiseFn, Simplex};

    let continental = Simplex::new(rand::random::<u32>());
    let biome_noise = Simplex::new(rand::random::<u32>());

    let cont_freq: f64 = 0.0008;  // very low — big blobs
    let biome_freq: f64 = 0.004;  // medium — biome regions
    let world_w = grid.width as f32 * grid.cell_size;
    let world_h = grid.height as f32 * grid.cell_size;

    for row in 0..grid.height {
        for col in 0..grid.width {
            let world_pos = grid.grid_to_world(col, row);

            // Edge falloff: 0.0 at center, 1.0 at edges
            let dx = (world_pos.x / world_w - 0.5) * 2.0; // [-1, 1]
            let dy = (world_pos.y / world_h - 0.5) * 2.0;
            let edge = dx.abs().max(dy.abs()); // square falloff
            // smoothstep: 0 below 0.6, ramps to 1 at 1.0
            let t = ((edge - 0.6) / 0.4).clamp(0.0, 1.0);
            let falloff = t * t * (3.0 - 2.0 * t);

            // Continental shelf value
            let c = continental.get([
                world_pos.x as f64 * cont_freq,
                world_pos.y as f64 * cont_freq,
            ]);
            let c = c as f32 - falloff * 1.5; // push edges to ocean

            let biome = if c < -0.05 {
                Biome::Water
            } else {
                let n = biome_noise.get([
                    world_pos.x as f64 * biome_freq,
                    world_pos.y as f64 * biome_freq,
                ]) as f32;
                if n < -0.2 {
                    Biome::Grass
                } else if n < 0.25 {
                    Biome::Forest
                } else {
                    Biome::Rock
                }
            };

            grid.cells[row * grid.width + col].terrain = biome;
        }
    }
}
```

Key parameters to tune visually:
- `cont_freq` (0.0008): lower = bigger continents, higher = more fragmented islands
- `biome_freq` (0.004): lower = bigger biome regions ("countries"), higher = more varied
- ocean threshold (-0.05): lower = more land, higher = more ocean
- falloff start (0.6): lower = ocean starts further from edges, higher = more center land
- falloff strength (1.5): higher = stronger push to ocean at edges

**`stamp_dirt()`** (`world.rs`, new function):

Extracted from existing `generate_terrain()` Dirt override logic. Both modes call this after town placement.

```rust
fn stamp_dirt(
    grid: &mut WorldGrid,
    town_positions: &[Vec2],
    camp_positions: &[Vec2],
) {
    let town_clear_radius = 6.0 * grid.cell_size;
    let camp_clear_radius = 5.0 * grid.cell_size;

    for row in 0..grid.height {
        for col in 0..grid.width {
            let world_pos = grid.grid_to_world(col, row);
            let near_town = town_positions.iter().any(|tc| world_pos.distance(*tc) < town_clear_radius);
            let near_camp = camp_positions.iter().any(|cp| world_pos.distance(*cp) < camp_clear_radius);
            if near_town || near_camp {
                grid.cells[row * grid.width + col].terrain = Biome::Dirt;
            }
        }
    }
}
```

**Town placement land constraint** (Continents mode only):

In the town placement loop, after generating a random position, check terrain:

```rust
// Inside the while loop that places towns
let (gc, gr) = grid.world_to_grid(pos);
if let Some(cell) = grid.cell(gc, gr) {
    if cell.terrain == Biome::Water {
        continue; // reject, try again
    }
}
```

Same constraint for camp positions — `find_camp_position()` should also reject Water cells. Add an optional `&WorldGrid` parameter (or make it a separate Continents-mode camp finder).

Increase `max_attempts` for Continents mode (e.g., 5000) since many random positions will land in ocean.

**Main menu** (`ui/main_menu.rs`):

Add `gen_style: i32` to `MenuState` (0=Classic, 1=Continents). Add a combo box:

```rust
ui.horizontal(|ui| {
    ui.label("World gen:");
    egui::ComboBox::from_id_salt("gen_style")
        .selected_text(match state.gen_style {
            1 => "Continents",
            _ => "Classic",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut state.gen_style, 0, "Classic");
            ui.selectable_value(&mut state.gen_style, 1, "Continents");
        });
});
```

On Play: `wg_config.gen_style = if state.gen_style == 1 { WorldGenStyle::Continents } else { WorldGenStyle::Classic };`

**Settings** (`settings.rs`):

Add `#[serde(default)] pub gen_style: u8` to `UserSettings`. Map 0↔Classic, 1↔Continents. The `serde(default)` ensures old settings files still load (defaults to 0 = Classic).

**Existing `generate_terrain()` — no changes.** Classic mode calls it exactly as before. It stays the default path.

**Tests** (`tests/world_gen.rs`):

No changes needed. Test uses default `WorldGenConfig` which defaults to Classic mode. All 6 phases (grid dims, town count, spacing, buildings, terrain=Dirt near towns, camps) pass unchanged.

Optional: add a 2nd test `world-gen-continents` that sets `gen_style = Continents` and validates:
- Phase 1: grid dimensions (same)
- Phase 2: town count (same)
- Phase 3: town centers are on land (terrain != Water at town center)
- Phase 4: buildings (same)
- Phase 5: terrain at town center is Dirt (same — stamp_dirt runs)
- Phase 6: ocean exists (count Water cells > 10% of total)

**Files changed:**

| File | Changes |
|---|---|
| `world.rs` | `WorldGenStyle` enum, add to `WorldGenConfig`, branch in `generate_world()`, new `generate_terrain_continents()`, new `stamp_dirt()`, land constraint in town placement |
| `settings.rs` | `gen_style: u8` field in `UserSettings` |
| `ui/main_menu.rs` | `gen_style` in `MenuState`, combo box UI, write to config + settings |

**Verification:**

1. `cargo check` — compiles
2. `cargo run --release` → select "Classic" → identical to current behavior
3. `cargo run --release` → select "Continents" → ocean at edges, continent blobs in center, biome variety on land, towns on land with Dirt clearings
4. Debug Tests → `world-gen` test passes (Classic mode, 6 phases)
5. Try small world (4000) and large world (32000) with Continents — land/ocean ratio looks reasonable
6. Verify towns never spawn in ocean (if world is mostly ocean and town placement fails, `warn!` fires but game still runs)

### AI Players

The player is one town in a sea of hostile AI. Two AI archetypes compete: **destroyers** (raider camps that build tents and send raids) and **builders** (enemy towns that mirror the player — farms, houses, barracks, guard posts, upgrades). Each AI settlement gets its own faction. Everyone fights everyone. AI players follow the same rules as the human player — same building costs, same spawn timers, same upgrade system. They don't cheat. They make 1 decision every N real seconds (configurable, default 5.0s).

**Faction model**: Player = faction 0. Each AI settlement = unique faction (1, 2, 3, ...). GPU targeting treats any NPC with a different faction as an enemy, so three-way (N-way) conflicts emerge naturally.

**`AiPlayerConfig` resource** (`systems/ai_player.rs`):

```rust
#[derive(Resource)]
pub struct AiPlayerConfig {
    pub decision_interval: f32, // real seconds between decisions (default 5.0)
}
```

Configurable from main menu slider. Uses `Res<Time>` (real time), not game time — the AI thinks at the same pace regardless of time_scale, just like a human player can't click faster when the game speeds up.

**`AiPlayerState` resource** (`systems/ai_player.rs`):

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AiKind {
    Raider,  // builds tents only — offensive
    Builder, // builds farms, houses, barracks, guard posts — mirrors player
}

pub struct AiPlayer {
    pub town_data_idx: usize,  // index into WorldData.towns
    pub grid_idx: usize,       // index into TownGrids.grids
    pub kind: AiKind,
    pub last_decision: f32,    // time.elapsed_secs() of last decision
}

#[derive(Resource, Default)]
pub struct AiPlayerState {
    pub players: Vec<AiPlayer>,
}
```

One `AiPlayer` per AI settlement. `kind` is determined by `Town.sprite_type`: 0 (fountain) = Builder, 1 (tent) = Raider.

**`ai_player_system`** (`systems/ai_player.rs`, `Step::Behavior`):

Runs every frame. For each AI player, checks if `time.elapsed_secs() - last_decision >= config.decision_interval`. If so, makes exactly one action:

**Raider AI priorities** (first affordable action wins):
1. **Build tent** — if `food >= TENT_BUILD_COST` and empty unlocked slot exists
2. **Unlock slot** — if `food >= SLOT_UNLOCK_COST` and no empty slots but `get_adjacent_locked_slots()` returns options
3. **Buy upgrade** — try `UpgradeType::AttackSpeed` first, then `UpgradeType::MoveSpeed`

**Builder AI priorities** (first affordable action wins):
1. **Build farm** — if farm count < house count (need food income to support population)
2. **Build house** — if house count <= farm count (need farmers to tend farms)
3. **Build barracks** — if barracks count == 0, or barracks < house count / 2 (need defense)
4. **Build guard post** — if guard post count < barracks count (guards need patrol routes)
5. **Unlock slot** — if no empty unlocked slots
6. **Buy upgrade** — cycle through: `GuardHealth`, `GuardAttack`, `FarmYield`, `AttackSpeed`, `MoveSpeed` (pick first affordable)

Building counts: iterate `WorldData.farms/houses/barracks/guard_posts/tents`, filter `town_idx == my_town_idx` and `position.x > -9000.0` (skip tombstoned).

**Helper: `find_empty_slot()`**:
```rust
fn find_empty_slot(
    town_grid: &TownGrid,
    world_grid: &WorldGrid,
    center: Vec2,
) -> Option<(i32, i32)> {
    for &(row, col) in &town_grid.unlocked {
        if row == 0 && col == 0 { continue; } // skip center (fountain/camp)
        let pos = town_grid_to_world(center, row, col);
        let (gc, gr) = world_grid.world_to_grid(pos);
        if let Some(cell) = world_grid.cell(gc, gr) {
            if cell.building.is_none() {
                return Some((row, col));
            }
        }
    }
    None
}
```

**Building placement**: call `place_building()` (same function the player's build menu uses). After placement, push `SpawnerEntry` to `SpawnerState` for House/Barracks/Tent (same pattern as `build_menu.rs:180-310`). Farm and GuardPost don't need SpawnerEntry (they don't spawn NPCs).

**Upgrade purchases**: push `(town_data_idx, upgrade_idx)` to `UpgradeQueue` — identical to player UI. `process_upgrades_system` handles deducting food and re-resolving NPC stats.

**Combat log**: all AI actions logged as `CombatEventKind::Harvest` with prefix "AI: ", e.g. `"AI: Raider Camp built tent"`, `"AI: Tampa built farm"`.

**World gen changes** (`world.rs`):

Add to `WorldGenConfig`:
```rust
pub ai_towns: usize,       // default 1
pub raider_camps: usize,    // default 1
```

Restructure `generate_world()` — currently pairs each player town with a raider camp. New flow places all settlements independently with min_distance:

```
1. Collect all_positions: Vec<Vec2> (for min_distance checks)

2. Place player town centers (faction 0) — existing logic
   Push each to all_positions

3. Place enemy AI town centers
   let mut next_faction = 1;
   for i in 0..config.ai_towns:
       loop { random position, check min_distance from all_positions }
       Create Town { faction: next_faction, sprite_type: 0 }  // fountain = builder
       Create TownGrid
       place_town_buildings() — same layout as player (fountain + farms + houses + barracks + guard posts)
       next_faction += 1
       Push to all_positions

4. Place raider camp centers
   for i in 0..config.raider_camps:
       loop { random position, check min_distance from all_positions }
       Create Town { faction: next_faction, sprite_type: 1 }  // tent = raider
       Create TownGrid
       place_camp_buildings() — existing (camp center + tents)
       next_faction += 1
       Push to all_positions

5. Generate terrain — pass all_positions (player + AI + camp) for Dirt clearing
```

Remove the implicit 1:1 player-town:raider-camp pairing from the old `generate_world()` loop.

**Bug fix: faction hardcoding** — two places hardcode `faction: 0` for House→Farmer and Barracks→Guard:

1. `spawner_respawn_system` (`systems/economy.rs:267-288`):
```rust
// Current:
0 => { (0, 0, farm.x, farm.y, -1, 0, "Farmer", "House") }
1 => { (1, 0, -1.0, -1.0, post_idx, 1, "Guard", "Barracks") }

// Fix: look up town faction
let faction = world_data.towns.get(town_data_idx)
    .map(|t| t.faction).unwrap_or(0);
0 => { (0, faction, farm.x, farm.y, -1, 0, "Farmer", "House") }
1 => { (1, faction, -1.0, -1.0, post_idx, 1, "Guard", "Barracks") }
```

2. `game_startup_system` (`ui/mod.rs:184-207`) — same fix for initial spawn loop.

Without this fix, enemy town farmers/guards spawn as faction 0 (player) instead of their town's faction.

**Bug fix: `NpcsByTownCache` initialization** — `NpcsByTownCache` is init'd as empty Vec and never resized. `spawn_npc_system` checks bounds before inserting, so no NPCs are tracked per-town. This means `process_upgrades_system` (which reads `NpcsByTownCache` to find NPCs to re-resolve stats) silently skips all NPCs.

Fix in `game_startup_system` after world gen:
```rust
npcs_by_town.0.resize(num_towns, Vec::new());
```

**Main menu** (`ui/main_menu.rs`):

Add to `MenuState`: `ai_towns: f32` (default 1.0), `raider_camps: f32` (default 1.0), `ai_interval: f32` (default 5.0).

Add sliders in main config area (between Towns and Play button):
```
AI Towns:       [===slider===] 1     (range 0..=10, step 1)
Raider Camps:   [===slider===] 1     (range 0..=10, step 1)
AI Speed:       [===slider===] 5.0s  (range 1.0..=30.0, step 0.5)
```

On Play: write to `WorldGenConfig` (`ai_towns`, `raider_camps`) and `AiPlayerConfig` (`decision_interval`).

**Settings** (`settings.rs`):

Add to `UserSettings`:
```rust
#[serde(default = "default_one")]
pub ai_towns: usize,
#[serde(default = "default_one")]
pub raider_camps: usize,
#[serde(default = "default_ai_interval")]
pub ai_interval: f32,
```

`fn default_one() -> usize { 1 }`, `fn default_ai_interval() -> f32 { 5.0 }`

**Startup** (`ui/mod.rs:game_startup_system`):

After world gen, populate `AiPlayerState`:
```rust
let mut ai_players = Vec::new();
for (grid_idx, town_grid) in town_grids.grids.iter().enumerate() {
    let tdi = town_grid.town_data_idx;
    if let Some(town) = world_data.towns.get(tdi) {
        if town.faction > 0 {
            let kind = if town.sprite_type == 1 { AiKind::Raider } else { AiKind::Builder };
            ai_players.push(AiPlayer {
                town_data_idx: tdi, grid_idx, kind, last_decision: 0.0,
            });
        }
    }
}
ai_player_state.players = ai_players;
```

Reset `AiPlayerState` in `game_cleanup_system`.

**Registration** (`lib.rs`):

```rust
.init_resource::<AiPlayerState>()
.insert_resource(AiPlayerConfig { decision_interval: 5.0 })
// In Step::Behavior:
ai_player_system,
```

**Constants** (`constants.rs`):

```rust
pub const DEFAULT_AI_INTERVAL: f32 = 5.0;
```

**Files changed:**

| File | Changes |
|---|---|
| `systems/ai_player.rs` | **New file.** `AiPlayerConfig`, `AiPlayerState`, `AiPlayer`, `AiKind`, `ai_player_system`, `find_empty_slot()` |
| `systems/mod.rs` | Add `pub mod ai_player;` |
| `world.rs` | Add `ai_towns`/`raider_camps` to `WorldGenConfig`. Restructure `generate_world()` for independent placement. |
| `systems/economy.rs` | Fix faction in `spawner_respawn_system` — lookup `world_data.towns[idx].faction` instead of hardcoded 0 |
| `ui/mod.rs` | Fix faction in `game_startup_system`. Init `AiPlayerState` + `NpcsByTownCache`. Reset in cleanup. |
| `ui/main_menu.rs` | Add `ai_towns`, `raider_camps`, `ai_interval` to `MenuState`. Add 3 sliders. Write to config on Play. |
| `settings.rs` | Add `ai_towns`, `raider_camps`, `ai_interval` to `UserSettings` with serde defaults. |
| `constants.rs` | Add `DEFAULT_AI_INTERVAL` |
| `lib.rs` | Register `AiPlayerConfig`, `AiPlayerState`. Add `ai_player_system` to `Step::Behavior`. |

**Verification:**

1. `cargo check` — compiles without errors
2. `cargo run --release` → set 1 player town, 1 AI town, 1 raider camp → Play
3. AI town: after ~5 real seconds, starts building. Pan camera to enemy town — see farm/house tiles appear. Farmers spawn with enemy faction color, tend farms. Guards spawn, patrol.
4. Raider camp: after ~5s, starts building tents. Raiders spawn, form raid groups (existing `decision_system` raid queue), attack farms — both player farms AND AI town farms.
5. Three-way combat: AI town guards (faction 1) fight raiders (faction 2). Player guards (faction 0) fight both. GPU targeting handles it automatically.
6. Combat log: "AI: [town name] built farm", "AI: Raider Camp built tent", "AI: [name] upgraded AttackSpeed to Lv.2"
7. AI respects food: with 0 food, AI makes no actions. Builder AI accumulates food from foraging + farm harvests before building.
8. AI Speed slider: change to 1.0s → AI builds rapidly. Change to 30.0s → AI builds slowly. Restart to apply.
9. All debug tests pass (AI systems only run in `AppState::Playing`, tests use `AppState::Running`)
10. Enemy town NPCs have correct faction (not 0) — click an enemy farmer in roster, verify faction color is different from player
11. Multiple AI towns + camps: set 3 AI towns + 3 raider camps. All factions distinct. Multi-faction wars emerge.

## References

- [Simon Green's CUDA Particles](https://developer.download.nvidia.com/assets/cuda/files/particles.pdf) — GPU spatial grid approach
- [FSM in ECS](https://www.richardlord.net/blog/ecs/finite-state-machines-with-ash) — marker component pattern
- [Bevy Render Graph](https://docs.rs/bevy/latest/bevy/render/render_graph/) — compute + render pipeline
- [Factorio FFF #251](https://www.factorio.com/blog/post/fff-251) — sprite batching, per-layer draw queues
- [Factorio FFF #421](https://www.factorio.com/blog/post/fff-421) — entity update optimization, lazy activation
