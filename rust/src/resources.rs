//! ECS Resources - Shared state accessible by all systems

use godot_bevy::prelude::bevy_ecs_prelude::*;
use std::collections::HashMap;

/// Tracks total number of active NPCs.
#[derive(Resource, Default)]
pub struct NpcCount(pub usize);

/// Delta time for the current frame (seconds).
#[derive(Resource, Default)]
pub struct DeltaTime(pub f32);

/// O(1) lookup from NPC slot index to Bevy Entity.
/// Populated on spawn, used by damage_system for fast entity lookup.
#[derive(Resource, Default)]
pub struct NpcEntityMap(pub HashMap<usize, Entity>);

/// Population counts per (job_id, clan_id).
#[derive(Default, Clone)]
pub struct PopStats {
    pub alive: i32,
    pub working: i32,
}

/// Aggregated population stats, updated incrementally at spawn/death/state transitions.
#[derive(Resource, Default)]
pub struct PopulationStats(pub HashMap<(i32, i32), PopStats>);

/// Game config pushed from GDScript at startup.
#[derive(Resource)]
pub struct GameConfig {
    pub farmers_per_town: i32,
    pub guards_per_town: i32,
    pub raiders_per_camp: i32,
    pub spawn_interval_hours: i32,
    pub food_per_work_hour: i32,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            farmers_per_town: 10,
            guards_per_town: 30,
            raiders_per_camp: 15,
            spawn_interval_hours: 4,
            food_per_work_hour: 1,
        }
    }
}

/// Game time tracking - Bevy-owned, uses PhysicsDelta from godot-bevy.
#[derive(Resource)]
pub struct GameTime {
    pub elapsed_seconds: f32,
    pub current_hour: i32,
    pub seconds_per_hour: f32,
}

impl Default for GameTime {
    fn default() -> Self {
        Self {
            elapsed_seconds: 0.0,
            current_hour: 6, // Start at 6am
            seconds_per_hour: 60.0, // 1 game-hour = 60 real seconds (adjustable)
        }
    }
}

/// Per-clan respawn cooldowns. Maps clan_id -> hours until next spawn check.
#[derive(Resource, Default)]
pub struct RespawnTimers(pub HashMap<i32, i32>);
