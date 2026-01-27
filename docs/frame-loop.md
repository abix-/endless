# Frame Loop

## Overview

Each Godot frame executes two independent systems: the Godot `process()` function (GPU compute + rendering) and the Bevy `Update` schedule (game logic). They communicate through static Mutex queues.

## Per-Frame Execution Order

```
BevyApp._process() (autoload — runs FIRST in frame)
│
├─ Step::Drain
│     reset_bevy_system, drain_spawn_queue, drain_target_queue,
│     drain_arrival_queue, drain_damage_queue
│
├─ ApplyDeferred (flush entity commands)
│
├─ Step::Spawn
│     spawn_npc_system (pushes GPU_UPDATE_QUEUE + updates GPU_DISPATCH_COUNT),
│     apply_targets_system
│
├─ ApplyDeferred (flush so Combat sees new entities)
│
├─ Step::Combat (chained — sequential execution)
│     cooldown_system → attack_system → damage_system →
│     death_system → death_cleanup_system
│
└─ Step::Behavior
      handle_arrival_system, steal_arrival_system, energy_system,
      flee_system, leash_system, tired_system, wounded_rest_system,
      recovery_system, steal_decision_system, resume_patrol_system,
      resume_work_system, patrol_system

EcsNpcManager._process(delta) (scene node — runs AFTER autoloads)
│
├─ 1. Write FRAME_DELTA for Bevy systems
├─ 2. Read npc_count from GPU_DISPATCH_COUNT
│
├─ 3. Drain GPU_UPDATE_QUEUE → write to GPU buffers
│     Guard: idx < MAX_NPC_COUNT (buffer size, not dispatch count)
│     SetTarget, SetHealth, SetFaction, SetPosition,
│     SetSpeed, SetColor, ApplyDamage, HideNpc
│
├─ 4. GPU NPC Dispatch (npc_compute.glsl)
│     ├─ Build spatial grid (CPU) → upload grid buffers
│     ├─ Dispatch compute shader (separation + movement + combat targeting)
│     ├─ Read back: positions, combat_targets
│     └─ Write GPU_READ_STATE (positions, targets, health, factions)
│
├─ 5. Detect arrivals (compare arrival_buffer to prev_arrivals)
│     └─ Push new arrivals to ARRIVAL_QUEUE
│
├─ 6. Build NPC MultiMesh buffer → upload to RenderingServer
│
├─ 7. GPU Projectile Dispatch (projectile_compute.glsl)
│     ├─ Dispatch compute shader (movement + collision)
│     ├─ Read hit results → push to DAMAGE_QUEUE + FREE_PROJ_SLOTS
│     ├─ Read positions + active flags
│     ├─ Resize projectile MultiMesh to proj_count (if changed)
│     └─ Build projectile MultiMesh buffer → upload
│
└─ 8. (Godot renders the frame)
```

## Communication Bridges

| Direction | Mechanism | Examples |
|-----------|-----------|---------|
| GDScript → Bevy | Static Mutex queues (SPAWN_QUEUE, etc.) | spawn_npc() pushes SpawnNpcMsg |
| Bevy → GPU | GPU_UPDATE_QUEUE (drained in process step 3) | SetTarget, SetHealth, SetPosition, HideNpc |
| GPU → Bevy | GPU_READ_STATE (written in process step 4) | Positions, combat_targets, health |
| GPU → Bevy | ARRIVAL_QUEUE (written in process step 5) | Arrival events |
| GPU → Bevy | DAMAGE_QUEUE (written in process step 7) | Projectile hit damage |

## Key Invariant: Two Separate Counts

`NPC_SLOT_COUNTER` (high-water mark) is incremented immediately by `allocate_slot()` when GDScript calls `spawn_npc()`. `GPU_DISPATCH_COUNT` is only updated by `spawn_npc_system` after it pushes GPU buffer data to `GPU_UPDATE_QUEUE`. `process()` reads `GPU_DISPATCH_COUNT` for dispatch, so it never dispatches NPCs with uninitialized buffers.

The drain guard uses `idx < MAX_NPC_COUNT` (buffer size) instead of `idx < npc_count`, allowing spawn buffer writes to land even before `GPU_DISPATCH_COUNT` catches up.

## Timing

BevyApp (autoload) processes before scene nodes. godot-bevy ticks Bevy's `app.update()` during the BevyApp `_process()` call. EcsNpcManager (scene node) processes after, draining GPU_UPDATE_QUEUE and dispatching. This ordering means spawn data flows: `spawn_npc()` (frame N) → SPAWN_QUEUE → Bevy spawn_npc_system (frame N+1) → GPU_UPDATE_QUEUE → process() drain (frame N+1) → dispatch. One-frame latency from spawn call to first render.

## Known Issues

- **One-frame latency**: Bevy systems read GPU_READ_STATE from the *previous* frame's dispatch. Combat targeting uses positions that are one frame old.
- **No generational indices on GPU side**: NPC slot indices are raw `usize`. Currently safe because chained Combat systems prevent stale references within a frame. See [combat.md](combat.md) for analysis.

## Rating: 8/10

The frame loop is well-structured with clear separation between GPU compute, Bevy logic, and rendering. The one-frame latency is standard for CPU/GPU architectures. The Mutex-based communication is simple and correct, though it won't scale to multi-threaded Bevy (not needed today).
