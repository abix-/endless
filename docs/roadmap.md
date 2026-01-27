# Rust Migration Roadmap

Target: 20,000+ NPCs @ 60fps by combining Rust game logic + GPU compute + bulk rendering.

## Current State
- [x] GPU compute shader for separation forces (`shaders/separation_compute.glsl`)
- [x] 10,000 NPCs @ 140fps validated (release build, bevy_poc scene)
- [x] Spatial grid built on CPU, uploaded to GPU each frame
- [x] Godot RenderingDevice with submit/sync pipeline
- [x] Bulk `set_buffer()` MultiMesh rendering
- [x] godot-bevy 0.11 + Bevy 0.18 integration
- [x] Chunk 1: EcsNpcManager spawns entities, renders via MultiMesh
- [x] Chunk 2: CPU movement with velocity, target, arrival detection

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
| Colors | GPU | Write-once | Set at spawn based on Job |
| Speeds | GPU | Write-once | Movement speed per NPC |
| **Bevy-Owned (Logical State)** ||||
| NpcIndex | Bevy | Internal | Links Bevy entity to GPU slot index |
| Job | Bevy | Internal | Guard, Farmer, Raider - determines behavior |
| Energy | Bevy | Internal | Drives tired/rest decisions (drain/recover rates) |
| Health | **Both** | Bevy → GPU | Bevy authoritative, synced to GPU for targeting |
| State markers | Bevy | Internal | Dead, InCombat, Patrolling, OnDuty, Resting, etc. |
| AttackTimer | Bevy | Internal | Cooldown between attacks |
| AttackStats | Bevy | Internal | Damage, range, cooldown per NPC |
| PatrolRoute | Bevy | Internal | Guard post sequence for patrols |
| Home | Bevy | Internal | Rest location (bed or camp) |
| WorkPosition | Bevy | Internal | Farm location for farmers |

### Key Optimizations

- **O(1) entity lookup**: `NpcEntityMap` (HashMap<usize, Entity>) for instant damage routing
- **Slot reuse**: `FREE_SLOTS` pool recycles dead NPC indices (infinite churn, no 10K cap)
- **Grid sizing**: 100px cells ensure 3×3 neighborhood covers 300px detection range
- **Single locks**: One Mutex per direction instead of 10+ scattered queues

## Migration Chunks

Each chunk is a working game state. Old GDScript code kept as reference, hard cutover per chunk.

**Chunk 1: Bevy Renders Static NPCs** ✓
- [x] GDScript calls `spawn_npc(pos, job)` → Bevy creates entity with Position + Job
- [x] Bevy system builds MultiMesh buffer from Position/Job components
- [x] Bevy calls `RenderingServer.multimesh_set_buffer()` with full buffer
- [x] Result: Colored NPCs render (green=Farmer, blue=Guard, red=Raider)

**Chunk 2: CPU Movement** ✓
- [x] Add Velocity, Target, Speed, NpcIndex components
- [x] Movement system: `position += velocity * delta`
- [x] Velocity system: calculate direction toward target
- [x] Arrival detection: stop and remove Target when close
- [x] GDScript API: `set_target(npc_index, x, y)`
- [x] Result: NPCs walk to targets and stop on arrival (proof of concept)

**Chunk 3: GPU Physics** ✓
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

**Chunk 4: World Data** ✓
- [x] Towns, patrol posts, beds, farms as Bevy Resources
- [x] GDScript API: init_world, add_town/farm/bed/guard_post
- [x] Query API: get_town_center, get_camp_position, get_patrol_post
- [x] Query API: get_nearest_free_bed/farm
- [x] Occupancy API: reserve/release bed/farm
- [x] Test 6 with visual markers (town, camp, farms, beds, posts)
- [ ] Wire up main.gd to sync world data on game start
- [x] Result: Bevy + GPU know the world layout

**Chunk 5: Guard Logic** ✓
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

**Chunk 6: Behavior-Based Architecture** ✓
- [x] Refactor to behavior-based systems (systems ARE behaviors)
- [x] Generic components: Home, PatrolRoute, WorkPosition
- [x] Generic systems: tired_system, resume_patrol_system, resume_work_system, patrol_system
- [x] Farmer component with spawn_farmer() API
- [x] Test 8: Farmer Work Cycle
- [x] Result: NPCs defined by component bundles, behaviors are reusable

**Chunk 7: Combat** ✓
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
- [x] 7c: TDD test harness (Test 11) covering all projectile behaviors
- [x] Result: Combat working with GPU-accelerated targeting and projectiles

**Chunk 8: Raider Logic** (in progress)
- [x] Generic components: Stealer, CarryingFood, Raiding, Returning, Recovering
- [x] Config components: FleeThreshold, LeashRange, WoundedThreshold
- [x] steal_decision_system (priority: wounded → carrying → tired → raid nearest farm)
- [x] steal_arrival_system (farm pickup → camp delivery with food storage)
- [x] flee_system (exit combat below HP threshold)
- [x] leash_system (disengage if too far from home)
- [x] wounded_rest_system + recovery_system (rest until healed)
- [x] FoodStorage resource with GDScript API (init, add, get, events)
- [x] Raider spawn bundle includes Energy, Stealer, flee/leash/wounded config
- [ ] Wire up main.gd to sync world data and food on game start
- [ ] Multi-camp food delivery (currently hardcoded camp_food[0])
- [ ] HP regen system in Bevy (recovery_system checks threshold but no regen)
- [ ] Result: Full game loop

**Chunk 8.5: Eliminate Direct GPU Writes**
- [ ] Add slot index to all spawn messages (SpawnGuardMsg, SpawnFarmerMsg, SpawnRaiderMsg, SpawnNpcMsg)
- [ ] Spawn systems push GPU_UPDATE_QUEUE (SetPosition, SetTarget, SetColor, SetSpeed, SetFaction, SetHealth) instead of writing GpuData
- [ ] Remove direct buffer_update() calls from lib.rs spawn methods (spawn_npc, spawn_guard, spawn_guard_at_post, spawn_farmer, spawn_raider)
- [ ] Remove GpuData Bevy Resource (dead-end intermediary)
- [ ] Fix slot index mismatch bug (Bevy entity gets correct NpcIndex from message)
- [ ] Verify BevyApp.process() runs before EcsNpcManager.process() in scene tree order
- [ ] Result: Single write path (GPU_UPDATE_QUEUE), spawn matches all other systems

**Chunk 9: UI Integration**
- [ ] Signals to GDScript (death, level up, food)
- [ ] Selection queries
- [ ] Result: UI works again

## Performance Targets

| Phase | NPCs | FPS | Status |
|-------|------|-----|--------|
| GDScript baseline | 3,000 | 60 | Reference |
| Chunk 1-2 (CPU Bevy) | 5,000 | 60+ | ✅ Done |
| Chunk 3 (GPU physics) | 10,000+ | 140 | ✅ Done |
| Chunk 4 (world data) | 10,000+ | 140 | ✅ Done |
| Chunk 5 (guard logic) | 10,000+ | 140 | ✅ Done |
| Chunk 6 (behaviors) | 10,000+ | 140 | ✅ Done |
| Chunk 7a (health/death) | 10,000+ | 140 | ✅ Done |
| Chunk 7b (GPU targeting) | 10,000+ | 140 | ✅ Done |
| Chunk 7c (GPU projectiles) | 10,000+ | 140 | ✅ Done |
| Chunk 8-9 (full game) | 10,000+ | 60+ | Planned |
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

**What worked:**
- Build multimesh from cached positions on CPU (eliminates 480KB GPU readback)
- Throttle expensive operations to once per second
- Advance test_phase immediately to prevent repeated assertion runs

## References

- [Simon Green's CUDA Particles](https://developer.download.nvidia.com/assets/cuda/files/particles.pdf) — GPU spatial grid approach
- [godot-bevy Book](https://bytemeadow.github.io/godot-bevy/getting-started/basic-concepts.html)
- [FSM in ECS](https://www.richardlord.net/blog/ecs/finite-state-machines-with-ash)
