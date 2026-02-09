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
- [x] GPU→ECS readback (positions + combat targets via staging buffer)
- [x] projectile_compute.wgsl compute shader
- [x] Multi-dispatch NpcComputeNode with 3 bind groups
- [x] atomicAdd for thread-safe grid cell insertion

### Instanced Rendering
- [x] RenderCommand + Transparent2d phase (single instanced draw call)
- [x] 2D camera setup, texture atlas loading (char + world sprites)
- [x] Sprite texture sampling with alpha discard and color tinting

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
- [x] Damage flash effect
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
- [x] Procedural town generation (1-7 towns, 1200px minimum spacing)
- [x] Named towns from pool of 15 Florida cities
- [x] Farms (2 per town, 200-300px from center)
- [x] Homes/beds for farmers (ring 350-450px from center, 16 starting beds)
- [x] Guard posts (4 per town at corners, clockwise patrol)
- [x] Raider camps (positioned away from all towns)
- [x] Visible world border with corner markers
- [x] Building grid (6x6 start, expandable to 100x100)
- [x] Destructible buildings (right-click slot → Destroy)
- [x] Build new structures (right-click empty slots - farms, beds, guard posts)
- [x] Double-click locked slots to unlock (1 food each)
- [x] Town circle indicator expands with building range

### World Data
- [x] Towns, farms, beds, guard posts as Bevy resources
- [x] Occupancy tracking (reserve/release beds and farms)
- [x] Query APIs: get_town_center, get_camp_position, get_patrol_post
- [x] Query APIs: get_nearest_free_bed/farm
- [x] init_world, add_town/farm/bed/guard_post APIs

### UI Integration
- [x] Click to select NPC (get_npc_at_position with radius)
- [x] Inspector panel (name, HP, job, energy, state)
- [x] Roster panel with sorting/filtering
- [x] Population stats and kill tracking (KILL_STATS)
- [x] Name generation ("Adjective Noun" by job)
- [x] NpcMetaCache resource (name, level, xp, trait, town_id, job per NPC)
- [x] NpcEnergyCache resource (energy per NPC)
- [x] NpcsByTownCache resource (per-town NPC lists)
- [x] PopulationStats, KillStats, SelectedNpc resources

### Testing & Debug
- [x] Test harness with automated PASS/FAIL assertions
- [x] Test 1-5: Movement scenarios (arrive, separation, both, circle, mass)
- [x] Test 6: World data visual markers (town, camp, farms, beds, posts)
- [x] Test 7: Guard patrol (4 guards patrol perimeter clockwise)
- [x] Test 8: Farmer work cycle
- [x] Test 9: Health/death validation
- [x] Test 10: Combat TDD (6 phases)
- [x] Test 11: Unified attacks TDD (7 phases)
- [x] Test 12: Vertical slice (8 phases, spawn→readback→farm→raid→combat→death→respawn, 6.8s)
- [x] get_npc_position() for position queries
- [x] HEALTH_DEBUG, COMBAT_DEBUG resources for diagnostics

### Architecture
- [x] Static queues → Bevy Messages (MessageWriter/MessageReader)
- [x] All statics → Bevy Resources (WorldData, Debug, KillStats, NpcMeta, FoodEvents, etc.)
- [x] GpuUpdateMsg batching via collect_gpu_updates
- [x] Godot bridge removed (channels.rs, api.rs, rendering.rs, EcsNpcManager)
- [x] All GDScript files removed (npc_manager, npc_combat, npc_needs, gpu_separation, etc.)
- [x] All .glsl shaders replaced with .wgsl

## Stages

**Stage 1: Standalone Bevy App ✓**

**Stage 2: GPU Compute ✓**

**Stage 3: GPU Instanced Rendering ✓**

*Lesson learned: render graph Nodes are for compute/post-processing, not geometry. Switched to RenderCommand pattern after first approach failed.*

**Stage 4: Core Loop ✓**

*Done when: 5 farmers grow food, 5 raiders form a group and steal it, 2 guards intercept with combat, someone dies, slot recycles, replacement spawns. Validated by Test 12 (8 phases, 6.8s).*

**Stage 5: Visual Feedback**

*Done when: you can watch the core loop happen on screen and understand what's going on without reading logs.*

Camera + viewport:
- [x] Replace hardcoded CAMERA_POS/VIEWPORT in npc_render.wgsl with camera uniform buffer
- [x] Camera pan (WASD) and zoom (scroll wheel toward cursor)
- [x] Click-to-select NPC wired to camera transform

Equipment rendering:
- [ ] Multi-layer equipment rendering (see [spec](#multi-layer-equipment-rendering) below)
- [ ] Guards spawn with weapon + helmet layers, raiders with weapon layer

Projectile rendering:
- [x] Projectile instanced pipeline (same RenderCommand pattern as NPC renderer)
- [x] Separate NpcInstanceData buffer for active projectiles

Visual state indicators:
- [ ] Farm growth state visible (Growing → Ready sprite change + progress bar)
- [ ] Health bars (3-color: green/yellow/red, configurable show mode: off/damaged/always)
- [ ] Damage flash in npc_render.wgsl (white overlay on hit, fade out — was Godot-only, needs re-impl)
- [ ] Healing glow effect (pulsing green tint + radial halo — needs TIME uniform in shader)
- [ ] Sleep indicator on resting NPCs (z icon overlay)
- [ ] Carried item icon (food sprite on returning raiders)

**Stage 6: Playable Game**

*Done when: someone who isn't you can open it, understand what's happening, and make decisions that affect the outcome.*

World setup:
- [ ] Procedural town/farm/bed/guard_post placement
- [ ] Port config.gd → Bevy Resource
- [ ] Port user_settings.gd → serde JSON

UI:
- [ ] bevy_egui start menu (new game, settings)
- [ ] left_panel.rs (stats, perf, inspector)
- [ ] roster_panel.rs, build_menu.rs, combat_log.rs
- [ ] upgrade_menu.rs, policies_panel.rs

Input:
- [ ] Click-to-build and click-to-destroy buildings
- [ ] Villager role assignment
- [ ] Train guards from population
- [ ] Time controls (pause, speed)

Building system:
- [ ] Runtime add/remove farm/bed/guard_post
- [ ] NPCs claim and use new buildings
- [ ] Guard post auto-attack (turret behavior, fires at enemies)
- [ ] Guard post upgrades (attack_enabled, range_level, damage_level)

Events:
- [ ] Death events (npc_idx, job, faction, town_idx)
- [ ] Combat log feed from events
- [ ] UI integration for event display

**Stage 7: Content + Polish**

*Done when: there's enough systems depth that emergent gameplay happens — raids succeed or fail based on guard upgrades, economy collapses if farms aren't defended, raiders starve if they can't steal.*

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

Combat depth:
- [ ] Target switching (prefer non-fleeing enemies over fleeing)
- [ ] Trait combat modifiers (Strong +25%, Berserker +50% at low HP, Efficient -25% cooldown, Lazy +20% cooldown)
- [ ] Trait flee modifiers (Brave never flees, Coward +20% threshold)
- [ ] Trait combinations (multiple traits per NPC)
- [ ] Player combat abilities
- [ ] Army units (peasant levy, archers, knights)
- [ ] Equipment crafting (weapons, armor)
- [ ] Army recruitment and movement
- [ ] Attack and capture enemy towns

Economy depth:
- [ ] Multi-camp food delivery (currently hardcoded to camp_food[0])
- [ ] HP regen system (3x sleeping, 10x fountain/camp with upgrade)
- [ ] Food consumption (eating restores HP/energy, npc_ate_food event)
- [ ] Food efficiency upgrade (chance of free meal)
- [ ] Starvation effects (HP drain, desertion)
- [ ] Multiple resources (wood, iron, gold)
- [ ] Production buildings (lumber mill, mine, blacksmith)

AI & coordination:
- [ ] AI lords that expand and compete
- [ ] count_nearby_raiders() for group behavior
- [ ] get_raider_group_center() for coordinated movement
- [ ] find_nearest_raider() for regrouping

Performance:
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

Same pipeline, same shader (`npc_render.wgsl`), same `NpcInstanceData` struct (32 bytes: position + sprite + color). Most layers use the character atlas (`roguelikeChar`, 918×203). See "Atlas and procedural notes" below for exceptions.

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
  instance_buffer: RawBufferVec<NpcInstanceData>
  instance_count: u32
```

**Implementation steps:**

- [ ] Add equipment sprite fields to `NpcBufferWrites` (`armor_sprites`, `helmet_sprites`, `weapon_sprites`, `item_sprites`)
- [ ] Add ECS components: `EquippedArmor(col, row)`, `EquippedHelmet(col, row)`, `EquippedWeapon(col, row)` — sprite atlas coordinates
- [ ] Add equipment to spawn: guards get weapon+helmet, farmers get nothing, raiders get weapon
- [ ] Update `collect_gpu_updates` to write equipment sprites to `NpcBufferWrites` when equipment changes
- [ ] Refactor `NpcRenderBuffers`: replace single `instance_buffer`/`instance_count` with `Vec<LayerBuffer>`
- [ ] Refactor `prepare_npc_buffers`: build one `LayerBuffer` per layer, skipping NPCs with -1 sentinel in that slot
- [ ] Refactor `queue_npcs`: add one `Transparent2d` phase item per non-empty layer with incrementing sort keys (body=0.0, armor=0.001, helmet=0.002, weapon=0.003, item=0.004)
- [ ] Refactor `DrawNpcs`: read layer index from batch entity to select correct `LayerBuffer`. Add `LayerIndex(usize)` component to batch entities, or spawn separate `NpcBatch` entities per layer
- [ ] Set `CarryingFood` → write food sprite to `item_sprites`, clear on delivery
- [ ] Set `Healing` → write halo sprite to `item_sprites` (or dedicated healing layer)
- [ ] Set `Resting` → write sleep icon to `item_sprites`
- [ ] Test: spawn 100 NPCs with mixed equipment, verify layers render in correct order
- [ ] Test: 10K NPCs × 5 layers, verify fps stays above 60

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

2. **World atlas for items.** The Godot `loot_icon.gdshader` and `item_icon.gdshader` sample from `roguelikeSheet` (world atlas, 968×526), not the character atlas (918×203). Carried items (food) and farm items use world sprites. Options: (a) copy needed item sprites into the character atlas, (b) bind the world atlas as a second texture and select per-layer, or (c) use a separate pipeline for item layers with world atlas bind group. Option (b) is cleanest — add a `texture_index` to `LayerBuffer` and bind both atlases.

**References:**
- [Factorio FFF #251](https://www.factorio.com/blog/post/fff-251) — sprite batching, per-layer draw queues
- [NSprites (Unity DOTS)](https://github.com/Antoshidza/NSprites) — one draw call per material, component-to-GPU sync
- Current implementation: `npc_render.rs` (RenderCommand pattern), `npc_render.wgsl` (unchanged)
- Architecture doc: [rendering.md](rendering.md)
- Godot reference shaders (kept until WGSL parity): `halo.gdshader`, `sleep_icon.gdshader`, `loot_icon.gdshader`, `item_icon.gdshader`, `npc_sprite.gdshader`

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
