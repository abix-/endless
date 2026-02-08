# Frame Loop

## Overview

Pure Bevy application. `App::run()` drives ECS game logic in the main world and GPU compute + rendering in the parallel render world. The two worlds synchronize once per frame at the extract barrier.

## Per-Frame Execution Order

```
MAIN WORLD — Bevy Update Schedule
│
├─ bevy_timer_start
│
├─ Step::Drain
│     reset_bevy_system, drain_arrival_queue, drain_game_config,
│     sync_gpu_state_to_bevy (GPU_READ_STATE → GpuReadState resource)
│
├─ gpu_position_readback (after Drain, before Spawn)
│     GpuReadState → ECS Position components
│
├─ Step::Spawn
│     spawn_npc_system, apply_targets_system
│
├─ ApplyDeferred (flush entity commands before combat)
│
├─ Step::Combat (chained)
│     cooldown_system → attack_system → damage_system →
│     death_system → death_cleanup_system
│
├─ Step::Behavior
│     arrival_system, energy_system, healing_system,
│     on_duty_tick_system, game_time_system, farm_growth_system,
│     camp_forage_system, raider_respawn_system, starvation_system,
│     decision_system
│
├─ collect_gpu_updates (after Step::Behavior)
│     GpuUpdateMsg events → GPU_UPDATE_QUEUE (single mutex lock)
│
├─ bevy_timer_end
│
├─ PostUpdate
│     populate_buffer_writes
│       GPU_UPDATE_QUEUE → NpcBufferWrites flat arrays
│
├─ update_gpu_data (sync npc_count + delta → NpcGpuData)
│
╞══════════════════════════════════════════════════════════════
│  EXTRACT BARRIER — clones resources to render world
│    NpcBufferWrites, NpcGpuData, NpcComputeParams, NpcSpriteTexture
│    extract_npc_batch (NpcBatch marker entity)
╞══════════════════════════════════════════════════════════════
│
RENDER WORLD — parallel with next frame's main world
│
├─ PrepareResources
│     write_npc_buffers          (only dirty fields → GPU storage buffers)
│     prepare_npc_buffers        (build instance buffer from GPU_READ_STATE positions)
│
├─ PrepareBindGroups
│     prepare_npc_bind_groups    (compute shader)
│     prepare_npc_texture_bind_group (sprite atlas)
│
├─ Queue
│     queue_npcs                 (NpcBatch → Transparent2d phase)
│
├─ Render Graph
│     NpcComputeNode → ProjectileComputeNode → CameraDriver → Transparent2d
│     NpcComputeNode: dispatch compute + copy positions → staging buffer
│
├─ Cleanup
│     readback_npc_positions     (map staging → GPU_READ_STATE static)
│
└─ Present frame
```

## Plugins

| Plugin | File | Responsibility |
|--------|------|----------------|
| `GpuComputePlugin` | `gpu.rs` | GPU buffer creation, compute pipeline, NpcComputeNode, readback |
| `RenderPlugin` | `render.rs` | Camera, sprite sheet loading, NpcSpriteTexture |
| `NpcRenderPlugin` | `npc_render.rs` | RenderCommand for Transparent2d, instanced draw call |

## Communication Flow

```
ECS → GPU:
  GpuUpdateMsg → collect_gpu_updates → GPU_UPDATE_QUEUE
    → populate_buffer_writes → NpcBufferWrites (per-field dirty flags)
    → ExtractResource → write_npc_buffers (only dirty fields)
    → NpcComputeNode: dispatch + copy positions to staging

GPU → ECS:
  readback_npc_positions: map staging → GPU_READ_STATE
    → sync_gpu_state_to_bevy → GpuReadState resource
    → gpu_position_readback → ECS Position components

GPU → Render:
  prepare_npc_buffers: reads GPU_READ_STATE positions + NpcBufferWrites sprites/colors
    → DrawNpcCommands: instanced draw
```

| Direction | Mechanism | Examples |
|-----------|-----------|---------|
| Systems → GPU | MessageWriter\<GpuUpdateMsg\> → collect → populate → extract → upload | SetPosition, SetTarget, SetColor, SetSpriteFrame |
| GPU → ECS | staging buffer → GPU_READ_STATE → GpuReadState → Position components | Positions after compute |
| Static queues → Bevy | Mutex queues drained in Step::Drain | ARRIVAL_QUEUE, GAME_CONFIG_STAGING |

Systems use MessageWriter for GPU updates so they can run in parallel. The single `collect_gpu_updates` call at frame end does one mutex lock to batch everything.

## Slot Allocation

`SlotAllocator` manages NPC indices with a free list for reuse. Slots are allocated in `spawn_npc_system` and recycled in `death_cleanup_system`. `NpcCount` tracks active NPCs. `NpcGpuData.npc_count` (extracted to render world) controls compute dispatch size and instance count.

## Timing

Pipelined rendering: the render world processes frame N while the main world computes frame N+1. The extract barrier is the sync point.

- **One-frame render latency**: GPU renders positions from the previous main world frame.
- **Spawn visibility**: SpawnNpcMsg → spawn_npc_system → GpuUpdateMsg → collect → populate → extract → GPU. Visible on the next rendered frame.

## Known Issues

- **No generational GPU indices**: NPC slot indices are raw `usize`. Safe because chained combat systems prevent stale references within a frame. See [combat.md](combat.md).
- **Hardcoded camera in render shader**: `npc_render.wgsl` uses constant `CAMERA_POS` and `VIEWPORT` instead of Bevy view uniforms. Camera movement/zoom won't affect NPC rendering.
- **Compute shader incomplete**: `npc_compute.wgsl` implements basic goal movement only. Separation physics, grid neighbor search, and combat targeting are not yet ported.
- **One-frame readback latency**: GPU positions are read back in Cleanup phase and consumed next frame in Step::Drain. Acceptable for gameplay.
