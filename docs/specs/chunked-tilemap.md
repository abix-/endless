# Chunked Tilemap

Stage 15. Implementation spec for splitting the world tilemap into frustum-culled chunks.

The world tilemap is spawned as one giant `TilemapChunk` entity per layer (terrain + buildings). At 250×250 that's 62,500 tiles per layer, all processed every frame for draw command generation even when most are off-screen. At 1000×1000 it's 1M tiles. Bevy can only skip entities whose bounding box is fully off-screen — one entity = no culling.

**Fix:** split into 32×32 tile chunks (Factorio-style). 250×250 → 8×8 = 64 chunks/layer. 1000×1000 → 32×32 = 1,024 chunks/layer. At typical zoom, only ~4-6 chunks are visible, so draw command generation drops from O(all tiles) to O(visible tiles).

**File: `rust/src/render.rs`**

Constants:
```rust
const CHUNK_SIZE: usize = 32;
```

Components — add grid origin to `BuildingChunk` (for sync):
```rust
#[derive(Component)]
pub struct BuildingChunk {
    pub origin_x: usize,
    pub origin_y: usize,
    pub chunk_w: usize,  // may be < 32 for edge chunks
    pub chunk_h: usize,
}
```

`spawn_world_tilemap` — replace single chunk spawn with nested loop:
```
for chunk_y in (0..grid.height).step_by(CHUNK_SIZE)
  for chunk_x in (0..grid.width).step_by(CHUNK_SIZE)
    cw = min(CHUNK_SIZE, grid.width - chunk_x)
    ch = min(CHUNK_SIZE, grid.height - chunk_y)
    // Extract tile data: iterate ly in 0..ch, lx in 0..cw
    //   grid index = (chunk_y + ly) * grid.width + (chunk_x + lx)
    // Terrain: TileData::from_tileset_index(cell.terrain.tileset_index(gi))
    // Building: cell.building.map(|b| TileData::from_tileset_index(b.tileset_index()))
    // Transform center = ((chunk_x + cw/2) * cell_size, (chunk_y + ch/2) * cell_size, z)
    // Spawn TilemapChunk with chunk_size = UVec2(cw, ch)
    // Building chunks get BuildingChunk { origin_x, origin_y, chunk_w, chunk_h }
```

`sync_building_tilemap` — each chunk re-reads only its sub-region:
```rust
fn sync_building_tilemap(
    grid: Res<WorldGrid>,
    mut chunks: Query<(&mut TilemapChunkTileData, &BuildingChunk)>,
) {
    if !grid.is_changed() || grid.width == 0 { return; }
    for (mut tile_data, chunk) in chunks.iter_mut() {
        for ly in 0..chunk.chunk_h {
            for lx in 0..chunk.chunk_w {
                let gi = (chunk.origin_y + ly) * grid.width + (chunk.origin_x + lx);
                let li = ly * chunk.chunk_w + lx;
                tile_data.0[li] = grid.cells[gi].building.as_ref()
                    .map(|b| TileData::from_tileset_index(b.tileset_index()));
            }
        }
    }
}
```

Cleanup (`ui/mod.rs:500`): already queries `Entity, With<TilemapChunk>` and despawns all — works unchanged with multiple chunks.

`spawn_chunk` helper: can be removed or inlined — no longer needed as a separate function since the loop body handles everything.

**Tileset handles:** `build_tileset()` returns a `Handle<Image>`. Clone it for each chunk — Bevy ref-counts texture assets, so all chunks share the same GPU texture.

**Verification:**
1. Build and run, pan camera — no gaps or offset errors at chunk boundaries
2. Place a building — appears correctly (sync still works)
3. Zoom out fully — all chunks visible, slight FPS drop expected vs close zoom
4. Tracy: `command_buffer_generation_tasks` should drop from ~10ms to ~1ms at default zoom
5. New game / restart — chunks despawn and respawn correctly
