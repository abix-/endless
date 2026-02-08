//! World Data - Towns, farms, beds, guard posts, sprite definitions

use bevy::prelude::*;
use std::collections::HashMap;

// ============================================================================
// SPRITE DEFINITIONS (from roguelikeSheet_transparent.png)
// ============================================================================

/// Sprite sheet constants
pub const CELL: f32 = 17.0;  // 16px sprite + 1px margin
pub const SPRITE_SIZE: f32 = 16.0;
pub const SHEET_SIZE: (f32, f32) = (968.0, 526.0);

/// Sprite definition: grid position, size in cells, optional scale
#[derive(Clone, Copy, Debug)]
pub struct SpriteDef {
    pub pos: (i32, i32),   // Grid position in sprite sheet
    pub size: (i32, i32),  // Size in grid cells (1x1, 2x2, etc.)
    pub scale: f32,        // Extra scale multiplier
}

impl SpriteDef {
    pub const fn new(pos: (i32, i32), size: (i32, i32)) -> Self {
        Self { pos, size, scale: 1.0 }
    }
    pub const fn scaled(pos: (i32, i32), size: (i32, i32), scale: f32) -> Self {
        Self { pos, size, scale }
    }
}

// Sprite definitions - discovered via sprite_browser tool
pub const SPRITE_FARM: SpriteDef = SpriteDef::new((2, 15), (2, 2));
pub const SPRITE_TENT: SpriteDef = SpriteDef::scaled((48, 10), (2, 2), 3.0);
pub const SPRITE_FOUNTAIN: SpriteDef = SpriteDef::scaled((50, 9), (1, 1), 2.0);
pub const SPRITE_BED: SpriteDef = SpriteDef::new((15, 2), (1, 1));
pub const SPRITE_GUARD_POST: SpriteDef = SpriteDef::scaled((20, 20), (1, 1), 2.0);

/// Location type for sprite rendering
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LocationType {
    Farm,
    Camp,
    Bed,
    GuardPost,
    Fountain,
}

impl LocationType {
    pub fn sprite_def(&self) -> SpriteDef {
        match self {
            LocationType::Farm => SPRITE_FARM,
            LocationType::Camp => SPRITE_TENT,
            LocationType::Bed => SPRITE_BED,
            LocationType::GuardPost => SPRITE_GUARD_POST,
            LocationType::Fountain => SPRITE_FOUNTAIN,
        }
    }
}

/// A sprite instance for MultiMesh rendering
#[derive(Clone, Debug)]
pub struct SpriteInstance {
    pub pos: Vec2,
    pub uv: (i32, i32),  // Grid coords in sprite sheet
    pub scale: f32,
}

// ============================================================================
// WORLD DATA STRUCTS
// ============================================================================

/// A town (villager or raider settlement).
#[derive(Clone, Debug)]
pub struct Town {
    pub name: String,
    pub center: Vec2,
    pub faction: i32,       // 0=Villager, 1+=Raider factions
    pub sprite_type: i32,   // 0=fountain, 1=tent
}

/// A farm building that farmers work at.
#[derive(Clone, Debug)]
pub struct Farm {
    pub position: Vec2,
    pub town_idx: u32,
}

/// A bed where NPCs sleep.
#[derive(Clone, Debug)]
pub struct Bed {
    pub position: Vec2,
    pub town_idx: u32,
}

/// A guard post where guards patrol.
#[derive(Clone, Debug)]
pub struct GuardPost {
    pub position: Vec2,
    pub town_idx: u32,
    /// Patrol order (0-3 for clockwise perimeter)
    pub patrol_order: u32,
}

// ============================================================================
// WORLD RESOURCES
// ============================================================================

/// Contains all world layout data (immutable after init).
#[derive(Resource, Default)]
pub struct WorldData {
    pub towns: Vec<Town>,
    pub farms: Vec<Farm>,
    pub beds: Vec<Bed>,
    pub guard_posts: Vec<GuardPost>,
}

impl WorldData {
    /// Generate all sprite instances for location MultiMesh rendering.
    /// Each multi-cell sprite (2x2 farm, 2x2 tent) generates multiple instances.
    pub fn get_all_sprites(&self) -> Vec<SpriteInstance> {
        let mut sprites = Vec::new();

        // Farms (2x2)
        for farm in &self.farms {
            Self::add_sprite_instances(&mut sprites, farm.position, LocationType::Farm);
        }

        // Beds (1x1)
        for bed in &self.beds {
            Self::add_sprite_instances(&mut sprites, bed.position, LocationType::Bed);
        }

        // Guard posts (1x1)
        for post in &self.guard_posts {
            Self::add_sprite_instances(&mut sprites, post.position, LocationType::GuardPost);
        }

        // Town centers (sprite based on sprite_type: 0=fountain, 1=tent)
        for town in &self.towns {
            let loc_type = match town.sprite_type {
                1 => LocationType::Camp,  // tent
                _ => LocationType::Fountain,
            };
            Self::add_sprite_instances(&mut sprites, town.center, loc_type);
        }

        sprites
    }

    /// Add sprite instances for a location (handles multi-cell sprites).
    fn add_sprite_instances(sprites: &mut Vec<SpriteInstance>, center: Vec2, loc_type: LocationType) {
        let def = loc_type.sprite_def();
        let total_scale = def.scale;

        // Build grid of sprites for multi-cell definitions
        for row in 0..def.size.1 {
            for col in 0..def.size.0 {
                let uv = (def.pos.0 + col, def.pos.1 + row);
                // Offset each cell: center the whole sprite, then position each cell
                let cell_offset_x = (col as f32 - (def.size.0 - 1) as f32 / 2.0) * SPRITE_SIZE;
                let cell_offset_y = (row as f32 - (def.size.1 - 1) as f32 / 2.0) * SPRITE_SIZE;
                let world_pos = Vec2::new(
                    center.x + cell_offset_x * total_scale,
                    center.y + cell_offset_y * total_scale,
                );
                sprites.push(SpriteInstance {
                    pos: world_pos,
                    uv,
                    scale: total_scale,
                });
            }
        }
    }
}

/// Location types for find_nearest_location.
#[derive(Clone, Copy, Debug)]
pub enum LocationKind {
    Farm,
    Bed,
    GuardPost,
    Town,
}

/// Find nearest location of a given kind (no radius limit, position only).
pub fn find_nearest_location(from: Vec2, world: &WorldData, kind: LocationKind) -> Option<Vec2> {
    find_location_within_radius(from, world, kind, f32::MAX).map(|(_, pos)| pos)
}

/// Find nearest location of a given kind within radius. Returns (index, position).
/// Core function used by both internal Rust code and FFI functions.
pub fn find_location_within_radius(
    from: Vec2,
    world: &WorldData,
    kind: LocationKind,
    radius: f32,
) -> Option<(usize, Vec2)> {
    let mut best: Option<(f32, usize, Vec2)> = None;

    let positions: Vec<Vec2> = match kind {
        LocationKind::Farm => world.farms.iter().map(|f| f.position).collect(),
        LocationKind::Bed => world.beds.iter().map(|b| b.position).collect(),
        LocationKind::GuardPost => world.guard_posts.iter().map(|g| g.position).collect(),
        LocationKind::Town => world.towns.iter().map(|t| t.center).collect(),
    };

    for (idx, pos) in positions.iter().enumerate() {
        let dx = pos.x - from.x;
        let dy = pos.y - from.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist <= radius && (best.is_none() || dist < best.unwrap().0) {
            best = Some((dist, idx, *pos));
        }
    }

    best.map(|(_, idx, pos)| (idx, pos))
}

/// Convert Vec2 to integer key for HashMap lookup.
/// Uses rounded coordinates so slight position differences still match.
pub fn pos_to_key(pos: Vec2) -> (i32, i32) {
    (pos.x.round() as i32, pos.y.round() as i32)
}

/// Tracks which NPCs occupy each bed. Key = bed position, Value = NPC index (-1 = free).
#[derive(Resource, Default)]
pub struct BedOccupancy {
    pub occupants: HashMap<(i32, i32), i32>,
}

/// Tracks how many NPCs are working at each farm. Key = farm position, Value = count.
#[derive(Resource, Default)]
pub struct FarmOccupancy {
    pub occupants: HashMap<(i32, i32), i32>,
}

/// Find farm index by position (for FarmStates lookup which is still Vec-indexed).
pub fn find_farm_index_by_pos(farms: &[Farm], pos: Vec2) -> Option<usize> {
    let key = pos_to_key(pos);
    farms.iter().position(|f| pos_to_key(f.position) == key)
}
