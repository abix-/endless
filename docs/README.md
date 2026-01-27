# Endless Architecture Docs

## How to Use These Docs

These docs are the **source of truth** for system architecture. When building new features or modifying existing systems:

1. **Before coding**: Read the relevant doc to understand the current architecture, data flow, and known issues.
2. **During coding**: Follow the patterns documented here (dual-write spawn, chained combat, GPU buffer layout). If you need to deviate, update the doc first.
3. **After coding**: Update the doc if the architecture changed. Add new known issues discovered during implementation. Adjust ratings if improvements were made.
4. **Code comments** stay educational (explain what code does for someone learning Rust/GLSL). Architecture diagrams, data flow, buffer layouts, and system interaction docs live here, not in code.

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
| [gpu-compute.md](gpu-compute.md) | Compute shaders, 20 GPU buffers, spatial grid, CPU sync | 9/10 |
| [combat.md](combat.md) | Attack → damage → death → cleanup, slot recycling | 8/10 |
| [projectiles.md](projectiles.md) | Fire → move → collide → expire, dynamic MultiMesh | 8/10 |
| [spawn.md](spawn.md) | Dual-write pattern, slot allocation, Bevy entity creation | 8/10 |
| [behavior.md](behavior.md) | State machine, energy, patrol, rest/work cycles | 7/10 |
| [api.md](api.md) | Complete GDScript-to-Rust API (26 methods) | - |
| [messages.md](messages.md) | Static queues, GPU_UPDATE_QUEUE, GPU_READ_STATE | 8/10 |

## Aggregate Known Issues

Collected from all docs. Priority order:

1. **No generational indices** — GPU slot indices are raw `usize`. Currently safe (chained execution), risk grows with async patterns. ([combat.md](combat.md))
2. **npc_count/proj_count never shrink** — high-water marks. Grid and buffers sized to peak, not active count. ([spawn.md](spawn.md), [projectiles.md](projectiles.md))
3. **Spatial grid built on CPU** — uploaded every frame. GPU-side grid build would eliminate transfer. ([gpu-compute.md](gpu-compute.md))
4. **No pathfinding** — straight-line movement with separation physics. ([behavior.md](behavior.md))
5. **InCombat can stick** — no timeout if target dies out of detection range. ([behavior.md](behavior.md))
6. **Fixed stats** — all NPCs have identical attack stats. ([combat.md](combat.md))
