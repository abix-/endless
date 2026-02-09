# Roadmap

Target: 20,000+ NPCs @ 60fps with pure Bevy ECS + WGSL compute + GPU instanced rendering.

## How to Maintain This Roadmap

This file has two views of the same work:

- **Phases** = what order we build things. Each phase has a "done when" sentence and checkboxes grouped by problem. Phases are gameplay-driven milestones (core loop works → you can see it → someone can play it → it's a full game).
- **Capabilities** = what features exist and what's planned. Feature inventory with `[x]`/`[ ]` checkboxes. This is the backlog.

Rules:
1. **Phases are the priority.** When deciding what to work on, read the phases top-down. The first unchecked phase is the current sprint.
2. **Don't duplicate work items** between phases and capabilities. Phases reference capability sections when detail exists there (e.g., "per roadmap spec" pointing to Multi-Layer Equipment Rendering).
3. **Completed checkboxes are accomplishments.** Never delete them. Mark with `[x]` and add ✓ to the phase header when all items are done.
4. **"Done when" sentences don't change** unless the game design changes. They define the goal, not the implementation.
5. **Current State reflects phase priority.** Keep the Next → Then → Later structure in sync with the phases.
6. **New features** go in the Capabilities section first. They get pulled into a phase when it's time to build them.
7. **Godot lineage breadcrumbs** (like "Port config.gd → Bevy Resource") are intentional — they show where the design originated.

## Phases

**Phase 1: Standalone Bevy App ✓**
- [x] Cargo.toml (bevy 0.18, bevy_egui 0.39, bytemuck)
- [x] main.rs with App runner
- [x] Test: `cargo run` opens window

**Phase 2: GPU Compute via wgpu ✓**
- [x] npc_compute.wgsl compute shader
- [x] wgpu compute pipeline (Bevy render graph)
- [x] Pipeline compiles and dispatches
- [x] ECS→GPU buffer writes (NpcBufferWrites + write_npc_buffers)
- [x] projectile_compute.wgsl compute shader
- [x] GPU→ECS readback (positions for sprite sync)
- [x] Test: NPCs move (log positions)

**Phase 2.5: GPU Instanced Rendering ✓**

*Note: Original render graph Node approach failed — Nodes are for compute/post-processing, not geometry. Switched to RenderCommand pattern.*

Old approach (abandoned):
- [x] Add sprite_indices and colors buffers to NpcGpuBuffers
- [x] Create quad vertex/index buffers (NpcRenderMesh)
- [x] Create NpcRenderPipeline with bind group layouts
- [x] Create npc_render.wgsl shader (vertex + fragment)
- [x] Add NpcRenderNode to render graph
- [x] Create bind group preparation systems
- [x] Connect sprite texture from render module
- [x] Implement draw call in NpcRenderNode::run()
- [❌] Test: NPCs visible via GPU instancing - **FAILED: Nothing renders**

New approach (RenderCommand pattern):
- [x] Research correct Bevy pattern (mesh2d_manual, custom_phase_item examples)
- [x] Create npc_render.rs module with RenderCommand implementation
- [x] Update shader for instance vertex buffer input (@location 2-4)
- [x] Fix Bevy 0.18 API differences (RenderSystems, lifetimes, etc.)
- [x] Test: NPCs visible via Transparent2d phase
- [x] Enable sprite texture sampling (currently solid color debug)

**Phase 3: Sprite Rendering ✓**
- [x] 2D camera setup
- [x] Texture atlas loading (char + world sprites)
- [x] Test sprites rendering (8 visible)
- [ ] Full instancing for 10K NPCs (moved to Phase 2.5)
- [ ] Test: NPCs visible @ 140fps

**Phase 4: Core Loop**

*Done when: 5 farmers grow food, 5 raiders form a group and steal it, 2 guards intercept with combat, someone dies, slot recycles, replacement spawns. Validated by Test 12.*

GPU→CPU readback:
- [x] Populate GpuReadState with positions from staging buffer every frame
- [x] Measure readback latency (expect <1ms for 80KB at 10K NPCs)
- [x] Systems that read NPC positions (arrival, targeting, healing) use readback data

Spatial grid on GPU:
- [x] npc_compute.wgsl: 3-mode dispatch (clear grid, build grid, movement+targeting)
- [x] atomicAdd for thread-safe grid cell insertion
- [x] Combat targeting via grid neighbor search (300px range, ~6 cell radius)
- [x] Multi-dispatch NpcComputeNode with 3 bind groups
- [x] Combat targets readback (dual buffer map with positions)

Combat end-to-end:
- [x] attack_system: read target positions from GpuReadState, fire projectiles
- [x] Projectile firing via PROJ_GPU_UPDATE_QUEUE (slot allocation, velocity calculation)
- [x] Point-blank damage path for overlapping NPCs (avoids NaN velocity)
- [x] Projectile hit detection → readback → damage_system → health update
- [x] death_system → death_cleanup_system (release farm, remove from raid queue, update stats)
- [x] Respawn via raider_respawn_system consumes camp food, allocates recycled slot

Vertical slice test (Test 12):
- [x] Phase 1: Spawn 5 farmers (faction 0), 5 raiders (faction 1), 2 guards (faction 0)
- [x] Phase 2: GPU readback returns valid positions (not all zeros)
- [x] Phase 3: Farmers arrive at farms, begin working
- [x] Phase 4: Raiders form group (RaidQueue hits 3), dispatched to farm
- [x] Phase 5: Guards acquire targets via GPU spatial grid targeting
- [x] Phase 6: Damage applied (health decreases)
- [x] Phase 7: At least one death occurs, slot added to SlotAllocator free list
- [x] Phase 8: Replacement raider spawns from camp food budget
- [x] Test: PASS/FAIL with phase results showing timestamp + values at each gate (6.8s)

**Phase 5: Visual Feedback**

*Done when: you can watch the core loop happen on screen and understand what's going on without reading logs.*

Camera + viewport:
- [ ] Replace hardcoded CAMERA_POS/VIEWPORT in npc_render.wgsl with Bevy view uniforms
- [ ] Camera pan (WASD or drag) and zoom (scroll wheel)
- [ ] Click-to-select NPC wired to camera transform

Equipment rendering (multi-layer instanced):
- [ ] Implement multi-layer equipment rendering per roadmap spec (see capability section below)
- [ ] Guards spawn with weapon + helmet layers, raiders with weapon layer

Projectile rendering:
- [ ] Projectile instanced pipeline (same RenderCommand pattern as NPC renderer)
- [ ] Separate NpcInstanceData buffer for active projectiles

Visual state indicators:
- [ ] Farm growth state visible (Growing → Ready sprite change)
- [ ] Health bars or floating damage numbers
- [ ] Carried item icon (food sprite on returning raiders)

**Phase 6: Playable Game**

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
- [ ] Time controls (pause, speed)

**Phase 7: Content + Polish**

*Done when: there's enough systems depth that emergent gameplay happens — raids succeed or fail based on guard upgrades, economy collapses if farms aren't defended, raiders starve if they can't steal.*

- [ ] Config & upgrades: config-driven stats, apply_upgrade() API
- [ ] XP & leveling system
- [ ] Town policies (work schedules, off-duty behavior, recovery thresholds)
- [ ] Multiple resources (wood, iron, gold) + production buildings
- [ ] Army units, equipment crafting, recruitment
- [ ] AI lords that expand and compete
- [ ] Audio (bevy_audio)
- [ ] Entity sleeping (Factorio-style, NPCs outside camera radius sleep)

## Current State

**Done:**
- [x] Spawning, rendering, movement, physics (10,000+ NPCs @ 140fps)
- [x] Combat (GPU targeting, projectiles, melee/ranged)
- [x] NPC behaviors (guards, farmers, raiders with energy/flee/leash)
- [x] Economy (food production, theft, respawning)
- [x] World data (towns, farms, beds, guard posts)
- [x] UI integration (selection, inspector, roster panels)
- [x] Testing harness (11 test scenarios + Test 12 vertical slice)
- [x] Architecture cleanup (channels, Bevy resources, GPU messages)
- [x] Phase 4 core loop: GPU readback, spatial grid, combat targeting, arrival detection, Test 12 passes (6.8s)

**Next: Phase 5 (Visual Feedback)**
- [ ] Camera controls (remove hardcoded constants, add pan/zoom)
- [ ] Multi-layer equipment rendering (armor, helmet, weapon, carried item)
- [ ] Projectile instanced rendering
- [ ] Farm/health/item visual indicators

**Later: Phase 6-7 (Playable Game → Content)**
- [ ] World generation, start menu, UI panels, input handling
- [ ] Config & upgrades, XP, town policies, multiple resources

## Architecture

See [gpu-compute.md](gpu-compute.md) for GPU buffers, optimizations, and performance lessons. See [messages.md](messages.md) for data ownership and channel architecture. See [frame-loop.md](frame-loop.md) for execution order.

## Capabilities

### Spawning & Rendering ✓
- [x] NPCs spawn with jobs (guard, farmer, raider, fighter)
- [x] GPU instanced rendering via RenderCommand + Transparent2d (10,000+ @ 140fps)
- [x] Sprite frames, faction colors
- [x] Unified spawn API with job-as-template pattern (phase 8.5)
- [x] spawn_guard(), spawn_guard_at_post(), spawn_farmer() convenience APIs
- [x] Slot reuse for dead NPCs (SlotAllocator)
- [ ] Multi-layer equipment rendering (armor, helmet, weapon, carried item overlays)
- [ ] Health bar overlay

### Movement & Physics ✓
- [x] GPU compute shader for movement toward targets
- [x] set_target(npc_index, x, y) API for directing NPCs
- [x] Separation physics (boids-style, no pile-ups)
- [x] Spatial grid for O(1) neighbor lookups (80x80 cells, 100px each, 64 NPCs/cell max)
- [x] Arrival detection with persistent flag
- [x] TCP-style backoff for blocked NPCs
- [x] Zero-distance fallback (golden angle when NPCs overlap exactly)
- [x] reset() function for scene reload

### Combat ✓
- [x] GPU targeting (nearest enemy within 300px via spatial grid)
- [x] Projectile system (50k projectiles, GPU compute shader)
- [x] Melee attacks (range=150, speed=500, lifetime=0.5s)
- [x] Ranged attacks (range=300, speed=200, lifetime=3.0s)
- [x] Health, damage, death systems
- [x] O(1) entity lookup via NpcEntityMap
- [x] Projectile slot reuse (ProjSlotAllocator)
- [ ] Projectile instanced rendering
- [x] Damage flash effect
- [x] Guards have no leash (fight anywhere)
- [x] Alert nearby allies when combat starts
- [ ] Player combat abilities
- [ ] Army units (peasant levy, archers, knights)
- [ ] Equipment crafting (weapons, armor)
- [ ] Army recruitment and movement
- [ ] Attack and capture enemy towns

### NPC Behaviors ✓
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
- [ ] Target switching (prefer non-fleeing enemies over fleeing)
- [ ] Trait combat modifiers (Strong +25%, Berserker +50% at low HP, Efficient -25% cooldown, Lazy +20% cooldown)
- [ ] Trait flee modifiers (Brave never flees, Coward +20% threshold)
- [ ] Trait combinations (multiple traits per NPC)
- [ ] AI lords that expand and compete

### Economy ✓
- [x] Food production (farmers generate food per hour)
- [x] Food theft (raiders steal and deliver to camp)
- [x] Respawning (dead NPCs respawn after cooldown via RespawnTimers)
- [x] Per-town food storage (FoodStorage resource)
- [x] GameTime resource (time_scale, pause, hourly tick events)
- [x] GameConfig resource (farmers/guards per town, spawn interval, food per hour)
- [x] PopulationStats resource (alive/working counts per job/clan)
- [x] economy_tick_system (unified hourly economy)
- [ ] Multi-camp food delivery (currently hardcoded to camp_food[0])
- [ ] HP regen system (3x sleeping, 10x fountain/camp with upgrade)
- [ ] Food consumption (eating restores HP/energy, npc_ate_food event)
- [ ] Food efficiency upgrade (chance of free meal)
- [x] Population caps per town (upgradeable)
- [ ] Starvation effects (HP drain, desertion)
- [ ] Multiple resources (wood, iron, gold)
- [ ] Production buildings (lumber mill, mine, blacksmith)

### World Generation ✓
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
- [ ] Structure upgrades (increase output, capacity, defense)

### World Data ✓
- [x] Towns, farms, beds, guard posts as Bevy resources
- [x] Occupancy tracking (reserve/release beds and farms)
- [x] Query APIs: get_town_center, get_camp_position, get_patrol_post
- [x] Query APIs: get_nearest_free_bed/farm
- [x] init_world, add_town/farm/bed/guard_post APIs

### UI Integration ✓
- [x] Click to select NPC (get_npc_at_position with radius)
- [x] Inspector panel (name, HP, job, energy, state)
- [x] Roster panel with sorting/filtering
- [x] Population stats and kill tracking (KILL_STATS)
- [x] Name generation ("Adjective Noun" by job)
- [x] NpcMetaCache resource (name, level, xp, trait, town_id, job per NPC)
- [x] NpcEnergyCache resource (energy per NPC)
- [x] NpcsByTownCache resource (per-town NPC lists)
- [x] PopulationStats, KillStats, SelectedNpc resources
- [ ] Villager role assignment UI
- [ ] Train guards from population

### Multi-Layer Equipment Rendering

NPCs need visible equipment: armor, helmet, weapon, and carried items (food icon when raiding). Each layer is a separate sprite from the same atlas, drawn on top of the body sprite. Uses the same approach as Godot's stacked MultiMesh — one instanced draw call per layer, with Transparent2d sort keys controlling z-order.

**Architecture: Multiple draw calls, one per layer (Factorio-style)**

Current renderer does 1 batch entity → 1 instance buffer → 1 draw call for all 10K NPCs. Extend to N layers where each layer is an independent instanced draw call with its own instance buffer. Only NPCs that have equipment in that slot appear in that layer's buffer (a layer with 200 carried-item sprites = 200 instances, not 10K).

Same pipeline, same shader (`npc_render.wgsl`), same texture atlas, same `NpcInstanceData` struct (32 bytes: position + sprite + color). No shader changes needed.

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

**References:**
- [Factorio FFF #251](https://www.factorio.com/blog/post/fff-251) — sprite batching, per-layer draw queues
- [NSprites (Unity DOTS)](https://github.com/Antoshidza/NSprites) — one draw call per material, component-to-GPU sync
- Current implementation: `npc_render.rs` (RenderCommand pattern), `npc_render.wgsl` (unchanged)
- Architecture doc: [rendering.md](rendering.md)

### Building System
- [ ] Runtime add/remove farm/bed/guard_post
- [ ] Click-to-build and click-to-destroy input handling
- [ ] NPCs claim and use new buildings
- [ ] Guard post auto-attack (turret behavior, fires at enemies)
- [ ] Guard post upgrades (attack_enabled, range_level, damage_level)

### XP & Leveling
- [ ] Level, Xp components on NPCs
- [ ] grant_xp() API and system
- [ ] Level-up system (sqrt scaling: level 9999 = 100x stats)
- [ ] Stat scaling (damage, max_health based on level)
- [ ] Level-up event for UI notification

### Config & Upgrades
- [ ] CombatConfig Bevy resource (configurable melee/ranged stats)
- [ ] CombatConfig initialization from GameConfig at startup
- [ ] spawn_npc_system reads config instead of hardcoded AttackStats
- [ ] apply_upgrade(town_idx, upgrade_type, level) API for stat multipliers
- [ ] Guard upgrades: health, attack, range, size bonuses per town
- [ ] Farmer upgrades: HP bonus per town
- [ ] Healing rate upgrade (fountain/camp regen multiplier)

### Town Policies
- [ ] Work schedule policies (day only, night only, both shifts)
- [ ] Off-duty policies (go to bed, stay at fountain, wander town)
- [ ] Recovery threshold policies (prioritize_healing, custom recovery %)
- [ ] Fountain healing zone (radius + upgrade bonus)
- [ ] Camp healing zone for raiders

### Event System
- [ ] Death events (npc_idx, job, faction, town_idx)
- [ ] Combat log feed from events
- [ ] UI integration for event display

### Testing & Debug ✓
- [x] Test harness with automated PASS/FAIL assertions
- [x] Test 1-5: Movement scenarios (arrive, separation, both, circle, mass)
- [x] Test 6: World data visual markers (town, camp, farms, beds, posts)
- [x] Test 7: Guard patrol (4 guards patrol perimeter clockwise)
- [x] Test 8: Farmer work cycle
- [x] Test 9: Health/death validation
- [x] Test 10: Combat TDD (6 phases)
- [x] Test 11: Unified attacks TDD (7 phases)
- [x] get_npc_position() for position queries
- [x] HEALTH_DEBUG, COMBAT_DEBUG resources for diagnostics

### Architecture Cleanup ✓

- [x] Static queues → Bevy Messages (MessageWriter/MessageReader)
- [x] All statics → Bevy Resources (WorldData, Debug, KillStats, NpcMeta, FoodEvents, etc.)
- [x] GpuUpdateMsg batching via collect_gpu_updates
- [x] Godot bridge removed (channels.rs, api.rs, rendering.rs, EcsNpcManager)

### Performance Optimizations
- [ ] Entity sleeping (Factorio-style: NPCs outside camera radius sleep)
- [ ] awake/sleep_timers per NPC, ACTIVE_RADIUS check
- [ ] Combat/raiding states force awake

### Raider Coordination
- [ ] count_nearby_raiders() for group behavior
- [ ] get_raider_group_center() for coordinated movement
- [ ] find_nearest_raider() for regrouping

### GDScript Cleanup ✓

- [x] All GDScript files removed (npc_manager, npc_combat, npc_needs, gpu_separation, etc.)
- [x] All .glsl shaders replaced with .wgsl

## Performance

| Milestone | NPCs | FPS | Status |
|-----------|------|-----|--------|
| CPU Bevy | 5,000 | 60+ | ✓ |
| GPU physics | 10,000+ | 140 | ✓ |
| Full behaviors | 10,000+ | 140 | ✓ |
| Combat + projectiles | 10,000+ | 140 | ✓ |
| Full game integration | 10,000+ | 60+ | Partial |
| Future (GPU grid build) | 20,000+ | 60+ | Planned |

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
