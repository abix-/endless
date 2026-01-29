# Rust Migration Roadmap

Target: 20,000+ NPCs @ 60fps by combining Rust game logic + GPU compute + bulk rendering.

## Current State
- [x] Phases 1-8.5: Full ECS pipeline (spawn, movement, GPU physics, world data, guards, behaviors, combat, raider logic, unified spawn API)
- [x] Phase 9.1: EcsNpcManager wired into main.gd — game boots with Rust ECS, NPCs render/move/fight
- [x] Phase 9.2: Food production and respawning fully in Bevy (Clan, GameTime, PopulationStats, economy_tick_system)
- [x] Phase 9.4: UI data queries (NPC_META, NPC_STATES, NPC_ENERGY, selection, 10 query APIs, sprite rendering)
- [ ] Phase 9.3, 9.5-9.7: Events, building, upgrades, GDScript cleanup
- [ ] Phase 10: Idiomatic Bevy (static Mutex → Resources + Events)

## GPU-First Architecture

See [gpu-compute.md](gpu-compute.md) for buffer details, [messages.md](messages.md) for queue architecture, and [frame-loop.md](frame-loop.md) for execution order.

### Data Ownership

| Data | Owner | Direction | Notes |
|------|-------|-----------|-------|
| **GPU-Owned (Numeric/Physics)** ||||
| Positions | GPU | GPU → Bevy | Compute shader moves NPCs each frame |
| Targets | GPU | Bevy → GPU | Bevy decides destination, GPU interpolates movement |
| Factions | GPU | Write-once | Set at spawn (0=Villager, 1=Raider) |
| Combat targets | GPU | GPU → Bevy | GPU finds nearest enemy within 300px |
| Colors | GPU | Bevy → GPU | Set at spawn, updated by steal/flee systems |
| Speeds | GPU | Write-once | Movement speed per NPC |
| **Bevy-Owned (Logical State)** ||||
| NpcIndex | Bevy | Internal | Links Bevy entity to GPU slot index |
| Job | Bevy | Internal | Guard, Farmer, Raider, Fighter - determines behavior |
| Energy | Bevy | Internal | Drives tired/rest decisions (drain/recover rates) |
| Health | **Both** | Bevy → GPU | Bevy authoritative, synced to GPU for targeting |
| State markers | Bevy | Internal | Dead, InCombat, Patrolling, OnDuty, Resting, Raiding, Returning, Recovering, etc. |
| Config components | Bevy | Internal | FleeThreshold, LeashRange, WoundedThreshold, Stealer |
| AttackTimer | Bevy | Internal | Cooldown between attacks |
| AttackStats | Bevy | Internal | melee(range=150, speed=500) or ranged(range=300, speed=200) |
| PatrolRoute | Bevy | Internal | Guard post sequence for patrols |
| Home | Bevy | Internal | Rest location (bed or camp) |
| WorkPosition | Bevy | Internal | Farm location for farmers |

### Key Optimizations

- **O(1) entity lookup**: `NpcEntityMap` (HashMap<usize, Entity>) for instant damage routing
- **Slot reuse**: `FREE_SLOTS` pool recycles dead NPC indices (infinite churn, no 10K cap)
- **Grid sizing**: 100px cells ensure 3×3 neighborhood covers 300px detection range
- **Single locks**: One Mutex per direction instead of 10+ scattered queues

## Migration Phases

Each phase is a working game state. Old GDScript code kept as reference, hard cutover per phase.

**Phase 1: Bevy Renders Static NPCs** ✓
- [x] GDScript calls `spawn_npc(pos, job)` → Bevy creates entity with Position + Job
- [x] Bevy system builds MultiMesh buffer from Position/Job components
- [x] Bevy calls `RenderingServer.multimesh_set_buffer()` with full buffer
- [x] Result: Colored NPCs render (green=Farmer, blue=Guard, red=Raider)

**Phase 2: CPU Movement** ✓
- [x] Add Velocity, Target, Speed, NpcIndex components
- [x] Movement system: `position += velocity * delta`
- [x] Velocity system: calculate direction toward target
- [x] Arrival detection: stop and remove Target when close
- [x] GDScript API: `set_target(npc_index, x, y)`
- [x] Result: NPCs walk to targets and stop on arrival (proof of concept)

**Phase 3: GPU Physics** ✓
- [x] GPU owns positions (8-buffer architecture)
- [x] Bevy owns targets/jobs/states (logical state)
- [x] EcsNpcManager owns GpuCompute (RenderingDevice not Send-safe)
- [x] 8 GPU buffers: position, target, color, speed, grid_counts, grid_data, multimesh, arrivals
- [x] Push constants (48 bytes with alignment padding)
- [x] Spatial grid for O(1) neighbor lookup (80x80 cells, 64 NPCs/cell, 100px cells)
- [x] Colors and movement confirmed working
- [x] Separation algorithm (boids-style: accumulate proportionally, no normalization)
- [x] Persistent arrival flag (NPCs stay arrived after being pushed)
- [x] Zero-distance fallback (golden angle when NPCs overlap exactly)
- [x] reset() function for scene reload
- [x] Arrival flag initialization on spawn
- [x] Test harness with 5 scenarios (arrive, separation, both, circle, mass)
- [x] TDD assertions with automated PASS/FAIL (get_npc_position, min separation check)
- [x] Result: 500+ NPCs @ 130 FPS with separation forces (sync() is bottleneck, not GPU)
- [ ] Zero-copy rendering via `multimesh_get_buffer_rd_rid()` (blocked by Godot bug #105100)

**Phase 4: World Data** ✓
- [x] Towns, patrol posts, beds, farms as Bevy Resources
- [x] GDScript API: init_world, add_town/farm/bed/guard_post
- [x] Query API: get_town_center, get_camp_position, get_patrol_post
- [x] Query API: get_nearest_free_bed/farm
- [x] Occupancy API: reserve/release bed/farm
- [x] Test 6 with visual markers (town, camp, farms, beds, posts)
- [x] Wire up main.gd to sync world data on game start
- [x] Result: Bevy + GPU know the world layout

**Phase 5: Guard Logic** ✓
- [x] State marker components (Patrolling, OnDuty, Resting, GoingToRest)
- [x] Guard, Energy, HomePosition components
- [x] Energy system (drain while active, recover while resting)
- [x] Guard decision system (energy < 50 → go rest, energy > 80 → resume patrol)
- [x] Guard patrol system (OnDuty timer → move to next post clockwise)
- [x] Arrival detection from GPU buffer (ArrivalMsg queue)
- [x] GPU_TARGET_QUEUE for Bevy→GPU target updates
- [x] spawn_guard() and spawn_guard_at_post() GDScript API
- [x] Test 7: Guard Patrol (4 guards patrol perimeter clockwise)
- [x] Result: Guards patrol and rest autonomously

**Phase 6: Behavior-Based Architecture** ✓
- [x] Refactor to behavior-based systems (systems ARE behaviors)
- [x] Generic components: Home, PatrolRoute, WorkPosition
- [x] Generic systems: tired_system, resume_patrol_system, resume_work_system, patrol_system
- [x] Farmer component with spawn_farmer() API
- [x] Test 8: Farmer Work Cycle
- [x] Result: NPCs defined by component bundles, behaviors are reusable

**Phase 7: Combat** ✓
- [x] 7a: Health component, DamageMsg, death_system, death_cleanup_system
- [x] 7a: Test 9 Health/Death validation
- [x] 7b: GPU targeting shader (find nearest enemy within 300px, output target index)
- [x] 7b: Attack system (Bevy reads GPU targets, checks range, applies damage)
- [x] 7b: GPU-First Architecture refactor (consolidated 10+ queues → 2)
- [x] 7b: O(1) entity lookup via NpcEntityMap (replaces O(n) damage iteration)
- [x] 7b: Slot reuse for dead NPCs (FREE_SLOTS pool, infinite churn without 10K cap)
- [x] 7b: Grid cell fix (64px → 100px cells, properly covers 300px detection range)
- [x] 7c: GPU projectile system (50,000 projectiles, compute shader movement + collision)
- [x] 7c: Projectile slot reuse via FREE_PROJ_SLOTS pool
- [x] 7c: MultiMesh rendering with velocity-based rotation and faction colors
- [x] 7d: Unified attacks — melee and ranged both fire projectiles via PROJECTILE_FIRE_QUEUE
- [x] 7d: AttackStats::melee() (range=150, speed=500, lifetime=0.5s) and AttackStats::ranged() (range=300, speed=200, lifetime=3.0s)
- [x] 7d: Fighter job (job=3) — combat-only NPC for isolated testing
- [x] 7d: Remove GDScript fire_projectile() — all projectiles from Bevy attack_system
- [x] 7d: Test 10 (combat TDD, 6 phases) and Test 11 (unified attacks TDD, 7 phases)
- [x] Result: Combat working with GPU-accelerated targeting and unified projectile pipeline

**Phase 8: Raider Logic** (2 items remaining)
- [x] Generic components: Stealer, CarryingFood, Raiding, Returning, Recovering
- [x] Config components: FleeThreshold, LeashRange, WoundedThreshold
- [x] raider_idle_system (priority: wounded → carrying → tired → raid nearest farm)
- [x] raider_arrival_system (farm pickup → camp delivery with food storage)
- [x] flee_system (exit combat below HP threshold)
- [x] leash_system (disengage if too far from home)
- [x] wounded_rest_system + recovery_system (rest until healed)
- [x] FoodStorage resource with GDScript API (init, add, get, events)
- [x] Raider spawn bundle includes Energy, Stealer, flee/leash/wounded config
- [x] Wire up main.gd to sync world data and food on game start
- [ ] Multi-camp food delivery (currently hardcoded camp_food[0])
- [ ] HP regen system in Bevy (recovery_system checks threshold but no regen)
- [ ] Result: Full game loop

**Phase 8.5: Generic Spawn + Eliminate Direct GPU Writes** ✓
- [x] Single SpawnNpcMsg with job-as-template pattern (slot_idx, job, faction, home, work, town_idx, starting_post, attack_type)
- [x] Single spawn_npc(x, y, job, faction, opts: Dictionary) API — 4 required + Dictionary for optional params
- [x] Single SPAWN_QUEUE, single drain function, single spawn_npc_system
- [x] spawn_npc_system attaches components via `match job` template
- [x] spawn_npc_system pushes GPU_UPDATE_QUEUE (SetPosition, SetTarget, SetColor, SetSpeed, SetFaction, SetHealth) — no direct buffer_update()
- [x] Remove 4 job-specific spawn messages, queues, drain functions, spawn systems
- [x] Remove GpuData Bevy Resource (dead-end intermediary)
- [x] Remove direct buffer_update() calls from lib.rs spawn methods
- [x] Slot index carried in message — fixes slot mismatch bug (spawn.md 6→8/10)
- [x] Update GDScript callers (ecs_test.gd) to use new unified API
- [x] Optional params via Dictionary: home_x/y, work_x/y, town_idx, starting_post, attack_type (defaults to -1 or 0)
- [x] Result: Single spawn path, single write path, components define behavior, extensible without breaking callers

**Phase 9: Wire ECS into Real Game**

Each step is a working game state. Old GDScript npc_manager kept as reference until Phase 9.6.

*9.1: Boot ECS in main.gd* ✓
- [x] Replace npc_manager instantiation with EcsNpcManager (ClassDB.instantiate)
- [x] Wire init_world, add_town/farm/bed/guard_post from Location nodes
- [x] Replace spawn calls with spawn_npc(x, y, job, faction, opts)
- [x] Comment out broken signal connections, food, building, upgrades, selection
- [x] Fix multimesh culling (canvas_item_set_custom_rect for world-spanning MultiMesh)
- [x] Result: Game boots, NPCs render, move, patrol, fight

*9.2: Food production and respawning* ✓
- [x] Add Clan(i32) component — attached to every NPC at spawn
- [x] GameTime resource — Bevy-owned game time (no GDScript bridge needed)
- [x] GameConfig resource — farmers/guards per town, spawn interval, food per hour
- [x] PopulationStats resource — tracks alive/working counts per (job, clan)
- [x] RespawnTimers resource — per-clan respawn cooldowns
- [x] economy_tick_system — unified hourly economy (food production + respawning) using PhysicsDelta
- [x] Result: Food counter increases, dead NPCs respawn after timer (all in Bevy)

*9.3: Events from ECS to GDScript*
- [ ] DEATH_EVENT_QUEUE in messages.rs — pushed by death_cleanup_system (npc_idx, job, faction, town_idx)
- [ ] poll_events() API — drains all queues, returns { deaths: [...], food_delivered: [...] }
- [ ] main.gd _process() polls events, feeds combat_log and UI
- [ ] Result: Combat log shows deaths, food stolen events appear

*9.4: UI data queries* ✓
- [x] NPC_META static — per-NPC name/level/xp/trait cached for UI queries
- [x] NPC_STATES static — per-NPC state ID updated by behavior systems
- [x] NPC_ENERGY static — per-NPC energy synced from Bevy
- [x] KILL_STATS static — tracks guard/villager kills for UI display
- [x] SELECTED_NPC static — currently selected NPC index for inspector
- [x] NPCS_BY_TOWN static — per-town NPC lists for O(1) roster queries
- [x] Name generation — "Adjective Noun" names based on job (Swift Tiller, Brave Shield, etc.)
- [x] 10 query APIs: get_population_stats, get_town_population, get_npc_info, get_npcs_by_town, get/set_selected_npc, get_npc_name, get_npc_trait, set_npc_name, get_bed_stats
- [x] get_npc_at_position(x, y, radius) API for click selection
- [x] NPC click selection in main.gd — left-click selects nearest NPC within 20px
- [x] Sprite rendering — ShaderMaterial kept alive, custom_data in MultiMesh buffer for sprite frames
- [x] left_panel.gd, roster_panel.gd, upgrade_menu.gd use ECS query APIs
- [x] Result: Click NPC → inspector shows name/HP/job/energy. Roster panel lists NPCs with sorting/filtering.

*9.5: Building system*
- [ ] Runtime add/remove farm/bed/guard_post APIs that update Bevy world data resources
- [ ] Uncomment _on_build_requested(), _on_destroy_requested(), _get_clicked_farm()
- [ ] Replace npc_manager array writes with EcsNpcManager API calls
- [ ] Result: Build menu works, new farms/beds/posts appear, NPCs use them

*9.6: Config-driven stats and upgrades*
- [ ] CombatConfig Bevy resource with configurable melee/ranged stats
- [ ] set_combat_config() API — push Config.gd values to ECS at startup
- [ ] spawn_npc_system reads config instead of hardcoded AttackStats
- [ ] apply_upgrade(town_idx, upgrade_type, level) API for stat multipliers
- [ ] Result: NPC stats match Config.gd, upgrades affect combat

*9.7: Cleanup*
- [ ] Delete npc_manager.gd, npc_state.gd, npc_navigation.gd, npc_combat.gd, npc_needs.gd, npc_grid.gd, npc_renderer.gd
- [ ] Delete projectile_manager.gd, npc_manager.tscn, projectile_manager.tscn
- [ ] Remove preloads from main.gd
- [ ] Update README
- [ ] Result: No GDScript NPC code remains

**Phase 10: Proper godot-bevy Architecture (Static Mutex → Events + GodotAccess)**

Currently ~20 static Mutex variables handle all communication. Every behavior system directly locks `GPU_UPDATE_QUEUE` — this serializes all systems and hides data flow from Bevy's scheduler. The godot-bevy recommended architecture is event-driven:

```
Multi-threaded systems (pure logic) → emit Events → main thread system (GodotAccess) → Godot/GPU APIs
```

In godot-bevy, systems with `GodotAccess` are forced to main thread. This is the collector pattern - parallel systems do logic, one main-thread system syncs to Godot.

*10.1: Register EcsNpcManager as Bevy Entity*
- [ ] EcsNpcManager auto-registered as entity when added to scene tree (godot-bevy does this)
- [ ] Add `EcsNpcManagerMarker` component for querying
- [ ] Result: Can query EcsNpcManager from Bevy systems via `Query<&GodotNodeHandle, With<EcsNpcManagerMarker>>`

*10.2: GPU Update Events*
- [ ] Add `GpuUpdateEvent` Bevy Event (wraps current GpuUpdate enum)
- [ ] Replace `GPU_UPDATE_QUEUE.lock().push()` in all systems with `EventWriter<GpuUpdateEvent>`
- [ ] Add `render_sync_system` with `GodotAccess` (end of Step::Behavior) — reads events, calls EcsNpcManager GPU methods
- [ ] Result: Systems parallelizable, single main-thread sync point, Bevy sees data flow

*10.3: World Data + Debug Resources*
- [ ] Migrate WORLD_DATA, BED_OCCUPANCY, FARM_OCCUPANCY → Bevy Resources
- [ ] Migrate HEALTH_DEBUG, COMBAT_DEBUG, FOOD_STORAGE → Bevy Resources
- [ ] GDScript APIs use `get_bevy_app()` pattern (like Time API) instead of statics
- [ ] Result: All Bevy-internal state uses idiomatic access

*10.4: GPU Read State Resource*
- [ ] process() writes to staging static, Step::Drain copies to `Res<GpuReadState>`
- [ ] Update attack_system, raider systems → `Res<GpuReadState>`
- [ ] Result: No Bevy system locks GPU_READ_STATE directly

*10.5: Simplify process()*
- [ ] process() only does: dispatch GPU compute, read results, update MultiMesh
- [ ] All game logic in Bevy systems, all GPU writes via render_sync_system
- [ ] Result: Clean separation — Godot renders, Bevy thinks

*Statics that remain:*

| Category | Statics | Why |
|----------|---------|-----|
| Slot management | NPC_SLOT_COUNTER, FREE_SLOTS, FREE_PROJ_SLOTS | Allocation shared between GDScript spawn API and Bevy death cleanup |
| GPU dispatch | GPU_DISPATCH_COUNT | process() needs count before Bevy runs |
| UI query cache | NPC_META, NPC_STATES, NPC_ENERGY | Fast UI reads (could become components later) |

## Performance Targets

| Phase | NPCs | FPS | Status |
|-------|------|-----|--------|
| GDScript baseline | 3,000 | 60 | Reference |
| Phase 1-2 (CPU Bevy) | 5,000 | 60+ | ✅ Done |
| Phase 3 (GPU physics) | 10,000+ | 140 | ✅ Done |
| Phase 4 (world data) | 10,000+ | 140 | ✅ Done |
| Phase 5 (guard logic) | 10,000+ | 140 | ✅ Done |
| Phase 6 (behaviors) | 10,000+ | 140 | ✅ Done |
| Phase 7a (health/death) | 10,000+ | 140 | ✅ Done |
| Phase 7b (GPU targeting) | 10,000+ | 140 | ✅ Done |
| Phase 7c (GPU projectiles) | 10,000+ | 140 | ✅ Done |
| Phase 8-9 (full game) | 10,000+ | 60+ | 9.1-9.2, 9.4 done |
| GPU grid + targeting | 20,000+ | 60+ | Future |

## Performance Lessons Learned

**GPU sync() is the bottleneck, not compute:**
- `RenderingDevice.sync()` blocks CPU waiting for GPU (~2.5ms per frame)
- `buffer_get_data()` also stalls pipeline for GPU→CPU transfer
- Godot's local RenderingDevice requires sync() between submits (can't pipeline)
- `buffer_get_data_async()` doesn't work with local RD (Godot issue #105256)

**GDScript O(n²) traps:**
- Calling `get_npc_position()` in nested loops crosses GDScript→Rust boundary 124,750 times for 500 NPCs
- Test assertions must run ONCE when triggered, not every frame after timer passes
- Debug metrics (min separation) must be throttled to 1/sec, not every frame
- `get_debug_stats()` does GPU reads - don't call every frame

**MultiMesh culling:**
- Godot auto-calculates AABB for canvas items — wrong for world-spanning MultiMesh
- NPCs disappear at close zoom without `canvas_item_set_custom_rect` on the canvas item
- Fix: set large custom rect (-100K to +100K) to disable culling

**What worked:**
- Build multimesh from cached positions on CPU (eliminates 480KB GPU readback)
- Throttle expensive operations to once per second
- Advance test_phase immediately to prevent repeated assertion runs

## References

- [Simon Green's CUDA Particles](https://developer.download.nvidia.com/assets/cuda/files/particles.pdf) — GPU spatial grid approach
- [godot-bevy Book](https://bytemeadow.github.io/godot-bevy/getting-started/basic-concepts.html)
- [FSM in ECS](https://www.richardlord.net/blog/ecs/finite-state-machines-with-ash)
