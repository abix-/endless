# Authority Contract

This document is the **single source of truth** for data ownership across the entire codebase. Every piece of gameplay data has exactly one authoritative owner.

## Why This Exists

We had regressions where throttled GPU readback fields were treated as authoritative gameplay state.
This contract prevents that class of bug.

## Data Ownership

| Data | Authority | Direction | Notes |
|------|-----------|-----------|-------|
| **GPU-Authoritative** (written by compute shader, read back ~1 frame later) ||||
| Positions | GPU | GPU → CPU | Compute shader moves NPCs; Bevy async Readback → GpuReadState. Always-on. |
| Spatial grid | GPU | Internal | Built each frame (clear → insert → query). Not read back. |
| Combat targets | GPU | GPU → CPU | Nearest enemy index via grid neighbor search; readback to GpuReadState. Always-on. Candidate only — must re-validate in ECS. |
| Threat counts | GPU | GPU → CPU | Derived metric. Throttled readback (every 30 frames). Heuristic input only; never identity/ownership truth. |
| **CPU-Authoritative** (written by ECS systems, uploaded to GPU next frame) ||||
| Health | CPU (`Health` component) | CPU → GPU | damage_system/healing_system write; uploaded for GPU targeting threshold. GPU mirror is advisory. |
| Targets/Goals | CPU | CPU → GPU | Systems submit to MovementIntents; resolve_movement_system emits SetTarget; GPU interpolates movement. |
| Factions | CPU (`Faction`, `EntityMap`) | CPU → GPU | Set at spawn, never changes (0=Villager, 1+=Raider towns). Throttled readback (every 60 frames) — debug/advisory only. |
| Speeds | CPU | CPU → GPU | Set at spawn, modified by starvation_system. |
| Arrivals | CPU | Internal | `HasTarget` + `gpu_position_readback` distance check → `AtDestination`. |
| Slot identity | CPU (`GpuSlotPool`, `EntityMap`) | N/A | Authoritative for routing and ownership checks. |
| Entity slot count | `GpuSlotPool.count()` | `RenderFrameConfig.npc.count` (FixedUpdate copy) | GPU buffer sizing, dispatch bounds. Main-world systems MUST read `GpuSlotPool` directly. |
| Live NPC count | `EntityMap.npc_count()` | None | Gameplay logic, UI, test assertions. |
| Live building count | `EntityMap.building_count()` | None | Gameplay logic, UI, test assertions. |
| **CPU-Only** (never sent to GPU compute) ||||
| GpuSlot | CPU | Internal | Links Bevy entity to GPU slot index (unified namespace for NPCs + buildings). |
| Job | CPU | Internal | Archer, Farmer, Raider, Fighter, Miner — determines behavior. |
| Energy | CPU | Internal | Drives tired/rest decisions (drain/recover rates). |
| State markers | CPU | Internal | Dead, InCombat, ActivityKind (Patrol/Rest/Raid/etc.), NpcFlags.at_destination |
| Config components | CPU | Internal | FleeThreshold, LeashRange, WoundedThreshold, Stealer. |
| AttackTimer | CPU | Internal | Cooldown between attacks. |
| AttackStats | CPU | Internal | melee(range=100, cooldown=1.0) or ranged(range=200, cooldown=1.5). |
| PatrolRoute | CPU | Internal | Guard post sequence for patrols. |
| Home | CPU | Internal | Rest location (spawner building). |
| WorkPosition | CPU | Internal | Farm location for farmers. |
| Path waypoints | CPU (`NpcPath` component) | CPU → GPU (via `goals[]`) | A* produces waypoints; CPU advances on arrival; GPU steers to current waypoint via existing `goals[]` upload. |
| **Render-Only** (uploaded to visual/equip storage buffers, never in compute shader) ||||
| Sprite indices | EntityGpuState | CPU → NpcVisualUpload → NpcVisualBuffers | Atlas col/row per NPC; packed into visual storage buffer by build_visual_upload. |
| Colors | ECS → NpcVisualUpload | CPU → NpcVisualBuffers | RGBA tint from Faction/Job; packed into visual storage buffer by build_visual_upload. |
| Equipment sprites | ECS → NpcVisualUpload | CPU → NpcVisualBuffers | Per-layer col/row (armor/helmet/weapon/item/status/healing); -1.0 sentinel = unequipped/inactive. Event-driven via MarkVisualDirty. |

## Hard Rules

1. Never use throttled readback (`factions`, `threat_counts`) as a hard validity gate for combat, ownership, or identity.
2. Validate ownership/faction/identity through ECS (`Faction`, `EntityMap`) first.
3. Treat `combat_targets` as "candidate index only"; re-validate in ECS before applying gameplay effects.
4. If GPU and ECS disagree on identity or ownership, ECS wins.
5. Readback is asynchronous; stale values are expected, not exceptional.
6. Main-world systems that size buffers or iterate entity slots must read `GpuSlotPool.count()` directly. `RenderFrameConfig.npc.count` is a render-world convenience copy updated in FixedUpdate — it lags behind allocations in OnEnter/Update.
7. For live NPC/building counts, use `EntityMap` methods (`npc_count()`, `building_count()`, `iter_npcs()`, `iter_instances()`). Never use `GpuSlotPool.alive()` for NPC-only or building-only counts — it's the combined total of both.
8. No system may read from GPU readback AND write back to the same GPU field in the same frame. That creates a feedback loop where 1-frame delay compounds into oscillation.

## Practical Pattern

For combat target validation:

1. Read candidate target slot from `GpuReadState.combat_targets`.
2. Resolve slot in `EntityMap`.
3. Validate faction/ownership from ECS data.
4. Use GPU-readback fields only for geometry/range (`positions`) and heuristics.

## Staleness Budget

- Always-on readbacks (`positions`, `combat_targets`, `health`) are typically ~1 frame stale.
- Throttled readbacks (`factions`, `threat_counts`) are intentionally multi-frame stale and must not be used as hard gameplay authority.
- Throttled readback also has async delay (spawn at frame N, consumed later).

## Slot Namespace

NPCs and buildings share one `GpuSlotPool` namespace.
- `GpuSlotPool.count()` = high-water mark of all slots (NPCs + buildings). Use for buffer sizing.
- `GpuSlotPool.alive()` = allocated minus freed (combined). Don't use for NPC-only or building-only counts.
- `EntityMap.npc_count()` / `EntityMap.building_count()` = type-specific live counts.

## Buffer Sizing

All GPU storage and readback buffers that index by slot MUST use `MAX_ENTITIES`, not `MAX_NPC_COUNT`. The unified `GpuSlotPool` interleaves NPCs and buildings in a single namespace -- any slot from 0..MAX_ENTITIES could be either type.

- `MAX_ENTITIES` = `MAX_NPC_COUNT + MAX_BUILDINGS` = buffer sizing for GPU slot-indexed data
- `MAX_NPC_COUNT` = NPC-specific ECS queries only, never for GPU buffer sizing
- Readback copy sizes must never exceed the destination buffer capacity
- `GpuSlotPool.count()` (high-water mark) can reach `MAX_ENTITIES`; all readback buffers must accommodate this

See:
- `rust/src/gpu.rs` (`sync_readback_ranges`, `setup_readback_buffers`, `build_visual_upload`)
- `rust/src/resources.rs` (`GpuSlotPool`, `GpuReadState`, `EntityMap`)
