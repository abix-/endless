//! Auto-tile logic and atlas construction for buildings and terrain.
//!
//! - autotile_variant / update_autotile_around / update_all_autotile
//!   compute 4-neighbor sprite variants for roads and walls.
//! - build_tile_strip / build_tileset / build_building_atlas / build_extras_atlas
//!   pack sprite sheets into GPU texture arrays.

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::resources::EntityMap;

use super::{ATLAS_CELL, BuildingKind, CELL, SPRITE_SIZE, WorldGrid};

// ============================================================================
// AUTO-TILE
// ============================================================================

/// Check if a grid cell contains a building of the given kind.
/// For roads, matches any road tier so different tiers auto-connect.
fn is_kind_at(entity_map: &EntityMap, col: usize, row: usize, kind: BuildingKind) -> bool {
    entity_map
        .get_at_grid(col as i32, row as i32)
        .is_some_and(|inst| {
            if kind.is_road() {
                inst.kind.is_road()
            } else if kind.is_wall_like() {
                inst.kind.is_wall_like()
            } else {
                inst.kind == kind
            }
        })
}

/// Compute auto-tile variant (0-10) for a building at grid (col, row).
/// Uses 4-neighbor NSEW matching. Works for any autotile-enabled building kind.
/// Roads of different tiers connect to each other via is_kind_at.
pub fn autotile_variant(entity_map: &EntityMap, col: usize, row: usize, kind: BuildingKind) -> u16 {
    let n = row > 0 && is_kind_at(entity_map, col, row - 1, kind);
    let s = is_kind_at(entity_map, col, row + 1, kind);
    let e = is_kind_at(entity_map, col + 1, row, kind);
    let w = col > 0 && is_kind_at(entity_map, col - 1, row, kind);
    use crate::constants::*;
    match (n, s, e, w) {
        (false, false, true, true) => AUTOTILE_EW,
        (false, false, true, false) => AUTOTILE_EW,
        (false, false, false, true) => AUTOTILE_EW,
        (true, true, false, false) => AUTOTILE_NS,
        (true, false, false, false) => AUTOTILE_NS,
        (false, true, false, false) => AUTOTILE_NS,
        (true, false, true, false) => AUTOTILE_TR,
        (true, false, false, true) => AUTOTILE_TL,
        (false, true, false, true) => AUTOTILE_BL,
        (false, true, true, false) => AUTOTILE_BR,
        (true, false, true, true) => AUTOTILE_T_OPEN_N,
        (true, true, true, false) => AUTOTILE_T_OPEN_W,
        (false, true, true, true) => AUTOTILE_T_OPEN_S,
        (true, true, false, true) => AUTOTILE_T_OPEN_E,
        (true, true, true, true) => AUTOTILE_CROSS,
        _ => AUTOTILE_EW,
    }
}

/// Recompute auto-tile sprites for the building at (col, row) and its 4 neighbors.
pub fn update_autotile_around(
    grid: &WorldGrid,
    entity_map: &EntityMap,
    col: usize,
    row: usize,
    kind: BuildingKind,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) {
    let offsets: [(i32, i32); 5] = [(0, 0), (0, -1), (0, 1), (1, 0), (-1, 0)];
    for (dc, dr) in offsets {
        let c = col as i32 + dc;
        let r = row as i32 + dr;
        if c < 0 || r < 0 {
            continue;
        }
        let (c, r) = (c as usize, r as usize);
        // For roads, update any road-tier neighbor; use the neighbor's actual kind for sprite
        let pos = grid.grid_to_world(c, r);
        let Some(inst) = entity_map.find_by_position(pos) else {
            continue;
        };
        let neighbor_kind = inst.kind;
        if kind.is_road() {
            if !neighbor_kind.is_road() {
                continue;
            }
        } else if kind.is_wall_like() {
            if !neighbor_kind.is_wall_like() {
                continue;
            }
        } else if neighbor_kind != kind {
            continue;
        }
        let variant = autotile_variant(entity_map, c, r, neighbor_kind);
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame {
            idx: inst.slot,
            col: crate::constants::autotile_col(neighbor_kind, variant),
            row: 0.0,
            atlas: crate::constants::ATLAS_BUILDING,
        }));
    }
}

/// Set auto-tile sprites for all buildings of a given kind. Call after building instances are created.
pub fn update_all_autotile(
    grid: &WorldGrid,
    entity_map: &EntityMap,
    kind: BuildingKind,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) {
    for inst in entity_map.iter_kind(kind) {
        let (gc, gr) = grid.world_to_grid(inst.position);
        let variant = autotile_variant(entity_map, gc, gr, kind);
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame {
            idx: inst.slot,
            col: crate::constants::autotile_col(kind, variant),
            row: 0.0,
            atlas: crate::constants::ATLAS_BUILDING,
        }));
    }
}

// ============================================================================
// TILE STRIP / ATLAS CONSTRUCTION
// ============================================================================

/// Build the BUILDING_TILES array from the registry (for atlas construction).
pub fn building_tiles() -> Vec<crate::constants::TileSpec> {
    crate::constants::BUILDING_REGISTRY
        .iter()
        .map(|d| d.tile)
        .collect()
}

/// Composite tiles into a vertical strip buffer (ATLAS_CELL x ATLAS_CELL*layers).
/// Core logic shared by tilemap tileset and building atlas.
/// `bases` provides an optional base tile per layer -- layers with a base are pre-filled
/// and subsequent sprites are alpha-composited on top (transparent pixels keep the base).
/// Pass an empty slice for no bases (building atlas).
fn build_tile_strip(
    atlas: &Image,
    tiles: &[crate::constants::TileSpec],
    extra: &[&Image],
    bases: &[Option<(u32, u32)>],
) -> (Vec<u8>, u32) {
    let sprite = SPRITE_SIZE as u32; // 16 (source texel size)
    let out_size = ATLAS_CELL; // 64
    let scale = out_size / sprite; // 4x upscale from 16px source
    let half = out_size / 2; // 32 — each quadrant in a Quad tile
    let cell_size = CELL as u32; // 17 (16px + 1px margin in source sheet)
    let atlas_width = atlas.width();
    let layers = tiles.len() as u32;
    let layer_bytes = (out_size * out_size * 4) as usize;

    let mut data = vec![0u8; layer_bytes * layers as usize];
    let atlas_data = atlas.data.as_ref().expect("atlas image has no data");

    // Pre-fill layers that have a base tile so decorations composite
    // over terrain instead of showing a black/transparent background.
    // Cache rendered base tiles to avoid redundant blitting.
    let mut base_cache: std::collections::HashMap<(u32, u32), Vec<u8>> =
        std::collections::HashMap::new();
    for (l, base_opt) in bases.iter().enumerate() {
        if let Some(&(base_col, base_row)) = base_opt.as_ref() {
            let base_layer = base_cache.entry((base_col, base_row)).or_insert_with(|| {
                let src_x = base_col * cell_size;
                let src_y = base_row * cell_size;
                let mut buf = vec![0u8; layer_bytes];
                for ty in 0..sprite {
                    for tx in 0..sprite {
                        let si = ((src_y + ty) * atlas_width + (src_x + tx)) as usize * 4;
                        for oy in 0..scale {
                            for ox in 0..scale {
                                let di =
                                    ((ty * scale + oy) * out_size + (tx * scale + ox)) as usize * 4;
                                buf[di..di + 4].copy_from_slice(&atlas_data[si..si + 4]);
                            }
                        }
                    }
                }
                buf
            });
            let off = l * layer_bytes;
            data[off..off + layer_bytes].copy_from_slice(base_layer);
        }
    }

    // Blit a 16x16 source sprite with 2x upscale into a 32x32 quadrant at (dx, dy).
    // When `skip_transparent` is true, transparent source pixels preserve the base.
    let blit_2x = |data: &mut [u8],
                   layer: u32,
                   col: u32,
                   row: u32,
                   dx: u32,
                   dy: u32,
                   skip_transparent: bool| {
        let src_x = col * cell_size;
        let src_y = row * cell_size;
        for ty in 0..sprite {
            for tx in 0..sprite {
                let si = ((src_y + ty) * atlas_width + (src_x + tx)) as usize * 4;
                if skip_transparent && atlas_data[si + 3] == 0 {
                    continue;
                }
                for oy in 0..2u32 {
                    for ox in 0..2u32 {
                        let di = (layer * out_size * out_size
                            + (dy + ty * 2 + oy) * out_size
                            + (dx + tx * 2 + ox)) as usize
                            * 4;
                        data[di..di + 4].copy_from_slice(&atlas_data[si..si + 4]);
                    }
                }
            }
        }
    };

    let mut ext_counter = 0usize;
    for (layer, spec) in tiles.iter().enumerate() {
        let l = layer as u32;
        match *spec {
            crate::constants::TileSpec::Pick(variants) => {
                // Bake the first variant as the base layer; extras are appended later.
                let (col, row) = variants.first().copied().unwrap_or((0, 0));
                let src_x = col * cell_size;
                let src_y = row * cell_size;
                for ty in 0..sprite {
                    for tx in 0..sprite {
                        let si = ((src_y + ty) * atlas_width + (src_x + tx)) as usize * 4;
                        for oy in 0..scale {
                            for ox in 0..scale {
                                let di = (l * out_size * out_size
                                    + (ty * scale + oy) * out_size
                                    + (tx * scale + ox))
                                    as usize
                                    * 4;
                                data[di..di + 4].copy_from_slice(&atlas_data[si..si + 4]);
                            }
                        }
                    }
                }
            }
            crate::constants::TileSpec::Single(col, row) => {
                // Nearest-neighbor 4x upscale: each 16px src pixel -> 4x4 dst pixels
                let src_x = col * cell_size;
                let src_y = row * cell_size;
                for ty in 0..sprite {
                    for tx in 0..sprite {
                        let si = ((src_y + ty) * atlas_width + (src_x + tx)) as usize * 4;
                        let skip = bases.get(layer).is_some_and(|b| b.is_some());
                        if skip && atlas_data[si + 3] == 0 {
                            continue;
                        }
                        for oy in 0..scale {
                            for ox in 0..scale {
                                let di = (l * out_size * out_size
                                    + (ty * scale + oy) * out_size
                                    + (tx * scale + ox))
                                    as usize
                                    * 4;
                                data[di..di + 4].copy_from_slice(&atlas_data[si..si + 4]);
                            }
                        }
                    }
                }
            }
            crate::constants::TileSpec::Quad(q) => {
                let skip = bases.get(layer).is_some_and(|b| b.is_some());
                // Each 16px quadrant is 2x upscaled to 32px, filling the 64px cell
                blit_2x(&mut data, l, q[0].0, q[0].1, 0, 0, skip); // TL
                blit_2x(&mut data, l, q[1].0, q[1].1, half, 0, skip); // TR
                blit_2x(&mut data, l, q[2].0, q[2].1, 0, half, skip); // BL
                blit_2x(&mut data, l, q[3].0, q[3].1, half, half, skip); // BR
            }
            crate::constants::TileSpec::External(_path) => {
                let Some(ext) = extra.get(ext_counter).copied() else {
                    continue;
                };
                ext_counter += 1;
                let ext_data = ext.data.as_ref().expect("external image has no data");
                let layer_offset = (l * out_size * out_size * 4) as usize;
                let ext_w = ext.width();
                let ext_h = ext.height();

                if ext_w == out_size && ext_h == out_size {
                    // Native ATLAS_CELL size — direct blit
                    let layer_bytes = (out_size * out_size * 4) as usize;
                    if ext_data.len() >= layer_bytes {
                        data[layer_offset..layer_offset + layer_bytes]
                            .copy_from_slice(&ext_data[..layer_bytes]);
                    }
                } else {
                    // Scale to fit (handles both old 32px art and any other size)
                    let src_w = ext_w.max(1);
                    let src_h = ext_h.max(1);
                    for y in 0..out_size {
                        for x in 0..out_size {
                            let sx = (x * src_w / out_size).min(src_w - 1);
                            let sy = (y * src_h / out_size).min(src_h - 1);
                            let si = ((sy * src_w + sx) * 4) as usize;
                            let di = (layer_offset as u32 + ((y * out_size + x) * 4)) as usize;
                            if si + 4 <= ext_data.len() && di + 4 <= data.len() {
                                data[di..di + 4].copy_from_slice(&ext_data[si..si + 4]);
                            }
                        }
                    }
                }
            }
        }
    }

    (data, layers)
}

/// Tilemap: strip -> texture_2d_array (for TilemapChunk).
pub fn build_tileset(
    atlas: &Image,
    tiles: &[crate::constants::TileSpec],
    extra: &[&Image],
    images: &mut Assets<Image>,
) -> Handle<Image> {
    let (data, layers) = build_tile_strip(atlas, tiles, extra, &super::TERRAIN_BASES);
    let out_size = ATLAS_CELL;
    let mut image = Image::new(
        Extent3d {
            width: out_size,
            height: out_size * layers,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        Default::default(),
    );
    image
        .reinterpret_stacked_2d_as_array(layers)
        .expect("tileset reinterpret failed");
    images.add(image)
}

/// Rotate NxN RGBA pixel data 90 degrees clockwise. Load-time only.
fn rotate_90_cw(src: &[u8], size: u32) -> Vec<u8> {
    let mut dst = vec![0u8; src.len()];
    for y in 0..size {
        for x in 0..size {
            let si = ((y * size + x) * 4) as usize;
            let di = ((x * size + (size - 1 - y)) * 4) as usize;
            dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    dst
}

/// Extract a `src_size` sprite from a wider strip at pixel offset `src_x`,
/// then nearest-neighbor upscale to ATLAS_CELL. Pass src_size=32 for existing art,
/// src_size=64 (==ATLAS_CELL) for new native-res art (no upscale needed).
pub fn extract_sprite(img: &Image, src_x: u32, src_size: u32) -> Vec<u8> {
    let iw = img.width();
    let data = img.data.as_ref().expect("image has no data");
    let dst = ATLAS_CELL;
    let mut out = vec![0u8; (dst * dst * 4) as usize];
    for dy in 0..dst {
        for dx in 0..dst {
            // Map dst pixel back to source pixel (nearest-neighbor)
            let sx = src_x + dx * src_size / dst;
            let sy = dy * src_size / dst;
            if sy < img.height() {
                let si = ((sy * iw + sx) * 4) as usize;
                let di = ((dy * dst + dx) * 4) as usize;
                if si + 4 <= data.len() {
                    out[di..di + 4].copy_from_slice(&data[si..si + 4]);
                }
            }
        }
    }
    out
}

/// Building atlas: strip as texture_2d (for NPC instanced shader).
/// Appends auto-tile variant layers for all autotile-enabled building kinds.
pub fn build_building_atlas(
    atlas: &Image,
    tiles: &[crate::constants::TileSpec],
    extra: &[&Image],
    images: &mut Assets<Image>,
) -> Handle<Image> {
    let (mut data, base_layers) = build_tile_strip(atlas, tiles, extra, &[]);
    let out_size = ATLAS_CELL;
    let layer_bytes = (out_size * out_size * 4) as usize;

    // For each autotile-enabled building, find its External sprite strip,
    // extract/rotate variants, overwrite the base layer, and append 10 extra layers.
    let mut extra_count = 0u32;
    for def in crate::constants::BUILDING_REGISTRY {
        if !def.autotile {
            continue;
        }
        // Find this kind's External image index in the extra slice
        let ext_idx = {
            let mut idx = 0usize;
            let mut found = None;
            for d in crate::constants::BUILDING_REGISTRY {
                if d.kind == def.kind {
                    if matches!(d.tile, crate::constants::TileSpec::External(_)) {
                        found = Some(idx);
                    }
                    break;
                }
                if matches!(d.tile, crate::constants::TileSpec::External(_)) {
                    idx += 1;
                }
            }
            found
        };

        let Some(ext_idx) = ext_idx else { continue };
        let Some(strip_img) = extra.get(ext_idx) else {
            continue;
        };

        // Extract source sprites: E-W at x=0, BR corner at x=66 (32px art with 1px+1px gaps)
        let ew_sprite = extract_sprite(strip_img, 0, 32);
        let br_sprite = extract_sprite(strip_img, 66, 32);

        // Overwrite base layer with clean E-W sprite (strip was stretched)
        let kind_base = crate::constants::tileset_index(def.kind) as usize;
        let base_offset = kind_base * layer_bytes;
        if base_offset + layer_bytes <= data.len() {
            data[base_offset..base_offset + layer_bytes].copy_from_slice(&ew_sprite);
        }

        // Generate rotated variants
        let ns_sprite = rotate_90_cw(&ew_sprite, out_size);
        let bl_sprite = rotate_90_cw(&br_sprite, out_size);
        let tl_sprite = rotate_90_cw(&bl_sprite, out_size);
        let tr_sprite = rotate_90_cw(&tl_sprite, out_size);

        // Extract junction/cross at x=33, T-junction at x=99 (32px art with 1px gaps)
        let cross_sprite = extract_sprite(strip_img, 33, 32);
        let t_sprite = extract_sprite(strip_img, 99, 32);
        let t_90 = rotate_90_cw(&t_sprite, out_size);
        let t_180 = rotate_90_cw(&t_90, out_size);
        let t_270 = rotate_90_cw(&t_180, out_size);

        // Append 10 extra layers: NS, BR, BL, TL, TR, Cross, T x4
        data.extend_from_slice(&ns_sprite);
        data.extend_from_slice(&br_sprite);
        data.extend_from_slice(&bl_sprite);
        data.extend_from_slice(&tl_sprite);
        data.extend_from_slice(&tr_sprite);
        data.extend_from_slice(&cross_sprite);
        data.extend_from_slice(&t_sprite);
        data.extend_from_slice(&t_90);
        data.extend_from_slice(&t_180);
        data.extend_from_slice(&t_270);

        extra_count += crate::constants::AUTOTILE_EXTRA_PER_KIND as u32;
    }

    // Append Pick extra variant layers (variants 1..N-1 for each Pick kind).
    let atlas_data = atlas.data.as_ref().expect("atlas image has no data");
    let atlas_width = atlas.width();
    let cell_size = CELL as u32;
    let sprite = SPRITE_SIZE as u32;
    let scale = out_size / sprite;
    for def in crate::constants::BUILDING_REGISTRY {
        let crate::constants::TileSpec::Pick(variants) = def.tile else {
            continue;
        };
        // Skip the first variant (already baked as base layer).
        for &(col, row) in variants.iter().skip(1) {
            let src_x = col * cell_size;
            let src_y = row * cell_size;
            let mut layer_data = vec![0u8; layer_bytes];
            for ty in 0..sprite {
                for tx in 0..sprite {
                    let si = ((src_y + ty) * atlas_width + (src_x + tx)) as usize * 4;
                    for oy in 0..scale {
                        for ox in 0..scale {
                            let di =
                                ((ty * scale + oy) * out_size + (tx * scale + ox)) as usize * 4;
                            layer_data[di..di + 4].copy_from_slice(&atlas_data[si..si + 4]);
                        }
                    }
                }
            }
            data.extend_from_slice(&layer_data);
            extra_count += 1;
        }
    }

    let total_layers = base_layers + extra_count;
    let mut img = Image::new(
        Extent3d {
            width: out_size,
            height: out_size * total_layers,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        Default::default(),
    );
    img.sampler = bevy::image::ImageSampler::nearest();
    images.add(img)
}

/// Extras atlas: composites individual 16x16 sprites into a horizontal grid (ATLAS_CELL cells, upscaled).
/// Used for heal, sleep, arrow, boat -- any single-sprite overlay. Order matches atlas_id mapping in shader.
pub fn build_extras_atlas(sprites: &[Image], images: &mut Assets<Image>) -> Handle<Image> {
    let cell = ATLAS_CELL;
    let count = sprites.len() as u32;
    let mut data = vec![0u8; (cell * count * cell * 4) as usize];

    for (i, img) in sprites.iter().enumerate() {
        let src = img.data.as_ref().expect("extras sprite has no data");
        let sw = img.width();
        let sh = img.height();
        // 2x nearest-neighbor upscale into the cell
        for dy in 0..cell {
            for dx in 0..cell {
                let sx = (dx * sw / cell).min(sw - 1);
                let sy = (dy * sh / cell).min(sh - 1);
                let si = (sy * sw + sx) as usize * 4;
                let di = (dy * cell * count + i as u32 * cell + dx) as usize * 4;
                if si + 4 <= src.len() && di + 4 <= data.len() {
                    data[di..di + 4].copy_from_slice(&src[si..si + 4]);
                }
            }
        }
    }

    images.add(Image::new(
        Extent3d {
            width: cell * count,
            height: cell,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        Default::default(),
    ))
}
