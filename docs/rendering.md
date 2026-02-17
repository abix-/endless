# Rendering System

## Overview

Two rendering systems work together: **terrain and buildings** use Bevy's built-in `TilemapChunk` (two layers on the same grid — terrain opaque at z=-1, buildings alpha-blended at z=-0.5, zero per-frame CPU cost), while **NPCs, equipment, farms, building HP bars, and projectiles** use a custom GPU pipeline via Bevy's RenderCommand pattern in the Transparent2d phase. Two render paths share one pipeline with a `storage_mode` specialization key:

- **Storage buffer path** (NPCs): `vertex_npc` shader entry point reads positions/health directly from compute shader's `NpcGpuBuffers` storage buffers (bind group 2). Visual/equipment data uploaded from CPU as flat storage buffers (`NpcVisualBuffers`). 7 layers (body + 4 equipment + 2 indicators) rendered via 7 `draw_indexed` calls with instance offset encoding — shader derives `slot = instance_index % npc_count`, `layer = instance_index / npc_count`.
- **Instance buffer path** (farms, building HP bars, projectiles): `vertex` shader entry point reads from classic per-instance `InstanceData` vertex attributes (slot 1).

Five textures bound simultaneously — `atlas_id` selects which to sample (0=character, 1=world, 2=heal halo, 3=sleep icon, 4=arrow). Bar-only modes: 5=building HP bar (green/yellow/red), 6=mining progress bar (gold).

Defined in: `rust/src/npc_render.rs`, `rust/src/render.rs`, `shaders/npc_render.wgsl`

## Why Custom Pipeline?

Bevy's built-in sprite renderer creates one entity per sprite. At 16K NPCs, that's 16K entities in the render world — the scheduling/extraction overhead dominates. The custom pipeline uses:

- **1 entity per batch** (NpcBatch, ProjBatch) instead of 16,384 entities
- **GPU compute data stays on GPU** — vertex shader reads positions/health directly from compute output via storage buffers (bind group 2), no readback needed for rendering
- **Flat storage buffer uploads** — visual [f32;8] + equip [f32;24] per slot, two `write_buffer` calls per frame (~3.84MB at 30K NPCs vs 10.9MB old instance buffer rebuild)
- **Multi-layer drawing** — body + up to 6 overlay layers (4 equipment + 2 visual indicators), each a separate `draw_indexed` call within one RenderCommand

## Data Flow

```
Main World                        Render World
───────────                       ────────────
NpcGpuState           ──Extract<Res<T>>──▶ zero-clone immutable read
NpcVisualUpload       ──Extract<Res<T>>──▶ zero-clone immutable read
NpcGpuData            ──ExtractResource──▶ NpcGpuData
NpcGpuBuffers         ──(render world)──▶ positions + healths (bind group 2)
Camera2d entity       ──extract_camera_state──▶ CameraState
NpcBatch entity       ──extract_npc_batch──▶ NpcBatch entity
                                      │
                                      ▼
                               extract_npc_data (ExtractSchedule)
                               (hybrid writes: per-dirty-index for GPU-authoritative,
                                bulk write_buffer for CPU-authoritative + visual/equip)
                                      │
                                      ▼
                               prepare_npc_buffers
                               (buffer creation + sentinel init on first frame,
                                build NpcMiscBuffers for farms/BHP/mining progress,
                                create bind group 2 from NpcGpuBuffers + NpcVisualBuffers)
                                      │
                                      ▼
                            prepare_npc_texture_bind_group
                            (5 textures: char + world + heal + sleep + arrow)
                            prepare_npc_camera_bind_group
                            (CameraUniform with npc_count)
                                      │
                                      ▼
                                  queue_npcs
                               (DrawMiscCommands sort_key=0.4,
                                DrawNpcStorageCommands sort_key=0.5)
                                      │
                                      ▼
                    DrawMiscCommands (farms/BHP, instance path):
                      SetItemPipeline → SetNpcTextureBindGroup<0>
                      → SetNpcCameraBindGroup<1> → DrawMisc

                    DrawNpcStorageCommands (NPCs, storage path):
                      SetItemPipeline → SetNpcTextureBindGroup<0>
                      → SetNpcCameraBindGroup<1> → DrawNpcsStorage

ProjBufferWrites     ──Extract<Res<T>>──▶ zero-clone immutable read
ProjPositionState    ──Extract<Res<T>>──▶ zero-clone immutable read
ProjGpuData          ──ExtractResource──▶ ProjGpuData
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

**Bind group 2** (NPC data, set by `DrawNpcsStorage`):

| Binding | Buffer | Source | Per-NPC Size |
|---------|--------|--------|-------------|
| 0 | `npc_positions` | `NpcGpuBuffers.positions` (compute output) | 8B (vec2) |
| 1 | `npc_healths` | `NpcGpuBuffers.healths` (compute output) | 4B (f32) |
| 2 | `npc_visual_buf` | `NpcVisualBuffers.visual` (CPU upload) | 32B ([f32;8]) |
| 3 | `npc_equip` | `NpcVisualBuffers.equip` (CPU upload) | 96B (6×[f32;4]) |

**Visual buffer layout** (`[f32; 8]` per slot): `[sprite_col, sprite_row, body_atlas, flash, r, g, b, a]`. Built by `build_visual_upload` from `NpcGpuState.sprite_indices`, `.flash_values`, and ECS Faction/Job components. Reset to `-1.0` sentinel each frame — phantom slots (e.g. waypoint NPC slots with no ECS entity) stay hidden via `sprite_col < 0`.

**Equipment buffer layout** (`[f32; 24]` per slot = 6 layers × `[col, row, atlas, _pad]`): Built by `build_visual_upload` from ECS components (EquippedArmor, EquippedHelmet, EquippedWeapon, Activity, Healing). Reset to `-1.0` sentinel each frame — `col < 0` means unequipped/inactive.

**Instance offset encoding:** 7 `draw_indexed` calls, each with `npc_count` instances. Shader derives:
```wgsl
let slot = in.instance_index % camera.npc_count;
let layer = in.instance_index / camera.npc_count;
```

**Layer 0 (body):** reads `npc_visual_buf[slot]` for sprite/color/flash, `npc_healths[slot] / 100.0` for health bar. Hidden: `pos.x < -9000.0` or `sprite_col < 0`.

**Layers 1-6 (equipment):** reads `npc_equip[slot * 6u + (layer - 1u)]`. Color/scale by atlas type (all in shader):
- `atlas >= 2.5` (sleep icon): scale=16, color=white — preserves sprite's natural blue Zz
- `atlas >= 1.5` (heal halo): scale=20, color=yellow [1.0, 0.9, 0.2]
- `atlas >= 0.5` (carried item/world atlas): scale=16, color=white
- `atlas < 0.5` (character atlas equipment): scale=16, color=NPC job color from `npc_visual_buf`

Equipment sprites derived by `build_visual_upload` from ECS components (`EquippedWeapon`, `EquippedHelmet`, `EquippedArmor`, `Activity`, `Healing`) each frame. NPC can show sleep AND healing simultaneously (independent layers).

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
    pub atlas_id: f32,       // 0.0=character, 1.0=world, 2.0=heal, 3.0=sleep, 4.0=arrow, 5.0=BHP bar, 6.0=mining progress bar (4 bytes)
    pub rotation: f32,       // radians, used for projectile orientation (4 bytes)
}
```

**Farm sprites** (in `NpcMiscBuffers`, drawn by `DrawMisc`):
- atlas_id=1.0 (world atlas), sprite=(24,9), scale=16
- Color: golden [1.0, 0.85, 0.0] when ready, green [0.4, 0.8, 0.2] when growing
- Health = growth progress (0.0-1.0, shown as progress bar)

**Building HP bars** (in `NpcMiscBuffers`, drawn by `DrawMisc`):
- atlas_id=5.0, scale=32.0 (building-sized)
- Shader discards all sprite pixels for atlas_id >= 4.5, keeping only the health bar in bottom 15%
- Only buildings with HP < max are included (from `BuildingHpRender` resource)

**Mining progress bars** (in `NpcMiscBuffers`, drawn by `DrawMisc`):
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

Both paths share `quad_vertex_layout()` (slot 0). The instance path adds `instance_vertex_layout()` (slot 1). Selected via `storage_mode` bool in pipeline specialization key `(hdr, samples, storage_mode)`.

## Sprite Atlases

Five textures are bound simultaneously at group 0 (bindings 0-9). Per-instance/per-slot `atlas_id` selects which to sample.

| Atlas | Bindings | File | Size | Grid | Used By |
|-------|----------|------|------|------|---------|
| Character | 0-1 | `roguelikeChar_transparent.png` | 918×203 | 54×12 | NPCs, equipment |
| World | 2-3 | `roguelikeSheet_transparent.png` | 968×526 | 57×31 | Terrain, buildings, farms |
| Heal halo | 4-5 | `heal.png` | 16×16 | 1×1 | Healing halo overlay |
| Sleep icon | 6-7 | `sleep.png` | 16×16 | 1×1 | Sleep indicator overlay |
| Arrow | 8-9 | `arrow.png` | 16×16 | 1×1 | Projectile sprite |

Character and world atlases use 16px sprites with 1px margin (17px cells). Heal, sleep, and arrow textures are single-sprite (UV = quad_uv directly). The shared `calc_uv()` helper selects atlas constants based on `atlas_id`:

```wgsl
fn calc_uv(sprite_col: f32, sprite_row: f32, atlas_id: f32, quad_uv: vec2<f32>) -> vec2<f32> {
    if atlas_id >= 1.5 {
        return quad_uv;  // Single-sprite textures (heal, sleep, arrow)
    } else if atlas_id < 0.5 {
        // Character atlas: 918×203
    } else {
        // World atlas: 968×526
    }
}
```

The fragment shader dispatches by `atlas_id` in descending order: mining progress bar (≥5.5) renders gold bar and discards, building HP bar-only (≥4.5) renders health bar and discards, arrow projectile (≥3.5) samples `arrow_texture`, sleep (≥2.5) samples `sleep_texture`, heal (≥1.5) samples `heal_texture`, then character (<0.5) or world atlas. Health bars, damage flash, and equipment layer masking only apply to character atlas sprites (`atlas_id < 0.5`). Sleep and heal both early-return after texture sampling with color tint applied.

Job sprite assignments (from constants.rs):
- Farmer: (1, 6)
- Archer: (0, 0)
- Raider: (0, 6)
- Fighter: (1, 9)

## Fragment Shader

The fragment shader handles both health bar rendering and sprite rendering. The vertex shader passes two UV sets: `uv` (atlas-transformed for texture sampling) and `quad_uv` (raw 0-1 within the sprite quad for health bar positioning).

**Dedicated texture overlays** (early-return before health bar / sprite rendering):
```wgsl
// Sleep icon (atlas_id 3): sample sleep_texture, discard transparent, apply color tint
if in.atlas_id >= 2.5 { ... return; }
// Heal halo (atlas_id 2): sample heal_texture, discard transparent, apply color tint
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
// Equipment layers: discard bottom pixels to preserve health bar visibility
if in.health >= 0.99 && in.quad_uv.y > 0.85 && in.atlas_id < 0.5 { discard; }
var final_color = vec4<f32>(tex_color.rgb * in.color.rgb, tex_color.a);
```

Texture color is multiplied by the instance's tint color. This is how faction colors work — player faction (0) NPCs get job-based colors (pure green/blue/red/yellow), while all other factions get per-faction RGB tints from a 10-color saturated palette. Equipment layers (health >= 0.99) discard pixels in the health bar region so the body's health bar remains visible underneath.

**Damage flash** (white overlay, applied after color tinting):
```wgsl
if in.flash > 0.0 {
    final_color = vec4<f32>(mix(final_color.rgb, vec3(1.0, 1.0, 1.0), in.flash), final_color.a);
}
```

Flash intensity starts at 1.0 (full white) on damage hit and decays to 0.0 over ~0.2s (rate 5.0/s). Decay happens on CPU in `populate_gpu_state` via `flash_values` in `NpcGpuState`. The `mix()` function interpolates between the tinted sprite color and pure white.

## Render World Phases

The render pipeline runs in Bevy's render world after extract:

| Phase | System | Purpose |
|-------|--------|---------|
| Extract | `extract_npc_batch` | Despawn stale render world NpcBatch, then clone fresh from main world |
| Extract | `extract_npc_data` | Zero-clone GPU upload: hybrid writes (per-dirty-index for GPU-authoritative positions/arrivals, bulk for CPU-authoritative targets/speeds/factions/healths/flags) + unconditional visual/equip writes via `Extract<Res<T>>` |
| Extract | `extract_proj_batch` | Despawn stale render world ProjBatch, then clone fresh from main world |
| Extract | `extract_camera_state` | Build CameraState from Camera2d Transform + Projection + Window |
| PrepareResources | `prepare_npc_buffers` | Buffer creation + sentinel init (first frame), build misc instance buffer (farms/BHP), create bind group 2 |
| Extract | `extract_proj_data` | Zero-clone GPU upload: per-dirty-index compute writes + projectile instance buffer build via `Extract<Res<T>>` |
| PrepareBindGroups | `prepare_npc_texture_bind_group` | Create texture bind group from NpcSpriteTexture (char + world + heal + sleep + arrow) |
| PrepareBindGroups | `prepare_npc_camera_bind_group` | Create camera uniform bind group (includes npc_count from NpcGpuData) |
| Queue | `queue_npcs` | Add DrawMiscCommands (sort_key=0.4) + DrawNpcStorageCommands (sort_key=0.5) |
| Queue | `queue_projs` | Add DrawProjCommands (sort_key=1.0, above NPCs) |
| Render | `DrawMiscCommands` | SetItemPipeline → SetNpcTextureBindGroup → SetNpcCameraBindGroup → DrawMisc |
| Render | `DrawNpcStorageCommands` | SetItemPipeline → SetNpcTextureBindGroup → SetNpcCameraBindGroup → DrawNpcsStorage |
| Render | `DrawProjCommands` | SetItemPipeline → SetNpcTextureBindGroup → SetNpcCameraBindGroup → DrawProjs |

## RenderCommand Pattern

Bevy's RenderCommand trait defines GPU commands for drawing. Three command chains share one pipeline (specialized via `storage_mode`):

**NPC storage path** — body + 6 equipment layers via storage buffers:
```rust
type DrawNpcStorageCommands = (
    SetItemPipeline,           // Pipeline specialized with storage_mode=true
    SetNpcTextureBindGroup<0>, // Bind sprite textures at group 0
    SetNpcCameraBindGroup<1>,  // Bind camera uniform at group 1 (includes npc_count)
    DrawNpcsStorage,           // Set bind group 2 + quad, 7 draw_indexed calls
);
```

`DrawNpcsStorage::render()` sets bind group 2 (NPC storage buffers), then issues 7 `draw_indexed` calls with instance offset encoding: `(layer * npc_count)..((layer + 1) * npc_count)`. The shader derives `slot` and `layer` from `instance_index`. Hidden NPCs (`pos.x < -9000`) and empty equipment slots (`col < 0`) are culled by moving clip_position off-screen.

**Misc instance path** — farms, building HP bars:
```rust
type DrawMiscCommands = (
    SetItemPipeline,           // Pipeline specialized with storage_mode=false
    SetNpcTextureBindGroup<0>,
    SetNpcCameraBindGroup<1>,
    DrawMisc,                  // Set quad + instance buffer, single draw_indexed
);
```

`DrawMisc::render()` reads `NpcMiscBuffers` — a small `RawBufferVec<InstanceData>` (~100-200 entries) built each frame from `FarmStates` + `BuildingHpRender` + `MinerProgressRender`.

**Projectile instance path** — shares quad geometry, separate instance buffer:
```rust
type DrawProjCommands = (
    SetItemPipeline,           // Pipeline specialized with storage_mode=false
    SetNpcTextureBindGroup<0>,
    SetNpcCameraBindGroup<1>,
    DrawProjs,                 // Uses NPC quad + proj instance buffer
);
```

`DrawProjs::render()` reads `ProjRenderBuffers` — sharing static quad/index from `NpcRenderBuffers`. Faction-colored: blue for villagers, per-faction color for raiders.

## Camera

Bevy's Camera2d is the single source of truth — input systems write directly to `Transform` (position) and `Projection::Orthographic` (zoom via `scale`). No intermediate `CameraState` resource in the main world.

**Main world systems** (registered in `RenderPlugin::build`, Update schedule):
- `camera_pan_system`: WASD at 400px/s, speed scaled by 1/zoom via `ortho_zoom()` helper, writes `Transform` directly
- `camera_zoom_system`: scroll wheel zoom toward mouse cursor (factor 0.1, range 0.1–4.0), writes `Projection::Orthographic.scale` and `Transform` directly
- `click_to_select_system`: left click → screen-to-world via camera `Transform` + `Projection` → find nearest NPC within 20px from GPU_READ_STATE. If no NPC found, checks `WorldGrid` for a building at the clicked cell and sets `SelectedBuilding` (col, row, active). Guarded by `ctx.wants_pointer_input() || ctx.is_pointer_over_area()` to avoid stealing clicks from egui UI panels.

**Render world**: `extract_camera_state` (ExtractSchedule, `npc_render.rs`) reads the camera entity's `Transform`, `Projection`, and `Window` to build a `CameraState` resource in the render world. `prepare_npc_camera_bind_group` writes this to a `CameraUniform` `UniformBuffer` each frame (including `npc_count` from `NpcGpuData`), creating a bind group at group 1.

**Shader** (`npc_render.wgsl`): reads camera from uniform buffer:
```wgsl
struct Camera {
    pos: vec2<f32>,
    zoom: f32,
    npc_count: u32,     // used by vertex_npc for instance offset decoding
    viewport: vec2<f32>,
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
| Heal halo | `heal.png` | 16×16 | 1×1 (single sprite) | Healing overlay |
| Sleep icon | `sleep.png` | 16×16 | 1×1 (single sprite) | Sleep indicator overlay |
| Farmer Home | `house.png` | 32×32 | 1×1 (standalone) | Building tileset (External) |
| Archer Home | `barracks.png` | 32×32 | 1×1 (standalone) | Building tileset (External) |
| Waypoint | `waypoint.png` | 32×32 | 1×1 (standalone) | Building tileset (External) |

`SpriteAssets` holds handles for all loaded textures including the three external building sprites (`house_texture`, `barracks_texture`, `waypoint_texture`). NPC instanced rendering textures are shared via `NpcSpriteTexture` resource (`handle` for character, `world_handle` for world atlas, `heal_handle` for heal halo, `sleep_handle` for sleep icon), extracted to render world for quad bind group creation.

## Equipment Layers

Multi-layer rendering uses `NpcVisualUpload.equip_data` (packed by `build_visual_upload` each frame):

| Layer | Index | ECS Source | Equip Offset | Sentinel | Set By |
|-------|-------|-----------|-------------|----------|--------|
| Armor | 1 | `EquippedArmor` | idx*24+0 | col < 0 | build_visual_upload |
| Helmet | 2 | `EquippedHelmet` | idx*24+4 | col < 0 | build_visual_upload |
| Weapon | 3 | `EquippedWeapon` | idx*24+8 | col < 0 | build_visual_upload |
| Item | 4 | `CarriedItem` | idx*24+12 | col < 0 | build_visual_upload |
| Status | 5 | `Activity::Resting` | idx*24+16 | col < 0 | build_visual_upload |
| Healing | 6 | `Healing` marker | idx*24+20 | col < 0 | build_visual_upload |

All overlay layers are packed by `build_visual_upload` each frame from ECS components. Dead NPCs are skipped by the renderer (position < -9000). Each layer stores atlas_id alongside sprite coordinates so items can reference either atlas.

Current equipment assignments:
- **Guards**: Weapon (45, 6) + Helmet (28, 0) — character atlas
- **Raiders**: Weapon (45, 6) — character atlas
- **Carried food**: Item layer (24, 9) — world atlas, set when raider steals food, cleared on delivery

## World Tilemap (Terrain + Buildings)

Both terrain and buildings are rendered via Bevy's built-in `TilemapChunk` — two separate layer entities on the same grid (default 250×250, up to 1000×1000). Each layer is a single quad mesh where a fragment shader does per-pixel tile lookup from a `texture_2d_array` tileset. Currently one chunk per layer — a future optimization (see roadmap: Chunked Tilemap spec) will split into 32×32 chunks for off-screen culling.

| Layer | Z | Alpha | Content | Tileset |
|-------|---|-------|---------|---------|
| Terrain | -1.0 | Blend | Every cell filled (biome tiles) | 11 tiles (`TERRAIN_TILES`) |
| Buildings | -0.5 | Blend | `None` for empty, building tile where placed | 8 tiles (`BUILDING_TILES`) |

Both layers use `AlphaMode2d::Blend` so they render in the Transparent2d phase alongside NPCs (sort_key=0.5). Using `Opaque` would place terrain in the Opaque2d phase which renders *after* Transparent2d, causing terrain to draw over NPCs regardless of z-value.

**Slot Indicators** (`ui/mod.rs`): Building grid indicators use Sprite entities at z=-0.3 with a `SlotIndicator` marker component — not gizmos, because Bevy gizmos render in a separate pass after all Transparent2d items and can't be z-sorted with them. Green "+" crosshairs mark empty unlocked slots, dim bracket corners mark adjacent locked slots. Indicators are rebuilt when `TownGrids` or `WorldGrid` changes, and despawned on game cleanup.

**`TileSpec` enum** (`world.rs`): `Single(col, row)` for a single 16×16 sprite, `Quad([(col,row); 4])` for a 2×2 composite of four 16×16 sprites (TL, TR, BL, BR), or `External(usize)` for a standalone 32×32 PNG (index into extra images slice). Rock terrain uses Quad; Farm, Camp, and Tent buildings use Quad; FarmerHome, ArcherHome, and Waypoint use External (dedicated PNGs).

**`build_tileset(atlas, tiles, extra, images)`** (`world.rs`): Extracts tiles from the world atlas and builds a 32×32 `texture_2d_array`. `Single` tiles are nearest-neighbor 2× upscaled (each pixel → 2×2 block). `Quad` tiles blit four 16×16 sprites into quadrants. `External` tiles copy raw pixel data from extra images directly into the layer. Called twice — once with `TERRAIN_TILES` (11 tiles, no extras) and once with `BUILDING_TILES` (8 tiles, extras: house.png, barracks.png, waypoint.png).

**`Biome::tileset_index(cell_index)`**: Maps biome + cell position to terrain tileset array index (0-10). Grass alternates 0/1, Forest cycles 2-7, Water=8, Rock=9, Dirt=10.

**`Building::tileset_index()`**: Maps building variant to building tileset array index (0-7). Fountain=0, Bed=1, Waypoint=2, Farm=3, Camp=4, FarmerHome=5, ArcherHome=6, Tent=7.

**`TilemapSpawned`** resource (`render.rs`): Tracks whether the tilemap has been spawned. Uses a `Resource` (not `Local`) so that `game_cleanup_system` can reset it when leaving Playing state, enabling tilemap re-creation on re-entry.

**`spawn_world_tilemap`** system (`render.rs`, Update schedule): Runs once when WorldGrid is populated and world atlas is loaded. Terrain layer spawned with `TerrainChunk` marker, building layer with `BuildingChunk` marker — both for runtime tile sync queries. Terrain layer has all cells filled (opaque). Building layer has `None` for empty cells — the alpha blend mode makes empty cells transparent so terrain shows through.

**`TerrainChunk`** marker component (`render.rs`): Attached to the terrain TilemapChunk entity so `sync_terrain_tilemap` can query it for runtime terrain updates (e.g. slot unlock → Dirt).

**`BuildingChunk`** marker component (`render.rs`): Attached to the building TilemapChunk entity so `sync_building_tilemap` can query it.

**`sync_terrain_tilemap`** system (`render.rs`, Update schedule): Runs when `WorldGrid.is_changed()`. Rebuilds terrain `TilemapChunkTileData` from current grid cells. Needed because slot unlocking (player or AI) changes terrain biome to Dirt at runtime.

**`sync_building_tilemap`** system (`render.rs`, Update schedule): Runs when `WorldGrid.is_changed()`. Rebuilds building `TilemapChunkTileData` from current grid cells, so buildings placed or destroyed at runtime appear/disappear on the tilemap immediately. Bevy detects `Changed<TilemapChunkTileData>` and re-uploads to GPU.

## Known Issues

- **Health bar mode hardcoded**: Only "when damaged" mode (show when health < 99%). Off/always modes need a uniform or config resource.
- **MaxHealth hardcoded**: Health normalization divides by 100.0. When upgrades change MaxHealth, normalization must use per-NPC max.
- **Equipment sprite tuning**: Equipment sprites have updated atlas coordinates — use `npc-visuals` test scene to review layers. Food sprite is on world atlas (24,9).
- **Single sort key for all NPC layers**: All 7 NPC layers share sort_key=0.5 in Transparent2d phase. Layer ordering is correct within the single DrawNpcsStorage call (7 sequential draw_indexed), but layers can't interleave with other phase items.
- **Single tilemap chunk per layer**: At 1000×1000 (1M tiles), `command_buffer_generation_tasks` costs ~10ms because Bevy processes all tiles even when most are off-screen. Splitting into 32×32 chunks enables off-screen culling (see roadmap spec).
