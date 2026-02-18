//! ECS Events - Commands sent between systems.

use bevy::prelude::*;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicBool};

// ============================================================================
// EVENT TYPES
// ============================================================================

/// Unified spawn event. Job determines component template at spawn time.
#[derive(Message, Clone)]
pub struct SpawnNpcMsg {
    pub slot_idx: usize,
    pub x: f32,
    pub y: f32,
    pub job: i32,           // 0=Farmer, 1=Archer, 2=Raider
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
pub struct DamageMsg {
    pub npc_index: usize,
    pub amount: f32,
    pub attacker: i32,  // NPC slot index of last attacker (-1 = no attacker, e.g. waypoint)
}

/// Damage applied to a building by a projectile or direct attack.
#[derive(Message, Clone)]
pub struct BuildingDamageMsg {
    pub kind: crate::world::BuildingKind,
    pub index: usize,
    pub amount: f32,
    pub attacker_faction: i32,
    /// NPC slot of the attacker (-1 = tower/unknown).
    pub attacker: i32,
}

/// Reassign an NPC to a different job (Farmer <-> Guard).
#[derive(Message, Clone)]
pub struct ReassignMsg {
    pub npc_index: usize,
    pub new_job: i32, // 0=Farmer, 1=Archer
}


// ============================================================================
// GPU DISPATCH COUNT
// ============================================================================

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
    /// Set sprite frame (column, row in sprite sheet, atlas: 0.0=character, 1.0=world)
    SetSpriteFrame { idx: usize, col: f32, row: f32, atlas: f32 },
    /// Set damage flash intensity (1.0 = full white, decays to 0.0)
    SetDamageFlash { idx: usize, intensity: f32 },
    /// Set NPC flags (bit 0: combat scan enabled)
    SetFlags { idx: usize, flags: u32 },
}

pub static GPU_UPDATE_QUEUE: Mutex<Vec<GpuUpdate>> = Mutex::new(Vec::new());

// ============================================================================
// PROJECTILE GPU UPDATES (Bevy -> GPU)
// ============================================================================

/// GPU update for projectile buffers.
#[derive(Clone, Debug)]
pub enum ProjGpuUpdate {
    /// Spawn a projectile at a slot index.
    Spawn {
        idx: usize,
        x: f32, y: f32,
        vx: f32, vy: f32,
        damage: f32,
        faction: i32,
        shooter: i32,
        lifetime: f32,
    },
    /// Deactivate a projectile (hit processed by CPU).
    Deactivate { idx: usize },
}

pub static PROJ_GPU_UPDATE_QUEUE: Mutex<Vec<ProjGpuUpdate>> = Mutex::new(Vec::new());

// GPU→CPU readback now uses Bevy's async Readback + ReadbackComplete observers.
// Static Mutexes (GPU_READ_STATE, PROJ_HIT_STATE, PROJ_POSITION_STATE) deleted.
// See gpu.rs setup_readback_buffers() and resources.rs (GpuReadState, ProjHitState, ProjPositionState).

// ============================================================================
// RENDER-WORLD PROFILING (atomic, lock-free)
// ============================================================================

pub static RENDER_PROFILING: AtomicBool = AtomicBool::new(false);

pub const RT_EXTRACT_NPC: usize = 0;
pub const RT_EXTRACT_PROJ: usize = 1;
pub const RT_PREPARE_NPC: usize = 2;
pub const RT_QUEUE_NPC: usize = 3;
pub const RT_GPU_COMPUTE: usize = 4;
pub const RT_PROJ_COMPUTE: usize = 5;
pub const RT_NPC_BINDS: usize = 6;
pub const RT_PROJ_BINDS: usize = 7;
pub const RT_COUNT: usize = 8;

pub static RENDER_TIMINGS: [AtomicU32; RT_COUNT] = [const { AtomicU32::new(0) }; RT_COUNT];

pub const RT_NAMES: [&str; RT_COUNT] = [
    "r:extract_npc", "r:extract_proj", "r:prepare_npc", "r:queue_npc",
    "r:gpu_compute", "r:proj_compute", "r:npc_binds", "r:proj_binds",
];

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

