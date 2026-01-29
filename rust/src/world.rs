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
