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
// BEVY MESSAGE QUEUES (GDScript -> Bevy ECS)
// ============================================================================

pub static SPAWN_QUEUE: Mutex<Vec<SpawnNpcMsg>> = Mutex::new(Vec::new());
pub static TARGET_QUEUE: Mutex<Vec<SetTargetMsg>> = Mutex::new(Vec::new());
pub static ARRIVAL_QUEUE: Mutex<Vec<ArrivalMsg>> = Mutex::new(Vec::new());
pub static DAMAGE_QUEUE: Mutex<Vec<DamageMsg>> = Mutex::new(Vec::new());

/// Projectile fire request from attack_system (Bevy -> GPU projectile system).
/// Drained in process() to create GPU projectiles.
pub struct FireProjectileMsg {
    pub from_x: f32,
    pub from_y: f32,
    pub to_x: f32,
    pub to_y: f32,
    pub damage: f32,
    pub faction: i32,
    pub shooter: usize,
    pub speed: f32,
    pub lifetime: f32,
}

pub static PROJECTILE_FIRE_QUEUE: Mutex<Vec<FireProjectileMsg>> = Mutex::new(Vec::new());

pub static RESET_BEVY: Mutex<bool> = Mutex::new(false);
pub static FRAME_DELTA: Mutex<f32> = Mutex::new(0.016);

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

// ============================================================================
// FOOD STORAGE (Bevy-owned, polled by GDScript)
// ============================================================================

/// Per-town and per-camp food counts. Owned by Bevy so raider eat-decisions
/// stay in Rust without crossing the GDScript boundary.
pub struct FoodStorage {
    pub town_food: Vec<i32>,
    pub camp_food: Vec<i32>,
}

impl Default for FoodStorage {
    fn default() -> Self {
        Self {
            town_food: Vec::new(),
            camp_food: Vec::new(),
        }
    }
}

pub static FOOD_STORAGE: Mutex<FoodStorage> = Mutex::new(FoodStorage {
    town_food: Vec::new(),
    camp_food: Vec::new(),
});

// ============================================================================
// FOOD EVENTS (Bevy -> GDScript, polled per frame)
// ============================================================================

/// A raider delivered stolen food to their camp.
#[derive(Clone, Debug)]
pub struct FoodDelivered {
    pub camp_idx: u32,
}

/// An NPC consumed food at their home location.
#[derive(Clone, Debug)]
pub struct FoodConsumed {
    pub location_idx: u32,
    pub is_camp: bool,
}

pub static FOOD_DELIVERED_QUEUE: Mutex<Vec<FoodDelivered>> = Mutex::new(Vec::new());
pub static FOOD_CONSUMED_QUEUE: Mutex<Vec<FoodConsumed>> = Mutex::new(Vec::new());
