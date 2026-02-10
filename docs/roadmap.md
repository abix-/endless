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
5. **New features** go in the appropriate stage. If no stage fits, add to Stage 7.
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
*Note: procedural generation, building grid, and building interactions were Godot-era features not yet ported to Bevy. They live in Stage 7.*

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

### Testing & Debug
- [x] Test harness with phased PASS/FAIL assertions
- [x] Test 12: Vertical slice (8 phases, spawn→readback→farm→raid→combat→death→respawn)
- [x] HEALTH_DEBUG, COMBAT_DEBUG resources for diagnostics

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

**Stage 5: Test Framework**

*Done when: every completed system has a dedicated test, tests are selectable from an in-game menu, and all tests pass.*

Test infrastructure:
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

Tests for completed features (one file each in `src/tests/`):

`movement` — Movement & Arrival (3 phases): **ALL PASS**
- [x] Phase 1: Spawn 3 NPCs, set targets — HasTarget added
- [x] Phase 2: GPU positions move toward target (not at origin)
- [x] Phase 3: NPCs reach destination — AtDestination added

`guard-patrol` — Guard Patrol Cycle (5 phases): **ALL PASS**
- [x] Phase 1: Guard spawns with OnDuty at first post
- [x] Phase 2: After GUARD_PATROL_WAIT ticks → Patrolling
- [x] Phase 3: Arrives at next post → OnDuty again
- [x] Phase 4: Energy < ENERGY_HUNGRY → goes to rest
- [x] Phase 5: Energy > ENERGY_RESTED → resumes patrol

`farmer-cycle` — Farmer Work Cycle (5 phases): **ALL PASS**
- [x] Phase 1: Farmer spawns with GoingToWork + HasTarget
- [x] Phase 2: Arrives at farm → Working marker
- [x] Phase 3: Energy drains below threshold → stops working
- [x] Phase 4: Goes home to rest
- [x] Phase 5: Energy recovers → returns to work

`raider-cycle` — Raider Raid Cycle (5 phases): **ALL PASS**
- [x] Phase 1: 3 raiders dispatched → Raiding marker on ≥3
- [x] Phase 2: Raiders arrive at farm
- [x] Phase 3: Food stolen (farm food decreases)
- [x] Phase 4: Raiders returning (Returning marker)
- [x] Phase 5: Food delivered (camp food increases)

`combat` — Combat Pipeline (6 phases): **ALL PASS**
- [x] Phase 1: 2 opposing NPCs — GPU targeting finds enemy
- [x] Phase 2: InCombat marker added
- [x] Phase 3: Projectile spawned or damage dealt
- [x] Phase 4: Health decreases
- [x] Phase 5: NPC dies (Dead marker or npc_count drops)
- [x] Phase 6: Slot freed, entity despawned

`economy` — Farm Growth & Respawn (5 phases): **ALL PASS**
- [x] Phase 1: Farm in Growing state
- [x] Phase 2: Farm transitions to Ready (farmer tending = faster rate)
- [x] Phase 3: Farmer harvests → food increases
- [x] Phase 4: Camp forage adds food over time
- [x] Phase 5: Raider respawns when camp has enough food

`energy` — Energy System (3 phases): **ALL PASS**
- [x] Phase 1: NPC starts at energy 100
- [x] Phase 2: Energy drains over time (< 90)
- [x] Phase 3: Energy reaches ENERGY_HUNGRY threshold

`healing` — Healing Aura (3 phases): **ALL PASS**
- [x] Phase 1: Damaged NPC (50 HP) inside town → Healing marker
- [x] Phase 2: Health increases toward max
- [x] Phase 3: Health reaches max → healing stops

`spawning` — Spawn & Slot Reuse (4 phases): **ALL PASS**
- [x] Phase 1: 5 NPCs exist with correct job components
- [x] Phase 2: Kill one (health → 0) → Dead marker
- [x] Phase 3: Slot freed in SlotAllocator
- [x] Phase 4: New spawn reuses freed slot index

`projectiles` — Projectile Pipeline (4 phases): **ALL PASS**
- [x] Phase 1: 2 ranged NPCs — combat targeting finds enemy
- [x] Phase 2: Projectile spawned (slot allocated)
- [x] Phase 3: Projectile hits → DamageMsg processed
- [x] Phase 4: Projectile slot freed

`world-gen` — World Generation (6 phases): **ALL PASS**
- [x] Phase 1: WorldGrid exists with correct dimensions (250x250)
- [x] Phase 2: Correct number of towns placed (villager + raider)
- [x] Phase 3: Towns are min 1200px apart
- [x] Phase 4: Each town has buildings: 1 fountain, 2 farms, 4 beds, 4 guard posts
- [x] Phase 5: Terrain near towns is Dirt
- [x] Phase 6: Raider camps exist with correct faction

`vertical-slice` — Full Core Loop (8 phases, time_scale=10): **ALL PASS**
- [x] Relocated from lib.rs to src/tests/vertical_slice.rs (same logic)

**Stage 6: Visual Feedback**

*Done when: you can watch the core loop happen on screen and understand what's going on without reading logs.*

Camera + viewport:
- [x] Replace hardcoded CAMERA_POS/VIEWPORT in npc_render.wgsl with camera uniform buffer
- [x] Camera pan (WASD) and zoom (scroll wheel toward cursor)
- [x] Click-to-select NPC wired to camera transform

Equipment rendering:
- [x] Multi-layer equipment rendering (see [spec](#multi-layer-equipment-rendering) below)
- [x] Guards spawn with weapon + helmet layers, raiders with weapon layer

Projectile rendering:
- [x] Projectile instanced pipeline (same RenderCommand pattern as NPC renderer)
- [x] Separate InstanceData buffer for active projectiles

Visual state indicators:
- [ ] Farm growth state visible (Growing → Ready sprite change + progress bar on tile)
- [x] Health bars (3-color: green/yellow/red, show-when-damaged mode in fragment shader)
- [x] Damage flash in npc_render.wgsl (white overlay on hit, fade out over ~0.2s via CPU-side decay)
- [ ] Healing glow effect (pulsing green tint + radial halo — needs TIME uniform in shader)
- [x] Sleep indicator on resting NPCs (SLEEP_SPRITE on status layer via sync_visual_sprites)
- [x] Healing indicator on healing NPCs (HEAL_SPRITE on healing layer via sync_visual_sprites)
- [x] Carried item icon (food sprite on returning raiders)

Visual indicator tests (green phase — dedicated render layers wired):

`sleep-visual` — Sleep Icon (3 phases): **ALL PASS**
- [x] Phase 1: Farmer spawns with energy > 0
- [x] Phase 2: Farmer rests → status_sprites shows SLEEP_SPRITE
- [x] Phase 3: Farmer wakes → status_sprites cleared to -1

`farm-visual` — Farm Ready Marker (3 phases): **ALL PASS**
- [x] Phase 1: Farm is Growing, no FarmReadyMarker entities
- [x] Phase 2: Farm reaches Ready → FarmReadyMarker entity spawned
- [x] Phase 3: Farmer harvests → FarmReadyMarker despawned, farm Growing again

`heal-visual` — Heal Icon (3 phases): **ALL PASS**
- [x] Phase 1: Damaged NPC (50 HP) → Healing marker
- [x] Phase 2: Healing NPC → healing_sprites shows HEAL_SPRITE
- [x] Phase 3: NPC healed → Healing removed, healing_sprites cleared

**Stage 7: Playable Game**

*Done when: someone who isn't you can open it, understand what's happening, and make decisions that affect the outcome.*

World setup:
- [x] Procedural town/farm/bed/guard_post placement (2 towns default, 1200px spacing, random layout)
- [x] Named towns from pool of Florida cities
- [x] WorldGrid (250x250 cells, 32px each, terrain biome + building per cell)
- [x] WorldGenConfig resource (world size, town count, spacing, NPC counts)
- [ ] Building grid expansion (6x6 start, expandable to 100x100)
- [ ] Visible world border with corner markers
- [x] GameConfig + WorldGenConfig Bevy resources (replaces config.gd)
- [ ] User settings persistence (serde JSON)

UI:
- [x] bevy_egui start menu with world config sliders (ui/main_menu.rs)
- [x] Game HUD: population, time, food, kill stats, NPC inspector (ui/game_hud.rs)
- [ ] roster_panel.rs (NPC list with sorting/filtering)
- [ ] build_menu.rs (building placement UI)
- [ ] combat_log.rs (event feed)
- [ ] upgrade_menu.rs, policies_panel.rs

Input:
- [ ] Click-to-build and click-to-destroy buildings
- [ ] Villager role assignment
- [ ] Train guards from population
- [x] Time controls: Space=pause, +/-=speed (ui/mod.rs), ESC=back to menu

Building system:
- [ ] Runtime add/remove farm/bed/guard_post
- [ ] Slot unlock system (spend food to unlock grid slots)
- [ ] Town circle indicator expands with building range
- [ ] NPCs claim and use new buildings
- [ ] Guard post auto-attack (turret behavior, fires at enemies)
- [ ] Guard post upgrades (attack_enabled, range_level, damage_level)

Events:
- [ ] Death events (npc_idx, job, faction, town_idx)
- [ ] Combat log feed from events
- [ ] UI integration for event display

**Stage 8: Config & Progression**

*Done when: upgrades change gameplay outcomes — upgraded guards survive longer, leveled NPCs deal more damage, and town policies visibly alter NPC behavior.*

Config & upgrades:
- [ ] CombatConfig Bevy resource (configurable melee/ranged stats)
- [ ] CombatConfig initialization from GameConfig at startup
- [ ] spawn_npc_system reads config instead of hardcoded AttackStats
- [ ] apply_upgrade(town_idx, upgrade_type, level) API for stat multipliers
- [ ] Guard upgrades: health, attack, range, size bonuses per town
- [ ] Farmer upgrades: HP bonus per town
- [ ] Healing rate upgrade (fountain/camp regen multiplier)
- [ ] Structure upgrades (increase output, capacity, defense)

XP & leveling:
- [ ] Level, Xp components on NPCs
- [ ] grant_xp() API and system
- [ ] Level-up system (sqrt scaling: level 9999 = 100x stats)
- [ ] Stat scaling (damage, max_health based on level)
- [ ] Level-up event for UI notification

Town policies:
- [ ] Work schedule policies (day only, night only, both shifts)
- [ ] Off-duty policies (go to bed, stay at fountain, wander town)
- [ ] Recovery threshold policies (prioritize_healing, custom recovery %)
- [ ] Fountain healing zone (radius + upgrade bonus)
- [ ] Camp healing zone for raiders

**Stage 9: Gameplay Depth**

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

**Stage 10: Endgame**

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
