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

## Hard Rules

1. Never use throttled readback (`factions`, `threat_counts`) as a hard validity gate for combat, ownership, or identity.
2. Validate ownership/faction/identity through ECS (`Faction`, `EntityMap`) first.
3. Treat `combat_targets` as "candidate index only"; re-validate in ECS before applying gameplay effects.
4. If GPU and ECS disagree on identity or ownership, ECS wins.
5. Readback is asynchronous; stale values are expected, not exceptional.

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

See:
- `rust/src/gpu.rs` (`sync_readback_ranges`)
- `rust/src/resources.rs` (`GpuReadState`)
- `docs/messages.md` (message + readback flow)
