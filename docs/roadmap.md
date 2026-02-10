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
- [x] Spatial grid for O(1) neighbor lookups (128x128 cells, 64px each, 48 NPCs/cell max)
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
- [x] Multi-layer equipment rendering (see [spec](#multi-layer-equipment-rendering))
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

**Stage 7: Playable Game**

*Done when: someone who isn't you can open it, understand what's happening, and make decisions that affect the outcome.*

- [ ] Visible world border with corner markers
- [ ] User settings persistence (serde JSON)
- [ ] Villager role assignment
- [ ] Train guards from population
- [ ] Guard post auto-attack (turret behavior, fires at enemies)
- [ ] Guard post upgrades (attack_enabled, range_level, damage_level)

**Stage 8: Data-Driven Stats** (see [spec](#stat-resolution--upgrades))

*Done when: all NPC stats resolve from `CombatConfig` resource via `resolve_combat_stats()`. Game plays identically — pure refactor, no behavior change. All existing tests pass.*

**Architecture: derive, don't store.** All NPC stats are computed at point-of-use from `base_stat[job] * upgrade_mult[town] * level_mult[npc] * trait_mult[npc]`. No computed stats stored on entities. See spec for full struct definitions, formulas, and file change table.

- [ ] `CombatConfig` Bevy resource with per-job base stats (see spec for struct)
- [ ] `resolve_combat_stats(job, town_idx, level, personality, &config, &upgrades) -> ResolvedStats` function
- [ ] Replace `AttackStats::melee()` / `AttackStats::ranged()` in `spawn_npc_system` — spawn with `BaseAttackType` component instead
- [ ] `attack_system` calls `resolve_combat_stats()` instead of reading stored `AttackStats`
- [ ] Replace `HEAL_RATE` / `HEAL_RADIUS` constants in `healing_system` with `CombatConfig` fields
- [ ] Wire existing `Personality::get_stat_multipliers()` into `resolve_combat_stats()` (currently defined but never called)
- [ ] Fix `starvation_system` speed inconsistency (hardcoded 60.0 vs `Speed::default()` 100.0)

**Stage 9: Upgrades & XP** (see [spec](#stat-resolution--upgrades))

*Done when: player can spend food on upgrades in the UI, guards with upgrades visibly outperform unupgraded ones, and NPCs gain levels from kills.*

Upgrades:
- [ ] `TownUpgrades` resource: `Vec<UpgradeSet>` per town (see spec for struct)
- [ ] `apply_upgrade(town_idx, upgrade_type) -> Result` — checks food cost `base * 2^level`, increments level
- [ ] `upgrade_multiplier(town_idx, upgrade_type, &upgrades) -> f32` — `1.0 + level * pct_per_level`
- [ ] Wire `upgrade_multiplier` into `resolve_combat_stats()`
- [ ] Enable `upgrade_menu.rs` buttons: click → spend food → increment level
- [ ] Guard upgrades: health (+10%), attack (+10%), range (+5%), size (+5%), attack speed (-8%), move speed (+5%), alert radius (+10%)
- [ ] Farm upgrades: yield (+15%), farmer HP (+20%), farmer cap (+2)
- [ ] Town upgrades: guard cap (+10), healing rate (+20%), food efficiency (10% free meal), fountain radius (+24px)

XP & leveling:
- [ ] `grant_xp(npc_meta, amount)` — updates `NpcMeta.xp`, recomputes `NpcMeta.level = sqrt(xp / 100)`
- [ ] `level_multiplier(level) -> f32` = `1.0 + level as f32 * 0.01` (level 100 = 2x stats)
- [ ] Wire into `resolve_combat_stats()`
- [ ] `death_cleanup_system`: grant XP to killer (use `combat_targets` to find attacker)
- [ ] Level-up → `CombatLog` event (LevelUp kind)
- [ ] `game_hud.rs` NPC inspector shows level/XP

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

Performance — entity sleeping:
- [ ] Entity sleeping (Factorio-style: NPCs outside camera radius sleep)
- [ ] awake/sleep_timers per NPC, ACTIVE_RADIUS check
- [ ] Combat/raiding states force awake

Audio:
- [ ] bevy_audio integration

## Specs

### Multi-Layer Equipment Rendering

NPCs need visible equipment: armor, helmet, weapon, and carried items (food icon when raiding). Each layer is a separate sprite from the same atlas, drawn on top of the body sprite. Uses the same approach as Godot's stacked MultiMesh — one instanced draw call per layer, with Transparent2d sort keys controlling z-order.

**Architecture: Multiple draw calls, one per layer (Factorio-style)**

Current renderer does 1 batch entity → 1 instance buffer → 1 draw call for all 10K NPCs. Extend to N layers where each layer is an independent instanced draw call with its own instance buffer. Only NPCs that have equipment in that slot appear in that layer's buffer (a layer with 200 carried-item sprites = 200 instances, not 10K).

Same pipeline, same shader (`npc_render.wgsl`), same `InstanceData` struct (48 bytes: position + sprite + color + health + flash + scale + atlas_id). Both character and world atlases are bound simultaneously — per-instance `atlas_id` selects which to sample.

**Data model:**

```
NpcBufferWrites (main world, extracted to render world):
  positions: Vec<f32>          ← existing (shared by all layers)
  sprite_indices: Vec<f32>     ← existing (body layer)
  colors: Vec<f32>             ← existing (body layer)
  armor_sprites: Vec<f32>      ← NEW (4 floats/NPC: col, row, 0, 0. Use -1 sentinel for "no armor")
  helmet_sprites: Vec<f32>     ← NEW (same layout)
  weapon_sprites: Vec<f32>     ← NEW (same layout)
  item_sprites: Vec<f32>       ← NEW (same layout, set when CarryingFood)
```

**Render world changes (`npc_render.rs`):**

```
NpcRenderBuffers:
  vertex_buffer: Buffer              ← existing (shared static quad)
  index_buffer: Buffer               ← existing (shared [0,1,2,0,2,3])
  layers: Vec<LayerBuffer>           ← NEW (replaces single instance_buffer)

LayerBuffer:
  instance_buffer: RawBufferVec<InstanceData>
  instance_count: u32
```

**Implementation steps:**

- [x] Add equipment sprite fields to `NpcBufferWrites` (`armor_sprites`, `helmet_sprites`, `weapon_sprites`, `item_sprites`)
- [x] Add ECS components: `EquippedArmor(col, row)`, `EquippedHelmet(col, row)`, `EquippedWeapon(col, row)` — sprite atlas coordinates
- [x] Add equipment to spawn: guards get weapon+helmet, farmers get nothing, raiders get weapon
- [x] Update `collect_gpu_updates` to write equipment sprites to `NpcBufferWrites` when equipment changes
- [x] Refactor `NpcRenderBuffers`: replace single `instance_buffer`/`instance_count` with `Vec<LayerBuffer>`
- [x] Refactor `prepare_npc_buffers`: build one `LayerBuffer` per layer, skipping NPCs with -1 sentinel in that slot
- [x] Refactor `queue_npcs`: add one `Transparent2d` phase item per non-empty layer with incrementing sort keys
- [x] Refactor `DrawNpcs`: read layer index from batch entity to select correct `LayerBuffer`
- [x] Set `CarryingFood` → write food sprite to `item_sprites`, clear on delivery
- [x] Set `Healing` → `sync_visual_sprites` writes HEAL_SPRITE to `healing_sprites` layer
- [x] Set `Resting` → `sync_visual_sprites` writes SLEEP_SPRITE to `status_sprites` layer

**Performance budget:**

| Layer | Instances (typical) | Buffer size | Draw call |
|-------|-------------------|-------------|-----------|
| Body | 10,000 | 320 KB | 1 |
| Armor | ~4,000 | 128 KB | 1 |
| Helmet | ~3,000 | 96 KB | 1 |
| Weapon | ~8,000 | 256 KB | 1 |
| CarriedItem | ~500 | 16 KB | 1 |
| **Total** | **~25,500** | **~816 KB** | **5** |

5 instanced draw calls is trivial GPU overhead. Factorio benchmarks 25K sprites/frame as normal load. Buffer upload is <1MB/frame. Bottleneck is fill rate (overdraw from transparent layers), not draw calls.

**Atlas and procedural notes:**

Two issues from the Godot shaders that the implementer needs to handle:

1. **Procedural effects vs sprite overlays.** The Godot `halo.gdshader` (golden healing halo, pulsing radial glow) and `sleep_icon.gdshader` (procedural "z" shape) were not atlas sprites — they were generated in the fragment shader. Options: (a) bake small icons into the character atlas and use the standard sprite pipeline, or (b) add a procedural shader path for overlay layers. Option (a) is simpler and keeps one shader for all layers.

2. **World atlas for items.** ✓ Resolved — both atlases are now bound simultaneously (group 0, bindings 0-3). Per-instance `atlas_id` selects character (0.0) or world (1.0) atlas. Carried items use world sprites via atlas_id=1.0.

**References:**
- [Factorio FFF #251](https://www.factorio.com/blog/post/fff-251) — sprite batching, per-layer draw queues
- [NSprites (Unity DOTS)](https://github.com/Antoshidza/NSprites) — one draw call per material, component-to-GPU sync
- Current implementation: `npc_render.rs` (RenderCommand pattern), `npc_render.wgsl` (unchanged)
- Architecture doc: [rendering.md](rendering.md)
- Godot reference shaders (kept until WGSL parity): `halo.gdshader`, `sleep_icon.gdshader`, `loot_icon.gdshader`, `item_icon.gdshader`, `npc_sprite.gdshader`

### Stat Resolution & Upgrades

All NPC stats are currently compile-time constants scattered across the codebase. Guards and raiders have identical `AttackStats::melee()`. `Personality::get_stat_multipliers()` computes damage/hp/speed/yield modifiers but nothing calls it. `HEAL_RATE`, `HEAL_RADIUS`, `Speed::default()` are all hardcoded. Upgrades, XP, traits, and policies are all the same thing — multipliers on base stats — so build one resolution system, not four.

**Architecture: Derive at point-of-use**

```
final_stat = base_stat[job] * upgrade_mult[town][stat] * level_mult[npc] * trait_mult[npc]
```

Never store computed stats on entities. Store `BaseAttackType(Melee | Ranged)` on the entity, resolve to full stats when needed. This avoids ordering bugs (which system writes last?), stale data, and debugging hell.

**`CombatConfig` resource** (`resources.rs`):

```rust
#[derive(Resource)]
pub struct CombatConfig {
    pub jobs: HashMap<Job, JobStats>,
    pub heal_rate: f32,          // replaces HEAL_RATE constant (5.0)
    pub heal_radius: f32,        // replaces HEAL_RADIUS constant (150.0)
    pub base_speed: f32,         // replaces Speed::default() (100.0)
}

pub struct JobStats {
    pub max_health: f32,         // 100.0
    pub damage: f32,             // guard=15, raider=10, farmer=5
    pub attack_range: f32,       // melee=150, ranged=300
    pub attack_cooldown: f32,    // melee=1.0, ranged=1.5
    pub projectile_speed: f32,   // melee=500, ranged=200
    pub projectile_lifetime: f32,// melee=0.5, ranged=3.0
    pub speed: f32,              // guard=100, raider=110, farmer=80
}
```

Initialize with current hardcoded values so Phase 1 is a pure refactor (no behavior change).

**`BaseAttackType` component** (replaces `AttackStats` on entities):

```rust
#[derive(Component, Clone, Copy)]
pub enum BaseAttackType { Melee, Ranged }
```

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
// Effect: 1.0 + level * pct_per_level (linear scaling)
const UPGRADE_PCT: [f32; UPGRADE_COUNT] = [
    0.10, 0.10, 0.05, 0.05,  // guard: health, attack, range, size
    0.08, 0.05, 0.10,         // attack speed (negative), move speed, alert radius
    0.15, 0.20, 0.0, 0.0,    // farm yield, farmer HP, farmer cap (+2 flat), guard cap (+10 flat)
    0.20, 0.10, 0.0,          // healing rate, food efficiency (10% per level), fountain radius (+24px flat)
];
```

**`resolve_combat_stats()` function** (new file `systems/stats.rs` or in `resources.rs`):

```rust
pub struct ResolvedStats {
    pub damage: f32,
    pub range: f32,
    pub cooldown: f32,
    pub projectile_speed: f32,
    pub projectile_lifetime: f32,
    pub max_health: f32,
    pub speed: f32,
}

pub fn resolve_combat_stats(
    job: Job,
    attack_type: BaseAttackType,
    town_idx: usize,
    level: i32,
    personality: &Personality,
    config: &CombatConfig,
    upgrades: &TownUpgrades,
) -> ResolvedStats {
    let base = &config.jobs[&job];
    let (trait_damage, trait_hp, trait_speed, _trait_yield) = personality.get_stat_multipliers();
    let level_mult = 1.0 + level as f32 * 0.01;
    let town = upgrades.levels.get(town_idx).copied().unwrap_or([0; UPGRADE_COUNT]);

    let upgrade_hp = 1.0 + town[UpgradeType::GuardHealth as usize] as f32 * UPGRADE_PCT[0];
    let upgrade_dmg = 1.0 + town[UpgradeType::GuardAttack as usize] as f32 * UPGRADE_PCT[1];
    let upgrade_range = 1.0 + town[UpgradeType::GuardRange as usize] as f32 * UPGRADE_PCT[2];
    let upgrade_speed = 1.0 + town[UpgradeType::MoveSpeed as usize] as f32 * UPGRADE_PCT[5];
    let cooldown_reduction = 1.0 - town[UpgradeType::AttackSpeed as usize] as f32 * UPGRADE_PCT[4];

    ResolvedStats {
        damage: base.damage * upgrade_dmg * trait_damage * level_mult,
        range: base.attack_range * upgrade_range,
        cooldown: base.attack_cooldown * cooldown_reduction.max(0.1),
        projectile_speed: base.projectile_speed,
        projectile_lifetime: base.projectile_lifetime,
        max_health: base.max_health * upgrade_hp * trait_hp * level_mult,
        speed: base.speed * upgrade_speed * trait_speed,
    }
}
```

**XP formula:**

```
level = floor(sqrt(xp / 100))
```

| Kills (1 XP each) | XP | Level | Stat mult |
|----|-----|-------|-----------|
| 1 | 100 | 1 | 1.01x |
| 4 | 400 | 2 | 1.02x |
| 25 | 2500 | 5 | 1.05x |
| 100 | 10000 | 10 | 1.10x |
| 10000 | 1000000 | 100 | 2.00x |

Grant 100 XP per kill. Use `combat_targets` in `death_cleanup_system` to identify the killer (the NPC whose `combat_target == dead_npc_index`).

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

**Files changed:**

| File | Changes |
|---|---|
| `resources.rs` | Add `CombatConfig`, `TownUpgrades`, `TownPolicies`, `UpgradeType`, `ResolvedStats`. Remove nothing (backward compatible). |
| `components.rs` | Add `BaseAttackType` enum. `AttackStats` kept temporarily for migration, removed after Phase 1 verified. |
| `systems/spawn.rs` | `spawn_npc_system` reads `CombatConfig`, inserts `BaseAttackType` instead of `AttackStats::melee()`. Sets `MaxHealth` from resolved stats. |
| `systems/combat.rs` | `attack_system` takes `Res<CombatConfig>`, `Res<TownUpgrades>`, `Res<NpcMetaCache>` params. Calls `resolve_combat_stats()` per-attacker instead of reading `&AttackStats`. |
| `systems/health.rs` | `healing_system` reads `CombatConfig.heal_rate` / `CombatConfig.heal_radius` instead of constants. Applies healing rate upgrade. |
| `systems/economy.rs` | `farm_growth_system` applies farm yield upgrade multiplier. `starvation_system` uses `CombatConfig.base_speed` instead of hardcoded 60.0. |
| `systems/behavior.rs` | `decision_system` reads `Res<TownPolicies>` for flee thresholds, work schedule, off-duty. Replaces `FleeThreshold` / `WoundedThreshold` component reads with policy lookups. |
| `ui/upgrade_menu.rs` | Enable buttons. Click → `apply_upgrade()` → deduct food → increment `TownUpgrades` level. Show current level and cost. |
| `ui/policies_panel.rs` | Replace `Local<PolicyState>` with `Res<TownPolicies>` read + `ResMut<TownPolicies>` write. Remove `ui.disable()`. |
| `ui/game_hud.rs` | NPC inspector shows level, XP, XP-to-next. |
| `constants.rs` | Remove `HEAL_RATE`-equivalent if any constants moved to `CombatConfig`. Keep grid/buffer constants unchanged. |

**Critical existing code to reuse:**

- `Personality::get_stat_multipliers()` (`components.rs:436`) — already computes `(damage, hp, speed, yield)` but nothing calls it. Wire into `resolve_combat_stats()`.
- `Personality::get_multipliers()` (`components.rs:402`) — already used by `decision_system` (behavior.rs:544) for utility AI scoring. No changes needed.
- `NpcMeta.level` / `NpcMeta.xp` (`resources.rs:261-262`) — already exist, set to 1/0 at spawn, never updated. Phase 3 activates these.
- `UPGRADES` array (`upgrade_menu.rs:17-32`) — 14 upgrade definitions with labels, tooltips, categories. Indices must match `UpgradeType` enum.
- `PolicyState` (`policies_panel.rs:10-24`) — exact field list for `TownPolicies::PolicySet`.
- `FleeThreshold`, `LeashRange`, `WoundedThreshold` (`components.rs:352-368`) — Phase 4 replaces these entity components with per-town policy lookups. Keep components for NPCs that need per-entity overrides (e.g., boss NPCs), but standard NPCs derive from policies.

**Verification per phase:**

Stage 8: `cargo check` clean. `cargo run --release` — game plays identically (pure refactor). All tests pass.
Stage 9: Upgrade guard attack in UI. Spawn new guards. They should deal more damage (visible in combat log kill speed). Let a guard get kills — NPC inspector shows level > 1, combat log shows "Level up" events.
Stage 10: Change raider flee threshold slider to 80%. Raiders should flee much earlier. Change work schedule to "Day Only" — farmers idle at night.

### GPU Readback & Extract Optimization

Two render bottlenecks dominate frame time: `readback_all` blocks 9ms with `device.poll(Wait)`, and `RenderExtractApp` clones 1.9MB of `NpcBufferWrites` at 18ms. Both are hand-rolled patterns that Bevy already solves.

**Problem 1: Blocking GPU readback (9ms)**

We manually create staging buffers, call `map_async`, then `device.poll(wait_indefinitely())` which blocks on ALL GPU work (not just our staging maps). Bevy 0.18 provides `Readback` + `ReadbackComplete` (`bevy::render::gpu_readback`) which handles async staging, mapping, and polling internally — zero blocking.

**Problem 2: ExtractResource clone (18ms)**

`NpcBufferWrites` is 1.9MB (15 Vecs × 16384 slots) cloned every frame via `ExtractResourcePlugin`. Only ~460KB is compute upload data (positions, targets, speeds, factions, healths, arrivals). The remaining ~1.4MB is render-only visual data (sprite_indices, colors, flash_values, 6 equipment/status sprite arrays) written by `sync_visual_sprites` and read by `prepare_npc_buffers`.

**Architecture: Bevy `Readback` for GPU→CPU, split struct for CPU→GPU**

```
BEFORE (hand-rolled):
  Main World                          Render World
  GPU_UPDATE_QUEUE (static Mutex)  →  populate_buffer_writes → NpcBufferWrites
  NpcBufferWrites (1.9MB)          →  ExtractResource CLONE → write_npc_buffers
                                      readback_all: map_async + device.poll(WAIT) → 9ms BLOCK
                                      → GPU_READ_STATE (static Mutex)
  sync_gpu_state_to_bevy           ←  GPU_READ_STATE (static Mutex)

AFTER (Bevy-native):
  Main World                          Render World
  GPU_UPDATE_QUEUE (static Mutex)  →  populate_buffer_writes → NpcComputeWrites (~50KB)
  NpcComputeWrites                 →  ExtractResource clone (tiny) → write_npc_buffers
  NpcVisualData                    →  static swap buffer (no clone) → prepare_npc_buffers
  ReadbackComplete observer        ←  Readback component (async, 0ms)
  → GpuReadState (Bevy Resource)
```

**We read back 4 buffers from GPU:**

| Buffer | Type | Size (700 NPCs) | Consumer |
|---|---|---|---|
| `positions` | `vec2<f32>` per NPC | 5.6 KB | ECS arrival detection + rendering |
| `combat_targets` | `i32` per NPC | 2.8 KB | `attack_system` |
| `proj_hits` | `[i32; 2]` per proj | varies | `process_proj_hits` |
| `proj_positions` | `vec2<f32>` per proj | varies | projectile rendering |

**Implementation steps:**

Step 1 — Create readback `ShaderStorageBuffer` assets (readback targets, not replacing compute buffers):
- [x] In `setup_readback_buffers` (Startup system), create 4 `ShaderStorageBuffer` assets via `Assets<ShaderStorageBuffer>`
- [x] Set `buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC` on each
- [x] Store handles in `ReadbackHandles` resource (derive `ExtractResource, Clone`)
- [x] Register `ExtractResourcePlugin::<ReadbackHandles>`
- [x] Sizes: npc_positions = `MAX_NPCS * 8` bytes, combat_targets = `MAX_NPCS * 4` bytes, proj_hits = `MAX_PROJECTILES * 8` bytes, proj_positions = `MAX_PROJECTILES * 8` bytes

```rust
#[derive(Resource, ExtractResource, Clone)]
pub struct ReadbackHandles {
    pub npc_positions: Handle<ShaderStorageBuffer>,
    pub combat_targets: Handle<ShaderStorageBuffer>,
    pub proj_hits: Handle<ShaderStorageBuffer>,
    pub proj_positions: Handle<ShaderStorageBuffer>,
}
```

Step 2 — Copy compute buffers to readback assets in compute nodes:
- [x] `NpcComputeNode::run`: `copy_buffer_to_buffer` from compute positions/combat_targets → readback asset buffers (via `RenderAssets<GpuShaderStorageBuffer>`)
- [x] `ProjectileComputeNode::run`: same for proj hits/positions
- [x] Note: compute buffers stay in `NpcGpuBuffers`/`ProjGpuBuffers` — readback assets are separate copy targets, not replacements

Step 3 — Spawn `Readback` entities:
- [x] Spawn 4 persistent entities with `Readback::buffer(handle.clone())` — Bevy re-reads every frame while component exists
- [x] Each entity gets an `.observe()` handler for `ReadbackComplete`

Step 4 — Handle `ReadbackComplete` events:
- [x] NPC positions observer: `to_shader_type::<Vec<f32>>()`, write to `Res<GpuReadState>` resource
- [x] Combat targets observer: `to_shader_type::<Vec<i32>>()`, write to `Res<GpuReadState>.combat_targets`
- [x] Proj hits observer: `to_shader_type::<Vec<[i32; 2]>>()`, write to `Res<ProjHitState>` Bevy Resource
- [x] Proj positions observer: `to_shader_type::<Vec<f32>>()`, write to `Res<ProjPositionState>` Bevy Resource

Step 5 — Delete hand-rolled readback code:
- [x] Delete `StagingIndex` resource
- [x] Delete `position_staging: [Buffer; 2]` and `combat_target_staging: [Buffer; 2]` from `NpcGpuBuffers`
- [x] Delete `hit_staging: [Buffer; 2]` and `position_staging: [Buffer; 2]` from `ProjGpuBuffers`
- [x] Delete all 8 staging buffer creations in `init_npc_compute_pipeline` and `init_proj_compute_pipeline`
- [x] Delete the entire `readback_all` function (~140 lines)
- [x] Delete system registration for `readback_all.in_set(RenderSystems::Cleanup)`
- [x] Delete `GPU_READ_STATE` static Mutex from `messages.rs`
- [x] Delete `PROJ_HIT_STATE` static Mutex from `messages.rs`
- [x] Delete `PROJ_POSITION_STATE` static Mutex from `messages.rs`

Step 6 — Update consumers of static Mutexes:
- [x] `systems/sync.rs`: deleted entirely — `ReadbackComplete` observers write directly to `Res<GpuReadState>`
- [x] `render.rs`: `click_to_select_system` reads `Res<GpuReadState>` directly
- [x] `systems/combat.rs`: `process_proj_hits` reads `Res<ProjHitState>` instead of `PROJ_HIT_STATE.lock()`
- [x] `npc_render.rs`: `prepare_npc_buffers` reads `Res<GpuReadState>` (extracted to render world)
- [x] `npc_render.rs`: `prepare_proj_buffers` reads `Res<ProjPositionState>` (extracted to render world)

Step 7 — Split `NpcBufferWrites` to reduce extract clone:
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
| `rust/src/gpu.rs` | ReadbackHandles resource, ShaderStorageBuffer creation, Readback entity spawning, ReadbackComplete observers, split NpcBufferWrites → NpcComputeWrites + NpcVisualData, delete staging buffers + readback_all + StagingIndex |
| `rust/src/messages.rs` | Delete `GPU_READ_STATE`, `PROJ_HIT_STATE`, `PROJ_POSITION_STATE` statics. Add `ProjHitState`, `ProjPositionState` Bevy Resources. |
| `rust/src/npc_render.rs` | `prepare_npc_buffers` reads visual data from `NPC_VISUAL_DATA` static, positions from `GpuReadState` resource |
| `rust/src/systems/sync.rs` | `sync_gpu_state_to_bevy` reads `GpuReadState` resource directly (no static) |
| `rust/src/systems/combat.rs` | `process_proj_hits` reads `ProjHitState` resource instead of static |
| `rust/src/resources.rs` | `GpuReadState` stays as Bevy Resource (already is), add `ProjHitState`, `ProjPositionState` |

**Verification:**

1. `cargo check` — 0 errors, 0 warnings
2. `cargo run --release` — NPCs move, patrol, combat works, projectiles hit
3. `cargo run --release --features tracy`:
   - `readback_all` span completely gone
   - `RenderExtractApp` < 5ms (down from 18ms)
   - No new blocking spans
4. Debug tests: run all — vertical-slice, combat, projectiles, healing should pass
5. Tracy: overall frame time should drop ~25ms (9ms readback + 13ms extract savings)

**Key API references:**

- `bevy::render::gpu_readback::{Readback, ReadbackComplete}` — [docs](https://docs.rs/bevy/latest/bevy/render/gpu_readback/enum.Readback.html)
- `bevy::render::storage::{ShaderStorageBuffer, GpuShaderStorageBuffer}` — buffer assets
- `bevy::render::render_asset::RenderAssets<GpuShaderStorageBuffer>` — access raw `Buffer` in render world
- `Readback::buffer(handle)` — per-frame async readback
- `ReadbackComplete::to_shader_type::<T>()` — typed deserialization from raw bytes
- Bevy example: `examples/shader/gpu_readback.rs`

## Performance

| Milestone | NPCs | FPS | Status |
|-----------|------|-----|--------|
| CPU Bevy | 5,000 | 60+ | ✓ |
| GPU physics | 10,000+ | 140 | ✓ |
| Full behaviors | 10,000+ | 140 | ✓ |
| Combat + projectiles | 10,000+ | 140 | ✓ |
| GPU spatial grid | 10,000+ | 140 | ✓ |
| Full game integration | 10,000+ | 60+ | Partial |
| Future (20K + equipment layers) | 20,000+ | 60+ | Planned |

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

## References

- [Simon Green's CUDA Particles](https://developer.download.nvidia.com/assets/cuda/files/particles.pdf) — GPU spatial grid approach
- [FSM in ECS](https://www.richardlord.net/blog/ecs/finite-state-machines-with-ash) — marker component pattern
- [Bevy Render Graph](https://docs.rs/bevy/latest/bevy/render/render_graph/) — compute + render pipeline
- [Factorio FFF #251](https://www.factorio.com/blog/post/fff-251) — sprite batching, per-layer draw queues
- [Factorio FFF #421](https://www.factorio.com/blog/post/fff-421) — entity update optimization, lazy activation
