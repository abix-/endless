# Rendering System

## Overview

Two rendering systems work together: **terrain and buildings** use Bevy's built-in `TilemapChunk` (two layers on the same grid — terrain opaque at z=-1, buildings alpha-blended at z=-0.5, zero per-frame CPU cost), while **NPCs, equipment, and projectiles** use a custom GPU instanced pipeline via Bevy's RenderCommand pattern in the Transparent2d phase. The instanced renderer uses 7 layers: NPC body (layer 0), 4 equipment layers (layers 1-4), and 2 visual indicator layers (status=5, healing=6), all drawn sequentially in a single DrawNpcs call. Projectiles use a separate draw call. Both character and world sprite atlases are bound simultaneously — per-instance `atlas_id` selects which atlas to sample.

Defined in: `rust/src/npc_render.rs`, `rust/src/render.rs`, `shaders/npc_render.wgsl`

## Why Custom Pipeline?

Bevy's built-in sprite renderer creates one entity per sprite. At 16K NPCs, that's 16K entities in the render world — the scheduling/extraction overhead dominates. The custom pipeline uses:

- **1 entity per batch** (NpcBatch, ProjBatch) instead of 16,384 entities
- **48 bytes/instance** (position + sprite + color + health + flash + scale + atlas_id) instead of ~80 bytes/entity
- **GPU compute data stays on GPU** — readback only for rendering
- **Multi-layer drawing** — body + up to 6 overlay layers (4 equipment + 2 visual indicators), each a separate `draw_indexed` call within one RenderCommand

## Data Flow

```
Main World                        Render World
───────────                       ────────────
NpcBufferWrites       ──ExtractResource──▶ NpcBufferWrites
NpcGpuData            ──ExtractResource──▶ NpcGpuData
Camera2d entity       ──extract_camera_state──▶ CameraState
NpcBatch entity       ──extract_npc_batch──▶ NpcBatch entity
                                      │
                                      ▼
                               prepare_npc_buffers
                               (build InstanceData[] for 5 layers)
                                      │
                                      ▼
                            prepare_npc_texture_bind_group
                            (dual atlas: char + world)
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
                               (build InstanceData[] from PROJ_POSITION_STATE)
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

Each instance is 48 bytes of per-instance data, shared across all layer types:

```rust
pub struct InstanceData {
    pub position: [f32; 2],  // world XY (8 bytes)
    pub sprite: [f32; 2],    // atlas cell col, row (8 bytes)
    pub color: [f32; 4],     // RGBA tint (16 bytes)
    pub health: f32,         // normalized 0.0-1.0 (4 bytes)
    pub flash: f32,          // damage flash 0.0-1.0 (4 bytes)
    pub scale: f32,          // world-space quad size (4 bytes)
    pub atlas_id: f32,       // 0.0=character, 1.0=world atlas (4 bytes)
}
```

Built each frame by `prepare_npc_buffers`. Seven layers are built per pass (terrain and buildings are handled by TilemapChunk — see World Tilemap section below):

**Layer 0 (body):**
- **Positions**: from GPU readback if available, else from CPU-side NpcBufferWrites
- **Sprites**: from `sprite_indices` (4 floats per NPC, uses first 2: col, row)
- **Colors**: from `colors` (4 floats per NPC: RGBA)
- **Health**: from `healths` (normalized by dividing by 100.0, clamped to 0-1)
- **Flash**: from `flash_values` (0.0-1.0, decays at 5.0/s in `populate_buffer_writes`)
- **Hidden NPCs** (position.x < -9000) are skipped

**Layers 1-4 (equipment: armor, helmet, weapon, item):**
- Same position as body (from readback)
- Sprite from `armor_sprites`/`helmet_sprites`/`weapon_sprites`/`item_sprites` (stride 2, col/row per NPC)
- Sentinel: col < 0 means unequipped → skip
- Color: job color tint (same RGBA as body) — guards' equipment renders blue, raiders' red
- Health: 1.0 (no health bar; shader discards bottom pixels for health >= 0.99)
- Flash: inherited from body (equipment flashes on hit)

**Layers 5-6 (visual indicators: status, healing):**
- Same position as body (from readback)
- Sprite from `status_sprites` (sleep icon) / `healing_sprites` (heal glow) (stride 2, col/row per NPC)
- Sentinel: col < 0 means inactive → skip
- Derived by `sync_visual_sprites` from `Activity::Resting` and `Healing` ECS components each frame
- Independent layers: NPC can show sleep AND healing simultaneously

**Projectiles**: health set to 1.0 (no health bar), flash set to 0.0

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

Two vertex buffer slots with different step modes:

| Slot | Step Mode | Data | Stride | Attributes |
|------|-----------|------|--------|------------|
| 0 | Vertex | Static quad (4 vertices) | 16B | @location(0) position, @location(1) uv |
| 1 | Instance | Per-instance data (N instances) | 48B | @location(2) position, @location(3) sprite, @location(4) color, @location(5) health, @location(6) flash, @location(7) scale, @location(8) atlas_id |

`VertexStepMode::Vertex` advances per-vertex (4 times per quad). `VertexStepMode::Instance` advances per-instance (once per NPC).

## Sprite Atlases

Both atlases are bound simultaneously at group 0 (bindings 0-3). Per-instance `atlas_id` selects which to sample.

| Atlas | Bindings | File | Size | Grid | Used By |
|-------|----------|------|------|------|---------|
| Character | 0-1 | `roguelikeChar_transparent.png` | 918×203 | 54×12 | NPCs, equipment, projectiles |
| World | 2-3 | `roguelikeSheet_transparent.png` | 968×526 | 57×31 | Terrain, buildings |

Both use 16px sprites with 1px margin (17px cells). The vertex shader selects atlas constants based on `atlas_id`:

```wgsl
if in.atlas_id < 0.5 {
    // Character atlas: 918×203
    out.uv = compute_uv(in.sprite_cell, CHAR_CELL, CHAR_SPRITE, CHAR_TEX_W, CHAR_TEX_H);
} else {
    // World atlas: 968×526
    out.uv = compute_uv(in.sprite_cell, WORLD_CELL, WORLD_SPRITE, WORLD_TEX_W, WORLD_TEX_H);
}
```

The fragment shader samples from the correct texture based on `atlas_id`. Health bars, damage flash, and equipment layer masking only apply to character atlas sprites (`atlas_id < 0.5`).

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

Texture color is multiplied by the instance's tint color. This is how faction colors work — raiders get per-faction RGB tints (10-color palette), while villagers get job-based colors. Equipment layers (health >= 0.99) discard pixels in the health bar region so the body's health bar remains visible underneath.

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
| Extract | `extract_npc_batch` | Despawn stale render world NpcBatch, then clone fresh from main world |
| Extract | `extract_proj_batch` | Despawn stale render world ProjBatch, then clone fresh from main world |
| Extract | `extract_camera_state` | Build CameraState from Camera2d Transform + Projection + Window |
| PrepareResources | `prepare_npc_buffers` | Build 7 layer buffers (body + 4 equipment + 2 indicators) |
| PrepareResources | `prepare_proj_buffers` | Build projectile instance buffer from PROJ_POSITION_STATE |
| PrepareBindGroups | `prepare_npc_texture_bind_group` | Create dual atlas bind group from NpcSpriteTexture (char + world) |
| PrepareBindGroups | `prepare_npc_camera_bind_group` | Create camera uniform bind group from CameraState |
| Queue | `queue_npcs` | Add NpcBatch to Transparent2d (sort_key=0.5) |
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

`DrawNpcs::render()` sets the shared vertex/index buffers, then iterates over all 7 `LayerBuffer`s in `NpcRenderBuffers.layers`, issuing a separate `draw_indexed` call per non-empty layer. Layers are drawn in order: body (0), armor (1), helmet (2), weapon (3), item (4), status (5), healing (6). If no layers have instances, it returns `Skip`.

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

Bevy's Camera2d is the single source of truth — input systems write directly to `Transform` (position) and `Projection::Orthographic` (zoom via `scale`). No intermediate `CameraState` resource in the main world.

**Main world systems** (registered in `RenderPlugin::build`, Update schedule):
- `camera_pan_system`: WASD at 400px/s, speed scaled by 1/zoom via `ortho_zoom()` helper, writes `Transform` directly
- `camera_zoom_system`: scroll wheel zoom toward mouse cursor (factor 0.1, range 0.1–4.0), writes `Projection::Orthographic.scale` and `Transform` directly
- `click_to_select_system`: left click → screen-to-world via camera `Transform` + `Projection` → find nearest NPC within 20px from GPU_READ_STATE. Guarded by `ctx.wants_pointer_input() || ctx.is_pointer_over_area()` to avoid stealing clicks from egui UI panels.

**Render world**: `extract_camera_state` (ExtractSchedule, `npc_render.rs`) reads the camera entity's `Transform`, `Projection`, and `Window` to build a `CameraState` resource in the render world. `prepare_npc_camera_bind_group` writes this to a `CameraUniform` `UniformBuffer` each frame, creating a bind group at group 1.

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

Both texture handles are shared via `NpcSpriteTexture` resource (`handle` for character, `world_handle` for world atlas), extracted to render world for dual bind group creation.

## Equipment Layers

Multi-layer rendering uses `NpcBufferWrites` fields for 6 overlay types:

| Layer | Index | NpcBufferWrites Field | Stride | Sentinel | Set By |
|-------|-------|----------------------|--------|----------|--------|
| Armor | 1 | `armor_sprites` | 2 (col, row) | col < 0 | SetEquipSprite |
| Helmet | 2 | `helmet_sprites` | 2 (col, row) | col < 0 | SetEquipSprite |
| Weapon | 3 | `weapon_sprites` | 2 (col, row) | col < 0 | SetEquipSprite |
| Item | 4 | `item_sprites` | 2 (col, row) | col < 0 | SetEquipSprite |
| Status | 5 | `status_sprites` | 2 (col, row) | col < 0 | SetSleeping |
| Healing | 6 | `healing_sprites` | 2 (col, row) | col < 0 | SetHealing |

Equipment layers (1-4) are set via `GpuUpdate::SetEquipSprite { idx, layer, col, row }`. Status and healing layers are set via dedicated `SetSleeping`/`SetHealing` messages that write sprite constants (`SLEEP_SPRITE`, `HEAL_SPRITE`) or clear to -1.0. At spawn, all layers are cleared to -1.0 (unequipped/inactive). Equipment is also cleared on death to prevent stale data on slot reuse.

Current equipment assignments:
- **Guards**: Weapon (0, 8) + Helmet (7, 9)
- **Raiders**: Weapon (0, 8)
- **Carried food**: Item layer set when raider steals food, cleared on delivery

## World Tilemap (Terrain + Buildings)

Both terrain and buildings are rendered via Bevy's built-in `TilemapChunk` — two separate layer entities on the same grid (default 250×250, up to 1000×1000). Each layer is a single quad mesh where a fragment shader does per-pixel tile lookup from a `texture_2d_array` tileset. Currently one chunk per layer — a future optimization (see roadmap: Chunked Tilemap spec) will split into 32×32 chunks for off-screen culling.

| Layer | Z | Alpha | Content | Tileset |
|-------|---|-------|---------|---------|
| Terrain | -1.0 | Blend | Every cell filled (biome tiles) | 11 tiles (`TERRAIN_TILES`) |
| Buildings | -0.5 | Blend | `None` for empty, building tile where placed | 5 tiles (`BUILDING_TILES`) |

Both layers use `AlphaMode2d::Blend` so they render in the Transparent2d phase alongside NPCs (sort_key=0.5). Using `Opaque` would place terrain in the Opaque2d phase which renders *after* Transparent2d, causing terrain to draw over NPCs regardless of z-value.

**Slot Indicators** (`ui/mod.rs`): Building grid indicators use Sprite entities at z=-0.3 with a `SlotIndicator` marker component — not gizmos, because Bevy gizmos render in a separate pass after all Transparent2d items and can't be z-sorted with them. Green "+" crosshairs mark empty unlocked slots, dim bracket corners mark adjacent locked slots. Indicators are rebuilt when `TownGrids` or `WorldGrid` changes, and despawned on game cleanup.

**`build_tileset(atlas, tiles, images)`** (`world.rs`): Generic function that extracts 16×16 tiles from the world atlas at specified (col, row) positions and builds a `texture_2d_array`. Called twice — once with `TERRAIN_TILES` (11 tiles: 2 grass, 6 forest, water, rock, dirt) and once with `BUILDING_TILES` (5 tiles: fountain, bed, guard post, farm, camp).

**`Biome::tileset_index(cell_index)`**: Maps biome + cell position to terrain tileset array index (0-10). Grass alternates 0/1, Forest cycles 2-7, Water=8, Rock=9, Dirt=10.

**`Building::tileset_index()`**: Maps building variant to building tileset array index (0-4). Fountain=0, Bed=1, GuardPost=2, Farm=3, Camp=4.

**`TilemapSpawned`** resource (`render.rs`): Tracks whether the tilemap has been spawned. Uses a `Resource` (not `Local`) so that `game_cleanup_system` can reset it when leaving Playing state, enabling tilemap re-creation on re-entry.

**`spawn_world_tilemap`** system (`render.rs`, Update schedule): Runs once when WorldGrid is populated and world atlas is loaded. Uses the shared `spawn_chunk()` helper for terrain. Building layer is spawned inline with a `BuildingChunk` marker component for runtime queries. Terrain layer has all cells filled (opaque). Building layer has `None` for empty cells — the alpha blend mode makes empty cells transparent so terrain shows through.

**`BuildingChunk`** marker component (`render.rs`): Attached to the building TilemapChunk entity so `sync_building_tilemap` can query it.

**`sync_building_tilemap`** system (`render.rs`, Update schedule): Runs when `WorldGrid.is_changed()`. Rebuilds `TilemapChunkTileData` from current grid cells, so buildings placed or destroyed at runtime appear/disappear on the tilemap immediately. Bevy detects `Changed<TilemapChunkTileData>` and re-uploads to GPU.

## Known Issues

- **Health bar mode hardcoded**: Only "when damaged" mode (show when health < 99%). Off/always modes need a uniform or config resource.
- **MaxHealth hardcoded**: Health normalization divides by 100.0. When upgrades change MaxHealth, normalization must use per-NPC max.
- **Equipment sprite placeholders**: Current equipment sprites (sword, helmet, food) use placeholder atlas coordinates — need tuning with sprite browser.
- **Single sort key for all layers**: All 7 NPC layers share sort_key=0.5 in Transparent2d phase. Layer ordering is correct within the single DrawNpcs call, but layers can't interleave with other phase items.
- **Single tilemap chunk per layer**: At 1000×1000 (1M tiles), `command_buffer_generation_tasks` costs ~10ms because Bevy processes all tiles even when most are off-screen. Splitting into 32×32 chunks enables off-screen culling (see roadmap spec).

## Rating: 9/10

Terrain and buildings rendered via two Bevy TilemapChunk layers (2 draw calls, zero per-frame CPU cost for 62K tiles + buildings). NPCs, equipment, and projectiles rendered through a custom instanced pipeline with dual atlas support. Per-instance data is compact (48 bytes). Fragment shader handles transparency, dual atlas sampling, faction color tinting, in-shader health bars (3-color, show-when-damaged), damage flash, and equipment layer health bar preservation. Camera controls work (WASD pan, scroll zoom, click-to-select). Projectiles render with GPU position readback and faction coloring. FPS counter overlay via egui.
