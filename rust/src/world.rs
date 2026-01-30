//! World Data - Towns, farms, beds, guard posts

use godot_bevy::prelude::godot_prelude::*;
use godot_bevy::prelude::bevy_ecs_prelude::*;

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
