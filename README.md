# Endless

A game about fighting entropy. Raiders steal your food. Guards die in combat. Farms lie fallow. Everything tends toward chaos and collapse. Can you build something that lasts?

Built in Godot 4.5 using Data-Oriented Design (DOD) with Factorio-style optimizations for high-performance NPC management.

**Inspirations:**
- **Asimov's "The Last Question"** - entropy as the ultimate antagonist
- **Lords of the Realm 2** - assign villagers to roles, manage production, raise armies, conquer rival towns
  - Farming: villagers work fields → grain → rations, weather affects yield, starvation causes unrest
  - Balance farming vs other jobs (woodcutting, mining, blacksmithing, army)
- **RimWorld** - colonist needs, AI storytelling, emergent chaos
- **Factorio** - scale to thousands of entities, predicted rendering, dormant states, spatial partitioning

## The Struggle

1. **Produce** - Farmers generate food. Without food, nothing else matters.
2. **Defend** - Guards protect what you've built. Raiders want it.
3. **Upgrade** - Invest food to make guards stronger. Trade present resources for future survival.
4. **Expand** - Claim neutral towns. More territory, more production, more to defend.
5. **Endure** - Entropy never stops. Neither can you.

## Features

### World
- [x] Procedural town generation (1-7 towns configurable, 1200px minimum spacing)
- [x] Named towns from pool of 15 Florida cities (Miami, Orlando, Tampa, etc.)
- [x] Farms (2 per town, 200-300px from center)
- [x] Homes for farmers (ring 350-450px from center)
- [x] Guard posts (4 per town at corners, clockwise perimeter patrol, individually upgradeable)
- [x] Raider camps (positioned away from all towns)
- [x] Visible world border with corner markers
- [x] Destructible buildings (right-click slot → Destroy)
- [x] Build new structures (right-click empty slots - farms, beds, guard posts)
- [x] Expandable building grid (6x6 start, unlock adjacent slots up to 100x100)
- [x] Double-click locked slots to unlock them
- [x] Town circle indicator expands with building range
- [x] 16 starting beds (4 beds in each of 4 corner slots)
- [ ] Structure upgrades (increase output, capacity, defense)

### Economy
- [x] Food production (farmers generate 1 food/hour when working)
- [x] Food theft (radius derived from sprite size)
- [x] Food delivery (raiders return loot to camp)
- [x] Loot icon above raiders carrying food
- [x] Per-town and per-camp food tracking in HUD
- [x] Food consumption (NPCs eat only when energy < 10, rest otherwise)
- [x] Population caps per town (upgradeable)
- [ ] Starvation effects (HP drain, desertion)
- [ ] Multiple resources (wood, iron, gold)
- [ ] Production buildings (lumber mill, mine, blacksmith)

### Combat
- [x] Faction-based auto-targeting (villagers vs raiders)
- [x] Ranged projectile combat (guards and raiders have equal stats: 120 HP, 15 dmg, 150 range)
- [x] Melee combat (farmers)
- [x] Level/XP system (sqrt scaling, level 9999 = 100x stats, size unchanged)
- [x] Damage flash effect
- [x] Leash system (farmers/raiders return home if combat drags 400px+)
- [x] Guards have no leash - fight anywhere
- [x] Alert nearby allies when combat starts
- [x] Target switching (stop chasing fleeing enemies if closer threat exists)
- [x] 500 projectile pool with faction coloring
- [ ] Player combat abilities
- [ ] Army units (peasant levy, archers, knights)

### NPC Identity
- [x] Named NPCs (55 first × 100 last = 5,500 unique combinations)
- [x] Rename NPCs via inspector (✎ button)
- [x] Personality traits (40% chance, 9 types)
- [x] Trait effects:
  - **Brave**: Never flees
  - **Coward**: Flees at +20% higher HP
  - **Efficient**: +25% farm yield, 25% faster attacks
  - **Hardy**: +25% max HP
  - **Lazy**: -20% farm yield, 20% slower attacks
  - **Strong**: +25% damage
  - **Swift**: +25% move speed
  - **Sharpshot**: +25% attack range
  - **Berserker**: +50% damage below 50% HP
- [ ] Trait combinations

### AI Behaviors
- [x] Farmers: day/night work schedule, always flee to town center
- [x] Guards: patrol 4 corner posts clockwise (perimeter), work 24/7, rest when energy low, flee below 33% HP
- [x] Raiders: priority system (wounded → exhausted → deliver loot → steal), flee to camp below 50% HP
- [x] Energy system (sleep +12/hr, rest +5/hr, activity -6/hr)
- [x] HP regen (2/hr awake, 6/hr sleeping, 10x at fountain/camp with upgrade scaling)
- [x] Recovery system (fleeing NPCs heal until 75% before resuming)
- [x] Bed tracking (NPCs reserve closest free bed, release when leaving)
- [x] Farm tracking (1 farmer per farm, nearest free farm, return if pushed)
- [x] 15-minute decision cycles (event-driven override on state changes)
- [x] Building arrival based on sprite size (not pixel coordinates)
- [x] Permadeath (dead NPCs free slots for new spawns)
- [x] Collision avoidance for all NPCs (stationary guards get pushed too)
- [x] Drift detection (working NPCs pushed off position walk back automatically)
- [ ] AI lords that expand and compete

### NPC States
Activity-specific states (no translation layer):

| State | Jobs | Description |
|-------|------|-------------|
| IDLE | All | Between decisions |
| RESTING | All | At home/camp, recovering |
| OFF_DUTY | All | At home/camp, awake |
| FIGHTING | Guard, Raider | In combat |
| FLEEING | All | Running from combat |
| WALKING | Farmer, Guard | Moving (to farm/home) |
| FARMING | Farmer | Working at farm |
| ON_DUTY | Guard | Stationed at post |
| PATROLLING | Guard | Moving between posts |
| RAIDING | Raider | Going to/at farm |
| RETURNING | Raider | Heading back to camp |
| WANDERING | Farmer, Guard | Off-duty wandering around town |

### Player Controls
- [x] WASD camera movement (configurable speed 100-2000)
- [x] Mouse wheel zoom (0.1x - 4.0x, centers on cursor)
- [x] Click to select and inspect NPCs (inspector with follow/copy)
- [x] Time controls (+/- speed, SPACE pause)
- [x] Settings menu (ESC) with HP bar modes, scroll speed, log filters
- [x] First town is player-controlled (click fountain for upgrades)
- [x] Guard upgrades: health, attack, range, size, speed (10 levels each)
- [x] Guard post upgrades: enable attack, range, damage (9999 levels each, click post to upgrade)
- [x] Economy upgrades: farm yield, farmer HP, population caps
- [x] Utility upgrades: healing rate, food efficiency
- [x] Town management panel with population stats and spawn timer
- [x] Faction policies panel (P) with tooltips
- [x] Policy controls: eat food, flee thresholds, recovery HP, off-duty behavior
- [x] Off-duty options: go to bed, stay at fountain, wander town
- [x] Town management buttons in Stats panel (Upgrades, Roster, Policies)
- [x] Resizable combat log at bottom of screen
- [x] Configurable start menu (world size, towns, farmers/guards/raiders up to 500 each)
- [ ] Villager role assignment UI
- [x] Build structures via grid slots (farms 50, beds 10, guard posts 25 food)
- [x] Unlock adjacent building slots (1 food each)
- [ ] Train guards from population
- [ ] Equipment crafting
- [ ] Army recruitment and movement
- [ ] Attack and capture enemy towns

### Victory Condition
There is no victory. Only the endless struggle against entropy.

### Performance (supports 3000+ NPCs at 60 FPS)

**Factorio-inspired optimizations:**
- [x] Predicted movement rendering (logic every 2-30 frames, render interpolates)
- [x] Dormant states (stationary NPCs skip navigation entirely)
- [x] Spatial threat registration (skip enemy scans when no threats in cell)
- [x] Event-driven wake-ups (state changes force immediate logic update)
- [x] LOD-based intervals (combat 2f, moving 5f, idle 30f, distance multiplied)

**Core architecture:**
- [x] Data-Oriented Design with PackedArrays
- [x] MultiMesh rendering (single draw call per layer)
- [x] Spatial grid for O(1) neighbor queries (64x64 cells)
- [x] Camera culling (only render visible NPCs)
- [x] Staggered scanning (1/8 NPCs per frame for combat)
- [x] Independent separation stagger (1/4 NPCs per frame for collision)
- [x] TCP-like collision avoidance (head-on, crossing, overtaking)
- [x] Co-movement separation reduction (groups move without oscillation)
- [x] Velocity damping for smooth collision avoidance
- [x] Parallel processing with thread-safe state transitions (pending arrivals)
- [x] GPU compute shader for separation forces
- [x] Rust/Bevy ECS POC (10,000 NPCs @ 140fps release build)
- [ ] Rust/Bevy full integration (see Rust Migration Roadmap below)
- [x] Combat log batching

**Visual effects:**
- [x] Golden halo shader for fountain/camp healing
- [x] Sleep "z" indicator for resting NPCs
- [x] Loot icon for raiders carrying food

---

## Architecture

```
main.gd                 # World generation, food tracking, game setup
autoloads/
  config.gd             # All tunable constants
  world_clock.gd        # Day/night cycle, time signals
  user_settings.gd      # Persistent user preferences
systems/
  npc_manager.gd        # Core orchestration, 30+ parallel data arrays
  npc_state.gd          # Activity-specific states, validation per job
  npc_navigation.gd     # Predicted movement, LOD intervals, separation
  npc_combat.gd         # Threat detection, targeting, damage, leashing
  npc_needs.gd          # Energy, schedules, decision trees
  npc_grid.gd           # Spatial partitioning (64x64 cells)
  npc_renderer.gd       # MultiMesh rendering, culling, indicators
  projectile_manager.gd # Projectile pooling, collision
  gpu_separation.gd     # Compute shader separation forces
entities/
  player.gd             # Camera controls
world/
  location.gd           # Sprite definitions, interaction radii
  terrain_renderer.gd   # Terrain tile rendering with sprite tiling
ui/
  start_menu.gd         # Start menu (world size, towns, populations)
  left_panel.gd         # Stats, performance, NPC inspector (collapsible)
  combat_log.gd         # Resizable event log at bottom
  settings_menu.gd      # Options menu with log filters
  upgrade_menu.gd       # Town management, upgrades, population caps
  build_menu.gd         # Grid slot building (farms, beds)
  policies_panel.gd     # Faction policies (flee thresholds, off-duty behavior)
  roster_panel.gd       # NPC roster with sorting and filtering
  farm_menu.gd          # Farm info popup (click farm to see occupant)
rust/
  Cargo.toml            # Bevy 0.18 + godot-bevy 0.11 dependencies
  src/lib.rs            # EcsNpcManager: spawn, movement, rendering
shaders/
  separation_compute.glsl  # GPU spatial hash + separation forces
  npc_compute.glsl         # All-in-one: movement + separation + render
scenes/
  ecs_test.tscn         # 6 behavior tests with visual markers and PASS/FAIL
  bevy_poc.tscn         # Original POC (5000 NPCs @ 140fps)
scripts/
  ecs_test.gd           # 5 test scenarios with configurable NPC count
```

## Controls

| Key | Action |
|-----|--------|
| WASD / Arrows | Move camera |
| Mouse Wheel | Zoom (centers on cursor) |
| Left Click (NPC) | Select and inspect |
| Left Click (Farm) | Show farm occupancy |
| Right Click (slot) | Build menu / unlock slot |
| + / = | Speed up time (2x) |
| - | Slow down time (0.5x) |
| SPACE | Pause/unpause |
| R | Roster panel (view all guards/farmers) |
| P | Policies panel (faction settings) |
| ESC | Settings menu |

## Configuration

Key values in `autoloads/config.gd`:

| Setting | Value | Notes |
|---------|-------|-------|
| FARMERS_PER_TOWN | 5 | Starting farmers (configurable via start menu) |
| GUARDS_PER_TOWN | 20 | Starting guards (configurable via start menu) |
| MAX_FARMERS_PER_TOWN | 5 | Population cap (upgradeable +2/level) |
| MAX_GUARDS_PER_TOWN | 20 | Population cap (upgradeable +10/level) |
| RAIDERS_PER_CAMP | 25 | Enemy forces (configurable via start menu) |
| GUARD_POSTS_PER_TOWN | 4 | Patrol points (clockwise corners) |
| WORLD_SIZE | 8000x8000 | Play area |
| MAX_NPC_COUNT | 3000 | Engine limit |
| ENERGY_STARVING | 10 | Eat food threshold |
| ENERGY_HUNGRY | 50 | Go home threshold |

## Rust Migration Roadmap

Target: 20,000+ NPCs @ 60fps by combining Rust game logic + GPU compute + bulk rendering.

### Current State
- [x] GPU compute shader for separation forces (`shaders/separation_compute.glsl`)
- [x] 10,000 NPCs @ 140fps validated (release build, bevy_poc scene)
- [x] Spatial grid built on CPU, uploaded to GPU each frame
- [x] Godot RenderingDevice with submit/sync pipeline
- [x] Bulk `set_buffer()` MultiMesh rendering
- [x] godot-bevy 0.11 + Bevy 0.18 integration
- [x] Chunk 1: EcsNpcManager spawns entities, renders via MultiMesh
- [x] Chunk 2: CPU movement with velocity, target, arrival detection

### Architecture

**Bevy ECS owns logical state. GPU owns physics. GDScript is UI only.**

```
┌─────────────────────────────────────────────────────────────────┐
│                        BEVY ECS                                 │
│  Owns: Target, Job, State, NpcIndex, Health, Energy            │
│  (Logical state - what NPCs WANT to do)                        │
└────────────────────────┬────────────────────────────────────────┘
                         │ Upload targets/states (one-way, cheap)
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│                     GPU COMPUTE                                 │
│  Owns: Positions, Velocities (physics simulation)              │
│  Does: Separation + Movement + Arrival detection               │
│  Writes: Directly to MultiMesh buffer (zero-copy)              │
└────────────────────────┬────────────────────────────────────────┘
                         │ Zero-copy (already on GPU)
                         ▼
┌─────────────────────────────────────────────────────────────────┐
│                     MULTIMESH RENDER                            │
│  (No CPU involvement - GPU wrote directly)                     │
└─────────────────────────────────────────────────────────────────┘
        │
        └───────────── GDScript (UI only) ◀─────────────────────┘
```

References:
- [godot-bevy Book](https://bytemeadow.github.io/godot-bevy/getting-started/basic-concepts.html)
- [FSM in ECS](https://www.richardlord.net/blog/ecs/finite-state-machines-with-ash)

### Migration Chunks

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
- [x] Spatial grid for O(n) neighbor lookup (128x128 cells, 48 NPCs/cell)
- [x] Colors and movement confirmed working
- [x] Separation algorithm (boids-style: accumulate proportionally, no normalization)
- [x] Persistent arrival flag (NPCs stay arrived after being pushed)
- [x] Zero-distance fallback (golden angle when NPCs overlap exactly)
- [x] reset() function for scene reload
- [x] Arrival flag initialization on spawn
- [x] Test harness with 5 scenarios (arrive, separation, both, circle, mass)
- [x] TDD assertions with automated PASS/FAIL (get_npc_position, min separation check)
- [ ] Zero-copy rendering via `multimesh_get_buffer_rd_rid()`
- [ ] Result: 10K+ NPCs with separation forces

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

**Chunk 6: Farmer Logic**
- [ ] Farming, Walking, Resting states
- [ ] Farm assignment, work schedule
- [ ] Result: Farmers work and rest

**Chunk 7: Combat**
- [ ] GPU spatial queries for targeting
- [ ] Damage system, death handling
- [ ] Result: NPCs fight

**Chunk 8: Raider Logic**
- [ ] Raiding, Returning states
- [ ] Food stealing/delivery
- [ ] Result: Full game loop

**Chunk 9: UI Integration**
- [ ] Signals to GDScript (death, level up, food)
- [ ] Selection queries
- [ ] Result: UI works again

### Performance Targets

| Phase | NPCs | FPS | Status |
|-------|------|-----|--------|
| GDScript baseline | 3,000 | 60 | Reference |
| Chunk 1-2 (CPU Bevy) | 5,000 | 60+ | ✅ Done |
| Chunk 3 (GPU physics) | 10,000+ | 140 | ✅ Done |
| Chunk 4 (world data) | 10,000+ | 140 | ✅ Done |
| Chunk 5 (guard logic) | 10,000+ | 140 | ✅ Done |
| Chunk 6-9 (full game) | 10,000+ | 60+ | Planned |
| Zero-copy optimization | 20,000+ | 60+ | Future |

## Credits

- Sprite assets: [Kenney Roguelike RPG Pack](https://kenney.nl/assets/roguelike-rpg-pack)
- Engine: Godot 4.5
