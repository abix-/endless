# Endless Architecture Docs

## How to Use These Docs

These docs are the **source of truth** for system architecture. When building new features or modifying existing systems:

1. **Before coding**: Read the relevant doc to understand the current architecture, data flow, and known issues.
2. **During coding**: Follow the patterns documented here (job-as-template spawn, chained combat, GPU buffer layout). If you need to deviate, update the doc first.
3. **After coding**: Update the doc if the architecture changed. Add new known issues discovered during implementation. Adjust ratings if improvements were made.
4. **Code comments** stay educational (explain what code does for someone learning Rust/GLSL). Architecture diagrams, data flow, buffer layouts, and system interaction docs live here, not in code.
5. **README.md** is the game intro (description, gameplay, controls). **docs/roadmap.md** has feature checkboxes. Don't mix them.

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
    ├─ Channels (GodotToBevy, BevyToGodot) ▶ [messages.md]
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
| [frame-loop.md](frame-loop.md) | Per-frame execution order, communication bridges, timing | 7/10 |
| [gpu-compute.md](gpu-compute.md) | Compute shaders, 11 GPU buffers, spatial grid, CPU sync | 6/10 |
| [combat.md](combat.md) | Attack → damage → death → cleanup, slot recycling | 7/10 |
| [projectiles.md](projectiles.md) | Fire → move → collide → expire, dynamic MultiMesh | 7/10 |
| [spawn.md](spawn.md) | Single spawn path, job-as-template, slot allocation | 7/10 |
| [behavior.md](behavior.md) | State machine, energy, patrol, rest/work/eat, steal/flee/recover, farm growth | 7/10 |
| [api.md](api.md) | Complete GDScript-to-Rust API (36 methods) | - |
| [messages.md](messages.md) | Static queues, GpuUpdateMsg messages, GPU_READ_STATE | 7/10 |
| [concepts.md](concepts.md) | Foundational patterns (DOD, spatial grid, compute shaders, ECS) | - |
| [roadmap.md](roadmap.md) | Feature tracking, performance targets, game design reference | - |

## File Map

```
autoloads/
  config.gd             # All tunable constants
  user_settings.gd      # Persistent user preferences
entities/
  player.gd             # Camera controls
world/
  location.gd           # Building sprites (SPRITES dict, LOCATION_SPRITES, *_PIECES arrays)
  terrain_renderer.gd   # Terrain tile rendering with sprite tiling
  terrain_sprite.gdshader # Terrain tile shader (used by location MultiMesh in Rust)
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
  guard_post_menu.gd    # Guard post upgrades
rust/
  Cargo.toml            # Bevy 0.18 + godot-bevy 0.11 dependencies
  src/lib.rs            # EcsNpcManager: GDScript API bridge, GPU dispatch, rendering
  src/gpu.rs            # GPU compute shader dispatch and buffer management
  src/messages.rs       # Static queues and message types (GDScript → Bevy)
  src/components.rs     # ECS components (NpcIndex, Job, Energy, Health, states, flee/leash)
  src/constants.rs      # Tuning parameters (grid size, separation, energy rates)
  src/resources.rs      # Bevy resources (NpcCount, NpcEntityMap, GameTime, GameConfig, PopulationStats, FactionStats)
  src/world.rs          # World data structs, sprite definitions, location MultiMesh rendering
  src/systems/
    spawn.rs            # Bevy spawn systems (drain queues → create entities)
    combat.rs           # Attack system (GPU targets → damage → chase)
    health.rs           # Damage, death, cleanup, slot recycling
    behavior.rs         # Unified decision system (priority cascade), energy, arrivals
    economy.rs          # Food production, respawning, population tracking
shaders/
  npc_compute.glsl      # GPU: movement + separation + combat targeting
  projectile_compute.glsl # GPU: projectile movement + collision
  npc_sprite.gdshader   # Visual: NPC sprites with HP bar, flash, sprite atlas
  halo.gdshader         # Visual: healing zone indicator (not yet used)
  loot_icon.gdshader    # Visual: raider carrying food (not yet used)
  sleep_icon.gdshader   # Visual: resting indicator (not yet used)
scenes/
  main.gd               # World generation, food tracking, game setup
  main.tscn             # Main game scene
  ecs_test.gd           # Test harness (500-10000 NPCs configurable)
  ecs_test.tscn         # 11 behavior tests with visual markers and PASS/FAIL
  bevy_poc.tscn         # Original POC (5000 NPCs @ 140fps)
tools/
  sprite_browser.gd     # Dev tool for browsing sprite atlas
```

## Configuration

Game constants are in `autoloads/config.gd` (world size, NPC counts, energy thresholds). Most values are configurable via the start menu.

## GDScript Patterns

**User Settings** (`autoloads/user_settings.gd`):
1. Add variable with default
2. Add setter that emits `settings_changed`
3. Add to `_save()` and `_load()`
4. Connect via `UserSettings.settings_changed.connect()`

**Shader Per-Instance Data** (INSTANCE_CUSTOM in `.gdshader`):
```
r = health percent
g = flash intensity
b = sprite frame X / 255
a = sprite frame Y / 255
```
HP bar modes: 0=off, 1=when damaged, 2=always (uniform int)

**Location Types** (valid `location_type` exports):
- `"field"` - farm (3x3)
- `"camp"` - raider camp (2x2)
- `"home"` - bed (1x1)
- `"guard_post"` - guard post (1x1)
- `"fountain"` - town center (1x1)

## Aggregate Known Issues

Collected from all docs. Priority order:

1. **No generational indices** — GPU slot indices are raw `usize`. Currently safe (chained execution), risk grows with async patterns. ([combat.md](combat.md))
2. **npc_count/proj_count never shrink** — high-water marks. Grid and buffers sized to peak, not active count. ([spawn.md](spawn.md), [projectiles.md](projectiles.md))
3. **No pathfinding** — straight-line movement with separation physics. ([behavior.md](behavior.md))
4. **InCombat can stick** — no timeout if target dies out of detection range. ([behavior.md](behavior.md))
5. **Two stat presets only** — AttackStats has melee and ranged constructors but no per-NPC variation beyond these. ([combat.md](combat.md))
6. **Healing halo visual not working** — healing_system heals NPCs but shader halo effect isn't rendering correctly yet. ([behavior.md](behavior.md))
