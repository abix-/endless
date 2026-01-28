# Endless Architecture Docs

## How to Use These Docs

These docs are the **source of truth** for system architecture. When building new features or modifying existing systems:

1. **Before coding**: Read the relevant doc to understand the current architecture, data flow, and known issues.
2. **During coding**: Follow the patterns documented here (job-as-template spawn, chained combat, GPU buffer layout). If you need to deviate, update the doc first.
3. **After coding**: Update the doc if the architecture changed. Add new known issues discovered during implementation. Adjust ratings if improvements were made.
4. **Code comments** stay educational (explain what code does for someone learning Rust/GLSL). Architecture diagrams, data flow, buffer layouts, and system interaction docs live here, not in code.

## Test-Driven Development

All systems are validated through TDD tests in `ecs_test.gd` (Test 1-11). Tests use **phased assertions** — each phase checks one layer of the pipeline and fails fast with diagnostic values.

**Pattern:**
1. Spawn minimal NPCs with **Fighter** job (job=3) — no behavior components, NPCs sit still
2. Each phase has a time gate and assertion (e.g., "at t=2s, GPU targeting should find enemies")
3. Phase results record timestamp + values at pass/fail, included in debug dump
4. Early phases isolate infrastructure (GPU buffers, grid) before testing logic (damage, death)

**Example (Test 10 — Combat):**
- Phase 1: GPU buffer integrity (faction/health written)
- Phase 2: Spatial grid populated
- Phase 3: GPU targeting finds enemies
- Phase 4: Damage processed
- Phase 5: Death occurs
- Phase 6: Slot recycling

When a test fails, the phase results show exactly which layer broke and what values it saw.

## System Map

```
GDScript (ecs_test.gd, game scenes)
    │
    ▼
EcsNpcManager (lib.rs) ── GDScript API ──▶ [api.md]
    │
    ├─ Static Queues ──────────────────────▶ [messages.md]
    │
    ├─ GPU Compute ────────────────────────▶ [gpu-compute.md]
    │   ├─ npc_compute.glsl
    │   └─ projectile_compute.glsl
    │
    └─ Bevy ECS
        ├─ Spawn systems ──────────────────▶ [spawn.md]
        ├─ Combat pipeline ────────────────▶ [combat.md]
        ├─ Behavior systems ───────────────▶ [behavior.md]
        └─ Projectile system ──────────────▶ [projectiles.md]

Frame execution order ─────────────────────▶ [frame-loop.md]
```

## Documents

| Doc | What it covers | Rating |
|-----|---------------|--------|
| [frame-loop.md](frame-loop.md) | Per-frame execution order, communication bridges, timing | 8/10 |
| [gpu-compute.md](gpu-compute.md) | Compute shaders, 20 GPU buffers, spatial grid, CPU sync | 8/10 |
| [combat.md](combat.md) | Attack → damage → death → cleanup, slot recycling | 8/10 |
| [projectiles.md](projectiles.md) | Fire → move → collide → expire, dynamic MultiMesh | 8/10 |
| [spawn.md](spawn.md) | Single spawn path, job-as-template, slot allocation | 8/10 |
| [behavior.md](behavior.md) | State machine, energy, patrol, rest/work, steal/flee/recover | 8/10 |
| [api.md](api.md) | Complete GDScript-to-Rust API (36 methods) | - |
| [messages.md](messages.md) | Static queues, GPU_UPDATE_QUEUE, GPU_READ_STATE | 8/10 |
| [concepts.md](concepts.md) | Foundational patterns (DOD, spatial grid, compute shaders, ECS) | - |
| [roadmap.md](roadmap.md) | Migration chunks, performance targets, lessons learned | - |

## File Map

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
  left_panel.gd         # Stats, performance, NPC inspector (uses ECS query APIs)
  combat_log.gd         # Resizable event log (ECS-only, waiting for signals)
  settings_menu.gd      # Options menu with log filters
  upgrade_menu.gd       # Town management, upgrades (uses ECS query APIs)
  build_menu.gd         # Grid slot building (farms, beds)
  policies_panel.gd     # Faction policies (flee thresholds, off-duty behavior)
  roster_panel.gd       # NPC roster with sorting/filtering (uses ECS query APIs)
  farm_menu.gd          # Farm info popup (ECS-only, waiting for farm API)
rust/
  Cargo.toml            # Bevy 0.18 + godot-bevy 0.11 dependencies
  src/lib.rs            # EcsNpcManager: GDScript API bridge, GPU dispatch, rendering
  src/gpu.rs            # GPU compute shader dispatch and buffer management
  src/messages.rs       # Static queues and message types (GDScript → Bevy)
  src/components.rs     # ECS components (NpcIndex, Job[Farmer/Guard/Raider/Fighter], Energy, Health, states, stealing, flee/leash)
  src/constants.rs      # Tuning parameters (grid size, separation, energy rates)
  src/resources.rs      # Bevy resources (NpcCount, NpcEntityMap, GameTime, GameConfig, PopulationStats, RespawnTimers)
  src/world.rs          # World data structs (Town, Farm, Bed, GuardPost)
  src/systems/
    spawn.rs            # Bevy spawn systems (drain queues → create entities)
    combat.rs           # Attack system (GPU targets → damage → chase)
    health.rs           # Damage, death, cleanup, slot recycling
    behavior.rs         # Energy, tired, rest, patrol, work, steal, flee, leash, recovery
    economy.rs          # Food production, respawning, population tracking (uses PhysicsDelta)
shaders/
  npc_compute.glsl      # GPU: movement + separation + combat targeting
  projectile_compute.glsl # GPU: projectile movement + collision
scenes/
  ecs_test.tscn         # 11 behavior tests with visual markers and PASS/FAIL
  bevy_poc.tscn         # Original POC (5000 NPCs @ 140fps)
scripts/
  ecs_test.gd           # Test harness (500-10000 NPCs configurable)
```

## Aggregate Known Issues

Collected from all docs. Priority order:

1. **No generational indices** — GPU slot indices are raw `usize`. Currently safe (chained execution), risk grows with async patterns. ([combat.md](combat.md))
2. **npc_count/proj_count never shrink** — high-water marks. Grid and buffers sized to peak, not active count. ([spawn.md](spawn.md), [projectiles.md](projectiles.md))
3. **Spatial grid built on CPU** — uploaded every frame. GPU-side grid build would eliminate transfer. ([gpu-compute.md](gpu-compute.md))
4. **No pathfinding** — straight-line movement with separation physics. ([behavior.md](behavior.md))
5. **InCombat can stick** — no timeout if target dies out of detection range. ([behavior.md](behavior.md))
6. **Two stat presets only** — AttackStats has melee and ranged constructors but no per-NPC variation beyond these. ([combat.md](combat.md))
