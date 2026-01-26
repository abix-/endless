//! ECS Messages - Commands sent from GDScript to Bevy
//!
//! Why static Mutexes?
//! - Godot calls (spawn_npc, set_target) happen on main thread
//! - Bevy systems run in their own scheduling context
//! - We can't pass references between them, so we use global queues
//! - Mutex ensures thread-safety (even though Godot is single-threaded, Bevy isn't)

use godot_bevy::prelude::*;
use godot_bevy::prelude::bevy_ecs_prelude::Message;
use std::sync::Mutex;

// ============================================================================
// MESSAGE TYPES
// ============================================================================

/// Request to spawn a new NPC at position (x, y) with the given job type.
#[derive(Message, Clone)]
pub struct SpawnNpcMsg {
    pub x: f32,
    pub y: f32,
    pub job: i32,
}

/// Request to set an NPC's movement target.
#[derive(Message, Clone)]
pub struct SetTargetMsg {
    pub npc_index: usize,
    pub x: f32,
    pub y: f32,
}

/// Request to spawn a guard with home position and town assignment.
#[derive(Message, Clone)]
pub struct SpawnGuardMsg {
    pub x: f32,
    pub y: f32,
    pub town_idx: u32,
    pub home_x: f32,
    pub home_y: f32,
    pub starting_post: u32,
}

/// Notification that an NPC has arrived at its target.
#[derive(Message, Clone)]
pub struct ArrivalMsg {
    pub npc_index: usize,
}

/// Request to spawn a farmer with home and work positions.
#[derive(Message, Clone)]
pub struct SpawnFarmerMsg {
    pub x: f32,
    pub y: f32,
    pub town_idx: u32,
    pub home_x: f32,
    pub home_y: f32,
    pub work_x: f32,
    pub work_y: f32,
}

/// Request to deal damage to an NPC.
#[derive(Message, Clone)]
pub struct DamageMsg {
    pub npc_index: usize,
    pub amount: f32,
}

// ============================================================================
// STATIC QUEUES - Thread-safe communication from Godot to Bevy
// ============================================================================

/// Queue of pending spawn requests. Drained each frame by drain_spawn_queue system.
pub static SPAWN_QUEUE: Mutex<Vec<SpawnNpcMsg>> = Mutex::new(Vec::new());

/// Queue of pending target updates. Drained each frame by drain_target_queue system.
pub static TARGET_QUEUE: Mutex<Vec<SetTargetMsg>> = Mutex::new(Vec::new());

/// Queue of pending guard spawn requests.
pub static GUARD_QUEUE: Mutex<Vec<SpawnGuardMsg>> = Mutex::new(Vec::new());

/// Queue of pending farmer spawn requests.
pub static FARMER_QUEUE: Mutex<Vec<SpawnFarmerMsg>> = Mutex::new(Vec::new());

/// Queue of arrival notifications (NPC index that just arrived).
pub static ARRIVAL_QUEUE: Mutex<Vec<ArrivalMsg>> = Mutex::new(Vec::new());

/// Queue of target updates that need to be uploaded to GPU.
/// Bevy systems push here, process() drains and uploads.
pub static GPU_TARGET_QUEUE: Mutex<Vec<SetTargetMsg>> = Mutex::new(Vec::new());

/// Queue of pending damage requests.
pub static DAMAGE_QUEUE: Mutex<Vec<DamageMsg>> = Mutex::new(Vec::new());

/// Authoritative NPC count. Updated immediately on spawn (not waiting for Bevy).
/// This ensures GPU gets correct count even before Bevy processes the spawn message.
pub static GPU_NPC_COUNT: Mutex<usize> = Mutex::new(0);

/// Flag to trigger Bevy entity despawn on next frame.
pub static RESET_BEVY: Mutex<bool> = Mutex::new(false);

// ============================================================================
// DEBUG INFO - Updated by systems, read by GDScript API
// ============================================================================

/// Health system debug info.
#[derive(Default)]
pub struct HealthDebugInfo {
    /// Number of damage messages processed this frame.
    pub damage_processed: usize,
    /// Number of entities marked Dead this frame.
    pub deaths_this_frame: usize,
    /// Number of entities despawned this frame.
    pub despawned_this_frame: usize,
    /// Bevy entity count (entities with Health component).
    pub bevy_entity_count: usize,
    /// Sample health values: [(npc_index, health), ...] for first 10.
    pub health_samples: Vec<(usize, f32)>,
}

pub static HEALTH_DEBUG: Mutex<HealthDebugInfo> = Mutex::new(HealthDebugInfo {
    damage_processed: 0,
    deaths_this_frame: 0,
    despawned_this_frame: 0,
    bevy_entity_count: 0,
    health_samples: Vec::new(),
});
