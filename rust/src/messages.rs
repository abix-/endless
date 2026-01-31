//! ECS Messages - Commands sent from GDScript to Bevy.
//! See docs/messages.md for architecture.

use godot_bevy::prelude::bevy_ecs_prelude::Message;
use std::sync::Mutex;

// ============================================================================
// MESSAGE TYPES (Bevy ECS internal messages)
// ============================================================================

/// Unified spawn message. Job determines component template at spawn time.
/// Replaces SpawnNpcMsg, SpawnGuardMsg, SpawnFarmerMsg, SpawnRaiderMsg.
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
// Phase 11.7: Removed PROJECTILE_FIRE_QUEUE, RESET_BEVY → channels

// ============================================================================
// SLOT ALLOCATION vs GPU DISPATCH (two separate counts)
// ============================================================================

/// High-water mark for slot allocation. Only grows (or resets to 0).
/// Used by allocate_slot() to assign indices. NOT used for GPU dispatch.
pub static NPC_SLOT_COUNTER: Mutex<usize> = Mutex::new(0);

/// Number of NPCs with initialized GPU buffers. Set by spawn_npc_system
/// after pushing GPU_UPDATE_QUEUE. Read by process() for dispatch count.
pub static GPU_DISPATCH_COUNT: Mutex<usize> = Mutex::new(0);

// ============================================================================
// SLOT REUSE: Free slot pools for recycling dead indices
// ============================================================================

/// Free NPC slot indices available for reuse.
/// When an NPC dies, its index is pushed here. Spawn pops from here first.
pub static FREE_SLOTS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// Free projectile slot indices available for reuse.
/// When a projectile expires or hits, its index is pushed here.
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
}

pub static GPU_UPDATE_QUEUE: Mutex<Vec<GpuUpdate>> = Mutex::new(Vec::new());

// ============================================================================
// GPU-FIRST: Single Read State (GPU -> Bevy)
// Replaces: GPU_POSITIONS, GPU_COMBAT_TARGETS, GPU_NPC_COUNT
// ============================================================================

#[derive(Default)]
pub struct GpuReadState {
    /// Positions: [x0, y0, x1, y1, ...] - 2 floats per NPC
    pub positions: Vec<f32>,
    /// Combat targets: index i = target for NPC i (-1 = no target)
    pub combat_targets: Vec<i32>,
    /// Health values (GPU authoritative)
    pub health: Vec<f32>,
    /// Factions (for Bevy queries)
    pub factions: Vec<i32>,
    /// Current NPC count
    pub npc_count: usize,
}

pub static GPU_READ_STATE: Mutex<GpuReadState> = Mutex::new(GpuReadState {
    positions: Vec::new(),
    combat_targets: Vec::new(),
    health: Vec::new(),
    factions: Vec::new(),
    npc_count: 0,
});

// ============================================================================
// FOOD STORAGE (Bevy-owned, polled by GDScript)
// ============================================================================

/// Per-town food counts. All settlements are "towns" (villager or raider).
/// Owned by Bevy so eat-decisions stay in Rust without crossing the GDScript boundary.
pub struct FoodStorage {
    pub food: Vec<i32>,  // One entry per town (villager towns first, then raider towns)
}

impl Default for FoodStorage {
    fn default() -> Self {
        Self {
            food: Vec::new(),
        }
    }
}

pub static FOOD_STORAGE: Mutex<FoodStorage> = Mutex::new(FoodStorage {
    food: Vec::new(),
});


// ============================================================================
// GAME CONFIG (write-once from GDScript, drained into Res<GameConfig>)
// ============================================================================

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

