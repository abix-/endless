//! World Data - Towns, farms, beds, guard posts

use godot_bevy::prelude::godot_prelude::*;
use godot_bevy::prelude::bevy_ecs_prelude::*;
use std::sync::{Mutex, LazyLock};

// ============================================================================
// WORLD DATA STRUCTS
// ============================================================================

/// A town with its center position and associated raider camp.
#[derive(Clone, Debug)]
pub struct Town {
    pub name: String,
    pub center: Vector2,
    pub camp_position: Vector2,
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

// ============================================================================
// STATIC WORLD DATA
// ============================================================================

/// World data (towns, farms, beds, guard posts). Initialized once from GDScript.
pub static WORLD_DATA: LazyLock<Mutex<WorldData>> = LazyLock::new(|| Mutex::new(WorldData::default()));

/// Bed occupancy tracking (-1 = free, >= 0 = NPC index).
pub static BED_OCCUPANCY: LazyLock<Mutex<BedOccupancy>> = LazyLock::new(|| Mutex::new(BedOccupancy::default()));

/// Farm occupancy tracking (count of NPCs working at each farm).
pub static FARM_OCCUPANCY: LazyLock<Mutex<FarmOccupancy>> = LazyLock::new(|| Mutex::new(FarmOccupancy::default()));
