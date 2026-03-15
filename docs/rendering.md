# Rendering System

## Overview

**Terrain** uses Bevy's built-in `TilemapChunk` (single layer, `AlphaMode2d::Opaque`, z=-1). **Everything else** — buildings, NPCs, equipment, farms, building HP bars, projectiles — uses a custom GPU pipeline via Bevy's RenderCommand pattern in the Transparent2d phase. Explicit sort keys guarantee deterministic layer ordering (`CompareFunction::Always`, no depth testing between passes). Two render paths share one pipeline with a `StorageDrawMode` specialization key:

- **Storage buffer path** (NPCs + selection brackets): `vertex_npc` shader entry point reads positions/health directly from compute shader's `NpcGpuBuffers` storage buffers (bind group 2). Visual/equipment data uploaded from CPU as flat storage buffers (`NpcVisualBuffers`). Three specialized variants via `#ifdef` shader defs: `MODE_NPC_BODY` (layer 0, non-building only), `MODE_NPC_OVERLAY` (layers 1-7, non-building only), `MODE_SELECTION_BRACKET` (procedural corner brackets from per-instance style data).
- **Instance buffer path** (buildings, building overlays, projectiles): `vertex` shader entry point reads from classic per-instance `InstanceData` vertex attributes (slot 1). Building bodies use `BuildingBodyInstances` built each frame from `EntityGpuState` via `EntityMap.iter_instances()`.

Four textures bound simultaneously (group 0, bindings 0-7) — `atlas_id` selects which to sample (0=character, 1=world, 2=heal/3=sleep/4=arrow/8=boat via extras atlas, 7=building). Bar-only modes: 5=building HP bar (green/yellow/red), 6=mining progress bar (gold). Procedural mode: 9=selection brackets (no texture sampling, corner brackets from quad_uv). Atlas ID constants defined in `constants.rs` (`ATLAS_CHAR` through `ATLAS_BOAT`).

Defined in: `rust/src/npc_render.rs`, `rust/src/render.rs`, `shaders/npc_render.wgsl`

## Why Custom Pipeline?

Bevy's built-in sprite renderer creates one entity per sprite. At 16K NPCs, that's 16K entities in the render world — the scheduling/extraction overhead dominates. The custom pipeline uses:

- **1 entity per batch** (NpcBatch, ProjBatch) instead of 16,384 entities
- **GPU compute data stays on GPU** — vertex shader reads positions/health directly from compute output via storage buffers (bind group 2), no readback needed for rendering
- **Per-dirty storage buffer uploads** — visual [f32;8] + equip [f32;28] per slot, only changed slots uploaded per frame via per-index `write_buffer` (typically <1KB vs ~3.84MB bulk at 30K NPCs). Flash-only slots (damage flash decay) upload visual_data only, skipping equip_data entirely
- **Multi-layer drawing** — body + up to 7 overlay layers (4 equipment + 3 visual indicators), each a separate `draw_indexed` call within one RenderCommand

## Data Flow

```
Main World                        Render World
───────────                       ────────────
EntityGpuState        ──Extract<Res<T>>──▶ zero-clone immutable read
NpcVisualUpload       ──Extract<Res<T>>──▶ zero-clone immutable read
RenderFrameConfig     ──ExtractResource──▶ RenderFrameConfig (bundles EntityGpuData + ProjGpuData + textures + readback)
OverlayInstances      ──Extract<Res<T>>──▶ zero-clone → BuildingOverlayBuffers
BuildingBodyInstances ──Extract<Res<T>>──▶ zero-clone → BuildingBodyRenderBuffers (built from EntityGpuState via EntityMap)
SelectionOverlayInstances ──Extract<Res<T>>──▶ zero-clone → SelectionRenderBuffers
NpcGpuBuffers         ──(render world)──▶ positions + healths (bind group 2)
Camera2d entity       ──extract_camera_state──▶ CameraState
NpcBatch entity       ──extract_npc_batch──▶ NpcBatch entity
                                      │
                                      ▼
                               extract_npc_data (ExtractSchedule)
                               (strict coalescing for GPU-authoritative buffers,
                                gap-based coalescing for CPU-authoritative + visual/equip)
                                      │
                                      ▼
                               prepare_npc_buffers
                               (buffer creation + sentinel init on first frame,
                                create bind group 2 from NpcGpuBuffers + NpcVisualBuffers)
                                      │
                                      ▼
                            prepare_npc_texture_bind_group
                            (4 textures: char + world + extras + building;
                             building/extras atlas falls back to char_image until loaded)
                            prepare_npc_camera_bind_group
                            (CameraUniform with entity_count)
                                      │
                                      ▼
                                  queue_npcs
                               (DrawBuildingBodyCommands sort_key=0.2,
                                DrawBuildingOverlayCommands sort_key=0.3,
                                DrawNpcBodyCommands sort_key=0.5,
                                DrawNpcOverlayCommands sort_key=0.6,
                                DrawSelectionBracketCommands sort_key=1.5)
                                      │
                                      ▼
                    DrawBuildingBodyCommands (buildings, instance path):
                      BuildingBodyInstances from EntityGpuState via EntityMap, building atlas

                    DrawBuildingOverlayCommands (farms/BHP, instance path):
                      Instance buffer, building HP bars + farm growth + mine progress

                    DrawNpcBodyCommands (NPC bodies, storage path):
                      MODE_NPC_BODY — layer 0, non-building only

                    DrawNpcOverlayCommands (NPC overlays, storage path):
                      MODE_NPC_OVERLAY — layers 1-7, non-building only

ProjBufferWrites     ──Extract<Res<T>>──▶ zero-clone immutable read
ProjPositionState    ──Extract<Res<T>>──▶ zero-clone immutable read
                                        (ProjGpuData via RenderFrameConfig)
ProjBatch entity     ──extract_proj_batch──▶ ProjBatch entity
                                      │
                                      ▼
                               extract_proj_data (ExtractSchedule)
                               (per-dirty-index write_buffer to ProjGpuBuffers,
                                build InstanceData[] + ProjRenderBuffers)
                                      │
                                      ▼
                                  queue_projs
                               (add to Transparent2d, sort_key=1.0)
                                      │
                                      ▼
                    DrawProjCommands (instance path):
                      SetItemPipeline → SetNpcTextureBindGroup<0>
                      → SetNpcCameraBindGroup<1> → DrawProjs
```

## NPC Storage Buffers (Storage Path)

NPC rendering uses GPU storage buffers instead of per-instance vertex attributes. The vertex shader (`vertex_npc`) reads positions and health directly from compute shader output — no GPU→CPU→GPU round-trip.

**Bind group 2** (NPC data, set by `DrawStoragePass`):

| Binding | Buffer | Source | Per-NPC Size |
|---------|--------|--------|-------------|
| 0 | `npc_positions` | `NpcGpuBuffers.positions` (compute output) | 8B (vec2) |
| 1 | `npc_healths` | `NpcGpuBuffers.healths` (compute output) | 4B (f32) |
| 2 | `npc_visual_buf` | `NpcVisualBuffers.visual` (CPU upload) | 32B ([f32;8]) |
| 3 | `npc_equip` | `NpcVisualBuffers.equip` (CPU upload) | 112B (7×[f32;4]) |

**Visual buffer layout** (`[f32; 8]` per slot): `[sprite_col, sprite_row, body_atlas, flash, r, g, b, a]`. Built by `build_visual_upload` (reads live `GpuSlotPool.count()` for buffer sizing — not the stale `RenderFrameConfig` copy) from `EntityGpuState.sprite_indices`, `.flash_values`, and ECS Faction/Job components. Hidden slots cleared via `hidden_indices` pre-pass (event-driven, not full-array fill). New capacity initialized to `-1.0` via `resize()`. Building slots filled by `iter_instances()` loop. Phantom slots stay hidden via `sprite_col < 0`.

**Equipment buffer layout** (`[f32; 28]` per slot = 7 layers × `[col, row, atlas, _pad]`): Built by `build_visual_upload` from ECS components (NpcEquipment armor/helm/weapon/shield, CarriedLoot, Activity for sleep, NpcFlags for healing). Building slots get equip block wiped to `-1.0` sentinels. `col < 0` means unequipped/inactive.

**Instance offset encoding:** 7 `draw_indexed` calls, each with `entity_count` instances. Shader derives:
```wgsl
let slot = in.instance_index % camera.entity_count;
let layer = in.instance_index / camera.entity_count;
```

**Layer 0 (body):** reads `npc_visual_buf[slot]` for sprite/color/flash, `npc_healths[slot] / 100.0` for health bar. Hidden: `pos.x < -9000.0` or `sprite_col < 0`.

**Layers 1-7 (equipment):** reads `npc_equip[slot * 7u + (layer - 1u)]`. Color/scale by atlas type (all in shader):
- `atlas >= 2.5` (sleep icon, extras atlas): scale=32, color=white — preserves sprite's natural blue Zz
- `atlas >= 1.5` (heal halo, extras atlas): scale=40, color=yellow [1.0, 0.9, 0.2]
- `atlas >= 0.5` (carried item/world atlas): scale=32, color=white
- `atlas < 0.5` (character atlas equipment): scale=32, color=NPC job color from `npc_visual_buf`

Equipment sprites derived by `build_visual_upload` from ECS `NpcEquipment` (armor/helm/weapon/shield), `CarriedLoot`, `Activity` (sleep), and `NpcFlags` (healing) each frame. NPC can show sleep AND healing simultaneously (independent layers).

## Instance Data (Misc/Projectile Path)

Farms, building HP bars, and projectiles use classic per-instance vertex attributes via `InstanceData` (52 bytes):

```rust
pub struct InstanceData {
    pub position: [f32; 2],  // world XY (8 bytes)
    pub sprite: [f32; 2],    // atlas cell col, row (8 bytes)
    pub color: [f32; 4],     // RGBA tint (16 bytes)
    pub health: f32,         // normalized 0.0-1.0 (4 bytes)
    pub flash: f32,          // damage flash 0.0-1.0 (4 bytes)
    pub scale: f32,          // world-space quad size (4 bytes)
    pub atlas_id: f32,       // 0.0=character, 1.0=world, 2.0=heal, 3.0=sleep, 4.0=arrow, 5.0=BHP bar, 6.0=mining progress bar, 7.0=building, 8.0=boat (4 bytes)
    pub rotation: f32,       // radians, used for projectile orientation (4 bytes)
}
```

**Selection brackets** (in `SelectionRenderBuffers`, drawn by `DrawSelectionBrackets`):
- atlas_id=9.0 (procedural — no texture sampling), fragment shader draws corner brackets from quad_uv
- `SelectionInstance` (32 bytes): `slot: u32, color: [f32;4], scale: f32, y_offset: f32, _pad: f32`
- Position read from `npc_positions[slot]` in vertex shader (storage buffer path)
- Colors: cyan (selected NPC), gold (selected building), green (DirectControl group, capped at 200)
- LOD-aware: discarded when `camera.zoom < camera.lod_zoom`

**Farm sprites** (in `BuildingOverlayBuffers`, drawn by `DrawBuildingOverlay`):
- atlas_id=1.0 (world atlas), sprite=(24,9), scale=16
- Color: golden [1.0, 0.85, 0.0] when ready, green [0.4, 0.8, 0.2] when growing
- Health = growth progress (0.0-1.0, shown as progress bar)

**Building HP bars** (in `BuildingOverlayBuffers`, drawn by `DrawBuildingOverlay`):
- atlas_id=5.0, scale=32.0 (building-sized)
- Shader discards all sprite pixels for atlas_id >= 4.5, keeping only the health bar in bottom 15%
- Only buildings with HP < max are included (from `BuildingHpRender` resource, gated behind `BuildingHealState.needs_healing` — query skipped when no buildings are damaged)

**Mining progress bars** (in `BuildingOverlayBuffers`, drawn by `DrawBuildingOverlay`):
- atlas_id=6.0, scale=12.0, positioned +12y above miner
- Shader renders gold-colored bar (1.0, 0.85, 0.0) in bottom 15%, discards rest
- Always shown while mining (no < 0.99 gate like building HP bars)
- From `MinerProgressRender` resource (populated by `sync_miner_progress_render`)

**Projectiles** (in `ProjRenderBuffers`, drawn by `DrawProjs`):
- atlas_id=4.0 (arrow texture), health=1.0 (no bar), rotation=velocity angle
- Faction-colored: blue for villagers, per-faction color for raiders

## The Quad

GPUs draw triangles. A sprite is a textured quad — two triangles forming a rectangle:

```
  3 ──── 2          Triangle 1: 0→1→2
  │    ╱ │          Triangle 2: 0→2→3
  │  ╱   │
  │╱     │          6 indices: [0, 1, 2, 0, 2, 3]
  0 ──── 1          4 vertices, shared by ALL NPCs
```

```rust
static QUAD_VERTICES: [QuadVertex; 4] = [
    QuadVertex { position: [-0.5, -0.5], uv: [0.0, 1.0] }, // bottom-left
    QuadVertex { position: [ 0.5, -0.5], uv: [1.0, 1.0] }, // bottom-right
    QuadVertex { position: [ 0.5,  0.5], uv: [1.0, 0.0] }, // top-right
    QuadVertex { position: [-0.5,  0.5], uv: [0.0, 0.0] }, // top-left
];
```

The vertex shader scales the unit quad by the per-instance `scale` field (16.0 for NPCs) and offsets by instance position.

## Vertex Buffers

**Storage path** (`vertex_npc`, NPCs) — slot 0 only, data from storage buffers:

| Slot | Step Mode | Data | Stride | Attributes |
|------|-----------|------|--------|------------|
| 0 | Vertex | Static quad (4 vertices) | 16B | @location(0) quad_pos, @location(1) quad_uv |
| — | — | Storage buffers (bind group 2) | — | `@builtin(instance_index)` → index into npc_positions, npc_healths, npc_visual_buf, npc_equip |

**Instance path** (`vertex`, farms/BHP/projectiles) — slot 0 + slot 1:

| Slot | Step Mode | Data | Stride | Attributes |
|------|-----------|------|--------|------------|
| 0 | Vertex | Static quad (4 vertices) | 16B | @location(0) quad_pos, @location(1) quad_uv |
| 1 | Instance | Per-instance data (N instances) | 52B | @location(2) instance_pos, @location(3) sprite_cell, @location(4) color, @location(5) health, @location(6) flash, @location(7) scale, @location(8) atlas_id, @location(9) rotation |

**Selection bracket path** (`vertex_selection`, selection overlays) — slot 0 + slot 1:

| Slot | Step Mode | Data | Stride | Attributes |
|------|-----------|------|--------|------------|
| 0 | Vertex | Static quad (4 vertices) | 16B | @location(0) quad_pos, @location(1) quad_uv |
| 1 | Instance | Per-bracket data (N instances) | 32B | @location(2) slot, @location(3) color, @location(4) scale, @location(5) y_offset |

All paths share `quad_vertex_layout()` (slot 0). The instance path adds `instance_vertex_layout()` (slot 1). The selection path adds `selection_instance_layout()` (slot 1). Selected via `StorageDrawMode` in pipeline specialization key `(hdr, samples, Option<StorageDrawMode>)`. `None` = instance path, `Some(mode)` = storage path with shader def gating.

## Sprite Atlases

Four textures are bound simultaneously at group 0 (bindings 0-7). Per-instance/per-slot `atlas_id` selects which to sample.

| Atlas | Bindings | atlas_id | File | Size | Used By |
|-------|----------|----------|------|------|---------|
| Character | 0-1 | 0 | `roguelikeChar_transparent.png` | 918×203 | NPCs, equipment |
| World | 2-3 | 1 | `roguelikeSheet_transparent.png` | 968×526 | Farms |
| Extras | 4-5 | 2,3,4,8 | (generated at runtime) | 128×32 | Heal halo, sleep icon, arrow, boat |
| Building | 6-7 | 7 | (generated at runtime) | 32×(N×32) | Building sprites + wall auto-tile (10 extra layers), nearest-neighbor sampled |

Character and world atlases use 16px sprites with 1px margin (17px cells). The **extras atlas** is a horizontal grid of 4×32px cells generated at runtime by `build_extras_atlas()` from individual sprites (heal.png, sleep.png, arrow.png, boat.png) — each 16px source is nearest-neighbor 2× upscaled to 32×32. Column mapping: 0=heal, 1=sleep, 2=arrow, 3=boat. The building atlas is a vertical strip of N tiles (32×32 each, currently 13), generated at runtime by `build_building_atlas()` from individual building sprites. Layer count is dynamic — `camera.bldg_layers` is set from `BUILDING_REGISTRY.len() + WALL_EXTRA_LAYERS` each frame, eliminating hardcoded shader constants. The shared `calc_uv()` helper selects atlas constants based on `atlas_id`:

```wgsl
fn calc_uv(sprite_col: f32, sprite_row: f32, atlas_id: f32, quad_uv: vec2<f32>) -> vec2<f32> {
    if is_building_atlas(atlas_id) {
        // Half-pixel inset prevents sampling at layer boundaries (GPU rounding artifact)
        let inset = 0.5 / (camera.bldg_layers * 32.0);
        let v = (sprite_col + clamp(quad_uv.y, inset, 1.0 - inset)) / camera.bldg_layers;
        return vec2<f32>(quad_uv.x, v);
    } else if atlas_id >= 1.5 {
        // Extras atlas: col selected by atlas_id, UV spans one cell
        var col: f32 = 0.0;
        if atlas_id >= 7.5 { col = 3.0; }       // boat (atlas 8)
        else if atlas_id >= 3.5 { col = 2.0; }  // arrow (atlas 4)
        else if atlas_id >= 2.5 { col = 1.0; }  // sleep (atlas 3)
        let px = (col + quad_uv.x) / camera.extras_cols;
        return vec2<f32>(px, quad_uv.y);
    } else if atlas_id < 0.5 {
        // Character atlas: 918×203
    } else {
        // World atlas: 968×526
    }
}
```

The fragment shader dispatches by `atlas_id` — building (7) first, then descending: mining progress bar (≥5.5) renders gold bar and discards, building HP bar-only (≥4.5) renders health bar and discards, extras (≥1.5) samples `extras_texture` with per-atlas_id color tint (arrow=white, sleep=white, heal=yellow, boat=white), then character (<0.5) or world atlas. Health bars, damage flash, and equipment layer masking only apply to character atlas sprites (`atlas_id < 0.5`).

Job sprite assignments (from constants.rs):
- Farmer: (1, 6)
- Archer: (0, 0)
- Raider: (0, 6)
- Fighter: (1, 9)

## Fragment Shader

The fragment shader handles both health bar rendering and sprite rendering. The vertex shader passes two UV sets: `uv` (atlas-transformed for texture sampling) and `quad_uv` (raw 0-1 within the sprite quad for health bar positioning).

**Selection brackets** (atlas_id 9, early-return before all other rendering):
```wgsl
if in.atlas_id >= 8.5 && in.atlas_id < 9.5 {
    if camera.zoom < camera.lod_zoom { discard; }
    // Procedural corner brackets from quad_uv: 35% length, 8% width per corner
    if !(in_tl || in_tr || in_bl || in_br) { discard; }
    return vec4<f32>(in.color.rgb, in.color.a);
}
```

**Extras atlas overlays** (early-return before health bar / sprite rendering):
```wgsl
// Extras atlas (atlas_id 2=heal, 3=sleep, 4=arrow, 8=boat): sample extras_texture, discard transparent, apply color tint
if in.atlas_id >= 1.5 { ... return; }
```

**Health bar** (bottom 15% of sprite, show-when-damaged mode):
```wgsl
if in.quad_uv.y > 0.85 && in.health < 0.99 {
    // Dark grey background, filled portion colored by health level
    // Green (>50%), Yellow (>25%), Red (≤25%)
}
```

**Mining progress bar** (atlas_id 6, checked first in fragment shader):
```wgsl
if in.atlas_id >= 5.5 {
    // Gold bar in bottom 15%, always shown while mining
    if in.quad_uv.y > 0.85 { bar_color = gold where uv.x < health; }
    discard;
}
```

**Building HP bar-only** (atlas_id 5):
```wgsl
if in.atlas_id >= 4.5 {
    // Render health bar in bottom 15% when damaged, discard everything else
    if in.quad_uv.y > 0.85 && in.health < 0.99 {
        // Same 3-color bar as NPC health (green/yellow/red)
    }
    discard;  // No sprite — just the bar
}
```

**Sprite rendering** (remaining 85%):
```wgsl
// Sample from correct atlas based on atlas_id
var tex_color: vec4<f32>;
if in.atlas_id < 0.5 {
    tex_color = textureSample(char_texture, char_sampler, in.uv);
} else {
    tex_color = textureSample(world_texture, world_sampler, in.uv);
}
if tex_color.a < 0.1 { discard; }
// Carried items (atlas_id >= 0.5, equipment layer): original colors, no grayscale tint
if in.atlas_id >= 0.5 && in.health >= 0.99 { return vec4<f32>(tex_color.rgb, tex_color.a); }
// Equipment layers: discard bottom pixels to preserve health bar visibility
if in.health >= 0.99 && in.quad_uv.y > 0.85 && in.atlas_id < 0.5 { discard; }
var final_color = vec4<f32>(tex_color.rgb * in.color.rgb, tex_color.a);
```

Texture color is multiplied by the instance's tint color via grayscale conversion (`dot(rgb, luma_weights) * color`). This is how faction colors work — player faction (0) NPCs get job-based colors (pure green/blue/red/yellow), while all other factions get per-faction RGB tints from a 10-color saturated palette. Carried items (world atlas on equipment layers) bypass the grayscale tint and render with original texture colors, so food and gold sprites appear naturally colored. Equipment layers (health >= 0.99) discard pixels in the health bar region so the body's health bar remains visible underneath.

**Damage flash** (white overlay, applied after color tinting):
```wgsl
if in.flash > 0.0 {
    final_color = vec4<f32>(mix(final_color.rgb, vec3(1.0, 1.0, 1.0), in.flash), final_color.a);
}
```

Flash intensity starts at 1.0 (full white) on damage hit and decays to 0.0 over ~0.2s (rate 5.0/s). Decay happens on CPU in `populate_gpu_state` via `flash_values` in `EntityGpuState`. The `mix()` function interpolates between the tinted sprite color and pure white.

## Render World Phases

The render pipeline runs in Bevy's render world after extract:

| Phase | System | Purpose |
|-------|--------|---------|
| Extract | `extract_npc_batch` | Despawn stale render world NpcBatch, then clone fresh from main world |
| Extract | `extract_npc_data` | Zero-clone GPU upload from EntityGpuState: GPU-authoritative buffers (positions, arrivals) use strict coalescing via `write_coalesced_exact_f32/i32` (exact-adjacent merging only, no gap, no bulk fallback). CPU-authoritative buffers (targets, speeds, factions, healths, flags, half_sizes) use gap-based `write_coalesced_f32/i32/u32` with 40% window fallback. Visual gap-based via `visual_uploaded_indices`, equip gap-based via `equip_uploaded_indices` (flash-only slots excluded from equip). Full upload only on startup/load via `visual_full_upload` flag |
| Extract | `extract_proj_batch` | Despawn stale render world ProjBatch, then clone fresh from main world |
| Extract | `extract_camera_state` | Build CameraState from Camera2d Transform + Projection + Window |
| Extract | `extract_building_body_instances` | Zero-clone read of BuildingBodyInstances → BuildingBodyRenderBuffers (building body sprites from EntityGpuState via EntityMap) |
| Extract | `extract_overlay_instances` | Zero-clone read of OverlayInstances → BuildingOverlayBuffers (farms/BHP/mining) with RawBufferVec reuse |
| Extract | `extract_selection_overlay` | Zero-clone read of SelectionOverlayInstances → SelectionRenderBuffers (selection brackets) |
| PrepareResources | `prepare_npc_buffers` | Buffer creation + sentinel init (first frame), create bind group 2 |
| Extract | `extract_proj_data` | Zero-clone GPU upload: per-dirty-index compute writes + projectile instance buffer build from `active_set` via `Extract<Res<T>>` |
| PrepareBindGroups | `prepare_npc_texture_bind_group` | Create texture bind group from RenderFrameConfig.textures (4 textures: char + world + extras + building; building/extras fall back to char_image until atlas loads) |
| PrepareBindGroups | `prepare_npc_camera_bind_group` | Create camera uniform bind group (includes entity_count from RenderFrameConfig.npc) |
| Queue | `queue_npcs` | Add DrawBuildingBodyCommands (0.2), DrawBuildingOverlayCommands (0.3), DrawNpcBodyCommands (0.5), DrawNpcOverlayCommands (0.6), DrawSelectionBracketCommands (1.5) |
| Queue | `queue_projs` | Add DrawProjCommands (sort_key=1.0, above NPCs) |
| Render | `DrawBuildingBodyCommands` | Instance path — building body sprites from `BuildingBodyRenderBuffers` (built from `EntityGpuState` via `EntityMap`) |
| Render | `DrawBuildingOverlayCommands` | Instance path — farms, building HP bars, mine progress |
| Render | `DrawNpcBodyCommands` | Storage path, `#ifdef MODE_NPC_BODY` — layer 0, non-building only |
| Render | `DrawNpcOverlayCommands` | Storage path, `#ifdef MODE_NPC_OVERLAY` — layers 1-7, non-building only |
| Render | `DrawProjCommands` | Instance path — arrow projectiles |
| Render | `DrawSelectionBracketCommands` | Storage+instance hybrid — procedural selection brackets |

## RenderCommand Pattern

Bevy's RenderCommand trait defines GPU commands for drawing. Six command chains share one pipeline (specialized via `Option<StorageDrawMode>`):

**Generic storage draw** — `DrawStoragePass<const BODY_ONLY: bool>`:
```rust
// BODY_ONLY=true: 1 draw_indexed (layer 0 only)
// BODY_ONLY=false: 7 draw_indexed (layers 1-7)
type DrawNpcBodyCommands = (..., DrawStoragePass<true>);       // + MODE_NPC_BODY shader def
type DrawNpcOverlayCommands = (..., DrawStoragePass<false>);   // + MODE_NPC_OVERLAY shader def
```

**Building body instance path** — `DrawBuildingBody`:
```rust
type DrawBuildingBodyCommands = (..., DrawBuildingBody);
```
`DrawBuildingBody::render()` reads `BuildingBodyRenderBuffers` — a `RawBufferVec<InstanceData>` built each frame from `EntityGpuState` (positions, factions, health, sprite indices, flash) by `build_building_body_instances` (PostUpdate). Building slots are obtained by iterating `EntityMap.iter_instances()` and indexing into the unified `EntityGpuState` arrays.

The shader derives `slot` and `layer` from `instance_index`. Compile-time `#ifdef` gating discards unwanted slots per pass (buildings vs non-buildings). Hidden NPCs (`pos.x < -9000`) and empty equipment slots (`col < 0`) are culled by moving clip_position off-screen.

**Building overlay instance path** — farms, building HP bars, mine progress:
```rust
type DrawBuildingOverlayCommands = (..., DrawBuildingOverlay);
```

`DrawBuildingOverlay::render()` reads `BuildingOverlayBuffers` — a small `RawBufferVec<InstanceData>` (~100-200 entries) built each frame from `OverlayInstances` (populated by `build_overlay_instances` from `GrowthStates` + `BuildingHpRender` + `MinerProgressRender` in PostUpdate, zero-clone extracted to render world).

**Projectile instance path** — shares quad geometry, separate instance buffer:
```rust
type DrawProjCommands = (..., DrawProjs);
```

`DrawProjs::render()` reads `ProjRenderBuffers` — sharing static quad/index from `NpcRenderBuffers`. Faction-colored: blue for villagers, per-faction color for raiders.

**Selection bracket storage+instance hybrid path** — `DrawSelectionBrackets`:
```rust
type DrawSelectionBracketCommands = (..., DrawSelectionBrackets);
```
`DrawSelectionBrackets::render()` reads `SelectionRenderBuffers` — a `RawBufferVec<SelectionInstance>` built each frame by `build_selection_overlay` (PostUpdate) from `SelectedNpc`, `SelectedBuilding`, and `NpcFlags.direct_control`. Uses storage buffer bind group 2 for positions (from NPC compute output) and instance buffer slot 1 for per-bracket style (slot, color, scale, y_offset). Vertex shader reads `npc_positions[in.slot]` for world position, fragment shader renders procedural corner brackets.

**Sort key helper** — `queue_phase_item()` adds a single `Transparent2d` item, used by both `queue_npcs` and `queue_projs` to avoid repetitive phase item construction.

## Camera

Bevy's Camera2d is the single source of truth — input systems write directly to `Transform` (position) and `Projection::Orthographic` (zoom via `scale`). No intermediate `CameraState` resource in the main world.

**Main world systems** (registered in `RenderPlugin::build`, Update schedule):
- `camera_pan_system`: WASD at 400px/s, speed scaled by 1/zoom via `ortho_zoom()` helper, writes `Transform` directly
- `camera_zoom_system`: scroll wheel zoom toward mouse cursor, writes `Projection::Orthographic.scale` and `Transform` directly. Zoom speed, min, and max are user-configurable via `UserSettings` (defaults: speed=0.1, min=0.02, max=4.0)
- `click_to_select_system`: screen-to-world via camera `Transform` + `Projection`. Left click hit-tests live NPCs by iterating `EntityMap.iter_npcs()` and sampling `GpuReadState.positions` by slot; dead NPCs, hidden sentinels, and out-of-bounds slots are skipped. Building hit-tests stay live-only via `EntityMap.iter_instances()` within a separate radius, so one click can keep one NPC and one building selected at once and `UiState.inspector_prefer_npc` follows the nearer hit. Right-click DirectControl commands reuse the same live-NPC scan for enemy NPC targeting before falling back to live enemy buildings or ground move. Guarded by `ctx.wants_pointer_input() || ctx.is_pointer_over_area()` to avoid stealing clicks from egui UI panels.

**Render world**: `extract_camera_state` (ExtractSchedule, `npc_render.rs`) reads the camera entity's `Transform`, `Projection`, `Window`, and `UserSettings` (for `lod_transition`) to build a `CameraState` resource in the render world. `prepare_npc_camera_bind_group` writes this to a `CameraUniform` `UniformBuffer` each frame (including `entity_count` from `RenderFrameConfig.npc`, `bldg_layers` from `BUILDING_REGISTRY.len() + WALL_EXTRA_LAYERS`, `extras_cols` = 4.0, and `lod_zoom` from `CameraState`), creating a bind group at group 1.

**Shader** (`npc_render.wgsl`): reads camera from uniform buffer:
```wgsl
struct Camera {
    pos: vec2<f32>,
    zoom: f32,
    entity_count: u32,  // used by vertex_npc for instance offset decoding
    viewport: vec2<f32>,
    bldg_layers: f32,   // building atlas layer count (from BUILDING_REGISTRY.len())
    extras_cols: f32,   // extras atlas column count (currently 4.0)
    lod_zoom: f32,      // LOD transition threshold (from UserSettings.lod_transition)
    _pad: u32,
};
@group(1) @binding(0) var<uniform> camera: Camera;

// Shared helper (used by both vertex and vertex_npc):
fn world_to_clip(world_pos: vec2<f32>) -> vec4<f32> {
    let offset = (world_pos - camera.pos) * camera.zoom;
    let ndc = offset / (camera.viewport * 0.5);
    return vec4<f32>(ndc.x, ndc.y, 0.0, 1.0);
}
```

## Texture Loading

`render.rs` loads all sprite sheets at startup and shares texture handles with the instanced pipeline:

| Sheet | File | Size | Grid | Used By |
|-------|------|------|------|---------|
| Characters | `roguelikeChar_transparent.png` | 918×203 | 54×12 (16px + 1px margin) | NPC instanced rendering |
| World | `roguelikeSheet_transparent.png` | 968×526 | 57×31 (16px + 1px margin) | Building/terrain sprites |
| Heal halo | `heal.png` | 16×16 | 1×1 (single sprite) | Extras atlas source (col 0) |
| Sleep icon | `sleep.png` | 16×16 | 1×1 (single sprite) | Extras atlas source (col 1) |
| Arrow | `arrow.png` | 16×16 | 1×1 (single sprite) | Extras atlas source (col 2) |
| Boat | `boat.png` | 32×32 | 1×1 (single sprite) | Extras atlas source (col 3) |
| Farmer Home | `house.png` | 32×32 | 1×1 (standalone) | Building tileset (External) |
| Archer Home | `barracks.png` | 32×32 | 1×1 (standalone) | Building tileset (External) |
| Waypoint | `waypoint.png` | 32×32 | 1×1 (standalone) | Building tileset (External) |
| Miner Home | `miner_house.png` | 32×32 | 1×1 (standalone) | Building tileset (External) |
| Fighter Home | `fighter_home.png` | 32×32 | 1×1 (standalone) | Building tileset (External) |
| Wall | `wood_walls_131x32.png` | 131×32 | 4 sprites in strip (E-W, cross, BR corner, T-junction) | Building tileset (External) + 10 auto-tile layers |

`SpriteAssets` holds handles for all loaded textures. External building sprites are stored as a `Vec<Handle<Image>>` (`external_textures`), loaded dynamically from `BUILDING_REGISTRY` — each `TileSpec::External("sprites/foo.png")` entry is loaded at startup. NPC instanced rendering textures are shared via `RenderFrameConfig.textures` (NpcSpriteTexture: `handle` for character, `world_handle` for world atlas, `extras_handle` for extras atlas, `building_handle` for building atlas), extracted to render world for bind group creation. The building and extras atlas handles are set later by `spawn_world_tilemap` (not at startup like the others); `prepare_npc_texture_bind_group` falls back to `char_image` until they're available.

## Equipment Layers

Multi-layer rendering uses `NpcVisualUpload.equip_data` (packed by `build_visual_upload` each frame, 7 layers × 4 floats = 28 floats per slot):

| Layer | Index | ECS Source | Equip Offset | Sentinel | Set By |
|-------|-------|-----------|-------------|----------|--------|
| Armor | 1 | `NpcEquipment.armor_sprite()` | idx*28+0 | col < 0 | build_visual_upload |
| Helmet | 2 | `NpcEquipment.helm_sprite()` | idx*28+4 | col < 0 | build_visual_upload |
| Weapon | 3 | `NpcEquipment.weapon_sprite()` | idx*28+8 | col < 0 | build_visual_upload |
| Shield | 4 | `NpcEquipment.shield_sprite()` | idx*28+12 | col < 0 | build_visual_upload |
| Item | 5 | `CarriedLoot` | idx*28+16 | col < 0 | build_visual_upload |
| Status | 6 | `Activity::Resting` | idx*28+20 | col < 0 | build_visual_upload |
| Healing | 7 | `NpcFlags.healing` | idx*28+24 | col < 0 | build_visual_upload |

All overlay layers are packed by `build_visual_upload` each frame from ECS components. Dead NPCs are skipped by the renderer (position < -9000). Each layer stores atlas_id alongside sprite coordinates so items can reference either atlas. Building slots get all 28 floats wiped to -1.0 sentinels.

## World Tilemap (Terrain Only)

Terrain is rendered via Bevy's built-in `TilemapChunk` as a chunked layer over the grid (default 250×250, up to 1000×1000). The renderer splits terrain into `CHUNK_SIZE=32` tile chunks for frustum culling, each with its own `TilemapChunk` entity and tile data region. A fragment shader does per-pixel tile lookup from a `texture_2d_array` tileset.

| Layer | Z | Alpha | Content | Tileset |
|-------|---|-------|---------|---------|
| Terrain | -1.0 | Opaque | Every cell filled (biome tiles) | 11 tiles (`TERRAIN_TILES`) |

Terrain uses `AlphaMode2d::Opaque`. Buildings are rendered through the GPU instanced pipeline (storage buffer path with `atlas_id=7`, sort_key=0.2 via `DrawBuildingBodyCommands`).

**Slot Indicators** (`ui/mod.rs`): Building grid indicators use Sprite entities at z=-0.3 with a `SlotIndicator` marker component — not gizmos, because Bevy gizmos render in a separate pass after all Transparent2d items and can't be z-sorted with them. Green "+" crosshairs mark empty unlocked slots, dim bracket corners mark adjacent locked slots. Indicators are rebuilt when `TownGrids` or `WorldGrid` changes, and despawned on game cleanup.

**`TileSpec` enum** (`world.rs`): `Single(col, row)` for a single 16×16 sprite, `Quad([(col,row); 4])` for a 2×2 composite of four 16×16 sprites (TL, TR, BL, BR), or `External(&'static str)` for a standalone PNG (asset path, e.g. `"sprites/farmer_home_64x64.png"`). Rock terrain uses Quad; Tent uses Quad. Most buildings use External 64×64 PNGs from `BUILDING_REGISTRY`.

**`build_tileset(atlas, tiles, extra, images)`** (`world.rs`): Extracts tiles from the world atlas and builds a 64×64 `texture_2d_array` for terrain (`ATLAS_CELL=64`). `Single` tiles are nearest-neighbor 4× upscaled (16px→64px). `Quad` tiles `blit_2x` four 16×16 sprites into 32×32 quadrants filling 64px. `External` tiles copy raw pixel data (64px direct, smaller sizes NN upscaled). Called once with `TERRAIN_TILES` (11 tiles, no extras).

**`build_building_atlas(atlas, tiles, extra, images)`** (`world.rs`): Builds a 64×(N×64) vertical strip `texture_2d` for the building atlas with `ImageSampler::nearest()` to prevent texture bleeding between layers. Same tile extraction logic as `build_tileset` but outputs a single strip texture instead of a `texture_2d_array`. After base tiles (15 from BUILDING_REGISTRY), appends 10 wall auto-tile layers: E-W straight sprite extracted from `wood_walls_131x32.png` overwrites Wall's base layer, then N-S (90° rotation of E-W), 4 corner sprites (BR source at x=66, rotated 90°/180°/270° for BL/TL/TR), cross/junction sprite (x=33), and 4 T-junction sprites (T source at x=99, rotated 90°/180°/270°). Total layers = BUILDING_REGISTRY.len() + WALL_EXTRA_LAYERS (10). `camera.bldg_layers` includes the extra layers. Stored in `RenderFrameConfig.textures.building_handle`. `BUILDING_REGISTRY` order = tileset strip indices.

**`Biome::tileset_index(cell_index)`**: Maps biome + cell position to terrain tileset array index (0-10). Grass always uses index 0 (Grass A only). Forest picks 2-7 via a deterministic hash of `cell_index`, which breaks visible cycle patterns while keeping tile selection stable for the same world. Water=8, Rock=9, Dirt=10.

**`Building::tileset_index()`**: Maps building variant to building strip index (0-13). Delegates to `constants::tileset_index(kind)` which looks up position in `BUILDING_REGISTRY`. Fountain=0, Bed=1, Waypoint=2, Farm=3, FarmerHome=4, ArcherHome=5, Tent=6, GoldMine=7, MinerHome=8, CrossbowHome=9, FighterHome=10, Road=11, Wall=13 (14 base tiles via `building_tiles()`, plus 10 wall auto-tile layers at indices 14-23: N-S=14, BR=15, BL=16, TL=17, TR=18, Cross=19, T-open-N=20, T-open-W=21, T-open-S=22, T-open-E=23).

**Wall auto-tile** (`world.rs`): `wall_autotile_variant()` examines N/S/E/W neighbors to select one of 11 atlas layer offsets: 0=E-W, 1=N-S, 2-5=corners (BL/BR/TR/TL — screen-space), 6=cross (4-way), 7-10=T-junctions (open N/W/S/E — screen-space, named by missing neighbor). `update_wall_sprites_around()` pushes GPU `SetSpriteFrame` updates for the wall and its 4 neighbors on placement/removal. `update_all_wall_sprites()` resets all wall sprites on world load. Build menu extracts the first 32x32 sprite from all autotile building strips (roads, walls) for the toolbar icon.

**`TilemapSpawned`** resource (`render.rs`): Tracks whether the tilemap has been spawned. Uses a `Resource` (not `Local`) so that `game_cleanup_system` can reset it when leaving Playing state, enabling tilemap re-creation on re-entry.

**`spawn_world_tilemap`** system (`render.rs`, Update schedule): Runs once when WorldGrid is populated and world atlas is loaded. Spawns terrain chunk with `TerrainChunk` marker. Also creates the building atlas from `SpriteAssets.external_textures` (registry-driven) and the extras atlas from heal/sleep/arrow/boat sprites via `build_extras_atlas()`, storing both in `RenderFrameConfig.textures`.

**`TerrainChunk`** marker component (`render.rs`): Attached to the terrain TilemapChunk entity so `sync_terrain_tilemap` can query it for runtime terrain updates (e.g. slot unlock → Dirt).

**`sync_terrain_tilemap`** system (`render.rs`, Update schedule): Runs when `WorldGrid.is_changed()`. Rebuilds terrain `TilemapChunkTileData` from current grid cells. Needed because slot unlocking (player or AI) changes terrain biome to Dirt at runtime.

## Known Issues

- **Health bar mode hardcoded**: Only "when damaged" mode (show when health < 99%). Off/always modes need a uniform or config resource.
- **MaxHealth hardcoded**: Health normalization divides by 100.0. When upgrades change MaxHealth, normalization must use per-NPC max.
- **Equipment sprite tuning**: Equipment sprites have updated atlas coordinates — use `npc-visuals` test scene to review layers. Food sprite is on world atlas (24,9).
- **Single tilemap chunk**: At 1000×1000 (1M tiles), `command_buffer_generation_tasks` costs ~10ms because Bevy processes all tiles even when most are off-screen. Splitting into 32×32 chunks enables off-screen culling (see roadmap spec).
