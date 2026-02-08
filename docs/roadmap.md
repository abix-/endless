# Roadmap

Target: 20,000+ NPCs @ 60fps with pure Bevy ECS + wgpu compute + bevy_egui.

## Pure Bevy Migration (In Progress)

Migrating from Godot+Bevy hybrid to pure Bevy. Removes gdext/godot-bevy complexity.

### What We Keep (~2,500 lines pure Bevy)
- [x] components.rs, resources.rs, systems/*.rs
- [x] constants.rs, messages.rs, world.rs
- [x] build_app() system registration

### What We Delete (~2,000 lines Godot bridge)
- [x] gpu.rs (Godot RenderingDevice)
- [x] rendering.rs (Godot MultiMesh)
- [x] api.rs (GDScript FFI)
- [x] channels.rs (Godot-Bevy messaging)
- [x] EcsNpcManager struct

### Migration Phases

**Phase 1: Standalone Bevy App ✓**
- [x] Update Cargo.toml (bevy 0.18, bevy_egui 0.39, bytemuck)
- [x] Create main.rs with App runner
- [x] Update imports (godot_bevy → bevy)
- [x] Test: `cargo run` opens window

**Phase 2: GPU Compute via wgpu ✓**
- [x] Port npc_compute.glsl → npc_compute.wgsl
- [x] Create wgpu compute pipeline (Bevy render graph)
- [x] Pipeline compiles and dispatches
- [x] ECS→GPU buffer writes (NpcBufferWrites + write_npc_buffers)
- [ ] Port projectile_compute.glsl → projectile_compute.wgsl
- [ ] GPU→ECS readback (positions for sprite sync)
- [ ] Test: NPCs move (log positions)

**Phase 2.5: GPU Instanced Rendering (In Progress)**

*Note: Original render graph Node approach failed - Nodes are for compute/post-processing, not geometry. Switching to RenderCommand pattern.*

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
- [ ] Port main.gd world gen to Rust
- [ ] Town/farm/bed/guard_post placement
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
- [ ] Economy: multi-camp food delivery, HP regen
- [ ] Building system: runtime add/remove buildings
- [ ] Config & upgrades: config-driven stats, upgrade API
- [ ] Events: death/food events to GDScript
- [ ] Architecture: remove 9 remaining statics, old drain functions
- [ ] GDScript cleanup: delete old npc_manager files
- [ ] Zero-copy rendering (blocked by Godot bug #105100)

## Architecture

See [gpu-compute.md](gpu-compute.md) for GPU buffers, optimizations, and performance lessons. See [messages.md](messages.md) for data ownership and channel architecture. See [frame-loop.md](frame-loop.md) for execution order.

## Capabilities

### Spawning & Rendering ✓
- [x] NPCs spawn with jobs (guard, farmer, raider, fighter)
- [x] GPU-accelerated MultiMesh rendering (10,000+ @ 140fps)
- [x] Sprite frames, faction colors
- [x] Unified spawn API with job-as-template pattern (phase 8.5)
- [x] spawn_guard(), spawn_guard_at_post(), spawn_farmer() convenience APIs
- [x] Slot reuse for dead NPCs (FREE_SLOTS pool)
- [ ] Zero-copy rendering (blocked by Godot bug #105100)
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
- [x] Projectile slot reuse (FREE_PROJ_SLOTS pool)
- [x] MultiMesh projectile rendering with velocity-based rotation
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
- [x] GameTime resource (Bevy-owned, no GDScript bridge)
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
- [x] NPC_META static (name, level, xp, trait, town_id, job)
- [x] NPC_STATES static (state ID per NPC)
- [x] NPC_ENERGY static (energy per NPC)
- [x] NPCS_BY_TOWN static (per-town NPC lists)
- [x] 10 query APIs: get_population_stats, get_town_population, get_npc_info, get_npcs_by_town, get/set_selected_npc, get_npc_name, get_npc_trait, set_npc_name, get_bed_stats
- [x] left_panel.gd, roster_panel.gd, upgrade_menu.gd wired to ECS APIs
- [ ] Villager role assignment UI
- [ ] Train guards from population

### Building System
- [ ] Runtime add/remove farm/bed/guard_post APIs
- [ ] Wire up _on_build_requested(), _on_destroy_requested(), _get_clicked_farm()
- [ ] Replace npc_manager array writes with EcsNpcManager API calls
- [ ] NPCs claim and use new buildings
- [ ] Guard post auto-attack (turret behavior, fires at enemies)
- [ ] Guard post upgrades (attack_enabled, range_level, damage_level)

### XP & Leveling
- [ ] Level, Xp components on NPCs
- [ ] grant_xp() API and system
- [ ] Level-up system (sqrt scaling: level 9999 = 100x stats)
- [ ] Stat scaling (damage, max_health based on level)
- [ ] npc_leveled_up event to GDScript

### Config & Upgrades
- [ ] CombatConfig Bevy resource (configurable melee/ranged stats)
- [ ] set_combat_config() API to push Config.gd values at startup
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

### Events to GDScript
- [ ] DEATH_EVENT_QUEUE (npc_idx, job, faction, town_idx)
- [ ] poll_events() API returning { deaths: [...], food_delivered: [...] }
- [ ] main.gd _process() polls events, feeds combat_log

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

### Architecture Cleanup

**Channel Migration (5 of 14 statics removed):**
- [x] SPAWN_QUEUE → GodotToBevyMsg::SpawnNpc
- [x] TARGET_QUEUE → GodotToBevyMsg::SetTarget
- [x] DAMAGE_QUEUE → GodotToBevyMsg::ApplyDamage
- [x] PROJECTILE_FIRE_QUEUE → BevyToGodotMsg::FireProjectile
- [x] RESET_BEVY → GodotToBevyMsg::Reset
- [ ] ARRIVAL_QUEUE (lib.rs still uses for GPU→Bevy arrivals)
- [ ] GPU_UPDATE_QUEUE, GPU_READ_STATE, GPU_DISPATCH_COUNT (lib.rs process())
- [ ] NPC_SLOT_COUNTER, FREE_SLOTS, FREE_PROJ_SLOTS (lib.rs slot allocation)
- [ ] FOOD_STORAGE, GAME_CONFIG_STAGING (lib.rs APIs)

**Bevy Resources Migrated (phase 10.3):**
- [x] WORLD_DATA, BED_OCCUPANCY, FARM_OCCUPANCY → Bevy Resources
- [x] HEALTH_DEBUG, COMBAT_DEBUG → Bevy Resources
- [x] KILL_STATS, SELECTED_NPC → Bevy Resources
- [x] NPC_META, NPC_STATES, NPC_ENERGY, NPCS_BY_TOWN, NPC_LOGS → Bevy Resources
- [x] FOOD_DELIVERED_QUEUE, FOOD_CONSUMED_QUEUE → FoodEvents Resource

**GPU Update Messages (phase 10.2):**
- [x] GpuUpdateMsg Message type (wraps GpuUpdate enum)
- [x] Systems use MessageWriter<GpuUpdateMsg> instead of direct GPU_UPDATE_QUEUE locks
- [x] collect_gpu_updates system batches all messages with single Mutex lock

**Channel Infrastructure (phase 11.1-11.2):**
- [x] crossbeam-channel for lock-free message passing
- [x] GodotToBevyMsg enum (SpawnNpc, SetTarget, ApplyDamage, SelectNpc, Reset, SetPaused, SetTimeScale)
- [x] BevyToGodotMsg enum (SpawnView, DespawnView, SyncHealth, SyncColor, SyncSprite, FireProjectile)
- [x] Bevy Position component (synced from GPU)
- [x] SpawnView/DespawnView on spawn/death

**GPU Sync (phase 11.3, partial):**
- [x] gpu_position_readback system: GPU → Bevy Position (only if changed > epsilon)
- [ ] GpuBuffers resource (CPU-side mirrors)
- [ ] upload_to_gpu_system: Query components → fill buffers → upload

**Outbox/Changed Sync (phase 11.5-11.6):**
- [x] GDScript _process() drains outbox via bevy_to_godot()
- [x] bevy_to_godot_write: Query Changed<Health> → SyncHealth
- [x] death_cleanup_system: Query With<Dead> → DespawnView

**Old Drain Functions (phase 11.4, not done):**
- [ ] Remove drain_spawn_queue, drain_target_queue, drain_damage_queue
- [ ] Single godot_to_bevy_read handles all inbox messages

**godot-bevy Integration (phase 10.1, not done):**
- [ ] Register EcsNpcManager as Bevy entity
- [ ] EcsNpcManagerMarker component for querying

### Performance Optimizations
- [ ] Entity sleeping (Factorio-style: NPCs outside camera radius sleep)
- [ ] awake/sleep_timers per NPC, ACTIVE_RADIUS check
- [ ] Combat/raiding states force awake

### Raider Coordination
- [ ] count_nearby_raiders() for group behavior
- [ ] get_raider_group_center() for coordinated movement
- [ ] find_nearest_raider() for regrouping

### GDScript Cleanup ✓
- [x] Delete npc_state.gd (state/job/trait now strings from Rust)
- [x] Delete npc_manager.gd, npc_manager.tscn, npc_navigation.gd
- [x] Delete npc_combat.gd, npc_needs.gd, npc_grid.gd, npc_renderer.gd
- [x] Delete gpu_separation.gd, separation_compute.glsl
- [x] Delete guard_post_combat.gd, projectile_manager.gd
- [x] Remove .uid files from git tracking
- [ ] Remove unused preloads from main.gd
- [ ] Delete projectile_manager.tscn

## Performance

| Milestone | NPCs | FPS | Status |
|-----------|------|-----|--------|
| GDScript baseline | 3,000 | 60 | Reference |
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
- [godot-bevy Book](https://bytemeadow.github.io/godot-bevy/getting-started/basic-concepts.html)
- [FSM in ECS](https://www.richardlord.net/blog/ecs/finite-state-machines-with-ash)
