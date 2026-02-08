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

**NOTE: Phase 1 of Pure Bevy migration complete. Architecture is transitioning from Godot+Bevy hybrid to pure Bevy.**

```
Pure Bevy App (main.rs)
    │
    ▼
Bevy ECS (lib.rs build_app)
    │
    ├─ Messages (static queues) ───────────▶ [messages.md]
    │
    ├─ GPU Compute (TODO: Phase 2) ────────▶ [gpu-compute.md]
    │   └─ wgpu compute shaders (port from GLSL)
    │
    └─ Bevy Systems
        ├─ Spawn systems ──────────────────▶ [spawn.md]
        ├─ Combat pipeline ────────────────▶ [combat.md]
        ├─ Behavior systems ───────────────▶ [behavior.md]
        └─ Economy systems ────────────────▶ [behavior.md]

Frame execution order ─────────────────────▶ [frame-loop.md]
```

## Documents

| Doc | What it covers | Rating |
|-----|---------------|--------|
| [frame-loop.md](frame-loop.md) | Per-frame execution order, timing | 7/10 |
| [gpu-compute.md](gpu-compute.md) | Compute shaders (TODO: port to wgpu/WGSL) | 6/10 |
| [combat.md](combat.md) | Attack → damage → death → cleanup, slot recycling | 7/10 |
| [spawn.md](spawn.md) | Single spawn path, job-as-template, slot allocation | 7/10 |
| [behavior.md](behavior.md) | State machine, energy, patrol, rest/work/eat, steal/flee/recover, farm growth | 7/10 |
| [messages.md](messages.md) | Static queues, GpuUpdateMsg messages, GPU_READ_STATE | 7/10 |
| [concepts.md](concepts.md) | Foundational patterns (DOD, spatial grid, compute shaders, ECS) | - |
| [roadmap.md](roadmap.md) | Feature tracking, Pure Bevy migration plan | - |

## File Map

**NOTE: Phase 1 migration complete. Godot files listed for reference during port to bevy_egui.**

```
rust/
  Cargo.toml            # Pure Bevy 0.18 + bevy_egui (no godot deps)
  src/main.rs           # Bevy App entry point
  src/lib.rs            # build_app(), system scheduling, helpers
  src/messages.rs       # Static queues (GpuUpdate, Arrival), Message types
  src/components.rs     # ECS components (NpcIndex, Job, Energy, Health, states)
  src/constants.rs      # Tuning parameters (grid size, separation, energy rates)
  src/resources.rs      # Bevy resources (NpcCount, GameTime, FactionStats, etc.)
  src/world.rs          # World data structs, sprite definitions
  src/systems/
    spawn.rs            # Spawn system (MessageReader<SpawnNpcMsg>)
    drain.rs            # Queue drain systems, reset
    movement.rs         # Target application, GPU position readback
    combat.rs           # Attack cooldown, targeting
    health.rs           # Damage, death, cleanup, healing aura
    behavior.rs         # Unified decision system, arrivals
    economy.rs          # Game time, farm growth, respawning
    energy.rs           # Energy drain/recovery
    sync.rs             # GPU state sync

(Godot files - to be ported to bevy_egui in Phase 5-7)
autoloads/
  config.gd             # → Bevy Resource constants
  user_settings.gd      # → serde JSON persistence
ui/
  start_menu.gd         # → bevy_egui sliders
  left_panel.gd         # → bevy_egui dashboard
  upgrade_menu.gd       # → bevy_egui grid
  roster_panel.gd       # → bevy_egui table
  policies_panel.gd     # → bevy_egui forms
  build_menu.gd         # → bevy_egui popup
  combat_log.gd         # → bevy_egui window
scenes/
  main.gd               # → world_gen.rs (Bevy systems)
shaders/
  npc_compute.glsl      # → gpu/npc_compute.wgsl (Phase 2)
  projectile_compute.glsl # → gpu/projectile_compute.wgsl (Phase 2)
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
4. **Two stat presets only** — AttackStats has melee and ranged constructors but no per-NPC variation beyond these. ([combat.md](combat.md))
5. **Healing halo visual not working** — healing_system heals NPCs but shader halo effect isn't rendering correctly yet. ([behavior.md](behavior.md))
