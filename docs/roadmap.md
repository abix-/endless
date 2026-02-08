# Roadmap

Target: 20,000+ NPCs @ 60fps with pure Bevy ECS + WGSL compute + GPU instanced rendering.

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

**Phase 4: World Generation**
- [ ] Procedural town/farm/bed/guard_post placement
- [ ] Test: World generates correctly

**Phase 5: Start Menu + Config**
- [ ] Port config.gd → Bevy Resource
- [ ] Port user_settings.gd → serde JSON
- [ ] bevy_egui start menu
- [ ] Test: Menu → game start

**Phase 6: Core UI Panels**
- [ ] left_panel.rs (stats, perf, inspector)
- [ ] upgrade_menu.rs
- [ ] policies_panel.rs
- [ ] Test: UI shows live ECS data

**Phase 7: Remaining UI + Polish**
- [ ] roster_panel.rs, build_menu.rs, combat_log.rs
- [ ] Camera controls, click selection
- [ ] Audio (bevy_audio)
- [ ] Test: Full game playable

## Current State

**Done:**
- [x] Spawning, rendering, movement, physics (10,000+ NPCs @ 140fps)
- [x] Combat (GPU targeting, projectiles, melee/ranged)
- [x] NPC behaviors (guards, farmers, raiders with energy/flee/leash)
- [x] Economy (food production, theft, respawning)
- [x] World data (towns, farms, beds, guard posts)
- [x] UI integration (selection, inspector, roster panels)
- [x] Testing harness (11 test scenarios)
- [x] Architecture cleanup (channels, Bevy resources, GPU messages)

**Remaining:**
- [ ] GPU→CPU readback (positions, combat targets)
- [ ] Combat end-to-end (targeting → damage → death)
- [ ] Spatial grid build on GPU
- [ ] Building system: runtime add/remove buildings
- [ ] Config & upgrades: config-driven stats, upgrade API
- [ ] Camera controls, input handling
- [ ] UI panels (bevy_egui)

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
- [ ] Loot icon overlay (raider carrying food indicator)
- [ ] Halo icon overlay (healing zone indicator)
- [ ] Sleep icon overlay (resting indicator)

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
