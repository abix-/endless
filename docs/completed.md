# Completed Features

Completed items moved from roadmap for readability.

### Roadmap Migration (Stages 14, 14b, 14d, 15, 18)
- [x] Stage 14: food consumption added (hourly eating restores HP/energy)
- [x] Stage 14: starvation effects added (HP drain + speed penalty)
- [x] Stage 14: building costs rebalanced by difficulty (Easy/Normal/Hard)
- [x] Stage 15 GPU extract: zero-clone GPU upload (`Extract<Res<T>>` + `queue.write_buffer()`)
- [x] Stage 15 GPU extract: removed `ExtractResourcePlugin::<GpuReadState>` render-world clone path
- [x] Stage 15 GPU extract: `ProjBufferWrites` moved to zero-clone `extract_proj_data` path
- [x] Stage 15 GPU-native rendering: vertex reads `NpcGpuBuffers` storage directly (no CPU->GPU instance rebuild)
- [x] Stage 15 GPU-native rendering: `NpcVisualBuffers` added (`visual [f32;8]`, `equip [f32;24]`)
- [x] Stage 15 GPU-native rendering: `vertex_npc` instance encoding (`slot`/`layer`) with `npc_count` in `CameraUniform`
- [x] Stage 15 GPU-native rendering: pipeline specialization key `(hdr, samples, storage_mode)` with dual entry points
- [x] Stage 15 GPU-native rendering: farm sprites + building HP bars moved to `NpcMiscBuffers` + `DrawMisc`
- [x] Stage 15 perf: deleted `prepare_proj_buffers` (merged into `extract_proj_data`)
- [x] Stage 15 perf: eliminated `ProjPositionState` + `GpuReadState` extraction
- [x] Stage 15 perf: gated `rebuild_building_grid_system` to run on dirty world/building state
- [x] Stage 15 perf: replaced `decision_system` `count_nearby_factions` checks with GPU spatial-grid readback query
- [x] Stage 15 perf: optimized `healing_system` town-zone checks with faction-indexed/cached data
- [x] Stage 15 perf: optimized `guard_post_attack_system` target acquisition via slot-indexed GPU `combat_targets`
- [x] Stage 15 perf: combat log UI made incremental (skip full rebuild/sort when unchanged)
- [x] Stage 15 perf: DirtyFlags lifecycle hardened across startup/load/cleanup paths
- [x] Stage 15 perf: `squad_cleanup_system` changed from always-on to event/interval-driven
- [x] Stage 15 SystemParam bundles: `WorldState` added and adopted in high-churn systems
- [x] Stage 15 SystemParam bundles: `EconomyState` added and adopted in core systems
- [x] Stage 14b chunk 1: AI expansion brain upgrades (`miner_home_target`, fullness weighting, emergency multiplier, reweighted personalities)
- [x] Stage 14b chunk 2: waypoint turret defaults disabled + UI update + GuardPost -> Waypoint rename
- [x] Stage 14b chunk 3: wilderness waypoint placement + AI mine-aware waypoint expansion path
- [x] Stage 14d mining policy: `PolicySet.mining_radius`, `MiningPolicy`, `MinerHome.manual_mine`, `DirtyFlags.mining`
- [x] Stage 14d mining policy: discover/distribute/clear-stale mining assignments via `mining_policy_system`
- [x] Stage 14d mining policy: policies tab mining controls (radius slider, mine toggles, assignment summary)
- [x] Stage 14d mining policy: gold mine inspector auto-mining toggle
- [x] Stage 14d mining policy: manual miner assignment override preserved (`Set Mine`/`Clear`)
- [x] Stage 14d mining policy: startup/spawn/death integration for mining dirty-flag updates
- [x] Stage 18 save/load: full state serialization + F5/F9 quicksave/quickload + save/load toasts
- [x] Stage 18 save/load: main-menu load path + autosave rotation
- [x] Backlog bug fixed: projectile double-hit against buildings removed by unifying collision through GPU NPC-slot path

### Spawning & Rendering
- [x] NPCs spawn with jobs (archer, farmer, raider, fighter, miner)
- [x] GPU instanced rendering via RenderCommand + Transparent2d (10,000+ @ 140fps)
- [x] Sprite frames, faction colors
- [x] Unified spawn API with job-as-template pattern
- [x] spawn_archer(), spawn_archer_at_post(), spawn_farmer() convenience APIs
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
- [x] Archers have no leash (fight anywhere)
- [x] Alert nearby allies when combat starts

### NPC Behaviors
- [x] Archers: patrol posts clockwise, rest when tired (energy < 50), resume when rested (energy > 80)
- [x] Per-squad patrol policy (`patrol_enabled`) with immediate next-decision enforcement for squad archers
- [x] Farmers: work at assigned farm, rest when tired
- [x] Raiders: steal food from farms, flee when wounded, return to town, recover
- [x] Energy system (drain while active, recover while resting)
- [x] Leash system (disengage if too far from home)
- [x] Flee system (exit combat below HP threshold)
- [x] Wounded rest + recovery system
- [x] 15-minute decision cycles (event-driven override on state changes)
- [x] Building arrival based on sprite size (not pixel coordinates)
- [x] Drift detection (working NPCs pushed off position walk back)
- [x] `rebuild_patrol_routes_system` — rebuilds all archers' patrol routes when WorldData changes (guard post added/removed/reordered)

### Economy
- [x] Food production (farmers generate food per hour)
- [x] Food theft (raiders steal and deliver to town)
- [x] Raider passive forage is runtime-toggleable from menu settings (default OFF)
- [x] Respawning (dead NPCs respawn after cooldown via SpawnerState timers)
- [x] Per-town food storage (FoodStorage resource)
- [x] GameTime resource (time_scale, pause, hourly tick events)
- [x] GameConfig resource (farmers/archers per town, spawn interval, food per hour)
- [x] PopulationStats resource (alive/working counts per job/clan)
- [x] economy_tick_system (unified hourly economy)
- [x] Miner job type (Job::Miner, brown tint, separate behavior from farmer)
- [x] MinerHome spawner building (1:1 building→miner, replaces job_reassign system)
- [x] Population caps per town (upgradeable)
- [x] Gold mines: wilderness resource nodes placed between towns, unowned (any faction), slow regen, AI personality allocation

### World Generation
- [x] Procedural town/farm/guard_post placement (2 towns default, 1200px spacing, random layout)
- [x] Named towns from pool of Florida cities
- [x] WorldGrid (250x250 cells, 32px each, terrain biome + building per cell)
- [x] WorldGenConfig resource (world size, town count, spacing, NPC counts)
- [x] Building grid expansion (6x6 start, expandable to 100x100 via per-tile unlock)
- [x] Spiral building placement (`spiral_slots()` generates positions outward from center, auto-unlocks TownGrid slots)

### World Data
- [x] Towns, farms, guard posts as Bevy resources
- [x] BuildingOccupancy resource (private map + claim/release/is_occupied/count API, replaces FarmOccupancy)
- [x] Worksite trait + generic `find_nearest_free()`/`find_within_radius()`/`find_by_pos()` helpers
- [x] Query APIs: get_town_center, get_raider_position, get_patrol_post
- [x] init_world, add_town/farm/guard_post APIs

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
- [x] GameConfig + WorldGenConfig Bevy resources
- [x] roster_panel.rs (NPC list with sorting/filtering, select/follow)
- [x] build_menu.rs (bottom-center build bar with sprite previews, click-to-place with ghost preview)
- [x] combat_log.rs (event feed with color-coded timestamps, Kill/Spawn/Raid/Harvest)
- [x] left_panel.rs upgrades tab (16 upgrade rows with level/cost, spend food/gold to purchase)
- [x] policies_panel.rs (behavior config with live policy controls wired to TownPolicies resource)
- [x] Keyboard toggles: R=roster, B=build, U=upgrades, P=policies, T=patrols, F=follow
- [x] Building inspector (click building → farm growth/occupancy, spawner NPC status/respawn timer, guard post patrol order/turret, fountain heal radius/food)
- [x] Patrols tab (T) — view and reorder guard post patrol routes, swap buttons mutate WorldData
- [x] Left panel (renamed from right_panel): Roster / Upgrades / Policies / Patrols tabs
- [x] Squads tab updates: visible Default Squad, recruit transfer buttons (+1/+2/+4/+8/+16/+32), and hotkeys `1..9,0` to arm squad target placement for squads 1..10

### Building System
- [x] Runtime add/remove farm/guard_post (place_building/remove_building with tombstone deletion)
- [x] Slot unlock system (spend food to unlock adjacent grid slots)
- [x] Slot indicators (green "+" empty, dim brackets locked, gold ring town center)
- [x] NPCs claim new buildings (existing decision system finds nearest farm)
- [x] Build and destroy buildings (build bar + context actions)
- [x] Miner Home uses dedicated external sprite (`miner_house.png`) across tilemap, build menu, and placement ghost
- [x] Build menu order/polish updates (Farmer Home → Miner Home → Archer Home, larger Destroy tile, flush Build toggle button)

### Visual Feedback
- [x] Camera uniform buffer (replaces hardcoded CAMERA_POS/VIEWPORT in npc_render.wgsl)
- [x] Camera pan (WASD) and zoom (scroll wheel toward cursor)
- [x] Click-to-select NPC wired to camera transform
- [x] Camera follow selected NPC (F key toggle, WASD cancels follow)
- [x] Target indicator overlay (yellow line + diamond marker to NPC's movement target, blue circle on NPC)
- [x] Multi-layer equipment rendering (see [rendering.md](rendering.md))
- [x] Archers spawn with weapon + helmet layers, raiders with weapon layer
- [x] Projectile instanced pipeline (same RenderCommand pattern as NPC renderer)
- [x] Separate InstanceData buffer for active projectiles
- [x] Health bars (3-color: green/yellow/red, show-when-damaged mode in fragment shader)
- [x] Damage flash in npc_render.wgsl (white overlay on hit, fade out over ~0.2s via CPU-side decay)
- [x] Sleep indicator on resting NPCs (SLEEP_SPRITE on status layer via sync_visual_sprites)
- [x] Healing indicator on healing NPCs (HEAL_SPRITE on healing layer via sync_visual_sprites)
- [x] Carried item icon (food sprite on returning raiders)
- [x] Farm growth state visible (Growing → Ready sprite change via farm-visual test)
- [x] Build cursor hint sprite hides while snapped over a valid build slot (shows only when placement is invalid/outside)

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
- [x] `archer-patrol` — Archer Patrol Cycle (5 phases): OnDuty → Patrolling → OnDuty → rest → resume
- [x] `farmer-cycle` — Farmer Work Cycle (5 phases): GoingToWork → Working → tired → rest → resume
- [x] `raider-cycle` — Raider Raid Cycle (5 phases): dispatch → arrive → steal → return → deliver
- [x] `combat` — Combat Pipeline (6 phases): targeting → InCombat → projectile → damage → death → cleanup
- [x] `economy` — Farm Growth & Respawn (5 phases): Growing → Ready → harvest → forage → respawn
- [x] `energy` — Energy System (3 phases): start 100 → drain → reach threshold
- [x] `healing` — Healing Aura (3 phases): damaged in zone → heal → full HP
- [x] `spawning` — Spawn & Slot Reuse (4 phases): exist → kill → free slot → reuse slot
- [x] `projectiles` — Projectile Pipeline (4 phases): targeting → spawn proj → hit → free slot
- [x] `world-gen` — World Generation (6 phases): grid → towns → spacing → buildings → terrain → raider towns
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
- [x] `Personality` (4 traits: Brave/Tough/Swift/Focused) wired into `resolve_combat_stats()`. Display `trait_id` uses separate 9-name list — unification in Stage 16
- [x] Init values match hardcoded values: archer/raider damage=15, speeds=100, max_health=100, heal_rate=5, heal_radius=150
- [x] Stage 8 parity checks verified stats matched hardcoded values (removed in Stage 9)

### Settings & Config
- [x] User settings persistence (serde JSON, scroll speed + world gen sliders)
- [x] Cross-platform settings path (USERPROFILE on Windows, HOME fallback on macOS/Linux)
- [x] Main menu DragValue widgets alongside sliders for typeable config inputs
- [x] Build menu text scale setting (`build_menu_text_scale`) with pause-menu slider

### Guard Post Turrets
- [x] Guard post auto-attack (turret behavior, fires projectiles at enemies within 250px)
- [x] Guard post turret toggle (enable/disable via right-click build menu)

### Upgrades & XP
- [x] `UpgradeQueue` resource + `process_upgrades_system`: drains queue, validates food cost, increments `TownUpgrades`, re-resolves `CachedStats` for affected NPCs
- [x] `upgrade_cost(level) -> i32` = `10 * 2^level` (doubles each level, capped at 20)
- [x] Wire upgrade multipliers into `resolve_combat_stats()` via `UPGRADE_PCT` array
- [x] Enable upgrade buttons: click → push to `UpgradeQueue` → deduct food → increment level
- [x] Military upgrades: health (+10%), attack (+10%), range (+5%), attack speed (-8%), move speed (+5%), alert radius (+10%)
- [x] Farm upgrades: yield (+15%), farmer HP (+20%)
- [x] Town upgrades: healing rate (+20%), fountain radius (+24px flat)
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
- [x] Policy-driven flee: archers use `archer_flee_hp`, farmers use `farmer_flee_hp`, raiders hardcoded 0.50
- [x] `archer_aggressive` disables archer flee, `farmer_fight_back` disables farmer flee
- [x] `archer_leash` policy controls whether archers return to post after combat
- [x] `prioritize_healing` sends wounded NPCs to fountain before resuming work
- [x] Removed hardcoded `FleeThreshold`/`WoundedThreshold` from raider spawn — thresholds policy-driven

### Building Spawners
- [x] `Building::FarmerHome { town_idx }` and `Building::ArcherHome { town_idx }` variants in `world.rs`
- [x] `FarmerHome`/`ArcherHome` structs in `WorldData`, `BUILDING_TILES` extended
- [x] Wire `place_building()`/`remove_building()` for FarmerHome/ArcherHome (same tombstone pattern)
- [x] World gen: `place_town_buildings()` places N FarmerHomes + N ArcherHomes from config sliders
- [x] `SpawnerEntry` struct: `building_kind`, `town_idx`, `position`, `npc_slot` (-1=none), `respawn_timer`
- [x] `SpawnerState` resource: `Vec<SpawnerEntry>` — one entry per FarmerHome/ArcherHome/MinerHome
- [x] `spawner_respawn_system` in `systems/economy.rs` (Step::Behavior, hourly): detects dead NPC via `NpcEntityMap`, starts 12h timer, spawns replacement when timer expires
- [x] FarmerHome + ArcherHome + MinerHome buttons in build_menu.rs (push `SpawnerEntry` on build)
- [x] HUD shows spawner counts: `Farmers: alive/homes` / `Archers: alive/homes`
- [x] `game_startup_system` builds `SpawnerState` from world gen spawner buildings, spawns 1 NPC per entry (instant, no timer)
- [x] Replaced bulk farmer/archer spawn loops with spawner-based spawn — raider spawn loop kept
- [x] `.init_resource::<SpawnerState>()`, add `spawner_respawn_system` to Step::Behavior
- [x] Remove beds — NPCs rest at their spawner building (FarmerHome/ArcherHome) instead of separate beds. Home = spawner position.

### GPU Performance
- [x] Replace hand-rolled readback with Bevy's `Readback` + `ReadbackComplete` (eliminates 9ms blocking `device.poll`)
- [x] Eliminate `GPU_READ_STATE`, `PROJ_HIT_STATE`, `PROJ_POSITION_STATE` static Mutexes (replaced by `ReadbackComplete` events → Bevy Resources)
- [x] Convert 4 readback compute buffers to `ShaderStorageBuffer` assets with `COPY_SRC`

### Continent World Generation
- [x] `WorldGenStyle` enum (Classic/Continents) in `WorldGenConfig`, selectable from main menu combo box
- [x] 3-octave fBm elevation noise (freq 0.0008/0.0016/0.0032) + square-bump edge falloff + power redistribution
- [x] Independent moisture noise (freq 0.003) for biome selection: dry→Rock, moderate→Grass, wet→Forest
- [x] Town placement constrained to land cells in Continents mode (5000 max attempts)
- [x] `stamp_dirt()` clears terrain around settlements after placement
- [x] Setting persisted in UserSettings as `gen_style: u8`

### Architecture
- [x] Bevy Messages (MessageWriter/MessageReader) for all inter-system communication
- [x] All state as Bevy Resources (WorldData, Debug, KillStats, NpcMeta, FoodEvents, etc.)
- [x] GpuUpdateMsg batching via collect_gpu_updates

### In-Game Help
- [x] `HelpCatalog` resource (~35 entries, HashMap keyed by topic ID)
- [x] `help_tip()` helper: small "?" button with rich tooltip on hover
- [x] Top bar tips (getting started, food, gold, pop, farmers, archers, raiders)
- [x] Left panel tab tips (roster, upgrades, policies, patrols, squads, intel, profiler)
- [x] Build menu hover text (farm, farmer home, archer home, guard post, tent)
- [x] NPC inspector tips (level/xp, trait, energy, state)

### DRY & Consolidation
- [x] Rename role spawner buildings to `FarmerHome` / `ArcherHome` / `MinerHome` + rename `Job::Guard` → `Job::Archer` and all associated types/fields/UI labels
- [x] Consolidate farm harvest transitions into one authoritative path (previously split across `arrival_system` and `decision_system`)
- [x] Consolidate building placement side effects (place + food spend + spawner entry + HP push) into one shared helper used by player + AI
- [x] Consolidate spawner spawn mapping (`building_kind` -> `SpawnNpcMsg` fields) into one shared helper used by startup + respawn systems
- [x] Consolidate building destroy flow into `destroy_building()` (grid clear + WorldData tombstone + spawner tombstone + HP zero + combat log) — used by click-destroy, inspector-destroy, building_damage_system

### Building HP & NPC Building Attacks
- [x] `BuildingHpState` resource with parallel Vecs per building type (guard_posts, farmer_homes, archer_homes, tents, miner_homes, farms)
- [x] Building HP constants: GuardPost=200, ArcherHome=150, FarmerHome=100, MinerHome=100, Tent=100, Farm=80
- [x] `BuildingDamageMsg` message type (kind, index, amount) — direct damage on fire
- [x] `BuildingSpatialGrid` extended with ArcherHome, FarmerHome, Tent, MinerHome + `faction` field on `BuildingRef`
- [x] `Building::kind()` helper mapping Building → BuildingKind
- [x] `find_nearest_enemy_building()` — spatial grid query filtered by faction and job type
- [x] attack_system building fallback: archers/raiders fire at enemy buildings when no NPC target, raiders only target military buildings
- [x] `building_damage_system` in Step::Behavior: decrement HP, destroy on HP≤0, kill linked NPC
- [x] `AiBuildRes` SystemParam bundle (8 resources) in ai_player.rs — fixes 16-param limit on `ai_decision_system`
- [x] Init/cleanup BuildingHpState in game_startup/cleanup systems

### AI Players
- [x] `AiPlayerConfig` resource (decision interval in real seconds, configurable from main menu)
- [x] `AiPlayerState` resource with `Vec<AiPlayer>` — one per AI settlement
- [x] `AiKind::Raider` AI: builds tents, unlocks slots, buys AttackSpeed/MoveSpeed upgrades
- [x] `AiKind::Builder` AI: builds farms/farmer homes/archer homes/guard posts, buys all upgrade types
- [x] World gen: independent placement of player towns, AI towns, and raider towns (not paired)
- [x] Main menu sliders: AI Towns (0-10), Raider Towns (0-10), AI Speed (1-30s)
- [x] Fix faction hardcoding: `spawner_respawn_system` + `game_startup_system` use town faction instead of 0
- [x] Fix `NpcsByTownCache` initialization (resize to `num_towns` in `game_startup_system`)

### Tech Tree (Chunks 1-2)
- [x] `UpgradeNode` extended with `prereqs: &[UpgradePrereq]` and `cost: &[(ResourceKind, i32)]` in `UPGRADE_REGISTRY` (`stats.rs`)
- [x] `ResourceKind { Food, Gold }` enum — extensible for Stage 24 (Wood, Stone, Iron)
- [x] Cost model: each node has `&[(ResourceKind, base_amount)]` slice, scaled by `upgrade_cost(level)`
- [x] `upgrade_unlocked()`, `upgrade_available()`, `deduct_upgrade_cost()`, `missing_prereqs()`, `format_upgrade_cost()` — shared helpers
- [x] `TownUpgrades::town_levels()` method eliminates repeated `.get().copied().unwrap_or()` pattern
- [x] `process_upgrades_system` + `auto_upgrade_system`: prereq gate + multi-resource deduction via `GoldStorage`
- [x] `ai_decision_system`: prereq + multi-resource affordability gate, `GoldStorage` param added
- [x] Upgrade UI: locked nodes dimmed with prereq tooltip, cost shows "10g" or "10+10g", auto-upgrade disabled when locked
- [x] Redesigned from 14 nodes to 16 per-NPC-type nodes in 4 categories: Military (7), Farmer (3), Miner (3), Town (3)
- [x] `resolve_combat_stats()` dispatches HP/Attack/Range/MoveSpeed by job type
- [x] GoldYield multiplier wired into miner extraction
- [x] TownArea (Expansion) uses slot-proportional cost: `expansion_cost()` = 24+8*level
- [x] Render by branch/tier with depth indentation, "Now"/"Next" effect text, branch totals
- [x] AI upgrade weights resized to 16 per personality
- [x] Shallow logical prerequisites (max depth 2)
- [x] Centralize upgrade metadata — `UPGRADE_REGISTRY` in `stats.rs`, `UpgradeNode` struct with label/short/tooltip/category/cost/prereqs
- [x] Make trait display read from `Personality`/`TraitKind` instead of separate `trait_id` mapping in UI cache
- [x] Remove stale respawn legacy resource/path leftovers (`RespawnTimers`)
- [x] NPC rename in Bevy UI (inspector/roster edit of `NpcMetaCache.name`)
- [x] Persist auto-upgrade checkbox state in `UserSettings` (settings v2 `auto_upgrades`)

### Intentional Removals
- [x] Sprite atlas browser tool — intentional removal (Godot dev tool, not needed in Bevy)
- [x] World-space town labels — intentional removal (Godot scenes, not ported)
