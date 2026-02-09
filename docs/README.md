# Endless Architecture Docs

## How to Use These Docs

These docs are the **source of truth** for system architecture. When building new features or modifying existing systems:

1. **Before coding**: Read the relevant doc to understand the current architecture, data flow, and known issues.
2. **During coding**: Follow the patterns documented here (job-as-template spawn, chained combat, GPU buffer layout). If you need to deviate, update the doc first.
3. **After coding**: Update the doc if the architecture changed. Add new known issues discovered during implementation.
4. **Code comments** stay educational (explain what code does for someone learning Rust/WGSL). Architecture diagrams, data flow, buffer layouts, and system interaction docs live here, not in code.
5. **README.md** is the game intro (description, gameplay, controls). **docs/roadmap.md** has stages (priority order) and capabilities (feature inventory) — read its maintenance guide before editing.

## Test-Driven Development

All systems should be validated through tests. Tests use **phased assertions** — each phase checks one layer of the pipeline and fails fast with diagnostic values.

**Pattern:**
1. Startup system populates world data and spawns NPCs
2. Update system runs phased assertions with time gates
3. Each phase checks one pipeline layer and logs PASS/FAIL with diagnostic values
4. On failure, all prior phase results show exactly which layer broke

**Test 12 (Vertical Slice) — validates full core loop:**
- Phase 1: 12 NPCs spawned (5 farmers, 2 guards, 5 raiders)
- Phase 2: GPU readback returns valid positions
- Phase 3: Farmers arrive at farms, begin working
- Phase 4: Raiders form group, dispatched to farm
- Phase 5: Guards acquire combat targets via spatial grid
- Phase 6: Damage applied
- Phase 7: Death occurs, slot recycled
- Phase 8: Replacement raider respawns from camp food

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
    ├─ NPC Instanced Rendering (npc_render.rs) ─▶ [rendering.md]
    │   ├─ RenderCommand + Transparent2d phase
    │   ├─ Two vertex buffers (quad + per-instance)
    │   └─ WGSL shader (shaders/npc_render.wgsl)
    │
    ├─ Sprite Rendering (render.rs) ───────────▶ [rendering.md]
    │   ├─ 2D camera, texture atlases
    │   └─ Character + world sprite sheets
    │
    └─ Bevy Systems
        ├─ Spawn systems ──────────────────▶ [spawn.md]
        ├─ Combat pipeline ────────────────▶ [combat.md]
        ├─ Behavior systems ───────────────▶ [behavior.md]
        └─ Economy systems ────────────────▶ [economy.md]

Frame execution order ─────────────────────▶ [frame-loop.md]
```

## Documents

| Doc | What it covers | Rating |
|-----|---------------|--------|
| [frame-loop.md](frame-loop.md) | Per-frame execution order, main/render world timing | 8/10 |
| [gpu-compute.md](gpu-compute.md) | Compute shaders, spatial grid, separation physics, combat targeting, GPU→ECS readback | 9/10 |
| [rendering.md](rendering.md) | GPU instanced NPC rendering, sprite atlas, RenderCommand pipeline, camera controls | 7/10 |
| [combat.md](combat.md) | Attack → damage → death → cleanup, slot recycling | 4/10 |
| [spawn.md](spawn.md) | Single spawn path, job-as-template, slot allocation | 7/10 |
| [behavior.md](behavior.md) | Decision system, utility AI, state machine, energy, patrol, flee/leash | 8/10 |
| [economy.md](economy.md) | Farm growth, food theft, starvation, camp foraging, raider respawning | 7/10 |
| [messages.md](messages.md) | Static queues, GpuUpdateMsg messages, GPU_READ_STATE | 7/10 |
| [resources.md](resources.md) | Bevy resources, game state ownership, UI caches, world data | 7/10 |
| [projectiles.md](projectiles.md) | GPU projectile compute, hit detection, slot allocation | 4/10 |
| [concepts.md](concepts.md) | Foundational patterns (DOD, spatial grid, compute shaders, ECS) | - |
| [roadmap.md](roadmap.md) | Feature tracking, migration plan | - |

**Ratings reflect system quality, not doc accuracy.** Frame loop is clean with clear phase ordering. Rendering is 7/10 — custom instanced pipeline with camera controls (WASD pan, scroll zoom, click-to-select). GPU compute is 9/10 — 3-mode spatial grid, separation physics (boids + TCP dodge + backoff), combat targeting, full readback. Combat is 4/10 — pipeline exists but attack_system wired to GPU targeting. Projectiles are 4/10 — compute + hit readback working but no rendering. Behavior is 8/10 — central brain with utility AI. Spawn, economy, messages, and resources are solid at 7/10.

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
  npc_compute.wgsl      # WGSL compute shader (movement + spatial grid + combat targeting)
  npc_render.wgsl       # WGSL render shader (instanced quad + sprite atlas)
  projectile_compute.wgsl # WGSL compute shader (projectile movement + collision)

assets/
  roguelikeChar_transparent.png   # Character sprites (54x12 grid, 16px + 1px margin)
  roguelikeSheet_transparent.png  # World sprites (57x31 grid, 16px + 1px margin)
```

## Known Issues

Collected from all docs. Priority order:

1. **No generational indices** — GPU slot indices are raw `usize`. Safe with chained execution, risk grows with async patterns. ([combat.md](combat.md))
2. **No pathfinding** — straight-line movement with separation physics. ([behavior.md](behavior.md))
3. **npc_count never shrinks** — high-water mark. Grid and buffers sized to peak, not active count. ([spawn.md](spawn.md))
