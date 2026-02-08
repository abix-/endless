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
│                                      │   └─ Create GPU buffers, queue compute pipeline
│                                      │
│                                      ├─ write_npc_buffers (PrepareResources)
│                                      │   └─ Upload positions/targets/speeds/factions/healths
│                                      │
│                                      ├─ prepare_npc_bind_groups (PrepareBindGroups)
│                                      │   └─ Bind buffers + uniform params for compute
│                                      │
│                                      └─ NpcComputeNode (render graph)
│                                          └─ Dispatch compute shader
```

## Data Flow

```
Bevy systems emit GpuUpdateMsg (SetPosition, SetTarget, etc.)
    │
    ▼
collect_gpu_updates → GPU_UPDATE_QUEUE
    │
    ▼
populate_buffer_writes → NpcBufferWrites (flat f32/i32 arrays)
    │
    ▼ ExtractResource clone
    │
    ├──▶ write_npc_buffers → GPU storage buffers (compute shader input)
    │         uploads: positions, targets, speeds, factions, healths
    │
    └──▶ prepare_npc_buffers (npc_render.rs) → instance vertex buffer (render input)
              reads: positions, sprite_indices, colors from NpcBufferWrites directly
```

Note: `sprite_indices` and `colors` are in NpcBufferWrites but are not uploaded to GPU storage buffers. They're only consumed by the render pipeline's instance buffer, not the compute shader.

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

Created once in `init_npc_compute_pipeline`. All storage buffers are `read_write`.

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

Built per frame in `prepare_npc_buffers` from NpcBufferWrites (not from GPU storage buffers).

| Field | Type | Size | Source |
|-------|------|------|--------|
| position | [f32; 2] | 8B | NpcBufferWrites.positions |
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
- **No GPU→CPU readback**: Compute updates `positions` and `arrivals` on GPU, but results aren't read back to ECS. `GpuReadState` resource exists but isn't populated.
- **Single dispatch**: Should be 3 dispatches per frame (clear grid → insert NPCs → main logic) with uniform buffer mode updates between them. Currently dispatches once.
- **Render reads CPU data, not GPU output**: `npc_render.rs` builds instance data from `NpcBufferWrites` (CPU-side), not from the GPU positions buffer. GPU compute updates positions but rendering doesn't see those updates — it shows the position from before compute ran.
- **Hardcoded camera**: `npc_render.wgsl` has constant `CAMERA_POS` and `VIEWPORT`. Camera movement/zoom won't affect NPC rendering.
- **Health is CPU-authoritative**: GPU reads health for targeting but never modifies it.
- **sprite_indices/colors not uploaded to compute**: These fields exist in NpcBufferWrites for the render pipeline only. The compute shader has no access to them.

## Rating: 5/10

Pipeline compiles and dispatches each frame. Basic NPC movement works on GPU. Buffers are allocated for the full system (grid, separation, combat) but most are unused. The critical missing pieces are: multi-mode dispatch for spatial grid, separation physics, combat targeting, GPU→CPU readback, and render pipeline reading GPU output instead of CPU copies.
