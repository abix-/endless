# Frame Loop

## Overview

Pure Bevy application. `App::run()` drives ECS game logic in the main world and GPU compute + rendering in the parallel render world. The two worlds synchronize once per frame at the extract barrier.

## Per-Frame Execution Order

```
MAIN WORLD — Bevy Update Schedule (game systems gated on AppState::Running)
│
├─ bevy_timer_start
│
├─ Step::Drain
│     reset_bevy_system, drain_game_config
│
├─ gpu_position_readback (after Drain, before Spawn)
│     GpuReadState → ECS Position components
│
├─ Step::Spawn
│     spawn_npc_system
│
├─ ApplyDeferred (flush entity commands before combat)
│
├─ Step::Combat (chained)
│     cooldown_system → attack_system → damage_system →
│     death_system → xp_grant_system → death_cleanup_system →
│     waypoint_attack_system
│
├─ Step::Behavior
│     rebuild_building_grid_system (before decision_system, spawner_respawn_system),
│     arrival_system, energy_system, healing_system,
│     on_duty_tick_system, game_time_system, farm_growth_system,
│     camp_forage_system, raider_respawn_system, starvation_system,
│     decision_system, farm_visual_system, process_upgrades_system
│
├─ collect_gpu_updates (after Step::Behavior)
│     GpuUpdateMsg events → GPU_UPDATE_QUEUE (single mutex lock)
│
├─ bevy_timer_end
│
├─ PostUpdate (chained)
│     populate_gpu_state
│       GPU_UPDATE_QUEUE → NpcGpuState (per-field dirty indices + flash decay)
│     build_visual_upload
│       ECS query + NpcGpuState → NpcVisualUpload (GPU-ready packed visual + equip)
│
├─ update_gpu_data (sync npc_count + delta → NpcGpuData; delta=0 when paused)
│
╞══════════════════════════════════════════════════════════════
│  EXTRACT BARRIER — zero-clone reads + clones to render world
│    NpcGpuState (Extract<Res<T>>, zero-clone)
│    NpcVisualUpload (Extract<Res<T>>, zero-clone)
│    ProjBufferWrites, ProjPositionState (Extract<Res<T>>, zero-clone)
│    NpcGpuData, NpcComputeParams, NpcSpriteTexture (ExtractResource clone)
│    extract_npc_batch, extract_proj_batch (marker entities)
│    extract_npc_data (per-dirty-index + bulk write_buffer to GPU)
│    extract_proj_data (per-dirty-index write_buffer + proj instance buffer build)
╞══════════════════════════════════════════════════════════════
│
RENDER WORLD — parallel with next frame's main world
│
├─ PrepareResources
│     prepare_npc_buffers        (buffer creation + sentinel init, build misc instance buffer)
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
│     NpcComputeNode: dispatch compute + copy positions → ReadbackHandles assets
│     ProjectileComputeNode: dispatch + copy hits/positions → ReadbackHandles assets
│
├─ Bevy Readback (async, managed by Bevy runtime)
│     ReadbackComplete observers fire → write to GpuReadState, ProjHitState, ProjPositionState
│
└─ Present frame
```

## Plugins

| Plugin | File | Responsibility |
|--------|------|----------------|
| `GpuComputePlugin` | `gpu.rs` | GPU buffer creation, compute pipeline, NpcComputeNode, readback |
| `RenderPlugin` | `render.rs` | Camera, sprite sheet loading, NpcSpriteTexture |
| `NpcRenderPlugin` | `npc_render.rs` | RenderCommand for Transparent2d, dual-path: storage buffers (NPCs) + instance buffers (misc/projectiles) |

## Communication Flow

```
ECS → GPU:
  GpuUpdateMsg → collect_gpu_updates → GPU_UPDATE_QUEUE
    → populate_gpu_state → NpcGpuState (per-field dirty indices)
    → build_visual_upload → NpcVisualUpload (GPU-ready packed arrays)
    → extract_npc_data (Extract<Res<T>>, zero-clone) → write_buffer to GPU
    → NpcComputeNode: dispatch + copy positions → ReadbackHandles assets

GPU → ECS:
  Bevy Readback async-reads ShaderStorageBuffer assets
    → ReadbackComplete observers → GpuReadState, ProjHitState, ProjPositionState
    → gpu_position_readback → ECS Position components

GPU → Render:
  Vertex shader reads positions/health directly from NpcGpuBuffers (bind group 2)
  NpcVisualBuffers (visual + equip) written by extract_npc_data during Extract
    → DrawNpcStorageCommands (NPCs) + DrawMiscCommands (farms/BHP)
```

| Direction | Mechanism | Examples |
|-----------|-----------|---------|
| Systems → GPU | MessageWriter\<GpuUpdateMsg\> → collect → populate → extract → upload | SetPosition, SetTarget, SetSpriteFrame |
| GPU → ECS | Bevy Readback → ReadbackComplete → GpuReadState → Position components | Positions after compute |
| Static queues → Bevy | Mutex queues drained in Step::Drain | GAME_CONFIG_STAGING |

Systems use MessageWriter for GPU updates so they can run in parallel. The single `collect_gpu_updates` call at frame end does one mutex lock to batch everything.

## Slot Allocation

`SlotAllocator` manages NPC indices with a free list for reuse. Slots are allocated in `spawn_npc_system` and recycled in `death_cleanup_system`. `NpcCount` tracks active NPCs. `NpcGpuData.npc_count` (extracted to render world) controls compute dispatch size and instance count.

## Timing

Pipelined rendering: the render world processes frame N while the main world computes frame N+1. The extract barrier is the sync point.

- **One-frame render latency**: GPU renders positions from the previous main world frame.
- **Spawn visibility**: SpawnNpcMsg → spawn_npc_system → GpuUpdateMsg → collect → populate → extract → GPU. Visible on the next rendered frame.

## App States

The app uses `AppState` (TestMenu | Running) to gate system execution:
- **TestMenu**: Only test framework UI systems run (bevy_egui menu in `EguiPrimaryContextPass`)
- **Running**: All game systems execute (Drain → Spawn → Combat → Behavior), plus test HUD overlay and per-test tick systems

State transitions: `NextState<AppState>` set by menu clicks or test completion. `OnEnter(Running)` triggers test setup. `OnExit(Running)` triggers world cleanup (despawn NPCs, reset resources).

## Known Issues

- **No generational GPU indices**: NPC slot indices are raw `usize`. Safe because chained combat systems prevent stale references within a frame. See [combat.md](combat.md).
- **One-frame readback latency**: GPU positions are read back asynchronously by Bevy's Readback system. Data arrives ~1 frame later. Acceptable for gameplay.
