//! ECS Events - Commands sent between systems.

use bevy::prelude::*;
use std::sync::Mutex;

// ============================================================================
// EVENT TYPES
// ============================================================================

/// Unified spawn event. Job determines component template at spawn time.
#[derive(Message, Clone)]
pub struct SpawnNpcMsg {
    pub slot_idx: usize,
    pub x: f32,
    pub y: f32,
    pub job: i32,           // 0=Farmer, 1=Guard, 2=Raider
    pub faction: i32,       // 0=Villager, 1=Raider
    pub town_idx: i32,      // -1 = none
    pub home_x: f32,
    pub home_y: f32,
    pub work_x: f32,        // -1 = none
    pub work_y: f32,
    pub starting_post: i32, // -1 = none
    pub attack_type: i32,   // 0=melee, 1=ranged
}

#[derive(Message, Clone)]
pub struct SetTargetMsg {
    pub npc_index: usize,
    pub x: f32,
    pub y: f32,
}

#[derive(Message, Clone)]
pub struct ArrivalMsg {
    pub npc_index: usize,
}

#[derive(Message, Clone)]
pub struct DamageMsg {
    pub npc_index: usize,
    pub amount: f32,
}

// ============================================================================
// BEVY MESSAGE QUEUES (remaining statics - arrivals still needed by lib.rs)
// Phase 11.7: Removed SPAWN_QUEUE, TARGET_QUEUE, DAMAGE_QUEUE → channels
// ============================================================================

pub static ARRIVAL_QUEUE: Mutex<Vec<ArrivalMsg>> = Mutex::new(Vec::new());

// ============================================================================
// GPU DISPATCH COUNT
// ============================================================================

/// Number of NPCs with initialized GPU buffers. Set by spawn_npc_system
/// after pushing GPU_UPDATE_QUEUE. Read by process() for dispatch count.
/// NPC slot allocation itself is handled by Bevy's SlotAllocator resource.
pub static GPU_DISPATCH_COUNT: Mutex<usize> = Mutex::new(0);

// ============================================================================
// PROJECTILE SLOT REUSE
// ============================================================================

/// Free projectile slot indices available for reuse.
/// When a projectile expires or hits, its index is pushed here.
/// Note: NPC slots use Bevy's SlotAllocator resource instead of a static.
pub static FREE_PROJ_SLOTS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

// ============================================================================
// GPU-FIRST: Single Update Queue (Bevy -> GPU)
// Replaces: GPU_TARGET_QUEUE, HEALTH_SYNC_QUEUE, HIDE_NPC_QUEUE
// ============================================================================

/// GPU update message for event-driven architecture.
/// Systems send these via MessageWriter instead of locking GPU_UPDATE_QUEUE directly.
/// Uses Message pattern (not Observer) because GPU updates are high-frequency batch operations.
#[derive(Message, Clone)]
pub struct GpuUpdateMsg(pub GpuUpdate);

#[derive(Clone, Debug)]
pub enum GpuUpdate {
    /// Set movement target for NPC
    SetTarget { idx: usize, x: f32, y: f32 },
    /// Apply damage delta (GPU subtracts from current health)
    ApplyDamage { idx: usize, amount: f32 },
    /// Hide NPC visually (position = -9999)
    HideNpc { idx: usize },
    /// Set faction (usually at spawn only)
    SetFaction { idx: usize, faction: i32 },
    /// Set health directly (spawn/reset)
    SetHealth { idx: usize, health: f32 },
    /// Set position directly (spawn/teleport)
    SetPosition { idx: usize, x: f32, y: f32 },
    /// Set speed
    SetSpeed { idx: usize, speed: f32 },
    /// Set color
    SetColor { idx: usize, r: f32, g: f32, b: f32, a: f32 },
    /// Set sprite frame (column, row in sprite sheet)
    SetSpriteFrame { idx: usize, col: f32, row: f32 },
    /// Set healing aura flag (visual only)
    SetHealing { idx: usize, healing: bool },
    /// Set carried item (0 = none, 1 = food, etc.)
    SetCarriedItem { idx: usize, item_id: u8 },
}

pub static GPU_UPDATE_QUEUE: Mutex<Vec<GpuUpdate>> = Mutex::new(Vec::new());

// ============================================================================
// GPU-FIRST: Single Read State (GPU -> Bevy)
// Static for cross-thread access. Struct defined in resources.rs.
// ============================================================================

use crate::resources::GpuReadState;

pub static GPU_READ_STATE: Mutex<GpuReadState> = Mutex::new(GpuReadState {
    positions: Vec::new(),
    combat_targets: Vec::new(),
    health: Vec::new(),
    factions: Vec::new(),
    npc_count: 0,
});

// ============================================================================
// GAME CONFIG (write-once from GDScript, drained into Res<GameConfig>)
// ============================================================================
// Note: FoodStorage moved to resources.rs as a Bevy Resource

use crate::resources::GameConfig;

/// Staging area for GameConfig. Set by GDScript set_game_config(), drained once by Bevy.
pub static GAME_CONFIG_STAGING: Mutex<Option<GameConfig>> = Mutex::new(None);

// ============================================================================
// NPC STATE CONSTANTS (for UI display)
// ============================================================================

// State constants - GDScript must match these values
pub const STATE_IDLE: i32 = 0;
pub const STATE_WALKING: i32 = 1;
pub const STATE_RESTING: i32 = 2;
pub const STATE_WORKING: i32 = 3;
pub const STATE_PATROLLING: i32 = 4;
pub const STATE_ON_DUTY: i32 = 5;
pub const STATE_FIGHTING: i32 = 6;
pub const STATE_RAIDING: i32 = 7;
pub const STATE_RETURNING: i32 = 8;
pub const STATE_RECOVERING: i32 = 9;
pub const STATE_FLEEING: i32 = 10;
pub const STATE_GOING_TO_REST: i32 = 11;
pub const STATE_GOING_TO_WORK: i32 = 12;

