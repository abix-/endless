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

**NOTE: Phase 1-3 + 2.5 migration in progress. GPU compute working, instanced NPC rendering via RenderCommand.**

```
Pure Bevy App (main.rs)
    │
    ▼
Bevy ECS (lib.rs build_app)
    │
    ├─ Messages (static queues) ───────────▶ [messages.md]
    │
    ├─ GPU Compute (gpu.rs) ────────────▶ [gpu-compute.md]
    │   ├─ Bevy render graph integration
    │   └─ WGSL shader (assets/shaders/npc_compute.wgsl)
    │
    ├─ NPC Instanced Rendering (npc_render.rs)
    │   ├─ RenderCommand + Transparent2d phase
    │   ├─ Two vertex buffers (quad + per-instance)
    │   └─ WGSL shader (assets/shaders/npc_render.wgsl)
    │
    ├─ Sprite Rendering (render/mod.rs)
    │   ├─ 2D camera, texture atlases
    │   └─ Character + world sprite sheets
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
| [gpu-compute.md](gpu-compute.md) | Compute shaders (wgpu/WGSL via Bevy render graph) | 7/10 |
| [combat.md](combat.md) | Attack → damage → death → cleanup, slot recycling | 7/10 |
| [spawn.md](spawn.md) | Single spawn path, job-as-template, slot allocation | 7/10 |
| [behavior.md](behavior.md) | State machine, energy, patrol, rest/work/eat, steal/flee/recover, farm growth | 7/10 |
| [messages.md](messages.md) | Static queues, GpuUpdateMsg messages, GPU_READ_STATE | 7/10 |
| [concepts.md](concepts.md) | Foundational patterns (DOD, spatial grid, compute shaders, ECS) | - |
| [roadmap.md](roadmap.md) | Feature tracking, Pure Bevy migration plan | - |

## File Map

**NOTE: Phase 1-3 + 2.5 in progress. GPU compute, sprite rendering, instanced NPC rendering working.**

```
rust/
  Cargo.toml            # Pure Bevy 0.18 + bevy_egui + bytemuck
  src/main.rs           # Bevy App entry point, asset root = project root
  src/lib.rs            # build_app(), system scheduling, helpers
  src/gpu.rs            # GPU compute via Bevy render graph
  src/npc_render.rs     # GPU instanced NPC rendering (RenderCommand + Transparent2d)
  src/render.rs         # 2D camera, texture atlases, sprite rendering
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

shaders/
  npc_compute.wgsl          # WGSL compute shader (ported from GLSL)
  npc_render.wgsl           # WGSL render shader (instanced quad + sprite atlas)
  npc_compute.glsl          # Old GLSL compute shader (porting reference)
  projectile_compute.glsl   # Old GLSL projectile shader (to be ported)

assets/
  roguelikeChar_transparent.png   # Character sprites (54x12 grid)
  roguelikeSheet_transparent.png  # World sprites (57x31 grid)

(Godot files - to be ported to bevy_egui in Phase 5-7)
ui/*.gd               # → bevy_egui panels
scenes/main.gd        # → world_gen.rs
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
