# GPU-Native NPC Rendering + Readback Optimization

## Problem

`prepare_npc_buffers` (npc_render.rs:411) rebuilds all 7 layer instance buffers from scratch every frame — iterating every NPC on CPU, packing `InstanceData` structs, uploading to GPU. Positions and health **already live on the GPU** in `NpcGpuBuffers` storage buffers from the compute shader. The current pipeline does GPU→CPU readback→CPU rebuild→GPU upload — a pointless round-trip for rendering.

### Current flow (per frame)
```
GPU compute positions[] → Readback → CPU GpuReadState → prepare_npc_buffers loop → GPU vertex instance buffer
```

### Target flow
```
GPU compute positions[] ──→ vertex shader reads directly (bind group 2)
CPU only uploads: sprite/color/equipment storage buffers (dirty writes only)
```

## Part 1: Storage Buffer Rendering

### 1a. New `NpcVisualBuffers` resource (npc_render.rs)

Create once in render world. Two flat per-slot storage buffers uploaded from CPU with dirty writes:

| Buffer | Layout | Size @ 10K | Upload strategy |
|--------|--------|-----------|----------------|
| `npc_visual` | `[f32; 8]` per slot: `[sprite_col, sprite_row, body_atlas, flash, color_r, color_g, color_b, color_a]` | 320KB | Dirty-write per slot |
| `equip_data` | `[f32; 4]` per slot × 6 layers: `[col, row, atlas, _pad]` = `[f32; 24]` per slot | 960KB | Dirty-write per slot |

Already on GPU from compute shader (`NpcGpuBuffers`):
- `positions` — `array<vec2<f32>>` (compute shader output)
- `healths` — `array<f32>` (compute shader output)

```rust
#[derive(Resource)]
pub struct NpcVisualBuffers {
    pub visual: Buffer,      // [f32; 8] per slot
    pub equip: Buffer,       // [f32; 24] per slot (6 layers × 4 floats)
    pub npc_bind_group: Option<BindGroup>,  // bind group 2
}
```

Create with `BufferUsages::STORAGE | BufferUsages::COPY_DST`, sized to `MAX_NPCS`.

### 1b. New vertex shader architecture (npc_render.wgsl)

**Current:** Vertex inputs at slot 1 = packed `InstanceData` (52 bytes per visible NPC).
**New:** Vertex shader reads from storage buffers indexed by `instance_index` = NPC slot.

Add to `npc_render.wgsl`:
```wgsl
// Bind group 2: NPC storage buffers (compute output + CPU visual data)
@group(2) @binding(0) var<storage, read> npc_positions: array<vec2<f32>>;
@group(2) @binding(1) var<storage, read> npc_healths: array<f32>;
@group(2) @binding(2) var<storage, read> npc_visual: array<NpcVisual>;
@group(2) @binding(3) var<storage, read> npc_equip: array<EquipSlot>;

struct NpcVisual {
    sprite_col: f32, sprite_row: f32, atlas_id: f32, flash: f32,
    r: f32, g: f32, b: f32, a: f32,
}

struct EquipSlot {
    col: f32, row: f32, atlas: f32, _pad: f32,
}
```

New vertex entry point `vertex_npc` (keep existing `vertex` for farms/projectiles):
- `instance_index` = NPC slot index (0..npc_count)
- Read position from `npc_positions[instance_index]`
- **Skip hidden:** `if pos.x < -9000.0` → move vertex off-screen (set clip_position far away)
- Read health from `npc_healths[instance_index]`
- Read sprite/color from `npc_visual[instance_index]`
- Scale is always 16.0 for NPCs, rotation always 0.0

**Draw call:** `draw_indexed(0..6, 0, 0..npc_count)` — GPU iterates all slots, vertex shader skips hidden.

### 1c. Equipment layers

Each equipment layer = separate draw call (same as today — 7 `draw_indexed` calls in `DrawNpcs`). Use push constant or uniform for `layer_index` per draw call.

The shader indexes: `npc_equip[instance_index * 6 + layer_index]`. If `col < 0.0`, move vertex off-screen (discard equivalent).

Simplest approach: one draw call per layer, pass `layer_index` as a push constant. Body layer (index 0) reads from `npc_visual`. Equipment layers (1-6) read from `npc_equip`.

Color logic per equipment layer (same as current `prepare_npc_buffers`):
- `atlas >= 2.5` (sleep icon): scale=16, color=white
- `atlas >= 1.5` (heal halo): scale=20, color=yellow `[1.0, 0.9, 0.2, 1.0]`
- `atlas >= 0.5` (carried item/world atlas): scale=16, color=white
- `atlas < 0.5` (character atlas equipment): scale=16, color=NPC job color from `npc_visual`

### 1d. Farm sprites + building HP bars

These aren't NPCs — no compute buffer slots. Keep a small CPU-built `RawBufferVec<InstanceData>` for these (~100 entries). Separate draw call **before** NPC draws. Reuse existing instance buffer vertex layout (the current `vertex` entry point).

### 1e. CPU-side `prepare_npc_buffers` rewrite

The function shrinks to:
1. Upload dirty NPC visual data to `npc_visual` buffer (per-slot `queue.write_buffer` at offset, same pattern as `gpu.rs:1096`)
2. Upload dirty equipment data to `equip_data` buffer
3. Upload flash values that changed (flash decay still runs on CPU in `populate_buffer_writes`)
4. Build small farm + building HP instance buffer (~100 entries, same code as current)
5. Create bind group 2 pointing to `NpcGpuBuffers.positions`, `.healths`, + new visual/equip buffers

**Dirty tracking:** `NpcBufferWrites` already has per-frame data. On extract, compare to previous frame's data and only `write_buffer` at changed slot offsets. Or for v1, just upload all visual data each frame (still much cheaper than rebuilding instance buffers — it's a flat memcpy vs. per-NPC branching + push).

### 1f. Pipeline changes

Two separate pipelines:

1. **`NpcStoragePipeline`** — NPCs via storage buffers. No slot 1 instance vertex input. Bind group layout 2 = 4 storage buffer bindings. Uses `vertex_npc` entry point. Push constant for `layer_index`.
2. **Existing pipeline** — farms/buildings/projectiles keep current instance vertex input (`vertex` entry point).

Add bind group layout 2 to `NpcPipeline`:
```rust
let npc_storage_bind_group_layout = BindGroupLayoutDescriptor::new(
    "npc_storage_bind_group_layout",
    &BindGroupLayoutEntries::sequential(
        ShaderStages::VERTEX,
        (
            storage_buffer_read_only::<Vec2>(false),   // positions
            storage_buffer_read_only::<f32>(false),    // healths
            storage_buffer_read_only::<[f32; 8]>(false), // visual
            storage_buffer_read_only::<[f32; 4]>(false), // equip
        ),
    ),
);
```

Need separate `DrawNpcsStorage` render command that sets bind group 2 and issues `draw_indexed(0..6, 0, 0..npc_count)` per layer (7 calls). No slot 1 vertex buffer needed.

### 1g. Accessing `NpcGpuBuffers` from render pipeline

`NpcGpuBuffers` is created in `gpu.rs` and lives in the render world. The `positions` and `healths` buffers already exist there. `NpcVisualBuffers` needs references to these buffers for bind group creation.

In `prepare_npc_buffers`, access `NpcGpuBuffers` as a resource:
```rust
gpu_buffers: Option<Res<NpcGpuBuffers>>,
```
Then create bind group 2 from `gpu_buffers.positions`, `gpu_buffers.healths`, plus `visual_buffers.visual`, `visual_buffers.equip`.

## Part 2: Throttle Readback

### 2a. Factions — read back every 60 frames (~1s)

Factions only change on spawn/death. No gameplay system needs frame-accurate faction data.

In the render graph node (`NpcComputeNode::run`), gate the factions `copy_buffer_to_buffer`:
```rust
if frame_count % 60 == 0 {
    // copy factions to readback buffer
}
```

Add `frame_count: u32` field to `NpcComputeNode`, increment each `run()`.

### 2b. Threat counts — read back every 30 frames

`threat_counts` consumed in `ai_decision_system` at `CHECK_INTERVAL = 30`. Match readback to consumption.

### 2c. Size readback to `npc_count`

`copy_buffer_to_buffer` calls already use `npc_count` for size. But `Readback::buffer` maps the full `MAX_NPCS` buffer. Change to `Readback::buffer_range()` sized to actual count. Respawn readback entities when `npc_count` crosses a 1024-slot boundary (avoid respawning every frame).

### 2d. Reuse readback Vec allocations

`to_shader_type()` creates a new `Vec` each callback. Pre-allocate `GpuReadState` vecs to `MAX_NPCS` and `copy_from_slice` instead of replacing.

In `resources.rs`, change `GpuReadState` fields from `Vec<f32>` to pre-allocated `Vec<f32>` with capacity `MAX_NPCS * stride`. Update callbacks in `gpu.rs` to write into existing allocation.

## Files to Modify

| File | Changes |
|------|---------|
| `rust/src/npc_render.rs` | New `NpcVisualBuffers` resource, new `NpcStoragePipeline` with bind group 2, `DrawNpcsStorage` render command, rewrite `prepare_npc_buffers` to dirty-write visual/equip data, separate farm/BHP draw call via existing instance path |
| `rust/assets/shaders/npc_render.wgsl` | Add storage buffer bindings (group 2), new `vertex_npc` entry point reading from storage buffers, keep existing `vertex` for farm/proj path |
| `rust/src/gpu.rs` | Throttle factions/threat_counts readback (frame counter gating), expose `NpcGpuBuffers` for render pipeline bind group, `buffer_range` sizing |
| `rust/src/resources.rs` | Pre-allocate `GpuReadState` vecs, add `generation` counter |

## Execution Order

1. **1a-1b:** Create visual storage buffers + new shader `vertex_npc` entry point. Get NPC body layer rendering from storage buffers.
2. **1c:** Equipment layers via storage buffers + push constant `layer_index`.
3. **1d:** Separate farm/BHP draw call using existing instance path.
4. **1e-1f:** Pipeline plumbing, `DrawNpcsStorage` render command, remove old instance buffer path for NPCs.
5. **Part 2:** Readback throttling (independent, can be done in parallel with Part 1).

## Verification

1. `cargo check` — compiles
2. `cargo run --release` — visual correctness:
   - NPCs render at correct positions and move smoothly
   - Health bars display correctly
   - Damage flash works
   - Equipment/overlay layers render (armor, helmet, weapon, heal halo, sleep icon)
   - Farm sprites show growth + golden-when-ready
   - Building HP bars appear when damaged
   - Click-to-select still works (uses CPU readback positions, unaffected)
   - Camera follow works
3. Performance: time `prepare_npc_buffers` before/after. Expect ~1-3ms → <0.5ms.
4. Throttled readback: verify factions/threat_counts still work at reduced frequency. Test: spawn new NPCs, verify faction shows up within ~1s.
