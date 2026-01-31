//! World Data - Towns, farms, beds, guard posts, sprite definitions

use godot_bevy::prelude::godot_prelude::*;
use godot_bevy::prelude::bevy_ecs_prelude::*;

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
    pub pos: Vector2,
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
    pub center: Vector2,
    pub faction: i32,  // 0=Villager, 1=Raider
}

/// A farm building that farmers work at.
#[derive(Clone, Debug)]
pub struct Farm {
    pub position: Vector2,
    pub town_idx: u32,
}

/// A bed where NPCs sleep.
#[derive(Clone, Debug)]
pub struct Bed {
    pub position: Vector2,
    pub town_idx: u32,
}

/// A guard post where guards patrol.
#[derive(Clone, Debug)]
pub struct GuardPost {
    pub position: Vector2,
    pub town_idx: u32,
    /// Patrol order (0-3 for clockwise perimeter)
    pub patrol_order: u32,
}

/// A raider camp (separate from towns for sprite rendering).
#[derive(Clone, Debug)]
pub struct Camp {
    pub position: Vector2,
    pub town_idx: u32,  // Which town this camp raids
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
    pub camps: Vec<Camp>,
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

        // Fountains (town centers, 1x1)
        for town in &self.towns {
            Self::add_sprite_instances(&mut sprites, town.center, LocationType::Fountain);
        }

        // Camps (2x2 tent)
        for camp in &self.camps {
            Self::add_sprite_instances(&mut sprites, camp.position, LocationType::Camp);
        }

        sprites
    }

    /// Add sprite instances for a location (handles multi-cell sprites).
    fn add_sprite_instances(sprites: &mut Vec<SpriteInstance>, center: Vector2, loc_type: LocationType) {
        let def = loc_type.sprite_def();
        let total_scale = def.scale;

        // Build grid of sprites for multi-cell definitions
        for row in 0..def.size.1 {
            for col in 0..def.size.0 {
                let uv = (def.pos.0 + col, def.pos.1 + row);
                // Offset each cell: center the whole sprite, then position each cell
                let cell_offset_x = (col as f32 - (def.size.0 - 1) as f32 / 2.0) * SPRITE_SIZE;
                let cell_offset_y = (row as f32 - (def.size.1 - 1) as f32 / 2.0) * SPRITE_SIZE;
                let world_pos = Vector2::new(
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

/// Find nearest location of a given kind.
pub fn find_nearest_location(from: Vector2, world: &WorldData, kind: LocationKind) -> Option<Vector2> {
    let mut best: Option<(f32, Vector2)> = None;

    let positions: Box<dyn Iterator<Item = Vector2>> = match kind {
        LocationKind::Farm => Box::new(world.farms.iter().map(|f| f.position)),
        LocationKind::Bed => Box::new(world.beds.iter().map(|b| b.position)),
        LocationKind::GuardPost => Box::new(world.guard_posts.iter().map(|g| g.position)),
        LocationKind::Town => Box::new(world.towns.iter().map(|t| t.center)),
    };

    for pos in positions {
        let dx = pos.x - from.x;
        let dy = pos.y - from.y;
        let dist_sq = dx * dx + dy * dy;
        if best.is_none() || dist_sq < best.unwrap().0 {
            best = Some((dist_sq, pos));
        }
    }

    best.map(|(_, p)| p)
}

/// Tracks which NPCs occupy each bed (-1 = free).
#[derive(Resource, Default)]
pub struct BedOccupancy {
    pub occupant_npc: Vec<i32>,
}

/// Tracks how many NPCs are working at each farm.
#[derive(Resource, Default)]
pub struct FarmOccupancy {
    pub occupant_count: Vec<i32>,
}
