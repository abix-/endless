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
│                                      │   └─ 3 bind groups (one per mode, different uniform)
│                                      │
│                                      ├─ NpcComputeNode (render graph, 3 passes)
│                                      │   ├─ Mode 0: clear grid (atomicStore 0)
│                                      │   ├─ Mode 1: build grid (atomicAdd NPC indices)
│                                      │   ├─ Mode 2: movement + combat targeting
│                                      │   └─ copy positions + combat_targets → staging
│                                      │
│                                      └─ readback_npc_positions (Cleanup)
│                                          └─ map staging (positions + combat_targets) → GPU_READ_STATE
│
├─ sync_gpu_state_to_bevy (Step::Drain)
│     GPU_READ_STATE → GpuReadState resource
│
└─ gpu_position_readback (after Drain)
      GpuReadState → ECS Position components
      + CPU-side arrival detection (position vs goal within ARRIVAL_THRESHOLD → AtDestination)
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
      + arrival detection: if HasTarget && dist(pos, goal) < ARRIVAL_THRESHOLD → AtDestination

GPU → Render:
  prepare_npc_buffers: reads GPU_READ_STATE for positions (falls back to
    NpcBufferWrites on first frame), reads sprite_indices/colors from NpcBufferWrites
```

Note: `sprite_indices` and `colors` are in NpcBufferWrites but are not uploaded to GPU storage buffers. They're only consumed by the render pipeline's instance buffer, not the compute shader. Positions for rendering come from GPU readback, not NpcBufferWrites.

## NPC Compute Shader (npc_compute.wgsl)

Workgroup size: 64 threads. 3 dispatches per frame with different `mode` uniform values. Each mode dispatches `ceil(count / 64)` workgroups (mode 0 uses `ceil(grid_cells / 64)`).

### Mode 0: Clear Grid
One thread per grid cell. Atomically clears `grid_counts[cell]` to 0. Early exit if `i >= grid_cells`.

### Mode 1: Build Grid
One thread per NPC. Computes cell from `floor(pos / cell_size)`, atomically increments `grid_counts[cell]`, writes NPC index into `grid_data[cell * max_per_cell + slot]`. Skips hidden NPCs (pos.x < -9000).

### Mode 2: Movement + Combat Targeting
One thread per NPC. Two phases per thread:

**Movement**: Straight-line toward goal at NPC speed × delta. NPCs with `arrivals[i] == 1` are settled and skip movement. When close enough (< `arrival_threshold`), marks `settled = 1`.

**Combat targeting**: Searches grid cells within `combat_range / cell_size + 1` radius around NPC's cell. For each NPC in neighboring cells, checks: different faction, alive (health > 0), not self. Tracks nearest enemy by squared distance. Writes best target index to `combat_targets[i]` (-1 if none found).

## GPU Buffers

### Compute Buffers (gpu.rs NpcGpuBuffers)

Created once in `init_npc_compute_pipeline`. All storage buffers are `read_write`. A staging buffer (`position_staging`) is created with `MAP_READ | COPY_DST` for GPU→CPU readback.

| Binding | Name | Type | Per-NPC Size | Uploaded From | Purpose |
|---------|------|------|-------------|---------------|---------|
| 0 | positions | vec2\<f32\> | 8B | NpcBufferWrites.positions | Current XY, read/written by shader |
| 1 | goals | vec2\<f32\> | 8B | NpcBufferWrites.targets | Movement target |
| 2 | speeds | f32 | 4B | NpcBufferWrites.speeds | Movement speed |
| 3 | grid_counts | atomic\<i32\>[] | — | Not uploaded | NPCs per grid cell (atomically written by mode 0+1) |
| 4 | grid_data | i32[] | — | Not uploaded | NPC indices per cell (written by mode 1) |
| 5 | arrivals | i32 | 4B | NpcBufferWrites.arrivals | Settled flag (0=moving, 1=arrived), reset on SetTarget |
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
| mode | 0 | Dispatch mode (0=clear grid, 1=build grid, 2=movement+targeting) |
| combat_range | 300.0 | Maximum distance for combat targeting |

## Spatial Grid

Built on GPU each frame via 3-mode dispatch with atomic operations:

- **Cell size**: 64px
- **Grid dimensions**: 128x128 (covers 8192x8192 world)
- **Max per cell**: 48
- **Total cells**: 16,384
- **Memory**: grid_counts = 64KB, grid_data = 3MB

NPCs are binned by `floor(pos / cell_size)`. Mode 0 clears all cell counts, mode 1 inserts NPCs via `atomicAdd`, mode 2 searches `combat_range / cell_size + 1` radius neighborhood for combat targeting.

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

- **No separation physics**: NPCs converge to same position when chasing the same target. Spatial grid is ready for separation forces but not yet implemented.
- **Hardcoded camera**: `npc_render.wgsl` has constant `CAMERA_POS` and `VIEWPORT`. Camera movement/zoom won't affect NPC rendering.
- **Health is CPU-authoritative**: GPU reads health for targeting but never modifies it.
- **sprite_indices/colors not uploaded to compute**: These fields exist in NpcBufferWrites for the render pipeline only. The compute shader has no access to them.
- **Synchronous readback blocks render thread**: `device.poll(Wait)` blocks until staging buffer mapping completes. For 128KB this is sub-millisecond, but could be upgraded to async double-buffered readback if needed.

## Rating: 8/10

3-mode compute dispatch with spatial grid, combat targeting, and full GPU→ECS readback (positions + combat_targets). Per-field dirty flags prevent stale data from overwriting GPU output. Arrival flag reset on SetTarget ensures NPCs resume movement. Remaining: separation physics to prevent NPC overlap.
