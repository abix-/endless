# Frame Loop

## Overview

Each Godot frame executes two independent systems: the Godot `process()` function (GPU compute + rendering) and the Bevy `Update` schedule (game logic). They communicate through static Mutex queues.

## Per-Frame Execution Order

```
Godot calls process(delta)
│
├─ 1. Write FRAME_DELTA for Bevy systems
├─ 2. Read npc_count from GPU_READ_STATE
│
├─ 3. Drain GPU_UPDATE_QUEUE → write to GPU buffers
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

Bevy Update schedule (ticked by godot-bevy, same frame)
│
├─ Step::Drain
│     reset_bevy_system, drain_spawn_queue, drain_target_queue,
│     drain_guard_queue, drain_farmer_queue, drain_raider_queue,
│     drain_arrival_queue, drain_damage_queue
│
├─ ApplyDeferred (flush entity commands)
│
├─ Step::Spawn
│     spawn_npc_system, spawn_guard_system, spawn_farmer_system,
│     spawn_raider_system, apply_targets_system
│
├─ ApplyDeferred (flush so Combat sees new entities)
│
├─ Step::Combat (chained — sequential execution)
│     cooldown_system → attack_system → damage_system →
│     death_system → death_cleanup_system
│
└─ Step::Behavior
      handle_arrival_system, energy_system, tired_system,
      resume_patrol_system, resume_work_system, patrol_system
```

## Communication Bridges

| Direction | Mechanism | Examples |
|-----------|-----------|---------|
| GDScript → GPU | Direct `buffer_update()` in spawn functions | Position, color, health, faction written at spawn time |
| GDScript → Bevy | Static Mutex queues (SPAWN_QUEUE, etc.) | spawn_npc() pushes SpawnNpcMsg |
| Bevy → GPU | GPU_UPDATE_QUEUE (drained in process step 3) | SetTarget, SetHealth, HideNpc |
| GPU → Bevy | GPU_READ_STATE (written in process step 4) | Positions, combat_targets, health |
| GPU → Bevy | ARRIVAL_QUEUE (written in process step 5) | Arrival events |
| GPU → Bevy | DAMAGE_QUEUE (written in process step 7) | Projectile hit damage |

## Key Invariant

Spawn functions write GPU buffers **directly and synchronously** (not through GPU_UPDATE_QUEUE). This guarantees new NPC data is in the GPU buffers before the next `dispatch()` call. Bevy systems use GPU_UPDATE_QUEUE for updates to existing NPCs, which is drained at the start of the next frame's `process()`.

## Timing

Bevy's Update schedule and Godot's `process()` both run once per frame. The `#[bevy_app]` macro from godot-bevy manages the Bevy tick. The exact ordering between process() and Bevy Update within a frame depends on godot-bevy internals, but the Mutex queues make the ordering safe — data flows through queues with one-frame latency at most.

## Known Issues

- **One-frame latency**: Bevy systems read GPU_READ_STATE from the *previous* frame's dispatch. Combat targeting uses positions that are one frame old.
- **No generational indices on GPU side**: NPC slot indices are raw `usize`. Currently safe because chained Combat systems prevent stale references within a frame. See [combat.md](combat.md) for analysis.

## Rating: 8/10

The frame loop is well-structured with clear separation between GPU compute, Bevy logic, and rendering. The one-frame latency is standard for CPU/GPU architectures. The Mutex-based communication is simple and correct, though it won't scale to multi-threaded Bevy (not needed today).
