# Endless Architecture Docs

## How to Use These Docs

These docs are the **source of truth** for system architecture. When building new features or modifying existing systems:

1. **Before coding**: Read the relevant doc to understand the current architecture, data flow, and known issues.
2. **During coding**: Follow the patterns documented here (job-as-template spawn, chained combat, GPU buffer layout). If you need to deviate, update the doc first.
3. **After coding**: Update the doc if the architecture changed. Add new known issues discovered during implementation.
4. **Code comments** stay educational (explain what code does for someone learning Rust/WGSL). Architecture diagrams, data flow, buffer layouts, and system interaction docs live here, not in code.
5. **README.md** is the game intro (description, gameplay, controls). **docs/roadmap.md** has feature checkboxes. Don't mix them.

## Test-Driven Development

All systems should be validated through tests. Tests use **phased assertions** — each phase checks one layer of the pipeline and fails fast with diagnostic values.

**Pattern:**
1. Spawn minimal NPCs with **Fighter** job (job=3) — no behavior components, NPCs sit still
2. Each phase has a time gate and assertion (e.g., "at t=2s, GPU targeting should find enemies")
3. Phase results record timestamp + values at pass/fail, included in debug dump
4. Early phases isolate infrastructure (GPU buffers, grid) before testing logic (damage, death)

**Example (Combat test):**
- Phase 1: GPU buffer integrity (faction/health written)
- Phase 2: Spatial grid populated
- Phase 3: GPU targeting finds enemies
- Phase 4: Damage processed
- Phase 5: Death occurs
- Phase 6: Slot recycling

When a test fails, the phase results show exactly which layer broke and what values it saw.

## System Map

```
Pure Bevy App (main.rs)
    │
    ▼
Bevy ECS (lib.rs build_app)
    │
    ├─ Messages (static queues) ───────────▶ [messages.md]
    │
    ├─ GPU Compute (gpu.rs) ───────────────▶ [gpu-compute.md]
    │   ├─ Bevy render graph integration
    │   └─ WGSL shader (shaders/npc_compute.wgsl)
    │
    ├─ NPC Instanced Rendering (npc_render.rs)
    │   ├─ RenderCommand + Transparent2d phase
    │   ├─ Two vertex buffers (quad + per-instance)
    │   └─ WGSL shader (shaders/npc_render.wgsl)
    │
    ├─ Sprite Rendering (render.rs)
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
| [frame-loop.md](frame-loop.md) | Per-frame execution order, main/render world timing | 8/10 |
| [gpu-compute.md](gpu-compute.md) | Compute shaders (wgpu/WGSL via Bevy render graph) | 5/10 |
| [combat.md](combat.md) | Attack → damage → death → cleanup, slot recycling | 7/10 |
| [spawn.md](spawn.md) | Single spawn path, job-as-template, slot allocation | 7/10 |
| [behavior.md](behavior.md) | State machine, energy, patrol, rest/work/eat, steal/flee/recover, farm growth | 7/10 |
| [messages.md](messages.md) | Static queues, GpuUpdateMsg messages, GPU_READ_STATE | 7/10 |
| [concepts.md](concepts.md) | Foundational patterns (DOD, spatial grid, compute shaders, ECS) | - |
| [roadmap.md](roadmap.md) | Feature tracking, migration plan | - |

**Ratings reflect system quality, not doc accuracy.** Frame loop is clean with clear phase ordering. GPU compute is 5/10 — pipeline works but only basic movement is ported (no separation, no grid, no combat targeting, no readback). Combat, spawn, behavior, and messages are solid at 7/10.

## File Map

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
    drain.rs            # Queue drain systems, reset, collect_gpu_updates
    movement.rs         # Target application
    combat.rs           # Attack cooldown, targeting
    health.rs           # Damage, death, cleanup, healing
    behavior.rs         # Unified decision system, arrivals
    economy.rs          # Game time, farm growth, respawning
    energy.rs           # Energy drain/recovery
    sync.rs             # GPU state sync

shaders/
  npc_compute.wgsl      # WGSL compute shader (NPC movement physics)
  npc_render.wgsl       # WGSL render shader (instanced quad + sprite atlas)

assets/
  roguelikeChar_transparent.png   # Character sprites (54x12 grid, 16px + 1px margin)
  roguelikeSheet_transparent.png  # World sprites (57x31 grid, 16px + 1px margin)
```

## Known Issues

Collected from all docs. Priority order:

1. **No GPU→CPU readback** — GPU compute updates positions but results aren't read back to ECS. Combat targeting and arrival detection use stale CPU-side data. ([gpu-compute.md](gpu-compute.md), [frame-loop.md](frame-loop.md))
2. **Compute shader incomplete** — `npc_compute.wgsl` has basic goal movement only. Separation physics, grid neighbor search, and combat targeting not yet ported. ([gpu-compute.md](gpu-compute.md))
3. **Hardcoded camera in render shader** — `npc_render.wgsl` uses constant camera position and viewport instead of Bevy view uniforms. Camera movement/zoom won't affect NPC rendering. ([gpu-compute.md](gpu-compute.md))
4. **No generational indices** — GPU slot indices are raw `usize`. Safe with chained execution, risk grows with async patterns. ([combat.md](combat.md))
5. **No pathfinding** — straight-line movement with separation physics. ([behavior.md](behavior.md))
6. **npc_count never shrinks** — high-water mark. Grid and buffers sized to peak, not active count. ([spawn.md](spawn.md))
