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
- [x] NPCs spawn with jobs (guard, farmer, raider, fighter)
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
- [x] Sleep indicator (SLEEP_SPRITE on status layer via sync_visual_sprites)
- [x] Healing indicator (HEAL_SPRITE on healing layer via sync_visual_sprites)

### Movement & Physics
- [x] GPU compute shader for movement toward targets
- [x] set_target(npc_index, x, y) API for directing NPCs
- [x] Separation physics (boids-style, no pile-ups)
- [x] Spatial grid for O(1) neighbor lookups (256x256 cells, 128px each, 48 NPCs/cell max, covers 32,768px)
- [x] Arrival detection with persistent flag
- [x] TCP-style backoff for blocked NPCs
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

### Economy
- [x] Food production (farmers generate food per hour)
- [x] Food theft (raiders steal and deliver to camp)
- [x] Respawning (dead NPCs respawn after cooldown via RespawnTimers)
- [x] Per-town food storage (FoodStorage resource)
- [x] GameTime resource (time_scale, pause, hourly tick events)
- [x] GameConfig resource (farmers/guards per town, spawn interval, food per hour)
- [x] PopulationStats resource (alive/working counts per job/clan)
- [x] economy_tick_system (unified hourly economy)
- [x] Population caps per town (upgradeable)

### World Generation
- [x] Procedural town/farm/bed/guard_post placement (2 towns default, 1200px spacing, random layout)
- [x] Named towns from pool of Florida cities
- [x] WorldGrid (250x250 cells, 32px each, terrain biome + building per cell)
- [x] WorldGenConfig resource (world size, town count, spacing, NPC counts)
- [x] Building grid expansion (6x6 start, expandable to 100x100 via per-tile unlock)

### World Data
- [x] Towns, farms, beds, guard posts as Bevy resources
- [x] Occupancy tracking (reserve/release beds and farms)
- [x] Query APIs: get_town_center, get_camp_position, get_patrol_post
- [x] Query APIs: get_nearest_free_bed/farm
- [x] init_world, add_town/farm/bed/guard_post APIs

### UI Integration
- [x] Click to select NPC (click_to_select_system, nearest within 20px)
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
- [x] upgrade_menu.rs (14 upgrade rows scaffold, disabled until Stage 9 backend)
- [x] policies_panel.rs (behavior config scaffold, disabled until Stage 10 backend)
- [x] Keyboard toggles: R=roster, L=log, B=build, U=upgrades, P=policies

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
- [x] `TownUpgrades` resource stub (all zeros — Stage 9 activates)
- [x] `attack_system` reads `&CachedStats` instead of `&AttackStats`
- [x] `healing_system` reads `CombatConfig.heal_rate`/`heal_radius` instead of local constants
- [x] `MaxHealth` component removed — `CachedStats.max_health` is single source of truth
- [x] `Personality::get_stat_multipliers()` wired into resolver (previously defined but never called)
- [x] Init values match hardcoded values: guard/raider damage=15, speeds=100, max_health=100, heal_rate=5, heal_radius=150
- [x] `#[cfg(debug_assertions)]` parity checks assert resolved stats match old hardcoded values

### Architecture
- [x] Bevy Messages (MessageWriter/MessageReader) for all inter-system communication
- [x] All state as Bevy Resources (WorldData, Debug, KillStats, NpcMeta, FoodEvents, etc.)
- [x] GpuUpdateMsg batching via collect_gpu_updates

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

**Stage 6: Visual Feedback**

*Done when: you can watch the core loop happen on screen and understand what's going on without reading logs.*

- [ ] Farm growth state visible (Growing → Ready sprite change + progress bar on tile)
- [ ] Healing glow effect (pulsing green tint + radial halo — needs TIME uniform in shader)

**Stage 7: Playable Game** ✓

*Done when: someone who isn't you can open it, understand what's happening, and make decisions that affect the outcome.*

- [x] User settings persistence (serde JSON, scroll speed + world gen sliders)
- [x] Villager role assignment (Farmer↔Guard via roster panel)
- [x] Guard post auto-attack (turret behavior, fires projectiles at enemies within 250px)
- [x] Guard post turret toggle (enable/disable via right-click build menu)

**Stage 8: Data-Driven Stats** ✓ (see [spec](#stat-resolution--upgrades))

*Done when: all NPC stats resolve from `CombatConfig` resource via `resolve_combat_stats()`. Game plays identically — pure refactor, no behavior change. All existing tests pass.*

**Architecture: cache with explicit invalidation.** Stats resolve from config via `resolve_combat_stats()`. Resolved stats are cached on the entity as a `CachedStats` component and invalidated on the ~3 events that change them (spawn, upgrade purchased, level-up). See [combat.md](combat.md) and [resources.md](resources.md) for implementation detail.

**Stage 9: Upgrades & XP** (see [spec](#stat-resolution--upgrades))

*Done when: player can spend food on upgrades in the UI, guards with upgrades visibly outperform unupgraded ones, and NPCs gain levels from kills.*

Upgrades:
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

XP & leveling:
- [x] `level_from_xp(xp) -> i32` = `floor(sqrt(xp / 100))`, `level_multiplier = 1.0 + level * 0.01`
- [x] Wire level multiplier into `resolve_combat_stats()`
- [x] `xp_grant_system`: last-hit tracking via `DamageMsg.attacker` → `LastHitBy` component → grant 100 XP to killer on death
- [x] Level-up → `CombatLog` event (LevelUp kind, cyan color), rescale current HP proportionally to new max
- [x] `game_hud.rs` NPC inspector shows level, XP, XP-to-next-level

Gameplay fixes (deferred from Stage 8 to avoid breaking "identical"):
- [x] Fix `starvation_system` speed: uses `CachedStats.speed * STARVING_SPEED_MULT` instead of hardcoded 60.0
- [ ] Differentiate job base stats if desired (e.g., raider damage != guard damage)

Not yet wired (deferred):
- [ ] FarmerCap/GuardCap flat upgrades enforced in spawn cap checks
- [ ] FoodEfficiency upgrade wired into `decision_system` eat logic

**Stage 10: Town Policies**

*Done when: changing a policy slider immediately alters NPC behavior — raiders flee at the configured HP%, farmers sleep during night shift, off-duty guards wander to the fountain.*

- [ ] `TownPolicies` Bevy resource: per-town policy values (mirrors existing `PolicyState` scaffold in `policies_panel.rs`)
- [ ] Wire `policies_panel.rs` controls to read/write `TownPolicies` instead of `Local<PolicyState>`
- [ ] `decision_system` reads `TownPolicies` for: flee thresholds, work schedule, off-duty behavior, prioritize healing
- [ ] Remove hardcoded `FleeThreshold { pct: 0.50 }` from raider spawn — derive from policies
- [ ] Work schedule: `decision_system` checks `GameTime.hour()` against day/night policy before assigning work
- [ ] Off-duty behavior: idle NPCs choose bed/fountain/wander based on policy
- [ ] Fountain healing zone radius reads from `CombatConfig` + upgrade bonus
- [ ] Camp healing zone: raiders heal at camp center (same logic as town fountain, faction-matched)

**Stage 11: Combat & Economy Depth**

*Done when: emergent gameplay happens — raids succeed or fail based on guard upgrades, economy collapses if farms aren't defended, raiders starve if they can't steal.*

Combat depth:
- [ ] Target switching (prefer non-fleeing enemies over fleeing)
- [ ] Trait combat modifiers (Strong +25%, Berserker +50% at low HP, Efficient -25% cooldown, Lazy +20% cooldown)
- [ ] Trait flee modifiers (Brave never flees, Coward +20% threshold)
- [ ] Trait combinations (multiple traits per NPC)
- [ ] Player combat abilities

Economy depth:
- [ ] Multi-camp food delivery (currently hardcoded to camp_food[0])
- [ ] HP regen system (3x sleeping, 10x fountain/camp with upgrade)
- [ ] Food consumption (eating restores HP/energy, npc_ate_food event)
- [ ] Food efficiency upgrade (chance of free meal)
- [ ] Starvation effects (HP drain, desertion)
- [ ] Multiple resources (wood, iron, gold)
- [ ] Production buildings (lumber mill, mine, blacksmith)

**Stage 12: Endgame**

*Done when: AI factions compete autonomously, armies clash over territory, and the simulation runs efficiently at scale.*

Armies & conquest:
- [ ] Army units (peasant levy, archers, knights)
- [ ] Equipment crafting (weapons, armor)
- [ ] Army recruitment and movement
- [ ] Attack and capture enemy towns

AI & coordination:
- [ ] AI lords that expand and compete
- [ ] count_nearby_raiders() for group behavior
- [ ] get_raider_group_center() for coordinated movement
- [ ] find_nearest_raider() for regrouping

Performance — GPU readback + extract optimization (see [spec](#gpu-readback--extract-optimization)):
- [x] Replace hand-rolled readback with Bevy's `Readback` + `ReadbackComplete` (eliminates 9ms blocking `device.poll`)
- [x] Eliminate `GPU_READ_STATE`, `PROJ_HIT_STATE`, `PROJ_POSITION_STATE` static Mutexes (replaced by `ReadbackComplete` events → Bevy Resources)
- [ ] Split `NpcBufferWrites` (1.9MB) to reduce ExtractResource clone cost (18ms → <5ms)
- [x] Convert 4 readback compute buffers to `ShaderStorageBuffer` assets with `COPY_SRC`

Performance — chunked tilemap (see [spec](#chunked-tilemap)):
- [ ] Split single 250×250 (or up to 1000×1000) TilemapChunk per layer into 32×32 tile chunks
- [ ] Bevy frustum-culls off-screen chunk entities automatically — only visible chunks generate draw commands
- [ ] `sync_building_tilemap` updates only chunks whose grid region changed, not all 62K+ tiles
- [ ] Expected: `command_buffer_generation_tasks` drops from ~10ms to ~1ms at default zoom

Performance — entity sleeping:
- [ ] Entity sleeping (Factorio-style: NPCs outside camera radius sleep)
- [ ] awake/sleep_timers per NPC, ACTIVE_RADIUS check
- [ ] Combat/raiding states force awake

Audio:
- [ ] bevy_audio integration

## Specs

### Stat Resolution & Upgrades

Stage 8 (completed) established `CombatConfig`, `CachedStats`, `BaseAttackType`, and `resolve_combat_stats()` — see [combat.md](combat.md), [resources.md](resources.md), and `systems/stats.rs`. What follows is the Stage 9-10 implementation plan.

**`TownUpgrades` resource** (`resources.rs`):

```rust
pub const UPGRADE_COUNT: usize = 14; // matches UPGRADES array in upgrade_menu.rs

#[derive(Resource)]
pub struct TownUpgrades {
    pub levels: Vec<[u8; UPGRADE_COUNT]>,  // per-town, indexed by UpgradeType
}

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum UpgradeType {
    GuardHealth = 0, GuardAttack = 1, GuardRange = 2, GuardSize = 3,
    AttackSpeed = 4, MoveSpeed = 5, AlertRadius = 6,
    FarmYield = 7, FarmerHp = 8, FarmerCap = 9, GuardCap = 10,
    HealingRate = 11, FoodEfficiency = 12, FountainRadius = 13,
}

// Cost: base_cost * 2^level (doubles each level)
// Multiplicative upgrades: multiplier = 1.0 + level * pct_per_level
// Flat upgrades (FarmerCap, GuardCap, FountainRadius): use separate formulas in their owning systems
const UPGRADE_PCT: [f32; UPGRADE_COUNT] = [
    0.10, 0.10, 0.05, 0.05,  // guard: health, attack, range, size
    0.08, 0.05, 0.10,         // cooldown reduction, move speed, alert radius
    0.15, 0.20, 0.0, 0.0,    // farm yield, farmer HP | farmer cap (+2 flat), guard cap (+10 flat)
    0.20, 0.10, 0.0,          // healing rate, food efficiency | fountain radius (+24px flat)
];
```

**Flat upgrades** (FarmerCap, GuardCap, FountainRadius) have `0.0` in `UPGRADE_PCT` because they're not multiplicative. Their owning systems compute them directly:
- FarmerCap: `base_cap + level * 2`
- GuardCap: `base_cap + level * 10`
- FountainRadius: `base_radius + level * 24.0`

**Upgrade applicability by job** — not all upgrades apply to all NPCs:

| Upgrade | Applies to | Rationale |
|---------|-----------|-----------|
| GuardHealth, GuardAttack, GuardRange, GuardSize, AlertRadius | Guard only | Town defense investment |
| AttackSpeed, MoveSpeed | All combatants (Guard, Raider, Fighter) | Generic combat upgrades |
| FarmerHp | Farmer only | Farmer survivability |
| FarmYield | Economy system (not combat resolver) | `farm_growth_system` reads this directly |
| FarmerCap, GuardCap | Spawn system (not combat resolver) | Population caps, not per-NPC stats |
| HealingRate, FountainRadius | `healing_system` (not combat resolver) | Town infrastructure |
| FoodEfficiency | `decision_system` eat logic (not combat resolver) | Economy, not combat |

Only combat-relevant upgrades flow through `resolve_combat_stats()`. Economy/spawn/healing upgrades are read by their owning systems directly from `TownUpgrades`.

**XP formula:**

```
level = floor(sqrt(xp / 100))
```

| Kills (100 XP each) | XP | Level | Stat mult |
|----|-----|-------|-----------|
| 1 | 100 | 1 | 1.01x |
| 4 | 400 | 2 | 1.02x |
| 25 | 2500 | 5 | 1.05x |
| 100 | 10000 | 10 | 1.10x |
| 10000 | 1000000 | 100 | 2.00x |

Grant 100 XP per kill. Last-hit tracking: `DamageMsg.attacker` → `LastHitBy` component on target → `xp_grant_system` reads on death (runs between `death_system` and `death_cleanup_system`).

**`TownPolicies` resource** (`resources.rs`):

```rust
#[derive(Resource)]
pub struct TownPolicies {
    pub policies: Vec<PolicySet>,
}

#[derive(Clone)]
pub struct PolicySet {
    pub eat_food: bool,
    pub guard_aggressive: bool,
    pub guard_leash: bool,
    pub farmer_fight_back: bool,
    pub prioritize_healing: bool,
    pub farmer_flee_hp: f32,     // 0.0-1.0 percentage
    pub guard_flee_hp: f32,
    pub recovery_hp: f32,
    pub work_schedule: WorkSchedule,
    pub farmer_off_duty: OffDutyBehavior,
    pub guard_off_duty: OffDutyBehavior,
}

#[derive(Clone, Copy)] pub enum WorkSchedule { Both, DayOnly, NightOnly }
#[derive(Clone, Copy)] pub enum OffDutyBehavior { GoToBed, StayAtFountain, WanderTown }
```

Mirrors existing `PolicyState` in `policies_panel.rs` (lines 10-24). Wire the UI to write `TownPolicies` instead of `Local<PolicyState>`.

**Files changed per stage:**

Stage 9 (upgrades + XP — new behavior):

| File | Changes |
|---|---|
| `systems/stats.rs` | `UpgradeQueue`, `level_from_xp()`, `upgrade_cost()`, `process_upgrades_system`, `xp_grant_system` |
| `messages.rs` | `DamageMsg.attacker: i32` for last-hit tracking |
| `components.rs` | `LastHitBy(i32)` component |
| `resources.rs` | `CombatEventKind::LevelUp` variant |
| `systems/combat.rs` | `attack_system` + `process_proj_hits` pass attacker index through `DamageMsg` |
| `systems/health.rs` | `damage_system` inserts `LastHitBy`; `healing_system` applies per-town HealingRate + FountainRadius upgrades |
| `systems/economy.rs` | `farm_growth_system` applies FarmYield upgrade; `starvation_system` uses `CachedStats.speed` |
| `systems/spawn.rs` | Remove debug assertions; fix level passthrough; init `level: 0` |
| `ui/upgrade_menu.rs` | Functional buttons: show level/cost, push to `UpgradeQueue`, disable when unaffordable |
| `ui/combat_log.rs` | `LevelUp` filter + cyan color |
| `ui/game_hud.rs` | NPC inspector shows XP / XP-to-next-level |
| `ui/main_menu.rs` | DragValue widgets alongside sliders for typeable config inputs |
| `lib.rs` | Register `UpgradeQueue`, wire `xp_grant_system` + `process_upgrades_system` |

Stage 10 (policies — behavior config):

| File | Changes |
|---|---|
| `resources.rs` | Add `TownPolicies`, `PolicySet`, `WorkSchedule`, `OffDutyBehavior`. |
| `ui/policies_panel.rs` | Replace `Local<PolicyState>` with `ResMut<TownPolicies>`. Remove `ui.disable()`. |
| `systems/behavior.rs` | `decision_system` reads `Res<TownPolicies>` for flee thresholds, work schedule, off-duty. |

**Critical existing code to reuse:**

- `Personality::get_stat_multipliers()` (`components.rs:436`) — already computes `(damage, hp, speed, yield)` but nothing calls it. Wire into `resolve_combat_stats()`.
- `Personality::get_multipliers()` (`components.rs:402`) — already used by `decision_system` (behavior.rs:544) for utility AI scoring. No changes needed.
- `NpcMeta.level` / `NpcMeta.xp` (`resources.rs:261-262`) — already exist, set to 1/0 at spawn, never updated. Stage 9 activates these.
- `UPGRADES` array (`upgrade_menu.rs:17-32`) — 14 upgrade definitions with labels, tooltips, categories. Indices must match `UpgradeType` enum.
- `PolicyState` (`policies_panel.rs:10-24`) — exact field list for `TownPolicies::PolicySet`.
- `FleeThreshold`, `LeashRange`, `WoundedThreshold` (`components.rs:352-368`) — Stage 10 replaces these entity components with per-town policy lookups. Keep components for NPCs that need per-entity overrides (e.g., boss NPCs), but standard NPCs derive from policies.

**Verification per stage:**

Stage 9: Upgrade guard attack in UI. Guards should deal more damage (visible in combat log kill speed). Let a guard get kills — NPC inspector shows level > 1, combat log shows "Level up" events. Level-up rescales current HP proportionally to new max. Starving NPCs now slow to 75 (100*0.75) instead of 60.
Stage 10: Change raider flee threshold slider to 80%. Raiders should flee much earlier. Change work schedule to "Day Only" — farmers idle at night.

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

## References

- [Simon Green's CUDA Particles](https://developer.download.nvidia.com/assets/cuda/files/particles.pdf) — GPU spatial grid approach
- [FSM in ECS](https://www.richardlord.net/blog/ecs/finite-state-machines-with-ash) — marker component pattern
- [Bevy Render Graph](https://docs.rs/bevy/latest/bevy/render/render_graph/) — compute + render pipeline
- [Factorio FFF #251](https://www.factorio.com/blog/post/fff-251) — sprite batching, per-layer draw queues
- [Factorio FFF #421](https://www.factorio.com/blog/post/fff-421) — entity update optimization, lazy activation
