# Authority Contract

This document defines the canonical source of truth for gameplay data and how GPU readback must be used.

## Why This Exists

We had regressions where throttled GPU readback fields were treated as authoritative gameplay state.
This contract prevents that class of bug.

## Source Of Truth Rules

| Data | Authoritative Owner | Readback Cadence | Allowed Gameplay Usage |
|------|----------------------|------------------|------------------------|
| `positions` | GPU compute | Always-on async readback (about 1 frame old) | Yes: movement sync, arrival checks, targeting distance |
| `combat_targets` | GPU compute | Always-on async readback (about 1 frame old) | Yes: candidate target selection |
| `health` (entity state) | ECS/CPU (`Health` components) | GPU mirror read back always-on | ECS is truth; GPU value is advisory |
| `factions` | ECS/CPU (`Faction`, `EntityMap` NPC metadata) | Throttled readback (every 60 frames) | No hard gates; debug/advisory only |
| `threat_counts` | GPU compute derived metric | Throttled readback (every 30 frames) | Heuristic input only; never identity/ownership truth |
| Slot identity (`slot -> entity`) | ECS/CPU (`GpuSlotPool`, `EntityMap`) | N/A | Authoritative for routing and ownership checks |
| Entity slot count (high-water mark) | `GpuSlotPool.count()` | `RenderFrameConfig.npc.count` (FixedUpdate copy, render world only) | GPU buffer sizing, dispatch bounds. Main-world systems MUST read `GpuSlotPool` directly. |
| Live NPC count | `EntityMap.npc_count()` | None | Gameplay logic, UI, test assertions |
| Live building count | `EntityMap.building_count()` | None | Gameplay logic, UI, test assertions |

## Hard Rules

1. Never use throttled readback (`factions`, `threat_counts`) as a hard validity gate for combat, ownership, or identity.
2. Validate ownership/faction/identity through ECS (`Faction`, `EntityMap`) first.
3. Treat `combat_targets` as "candidate index only"; re-validate in ECS before applying gameplay effects.
4. If GPU and ECS disagree on identity or ownership, ECS wins.
5. Readback is asynchronous; stale values are expected, not exceptional.
6. Main-world systems that size buffers or iterate entity slots must read `GpuSlotPool.count()` directly. `RenderFrameConfig.npc.count` is a render-world convenience copy updated in FixedUpdate — it lags behind allocations in OnEnter/Update.
7. For live NPC/building counts, use `EntityMap` methods (`npc_count()`, `building_count()`, `iter_npcs()`, `iter_instances()`). Never use `GpuSlotPool.alive()` for NPC-only or building-only counts — it's the combined total of both.

## Practical Pattern

For combat target validation:

1. Read candidate target slot from `GpuReadState.combat_targets`.
2. Resolve slot in `EntityMap`.
3. Validate faction/ownership from ECS data.
4. Use GPU-readback fields only for geometry/range (`positions`) and heuristics.

## Cadence Notes

- `positions`, `combat_targets`, and `health` are always-on readbacks.
- `factions` readback is intentionally throttled to every 60 frames.
- `threat_counts` readback is intentionally throttled to every 30 frames.
- Throttled readback also has async delay (spawn at frame N, consumed later).

## Slot Namespace

NPCs and buildings share one `GpuSlotPool` namespace.
- `GpuSlotPool.count()` = high-water mark of all slots (NPCs + buildings). Use for buffer sizing.
- `GpuSlotPool.alive()` = allocated minus freed (combined). Don't use for NPC-only or building-only counts.
- `EntityMap.npc_count()` / `EntityMap.building_count()` = type-specific live counts.

See:
- `rust/src/gpu.rs` (`sync_readback_ranges`, `build_visual_upload`)
- `rust/src/resources.rs` (`GpuSlotPool`, `GpuReadState`, `EntityMap`)
