# Rendering System

## Overview

NPCs and projectiles are rendered via a custom GPU instanced pipeline using Bevy's RenderCommand pattern in the Transparent2d phase. Each layer draws in a single instanced call — one static quad shared by all instances, with per-instance position, sprite, and color data. World sprites (buildings, terrain) use Bevy's built-in sprite system.

Defined in: `rust/src/npc_render.rs`, `rust/src/render.rs`, `shaders/npc_render.wgsl`

## Why Custom Pipeline?

Bevy's built-in sprite renderer creates one entity per sprite. At 16K NPCs, that's 16K entities in the render world — the scheduling/extraction overhead dominates. The custom pipeline uses:

- **1 entity per layer** (NpcBatch, ProjBatch) instead of 16,384 entities
- **40 bytes/instance** (position + sprite + color + health + flash) instead of ~80 bytes/entity
- **GPU compute data stays on GPU** — readback only for rendering
- **One draw call per layer** — `draw_indexed(0..6, 0, 0..instance_count)`

## Data Flow

```
Main World                        Render World
───────────                       ────────────
NpcBufferWrites ──ExtractResource──▶ NpcBufferWrites
NpcGpuData      ──ExtractResource──▶ NpcGpuData
CameraState     ──ExtractResource──▶ CameraState
NpcBatch entity ──extract_npc_batch──▶ NpcBatch entity
                                      │
                                      ▼
                               prepare_npc_buffers
                               (build NpcInstanceData[])
                                      │
                                      ▼
                            prepare_npc_texture_bind_group
                            prepare_npc_camera_bind_group
                                      │
                                      ▼
                                  queue_npcs
                               (add to Transparent2d)
                                      │
                                      ▼
                              SetItemPipeline
                              SetNpcTextureBindGroup<0>
                              SetNpcCameraBindGroup<1>
                              DrawNpcs

ProjBufferWrites ──ExtractResource──▶ ProjBufferWrites
ProjGpuData      ──ExtractResource──▶ ProjGpuData
ProjBatch entity ──extract_proj_batch──▶ ProjBatch entity
                                      │
                                      ▼
                               prepare_proj_buffers
                               (build NpcInstanceData[] from PROJ_POSITION_STATE)
                                      │
                                      ▼
                                  queue_projs
                               (add to Transparent2d, sort_key=1.0)
                                      │
                                      ▼
                              SetItemPipeline
                              SetNpcTextureBindGroup<0>
                              SetNpcCameraBindGroup<1>
                              DrawProjs
```

## Instance Data

Each NPC is 40 bytes of per-instance data:

```rust
pub struct NpcInstanceData {
    pub position: [f32; 2],  // world XY (8 bytes)
    pub sprite: [f32; 2],    // atlas cell col, row (8 bytes)
    pub color: [f32; 4],     // RGBA tint (16 bytes)
    pub health: f32,         // normalized 0.0-1.0 (4 bytes)
    pub flash: f32,          // damage flash 0.0-1.0 (4 bytes)
}
```

Built each frame by `prepare_npc_buffers` from `NpcBufferWrites`:
- **Positions**: from GPU readback if available, else from CPU-side NpcBufferWrites
- **Sprites**: from `sprite_indices` (4 floats per NPC, uses first 2: col, row)
- **Colors**: from `colors` (4 floats per NPC: RGBA)
- **Health**: from `healths` (normalized by dividing by 100.0, clamped to 0-1)
- **Flash**: from `flash_values` (0.0-1.0, decays at 5.0/s in `populate_buffer_writes`)
- **Hidden NPCs** (position.x < -9000) are skipped
- **Projectiles**: health set to 1.0 (no health bar), flash set to 0.0

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

The vertex shader scales the unit quad by `SPRITE_SIZE` (16 world units, matching 16px atlas cells) and offsets by instance position.

## Vertex Buffers

Two vertex buffer slots with different step modes:

| Slot | Step Mode | Data | Stride | Attributes |
|------|-----------|------|--------|------------|
| 0 | Vertex | Static quad (4 vertices) | 16B | @location(0) position, @location(1) uv |
| 1 | Instance | Per-NPC data (N instances) | 40B | @location(2) position, @location(3) sprite, @location(4) color, @location(5) health, @location(6) flash |

`VertexStepMode::Vertex` advances per-vertex (4 times per quad). `VertexStepMode::Instance` advances per-instance (once per NPC).

## Sprite Atlas

All NPC sprites come from `roguelikeChar_transparent.png` (918×203 pixels, 54 cols × 12 rows, 16px sprites with 1px margin).

The shader converts (col, row) to UV coordinates:

```wgsl
const CELL_SIZE: f32 = 17.0;      // 16px sprite + 1px margin
const SPRITE_TEX_SIZE: f32 = 16.0;
const TEXTURE_WIDTH: f32 = 918.0;
const TEXTURE_HEIGHT: f32 = 203.0;

// In vertex shader:
let pixel_x = sprite_cell.x * CELL_SIZE + quad_uv.x * SPRITE_TEX_SIZE;
let pixel_y = sprite_cell.y * CELL_SIZE + quad_uv.y * SPRITE_TEX_SIZE;
out.uv = vec2<f32>(pixel_x / TEXTURE_WIDTH, pixel_y / TEXTURE_HEIGHT);
```

Each quad corner's UV (`quad_uv` from 0,0 to 1,1) maps to a 16×16 pixel region within the sprite's cell. The alpha channel handles non-rectangular shapes — the fragment shader discards pixels with `alpha < 0.1`.

Job sprite assignments (from constants.rs):
- Farmer: (1, 6)
- Guard: (0, 11)
- Raider: (0, 6)
- Fighter: (7, 0)

## Fragment Shader

The fragment shader handles both health bar rendering and sprite rendering. The vertex shader passes two UV sets: `uv` (atlas-transformed for texture sampling) and `quad_uv` (raw 0-1 within the sprite quad for health bar positioning).

**Health bar** (bottom 15% of sprite, show-when-damaged mode):
```wgsl
if in.quad_uv.y > 0.85 && in.health < 0.99 {
    // Dark grey background, filled portion colored by health level
    // Green (>50%), Yellow (>25%), Red (≤25%)
}
```

**Sprite rendering** (remaining 85%):
```wgsl
let tex_color = textureSample(sprite_texture, sprite_sampler, in.uv);
if tex_color.a < 0.1 {
    discard;  // transparent pixels → not drawn
}
var final_color = vec4<f32>(tex_color.rgb * in.color.rgb, tex_color.a);
```

Texture color is multiplied by the instance's tint color. This is how faction colors work — raiders get per-faction RGB tints (10-color palette), while villagers get job-based colors.

**Damage flash** (white overlay, applied after color tinting):
```wgsl
if in.flash > 0.0 {
    final_color = vec4<f32>(mix(final_color.rgb, vec3(1.0, 1.0, 1.0), in.flash), final_color.a);
}
```

Flash intensity starts at 1.0 (full white) on damage hit and decays to 0.0 over ~0.2s (rate 5.0/s). Decay happens on CPU in `populate_buffer_writes` via `flash_values` in `NpcBufferWrites`. The `mix()` function interpolates between the tinted sprite color and pure white.

## Render World Phases

The render pipeline runs in Bevy's render world after extract:

| Phase | System | Purpose |
|-------|--------|---------|
| Extract | `extract_npc_batch` | Clone NpcBatch entity to render world |
| Extract | `extract_proj_batch` | Clone ProjBatch entity to render world |
| PrepareResources | `prepare_npc_buffers` | Build NPC instance buffer from GPU_READ_STATE |
| PrepareResources | `prepare_proj_buffers` | Build projectile instance buffer from PROJ_POSITION_STATE |
| PrepareBindGroups | `prepare_npc_texture_bind_group` | Create texture bind group from NpcSpriteTexture |
| PrepareBindGroups | `prepare_npc_camera_bind_group` | Create camera uniform bind group from CameraState |
| Queue | `queue_npcs` | Add NpcBatch to Transparent2d (sort_key=0.0) |
| Queue | `queue_projs` | Add ProjBatch to Transparent2d (sort_key=1.0, above NPCs) |
| Render | `DrawNpcCommands` | SetItemPipeline → SetNpcTextureBindGroup → SetNpcCameraBindGroup → DrawNpcs |
| Render | `DrawProjCommands` | SetItemPipeline → SetNpcTextureBindGroup → SetNpcCameraBindGroup → DrawProjs |

## RenderCommand Pattern

Bevy's RenderCommand trait defines the GPU commands for drawing. The NPC pipeline chains three commands:

```rust
type DrawNpcCommands = (
    SetItemPipeline,           // Bind the NPC render pipeline
    SetNpcTextureBindGroup<0>, // Bind sprite texture at group 0
    SetNpcCameraBindGroup<1>,  // Bind camera uniform at group 1
    DrawNpcs,                  // Set vertex/index buffers, draw_indexed
);
```

`DrawNpcs::render()` sets both vertex buffer slots and issues the instanced draw call. If `instance_count == 0` or the instance buffer is empty, it returns `Skip`.

Projectiles reuse the same pipeline, shader, and bind groups with a separate instance buffer:

```rust
type DrawProjCommands = (
    SetItemPipeline,           // Same NPC pipeline
    SetNpcTextureBindGroup<0>, // Same sprite texture
    SetNpcCameraBindGroup<1>,  // Same camera uniform
    DrawProjs,                 // Uses NPC quad + proj instance buffer
);
```

`DrawProjs::render()` reads `(NpcRenderBuffers, ProjRenderBuffers)` — sharing the static quad/index buffers from NPCs but using its own instance buffer. Faction-colored: blue for villagers, red for raiders.

## Camera

`render.rs` manages camera state via the `CameraState` resource (position, zoom, viewport) with `ExtractResource` for automatic main→render world cloning each frame.

**Main world systems** (registered in `RenderPlugin::build`, Update schedule):
- `camera_pan_system`: WASD at 400px/s, speed scaled by 1/zoom for consistent screen-space feel
- `camera_zoom_system`: scroll wheel zoom toward mouse cursor (factor 0.1, range 0.1–4.0), uses `AccumulatedMouseScroll` resource
- `camera_viewport_sync`: keeps viewport in sync with window size
- `camera_transform_sync`: syncs CameraState → Bevy Camera2d Transform (position only)
- `click_to_select_system`: left click → screen-to-world → find nearest NPC within 20px from GPU_READ_STATE

**Render world**: `prepare_npc_camera_bind_group` writes `CameraUniform` (camera_pos, zoom, viewport) to a `UniformBuffer` each frame, creating a bind group at group 1.

**Shader** (`npc_render.wgsl`): reads camera from uniform buffer:
```wgsl
struct Camera {
    pos: vec2<f32>,
    zoom: f32,
    _pad: f32,
    viewport: vec2<f32>,
};
@group(1) @binding(0) var<uniform> camera: Camera;

// Vertex shader:
let offset = (world_pos - camera.pos) * camera.zoom;
let ndc = offset / (camera.viewport * 0.5);
```

## Texture Loading

`render.rs` loads both sprite sheets at startup and shares the character texture with the instanced pipeline:

| Sheet | File | Size | Grid | Used By |
|-------|------|------|------|---------|
| Characters | `roguelikeChar_transparent.png` | 918×203 | 54×12 (16px + 1px margin) | NPC instanced rendering |
| World | `roguelikeSheet_transparent.png` | 968×526 | 57×31 (16px + 1px margin) | Building/terrain sprites |

The character texture handle is shared via `NpcSpriteTexture` resource (extracted to render world for bind group creation).

## Known Issues

- **No building rendering**: World sprite sheet is loaded but not used for instanced building rendering.
- **No carried item overlay**: CarriedItem component exists but no visual rendering.
- **Sprite texture in debug mode**: Fragment shader samples textures and applies color tint, but some visual artifacts may exist from atlas margin bleed.
- **Health bar mode hardcoded**: Only "when damaged" mode (show when health < 99%). Off/always modes need a uniform or config resource.
- **MaxHealth hardcoded**: Health normalization divides by 100.0. When upgrades change MaxHealth, normalization must use per-NPC max.

## Rating: 8/10

Custom instanced pipeline with two draw layers (NPCs + projectiles). Per-instance data is compact (40 bytes). Fragment shader handles transparency, faction color tinting, in-shader health bars (3-color, show-when-damaged), and damage flash (white overlay that fades out over ~0.2s). Camera controls work (WASD pan, scroll zoom, click-to-select). Projectiles render with GPU position readback and faction coloring. However: no building/overlay rendering, and positions fall back to CPU-side data when readback isn't available (one-frame latency at best).
