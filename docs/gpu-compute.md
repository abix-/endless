# GPU Compute

## Overview

GPU compute uses Bevy's render graph with wgpu/WGSL. The compute shader `shaders/npc_compute.wgsl` runs NPC movement physics on the GPU. NPC rendering uses a separate instanced draw pipeline in `npc_render.rs` via Bevy's RenderCommand pattern. See [frame-loop.md](frame-loop.md) for where these fit in the frame.

## Architecture

```
Main World (ECS)                       Render World (GPU)
│                                      │
├─ NpcGpuData ───────────────────────▶ ExtractResource
├─ NpcComputeParams ─────────────────▶ (cloned each frame)
├─ NpcBufferWrites ──────────────────▶
├─ NpcSpriteTexture ─────────────────▶
│                                      │
│                                      ├─ init_npc_compute_pipeline (RenderStartup)
│                                      │   └─ Create GPU buffers + staging buffer
│                                      │
│                                      ├─ write_npc_buffers (PrepareResources)
│                                      │   └─ Upload only dirty fields (per-field flags)
│                                      │
│                                      ├─ prepare_npc_bind_groups (PrepareBindGroups)
│                                      │   └─ Bind buffers + uniform params for compute
│                                      │
│                                      ├─ NpcComputeNode (render graph)
│                                      │   ├─ Dispatch compute shader
│                                      │   └─ copy_buffer_to_buffer(positions → staging)
│                                      │
│                                      └─ readback_npc_positions (Cleanup)
│                                          └─ map staging → GPU_READ_STATE
│
├─ sync_gpu_state_to_bevy (Step::Drain)
│     GPU_READ_STATE → GpuReadState resource
│
└─ gpu_position_readback (after Drain)
      GpuReadState → ECS Position components
```

## Data Flow

```
ECS → GPU (upload):
  GpuUpdateMsg → collect_gpu_updates → GPU_UPDATE_QUEUE
    → populate_buffer_writes → NpcBufferWrites (per-field dirty flags)
    → ExtractResource clone
    → write_npc_buffers (only uploads dirty fields)

GPU → ECS (readback):
  NpcComputeNode: dispatch compute + copy positions → staging buffer
    → readback_npc_positions: map staging, write to GPU_READ_STATE
    → sync_gpu_state_to_bevy: GPU_READ_STATE → GpuReadState resource
    → gpu_position_readback: GpuReadState → ECS Position components

GPU → Render:
  prepare_npc_buffers: reads GPU_READ_STATE for positions (falls back to
    NpcBufferWrites on first frame), reads sprite_indices/colors from NpcBufferWrites
```

Note: `sprite_indices` and `colors` are in NpcBufferWrites but are not uploaded to GPU storage buffers. They're only consumed by the render pipeline's instance buffer, not the compute shader. Positions for rendering come from GPU readback, not NpcBufferWrites.

## NPC Compute Shader (npc_compute.wgsl)

Workgroup size: 64 threads. Dispatched as `ceil(npc_count / 64)` workgroups. Single dispatch per frame.

The shader reads position, goal, speed, and arrival state per NPC. Movement is straight-line toward the goal at the NPC's speed, scaled by delta time. NPCs with `arrivals[i] == 1` are settled and don't move. Hidden NPCs (position.x < -9000) are skipped.

```
per NPC thread:
  1. Read pos, goal, speed, settled
  2. Skip if hidden (pos.x < -9000)
  3. If not settled and far from goal: move toward goal
  4. If not settled and close to goal: mark settled
  5. Write pos, arrivals
  6. Write combat_targets = -1 (placeholder)
```

## GPU Buffers

### Compute Buffers (gpu.rs NpcGpuBuffers)

Created once in `init_npc_compute_pipeline`. All storage buffers are `read_write`. A staging buffer (`position_staging`) is created with `MAP_READ | COPY_DST` for GPU→CPU readback.

| Binding | Name | Type | Per-NPC Size | Uploaded From | Purpose |
|---------|------|------|-------------|---------------|---------|
| 0 | positions | vec2\<f32\> | 8B | NpcBufferWrites.positions | Current XY, read/written by shader |
| 1 | goals | vec2\<f32\> | 8B | NpcBufferWrites.targets | Movement target |
| 2 | speeds | f32 | 4B | NpcBufferWrites.speeds | Movement speed |
| 3 | grid_counts | i32[] | — | Not uploaded | NPCs per grid cell (allocated, unused) |
| 4 | grid_data | i32[] | — | Not uploaded | NPC indices per cell (allocated, unused) |
| 5 | arrivals | i32 | 4B | Not uploaded | Settled flag (0=moving, 1=arrived) |
| 6 | backoff | i32 | 4B | Not uploaded | Collision backoff counter (allocated, unused) |
| 7 | factions | i32 | 4B | NpcBufferWrites.factions | 0=Villager, 1+=Raider camps |
| 8 | healths | f32 | 4B | NpcBufferWrites.healths | Current HP |
| 9 | combat_targets | i32 | 4B | Not uploaded | Nearest enemy index or -1 (written by shader) |
| 10 | params | Params (uniform) | — | NpcComputeParams | Count, delta, grid config, thresholds |

### Render Instance Data (npc_render.rs)

Built per frame in `prepare_npc_buffers`. Positions come from GPU readback; sprites/colors from NpcBufferWrites.

| Field | Type | Size | Source |
|-------|------|------|--------|
| position | [f32; 2] | 8B | GPU_READ_STATE (readback), fallback NpcBufferWrites |
| sprite | [f32; 2] | 8B | NpcBufferWrites.sprite_indices |
| color | [f32; 4] | 16B | NpcBufferWrites.colors |
| **Total** | | **32B/NPC** | |

## Uniform Params (NpcComputeParams)

| Field | Default | Purpose |
|-------|---------|---------|
| count | 0 | Active NPC count (set from NpcCount each frame) |
| separation_radius | 20.0 | Separation physics radius (unused by current shader) |
| separation_strength | 100.0 | Separation force strength (unused by current shader) |
| delta | 0.016 | Frame delta time |
| grid_width | 128 | Spatial grid columns (unused by current shader) |
| grid_height | 128 | Spatial grid rows (unused by current shader) |
| cell_size | 64.0 | Pixels per grid cell (unused by current shader) |
| max_per_cell | 48 | Max NPCs per cell (unused by current shader) |
| arrival_threshold | 8.0 | Distance to mark as arrived |
| mode | 0 | Dispatch mode (unused — single dispatch only) |

## Spatial Grid

Buffers are allocated but not used by the current shader. Intended layout:

- **Cell size**: 64px
- **Grid dimensions**: 128x128 (covers 8192x8192 world)
- **Max per cell**: 48
- **Total cells**: 16,384
- **Memory**: grid_counts = 64KB, grid_data = 3MB

When ported, the grid will be built on GPU each frame via atomic operations (clear pass → insert pass → main logic pass). NPCs are binned by `floor(pos / cell_size)`. The 3x3 neighborhood search will cover separation physics and combat targeting.

## NPC Rendering

Separate from compute. Uses `npc_render.rs` with Bevy's RenderCommand pattern hooked into the Transparent2d phase. Renders all NPCs in a single instanced draw call: one static quad (4 vertices, 6 indices) drawn `instance_count` times with per-instance position, sprite atlas cell, and color tint.

The render shader (`shaders/npc_render.wgsl`) expands each quad by `SPRITE_SIZE` (32px), applies an orthographic projection, and samples the sprite atlas. Fragment shader is currently in debug mode (solid colors, texture sampling commented out).

## Constants

```rust
const WORKGROUP_SIZE: u32 = 64;
const MAX_NPCS: u32 = 16384;
const GRID_WIDTH: u32 = 128;
const GRID_HEIGHT: u32 = 128;
const MAX_PER_CELL: u32 = 48;
```

## Known Issues

- **Compute shader simplified**: Only basic goal movement. Separation physics, grid-based neighbor search, and combat targeting are not ported from the old GLSL shader.
- **Single dispatch**: Should be 3 dispatches per frame (clear grid → insert NPCs → main logic) with uniform buffer mode updates between them. Currently dispatches once.
- **Hardcoded camera**: `npc_render.wgsl` has constant `CAMERA_POS` and `VIEWPORT`. Camera movement/zoom won't affect NPC rendering.
- **Health is CPU-authoritative**: GPU reads health for targeting but never modifies it.
- **sprite_indices/colors not uploaded to compute**: These fields exist in NpcBufferWrites for the render pipeline only. The compute shader has no access to them.
- **Synchronous readback blocks render thread**: `device.poll(Wait)` blocks until staging buffer mapping completes. For 128KB this is sub-millisecond, but could be upgraded to async double-buffered readback if needed.

## Rating: 6/10

Pipeline compiles and dispatches each frame. NPC movement works on GPU with full readback to ECS — positions flow GPU→staging→CPU→ECS and rendering shows GPU-computed positions. Per-field dirty flags prevent stale data from overwriting GPU output. Remaining: multi-mode dispatch for spatial grid, separation physics, combat targeting.
