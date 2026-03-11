//! ECS Events - Commands sent between systems.

use bevy::prelude::*;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32};

// ============================================================================
// EVENT TYPES
// ============================================================================

/// Unified spawn event. Job determines component template at spawn time.
#[derive(Message, Clone)]
pub struct SpawnNpcMsg {
    pub slot_idx: usize,
    pub x: f32,
    pub y: f32,
    pub job: i32,      // 0=Farmer, 1=Archer, 2=Raider
    pub faction: i32,  // 0=Neutral, 1=Player, 2+=AI
    pub town_idx: i32, // -1 = none
    pub home_x: f32,
    pub home_y: f32,
    pub work_x: f32, // -1 = none
    pub work_y: f32,
    pub starting_post: i32,              // -1 = none
    pub entity_override: Option<Entity>, // None = allocate fresh (save/load sets explicitly)
}

/// Unified damage message for both NPCs and buildings.
/// Target identified by Entity. damage_system resolves via ECS query.
#[derive(Message, Clone)]
pub struct DamageMsg {
    pub target: Entity,
    pub amount: f32,
    pub attacker: i32,         // NPC slot of attacker (-1 = tower/unknown)
    pub attacker_faction: i32, // for combat log attribution
}

/// Reassign an NPC to a different job (Farmer <-> Guard).
#[derive(Message, Clone)]
pub struct ReassignMsg {
    pub npc_index: usize,
    pub new_job: i32, // 0=Farmer, 1=Archer
}

/// Request to destroy a building at a world grid cell. Writer: UI inspector/build menu. Reader: process_destroy_system.
#[derive(Message, Clone)]
pub struct DestroyBuildingMsg(pub usize, pub usize); // (grid_col, grid_row)

/// Request to focus the Factions panel on a faction id.
#[derive(Message, Clone)]
pub struct SelectFactionMsg(pub i32);

/// Combat log entry emitted by any system. Drained into CombatLog resource by drain_combat_log.
#[derive(Message, Clone)]
pub struct CombatLogMsg {
    pub kind: crate::resources::CombatEventKind,
    pub faction: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub message: String,
    pub location: Option<bevy::math::Vec2>,
}

// ============================================================================
// DIRTY-FLAG MESSAGES (replace DirtyFlags resource)
// ============================================================================

/// Spatial grid rebuild needed (building placed/destroyed).
#[derive(Message, Clone)]
pub struct BuildingGridDirtyMsg;

/// Terrain tilemap refresh needed (terrain biome changed).
#[derive(Message, Clone)]
pub struct TerrainDirtyMsg;

/// Patrol routes need rebuild.
#[derive(Message, Clone)]
pub struct PatrolsDirtyMsg;

/// AI should resync perimeter waypoints.
#[derive(Message, Clone)]
pub struct PatrolPerimeterDirtyMsg;

/// Healing zone cache needs rebuild.
#[derive(Message, Clone)]
pub struct HealingZonesDirtyMsg;

/// Squad assignments need rebuild.
#[derive(Message, Clone)]
pub struct SquadsDirtyMsg;

/// Mining policy / assignment needs rebuild.
#[derive(Message, Clone)]
pub struct MiningDirtyMsg;

/// Work targeting intent — fire-and-forget from any system.
/// Single consumer: `resolve_work_targets` (owns all occupancy mutations).
#[derive(Message, Clone)]
pub struct WorkIntentMsg(pub WorkIntent);

/// What to do with an NPC's worksite assignment.
#[derive(Clone, Debug)]
pub enum WorkIntent {
    /// Search for best worksite, claim it, update NpcWorkState.
    Claim {
        entity: Entity,
        kind: crate::world::BuildingKind,
        town_idx: u32,
        from: Vec2,
    },
    /// Release current worksite, clear NpcWorkState.
    /// `worksite`: carried from sender so resolver doesn't depend on component state
    /// (decision_system write-back may clear the component before resolver runs).
    Release {
        entity: Entity,
        worksite: Option<Entity>,
    },
    /// Atomic release + re-claim at a new worksite.
    Retarget {
        entity: Entity,
        kind: crate::world::BuildingKind,
        town_idx: u32,
        from: Vec2,
    },
}

/// Patrol waypoint swap request from UI (slot-based identity).
#[derive(Message, Clone)]
pub struct PatrolSwapMsg {
    pub slot_a: usize,
    pub slot_b: usize,
}

/// Convenience bundle: all dirty-flag message writers.
/// Systems that signal changes take this instead of `ResMut<DirtyFlags>`.
#[derive(bevy::ecs::system::SystemParam)]
pub struct DirtyWriters<'w> {
    pub building_grid: MessageWriter<'w, BuildingGridDirtyMsg>,
    pub terrain: MessageWriter<'w, TerrainDirtyMsg>,
    pub patrols: MessageWriter<'w, PatrolsDirtyMsg>,
    pub patrol_perimeter: MessageWriter<'w, PatrolPerimeterDirtyMsg>,
    pub healing_zones: MessageWriter<'w, HealingZonesDirtyMsg>,
    pub squads: MessageWriter<'w, SquadsDirtyMsg>,
    pub mining: MessageWriter<'w, MiningDirtyMsg>,
    pub patrol_swap: MessageWriter<'w, PatrolSwapMsg>,
}

impl DirtyWriters<'_> {
    /// Emit messages equivalent to the old `DirtyFlags::mark_building_changed`.
    pub fn mark_building_changed(&mut self, kind: crate::world::BuildingKind) {
        self.building_grid.write(BuildingGridDirtyMsg);
        if kind == crate::world::BuildingKind::Waypoint {
            self.patrols.write(PatrolsDirtyMsg);
            self.patrol_perimeter.write(PatrolPerimeterDirtyMsg);
        }
        if matches!(
            kind,
            crate::world::BuildingKind::Farm
                | crate::world::BuildingKind::FarmerHome
                | crate::world::BuildingKind::ArcherHome
                | crate::world::BuildingKind::MinerHome
        ) {
            self.patrol_perimeter.write(PatrolPerimeterDirtyMsg);
        }
        if kind == crate::world::BuildingKind::MinerHome {
            self.mining.write(MiningDirtyMsg);
        }
    }

    /// Emit all dirty messages (used on startup / game reset to trigger initial rebuilds).
    pub fn emit_all(&mut self) {
        self.building_grid.write(BuildingGridDirtyMsg);
        self.terrain.write(TerrainDirtyMsg);
        self.patrols.write(PatrolsDirtyMsg);
        self.patrol_perimeter.write(PatrolPerimeterDirtyMsg);
        self.healing_zones.write(HealingZonesDirtyMsg);
        self.squads.write(SquadsDirtyMsg);
        self.mining.write(MiningDirtyMsg);
    }
}

// ============================================================================
// GPU DISPATCH COUNT
// ============================================================================

// ============================================================================
// GPU-FIRST: Single Update Queue (Bevy -> GPU)
// Replaces: GPU_TARGET_QUEUE, HEALTH_SYNC_QUEUE, HIDE_NPC_QUEUE
// ============================================================================

/// GPU update message for event-driven architecture.
/// Systems send these via MessageWriter from gameplay systems.
/// Uses Message pattern (not Observer) because GPU updates are high-frequency batch operations.
#[derive(Message, Clone)]
pub struct GpuUpdateMsg(pub GpuUpdate);

#[derive(Clone, Debug)]
pub enum GpuUpdate {
    /// Set movement target for NPC
    SetTarget { idx: usize, x: f32, y: f32 },
    /// Apply damage delta (GPU subtracts from current health)
    ApplyDamage { idx: usize, amount: f32 },
    /// Hide entity visually (position = -9999)
    Hide { idx: usize },
    /// Set faction (usually at spawn only)
    SetFaction { idx: usize, faction: i32 },
    /// Set max health for normalization (send before SetHealth)
    SetMaxHealth { idx: usize, max_health: f32 },
    /// Set health directly (spawn/reset) — stored normalized by max_health
    SetHealth { idx: usize, health: f32 },
    /// Set position directly (spawn/teleport)
    SetPosition { idx: usize, x: f32, y: f32 },
    /// Set speed
    SetSpeed { idx: usize, speed: f32 },
    /// Set sprite frame (column, row in sprite sheet, atlas: 0.0=character, 1.0=world)
    SetSpriteFrame {
        idx: usize,
        col: f32,
        row: f32,
        atlas: f32,
    },
    /// Set damage flash intensity (1.0 = full white, decays to 0.0)
    SetDamageFlash { idx: usize, intensity: f32 },
    /// Set entity flags (bit 0: combat scan enabled, bit 1: building)
    SetFlags { idx: usize, flags: u32 },
    /// Set entity hitbox half-size for projectile collision (Minkowski sum with arrow hitbox)
    SetHalfSize {
        idx: usize,
        half_w: f32,
        half_h: f32,
    },
    /// Mark slot's visual data dirty (activity/healing/equipment changed)
    MarkVisualDirty { idx: usize },
}

// ============================================================================
// PROJECTILE GPU UPDATES (Bevy -> GPU)
// ============================================================================

/// GPU update for projectile buffers.
#[derive(Clone, Debug)]
pub enum ProjGpuUpdate {
    /// Spawn a projectile at a slot index.
    Spawn {
        idx: usize,
        x: f32,
        y: f32,
        vx: f32,
        vy: f32,
        damage: f32,
        faction: i32,
        shooter: i32,
        lifetime: f32,
    },
    /// Deactivate a projectile (hit processed by CPU).
    Deactivate { idx: usize },
}

/// Projectile GPU update message for event-driven architecture.
#[derive(Message, Clone)]
pub struct ProjGpuUpdateMsg(pub ProjGpuUpdate);

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
pub const RT_EXTRACT_COMPUTE: usize = 8;
pub const RT_EXTRACT_VISUAL: usize = 9;
pub const RT_COUNT: usize = 10;

pub static RENDER_TIMINGS: [AtomicU32; RT_COUNT] = [const { AtomicU32::new(0) }; RT_COUNT];

pub const RT_NAMES: [&str; RT_COUNT] = [
    "r:extract_npc",
    "r:extract_proj",
    "r:prepare_npc",
    "r:queue_npc",
    "r:gpu_compute",
    "r:proj_compute",
    "r:npc_binds",
    "r:proj_binds",
    "r:ext_compute",
    "r:ext_visual",
];

// Extract dirty index counts — written by extract_npc_data, read by profiler tab.
pub const DC_COUNT: usize = 10;
pub const DC_NAMES: [&str; DC_COUNT] = [
    "pos", "arr", "tgt", "spd", "fac", "hp", "flg", "hs", "vis", "eq",
];
pub static EXTRACT_DIRTY_COUNTS: [AtomicU32; DC_COUNT] = [const { AtomicU32::new(0) }; DC_COUNT];

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
