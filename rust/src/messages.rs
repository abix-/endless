//! ECS Messages - Commands sent from GDScript to Bevy
//!
//! GPU-First Architecture:
//! - GPU owns: positions, targets, factions, health, combat_targets
//! - Bevy owns: state markers (Dead, Fleeing, InCombat)
//! - Bevy reads GPU_READ_STATE, writes via GPU_UPDATE_QUEUE
//! - One lock per direction, not 10+ scattered queues

use godot_bevy::prelude::bevy_ecs_prelude::Message;
use std::sync::Mutex;

// ============================================================================
// MESSAGE TYPES (Bevy ECS internal messages)
// ============================================================================

#[derive(Message, Clone)]
pub struct SpawnNpcMsg {
    pub x: f32,
    pub y: f32,
    pub job: i32,
}

#[derive(Message, Clone)]
pub struct SetTargetMsg {
    pub npc_index: usize,
    pub x: f32,
    pub y: f32,
}

#[derive(Message, Clone)]
pub struct SpawnGuardMsg {
    pub x: f32,
    pub y: f32,
    pub town_idx: u32,
    pub home_x: f32,
    pub home_y: f32,
    pub starting_post: u32,
}

#[derive(Message, Clone)]
pub struct ArrivalMsg {
    pub npc_index: usize,
}

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

#[derive(Message, Clone)]
pub struct DamageMsg {
    pub npc_index: usize,
    pub amount: f32,
}

#[derive(Message, Clone)]
pub struct SpawnRaiderMsg {
    pub x: f32,
    pub y: f32,
    pub camp_x: f32,
    pub camp_y: f32,
}

// ============================================================================
// BEVY MESSAGE QUEUES (GDScript -> Bevy ECS)
// These stay - they're for Bevy's internal message system
// ============================================================================

pub static SPAWN_QUEUE: Mutex<Vec<SpawnNpcMsg>> = Mutex::new(Vec::new());
pub static TARGET_QUEUE: Mutex<Vec<SetTargetMsg>> = Mutex::new(Vec::new());
pub static GUARD_QUEUE: Mutex<Vec<SpawnGuardMsg>> = Mutex::new(Vec::new());
pub static FARMER_QUEUE: Mutex<Vec<SpawnFarmerMsg>> = Mutex::new(Vec::new());
pub static RAIDER_QUEUE: Mutex<Vec<SpawnRaiderMsg>> = Mutex::new(Vec::new());
pub static ARRIVAL_QUEUE: Mutex<Vec<ArrivalMsg>> = Mutex::new(Vec::new());
pub static DAMAGE_QUEUE: Mutex<Vec<DamageMsg>> = Mutex::new(Vec::new());
pub static RESET_BEVY: Mutex<bool> = Mutex::new(false);
pub static FRAME_DELTA: Mutex<f32> = Mutex::new(0.016);

// ============================================================================
// SLOT REUSE: Free slot pool for recycling dead NPC indices
// ============================================================================

/// Free NPC slot indices available for reuse.
/// When an NPC dies, its index is pushed here. Spawn pops from here first.
pub static FREE_SLOTS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

// ============================================================================
// GPU-FIRST: Single Update Queue (Bevy -> GPU)
// Replaces: GPU_TARGET_QUEUE, HEALTH_SYNC_QUEUE, HIDE_NPC_QUEUE
// ============================================================================

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
// DEBUG INFO
// ============================================================================

#[derive(Default)]
pub struct HealthDebugInfo {
    pub damage_processed: usize,
    pub deaths_this_frame: usize,
    pub despawned_this_frame: usize,
    pub bevy_entity_count: usize,
    pub health_samples: Vec<(usize, f32)>,
}

pub static HEALTH_DEBUG: Mutex<HealthDebugInfo> = Mutex::new(HealthDebugInfo {
    damage_processed: 0,
    deaths_this_frame: 0,
    despawned_this_frame: 0,
    bevy_entity_count: 0,
    health_samples: Vec::new(),
});
