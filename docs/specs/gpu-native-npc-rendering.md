# GPU-Native NPC Rendering + Readback Optimization

## Problem

**Solved (Part 1):** `prepare_npc_buffers` used to rebuild all 7 layer instance buffers from scratch every frame — iterating every NPC on CPU, packing `InstanceData` structs, uploading to GPU. Now the vertex shader reads positions/health directly from compute shader's storage buffers. CPU only uploads flat visual/equipment arrays via two `write_buffer` calls.

**Remaining (Part 2):** GPU→CPU readback of factions, threat_counts, positions (for gameplay systems) still runs every frame at full buffer size. Can be throttled and range-sized.

### Flow (current)
```
GPU compute positions[] ──→ vertex shader reads directly (bind group 2)
CPU uploads: visual[f32;8] + equip[f32;24] per NPC slot (two write_buffer calls)
GPU→CPU readback: positions, health, factions, threat_counts (still every frame)
```

## Part 1: Storage Buffer Rendering ✅

### 1a. `NpcVisualBuffers` resource (npc_render.rs)

Created once in render world. Two flat per-slot storage buffers, full-buffer uploaded each frame (V1):

| Buffer | Layout | Size @ 30K | Upload strategy |
|--------|--------|-----------|----------------|
| `visual` | `[f32; 8]` per slot: `[sprite_col, sprite_row, body_atlas, flash, r, g, b, a]` | 960KB | Full `write_buffer` per frame |
| `equip` | `[f32; 4]` per slot × 6 layers: `[col, row, atlas, _pad]` = `[f32; 24]` per slot | 2.88MB | Full `write_buffer` per frame |

Already on GPU from compute shader (`NpcGpuBuffers`):
- `positions` — `array<vec2<f32>>` (compute shader output)
- `healths` — `array<f32>` (compute shader output)

```rust
#[derive(Resource)]
pub struct NpcVisualBuffers {
    pub visual: Buffer,              // [f32; 8] per slot
    pub equip: Buffer,               // [f32; 24] per slot (6 layers × 4 floats)
    pub bind_group: Option<BindGroup>,  // bind group 2
}
```

Created with `BufferUsages::STORAGE | BufferUsages::COPY_DST`, sized to `MAX_NPC_COUNT` (constants.rs).

### 1b. Vertex shader — instance offset encoding (npc_render.wgsl)

**Old:** Vertex inputs at slot 1 = packed `InstanceData` (52 bytes per visible NPC per layer).
**New:** `vertex_npc` entry point reads from storage buffers. Layer encoded via instance offset — no push constants (avoids `Features::PUSH_CONSTANTS` hardware requirement).

**How it works:** 7 draw calls, each with `npc_count` instances. Shader derives:
```wgsl
let slot = in.instance_index % camera.npc_count;
let layer = in.instance_index / camera.npc_count;
```

`npc_count` packed into `CameraUniform` padding slot (offset 12, between `zoom: f32` and `viewport: Vec2`):
```wgsl
struct Camera {
    pos: vec2<f32>,
    zoom: f32,
    npc_count: u32,    // was _pad
    viewport: vec2<f32>,
};
```

Storage buffer bindings (bind group 2):
```wgsl
struct NpcVisual {
    sprite_col: f32, sprite_row: f32, atlas_id: f32, flash: f32,
    r: f32, g: f32, b: f32, a: f32,
};
struct EquipSlot { col: f32, row: f32, atlas: f32, _pad: f32, };

@group(2) @binding(0) var<storage, read> npc_positions: array<vec2<f32>>;
@group(2) @binding(1) var<storage, read> npc_healths: array<f32>;
@group(2) @binding(2) var<storage, read> npc_visual_buf: array<NpcVisual>;
@group(2) @binding(3) var<storage, read> npc_equip: array<EquipSlot>;
```

**Hidden NPC culling:** `if pos.x < -9000.0` → set `clip_position = HIDDEN` (degenerate triangle, off-screen).

**Draw calls:** `DrawNpcsStorage` issues 7 calls:
```rust
for layer in 0..LAYER_COUNT as u32 {
    pass.draw_indexed(0..6, 0, (layer * npc_count)..((layer + 1) * npc_count));
}
```

### 1c. Equipment layers

Body layer (`layer == 0`): reads from `npc_visual_buf[slot]`.
Equipment layers (`layer 1-6`): reads from `npc_equip[slot * 6u + (layer - 1u)]`. If `eq.col < 0.0`, vertex hidden.

Color/scale logic per equipment atlas type (in shader):
- `atlas >= 2.5` (sleep icon): scale=16, color=white
- `atlas >= 1.5` (heal halo): scale=20, color=yellow `[1.0, 0.9, 0.2, 1.0]`
- `atlas >= 0.5` (carried item/world atlas): scale=16, color=white
- `atlas < 0.5` (character atlas equipment): scale=16, color=NPC job color from `npc_visual_buf`

### 1d. Farm sprites + building HP bars

Not NPCs — no compute buffer slots. Small CPU-built `NpcMiscBuffers` with `RawBufferVec<InstanceData>` (~100-200 entries). Separate `DrawMisc` command **before** NPC draws (sort_key 0.4 vs 0.5). Uses existing `vertex` entry point with instance vertex layout.

### 1e. CPU-side `prepare_npc_buffers`

V1 (current): full-buffer upload each frame. Builds flat `visual_data` and `equip_data` arrays in one pass over `npc_count`, two `write_buffer` calls. Still 2.8× less bandwidth than old instance buffer path (3.84MB vs 10.9MB at 30K NPCs) and 7× fewer CPU iterations (30K vs 210K).

V2 (future): dirty-write per changed slot. Compare to previous frame's data, only `write_buffer` at changed offsets.

Steps:
1. Build flat `visual_data` `[f32; npc_count * 8]` from `NpcBufferWrites` (sprite_indices, flash_values, colors)
2. Build flat `equip_data` `[f32; npc_count * 24]` from `EQUIP_LAYER_FIELDS` (6 equipment sprite sources)
3. `write_buffer` both
4. Build `NpcMiscBuffers` (farms + building HP bars)
5. Create bind group 2 from `NpcGpuBuffers.positions`, `.healths` + `NpcVisualBuffers.visual`, `.equip`

### 1f. One pipeline, two specializations

Single `NpcPipeline` with `storage_mode` bool in specialization key:

```rust
type Key = (bool, u32, bool); // (HDR, MSAA samples, storage_mode)
```

| `storage_mode` | Entry point | Bind group layouts | Vertex buffers | Used by |
|---|---|---|---|---|
| `true` | `vertex_npc` | 0 (textures) + 1 (camera) + 2 (NPC storage) | Slot 0 only (quad) | NPC body + equipment |
| `false` | `vertex` | 0 (textures) + 1 (camera) | Slot 0 (quad) + Slot 1 (InstanceData) | Farms, BHP, projectiles |

Bind group layout 2 (NPC storage):
```rust
let npc_data_bind_group_layout = BindGroupLayoutDescriptor::new(
    "npc_data_bind_group_layout",
    &BindGroupLayoutEntries::sequential(
        ShaderStages::VERTEX,
        (
            storage_buffer_read_only::<Vec<[f32; 2]>>(false),  // positions
            storage_buffer_read_only::<Vec<f32>>(false),       // healths
            storage_buffer_read_only::<Vec<[f32; 8]>>(false),  // visual
            storage_buffer_read_only::<Vec<[f32; 4]>>(false),  // equip
        ),
    ),
);
```

### 1g. DRY — shared helpers

**WGSL:** `calc_uv()`, `world_to_clip()`, `HIDDEN` constant shared by both `vertex` and `vertex_npc`.
**Rust:** `quad_vertex_layout()` and `instance_vertex_layout()` extracted as helpers, called by `specialize()`.

### 1h. Accessing `NpcGpuBuffers` from render pipeline

`NpcGpuBuffers` lives in the render world (created by gpu.rs). `prepare_npc_buffers` accesses it as `Option<Res<NpcGpuBuffers>>` and binds `.positions` + `.healths` into bind group 2.

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

## Files Modified (Part 1) ✅

| File | Changes |
|------|---------|
| `rust/src/npc_render.rs` | `NpcVisualBuffers` + `NpcMiscBuffers` resources, `NpcPipeline` with `npc_data_bind_group_layout` + `storage_mode` specialization key, `DrawNpcsStorage` + `DrawMisc` render commands, `prepare_npc_buffers` rewritten to flat array build + `write_buffer`, `NpcRenderBuffers` slimmed to quad geometry only |
| `rust/assets/shaders/npc_render.wgsl` | Storage buffer bindings (group 2), `vertex_npc` entry point with instance offset encoding, `NpcVisual`/`EquipSlot` structs, shared `calc_uv()`/`world_to_clip()`/`HIDDEN` helpers |

## Files to Modify (Part 2)

| File | Changes |
|------|---------|
| `rust/src/gpu.rs` | Throttle factions/threat_counts readback (frame counter gating), `buffer_range` sizing |
| `rust/src/resources.rs` | Pre-allocate `GpuReadState` vecs, `copy_from_slice` instead of per-frame `Vec` allocation |

## Verification

### Part 1 ✅
1. `cargo check` — compiles clean
2. `cargo run --release` — visual correctness verified:
   - NPCs render at correct positions and move smoothly
   - Health bars display correctly
   - Damage flash works
   - Equipment/overlay layers render (armor, helmet, weapon, heal halo, sleep icon)
   - Farm sprites show growth + golden-when-ready
   - Building HP bars appear when damaged
   - Click-to-select still works (uses CPU readback positions, unaffected)
   - Camera follow works

### Part 2 (future)
1. Throttled readback: verify factions/threat_counts still work at reduced frequency
2. Test: spawn new NPCs, verify faction shows up within ~1s
