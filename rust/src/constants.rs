//! Constants - Tuning parameters for the NPC system

/// Maximum NPCs the system can handle. Buffers are pre-allocated to this size.
pub const MAX_NPC_COUNT: usize = 100000;

/// Neutral faction — friendly to everyone. Used for world-owned buildings (gold mines).
pub const FACTION_NEUTRAL: i32 = -1;

// Spatial grid lives on GPU only — see gpu.rs (256×256 cells × 128px = 32,768px coverage).

/// Minimum distance NPCs try to maintain from each other.
pub const SEPARATION_RADIUS: f32 = 20.0;

/// How strongly NPCs push away from neighbors.
pub const SEPARATION_STRENGTH: f32 = 50.0;

/// Distance from target at which an NPC is considered "arrived".
pub const ARRIVAL_THRESHOLD: f32 = 20.0;

/// Floats per NPC instance in the MultiMesh buffer.
/// Transform2D (8) + Color (4) + CustomData (4) = 16
pub const FLOATS_PER_INSTANCE: usize = 16;

// Sprite frames (column, row) in the character sheet (17px cells with 1px margin)
pub const SPRITE_FARMER: (f32, f32) = (1.0, 6.0);
pub const SPRITE_ARCHER: (f32, f32) = (0.0, 0.0);
pub const SPRITE_RAIDER: (f32, f32) = (0.0, 6.0);
pub const SPRITE_FIGHTER: (f32, f32) = (1.0, 9.0);
pub const SPRITE_MINER: (f32, f32) = (1.0, 6.0);  // Same sprite as farmer, differentiated by brown tint

/// Size of push constants passed to the compute shader.
pub const PUSH_CONSTANTS_SIZE: usize = 48;

// Equipment sprite frames (column, row) — placeholder coordinates
pub const EQUIP_SWORD: (f32, f32) = (45.0, 6.0);
pub const EQUIP_HELMET: (f32, f32) = (28.0, 0.0);
pub const FOOD_SPRITE: (f32, f32) = (24.0, 9.0);
pub const GOLD_SPRITE: (f32, f32) = (41.0, 11.0);

// Visual indicator sprites (column, row) — placeholder coordinates, verify against atlas
pub const SLEEP_SPRITE: (f32, f32) = (24.0, 7.0);
pub const HEAL_SPRITE: (f32, f32) = (23.0, 0.0);

// Distinct colors for raider factions (warm/aggressive palette)
pub const RAIDER_COLORS: [(f32, f32, f32); 10] = [
    (1.0, 0.0, 0.0),   // Red
    (1.0, 0.5, 0.0),   // Orange
    (1.0, 0.0, 1.0),   // Magenta
    (0.5, 0.0, 1.0),   // Purple
    (1.0, 1.0, 0.0),   // Yellow
    (0.6, 0.3, 0.0),   // Brown
    (1.0, 0.4, 0.7),   // Pink
    (0.7, 0.0, 0.0),   // Dark red
    (1.0, 0.8, 0.0),   // Gold
    (0.6, 0.0, 0.4),   // Dark magenta
];

/// Get RGBA color for a raider faction (cycles through palette).
pub fn raider_faction_color(faction: i32) -> (f32, f32, f32, f32) {
    let idx = ((faction - 1).max(0) as usize) % RAIDER_COLORS.len();
    let (r, g, b) = RAIDER_COLORS[idx];
    (r, g, b, 1.0)
}

// ============================================================================
// BEHAVIOR CONSTANTS
// ============================================================================

/// Energy threshold below which NPCs go rest.
pub const ENERGY_HUNGRY: f32 = 50.0;

/// Energy threshold above which NPCs resume activity.
pub const ENERGY_RESTED: f32 = 80.0;

/// Ticks an archer waits at a post before moving to next.
pub const ARCHER_PATROL_WAIT: u32 = 60;

/// Energy threshold to wake up from resting.
pub const ENERGY_WAKE_THRESHOLD: f32 = 90.0;

/// Energy threshold to stop working and seek rest.
pub const ENERGY_TIRED_THRESHOLD: f32 = 30.0;

/// Energy threshold below which NPCs consider eating (emergency only).
pub const ENERGY_EAT_THRESHOLD: f32 = 10.0;

// ============================================================================
// UTILITY AI ACTION SCORES
// ============================================================================

/// Base score for fighting when in combat.
pub const SCORE_FIGHT_BASE: f32 = 50.0;

/// Base score for working (doing job).
pub const SCORE_WORK_BASE: f32 = 40.0;

/// Base score for wandering (idle movement).
pub const SCORE_WANDER_BASE: f32 = 10.0;

/// Multiplier for eat score (energy-based, slightly higher than rest).
pub const SCORE_EAT_MULT: f32 = 1.5;

/// Multiplier for rest score (energy-based).
pub const SCORE_REST_MULT: f32 = 1.0;

/// Multiplier for flee score (hp-based).
pub const SCORE_FLEE_MULT: f32 = 1.0;

// ============================================================================
// FARM GROWTH CONSTANTS
// ============================================================================

/// Growth progress per game hour when no farmer is tending.
pub const FARM_BASE_GROWTH_RATE: f32 = 0.08;

/// Growth progress per game hour when a farmer is working.
pub const FARM_TENDED_GROWTH_RATE: f32 = 0.25;

// Full growth = 1.0 progress
// Passive only: ~12 hours to grow
// With farmer: ~4 hours to grow

/// Maximum farms for item MultiMesh slot allocation.
pub const MAX_FARMS: usize = 500;

// ============================================================================
// PROJECTILE CONSTANTS
// ============================================================================

/// Maximum projectiles the system can handle.
pub const MAX_PROJECTILES: usize = 50000;

/// Oriented rectangle hitbox for arrow projectiles.
pub const PROJECTILE_HIT_HALF_LENGTH: f32 = 12.0; // along travel direction
pub const PROJECTILE_HIT_HALF_WIDTH: f32 = 4.0;   // perpendicular to travel

/// Floats per projectile instance in MultiMesh buffer.
pub const PROJ_FLOATS_PER_INSTANCE: usize = 12;

/// Size of push constants for projectile compute shader.
pub const PROJ_PUSH_CONSTANTS_SIZE: usize = 32;

// ============================================================================
// RAIDER CAMP CONSTANTS
// ============================================================================

/// Food gained per game hour from passive foraging.
pub const CAMP_FORAGE_RATE: i32 = 1;

/// Food cost to spawn one raider.
pub const RAIDER_SPAWN_COST: i32 = 5;

/// Hours between respawn attempts.
pub const RAIDER_RESPAWN_HOURS: f32 = 2.0;

/// Maximum raiders per camp.
pub const CAMP_MAX_POP: i32 = 500;

/// Minimum raiders needed to form a raid group.
pub const RAID_GROUP_SIZE: i32 = 3;

/// Villager population per raider camp (1 camp per 20 villagers).
pub const VILLAGERS_PER_CAMP: i32 = 20;

// ============================================================================
// MIGRATION CONSTANTS
// ============================================================================

/// Game hours between migration trigger checks.
pub const CAMP_SPAWN_CHECK_HOURS: f32 = 12.0;

/// Maximum dynamically-spawned camps.
pub const MAX_DYNAMIC_CAMPS: usize = 20;

/// Distance from a town at which migrating raiders settle (~30s walk at 100px/s).
pub const CAMP_SETTLE_RADIUS: f32 = 3000.0;

/// Minimum raiders in a migrating group.
pub const MIGRATION_BASE_SIZE: usize = 3;

// ============================================================================
// STARVATION CONSTANTS
// ============================================================================

/// Max HP multiplier when starving (50% of normal).
pub const STARVING_HP_CAP: f32 = 0.5;

/// Speed multiplier when starving (50% of normal).
pub const STARVING_SPEED_MULT: f32 = 0.5;

// ============================================================================
// BUILDING SYSTEM CONSTANTS
// ============================================================================

/// Food cost to build.
pub fn building_cost(kind: crate::resources::BuildKind) -> i32 {
    use crate::resources::BuildKind;
    match kind {
        BuildKind::Farm       => 2,
        BuildKind::FarmerHome => 2,
        BuildKind::MinerHome  => 4,
        BuildKind::ArcherHome => 4,
        BuildKind::Waypoint  => 1,
        BuildKind::Tent       => 3,
        BuildKind::Destroy    => 0,
    }
}

/// Game hours before a dead NPC respawns from its building.
pub const SPAWNER_RESPAWN_HOURS: f32 = 12.0;

/// Town building grid spacing in pixels (matches WorldGrid cell_size for 1:1 alignment).
pub const TOWN_GRID_SPACING: f32 = 32.0;

/// Base grid extent: rows/cols from -4 to +3 = 8x8 starting area.
pub const BASE_GRID_MIN: i32 = -4;
pub const BASE_GRID_MAX: i32 = 3;

/// Maximum grid extent (rows/cols -49 to +50 = 100x100).
pub const MAX_GRID_EXTENT: i32 = 49;

// ============================================================================
// BUILDING TURRET STATS
// ============================================================================

/// Combat stats for a building turret (any building kind that auto-shoots).
pub struct TurretStats {
    pub range: f32,
    pub damage: f32,
    pub cooldown: f32,
    pub proj_speed: f32,
    pub proj_lifetime: f32,
}

pub const WAYPOINT_TURRET: TurretStats = TurretStats {
    range: 250.0, damage: 8.0, cooldown: 3.0, proj_speed: 300.0, proj_lifetime: 1.5,
};

pub const FOUNTAIN_TURRET: TurretStats = TurretStats {
    range: 300.0, damage: 5.0, cooldown: 2.5, proj_speed: 250.0, proj_lifetime: 1.5,
};

// ============================================================================
// SQUAD CONSTANTS
// ============================================================================

/// Maximum number of player-controlled squads.
pub const MAX_SQUADS: usize = 10;

/// Default real-time seconds between AI decisions.
pub const DEFAULT_AI_INTERVAL: f32 = 5.0;

// ============================================================================
// GOLD MINE CONSTANTS
// ============================================================================

/// Gold extracted per harvest cycle (mine becomes Ready → miner takes this much).
pub const MINE_EXTRACT_PER_CYCLE: i32 = 5;

/// Tended growth rate for mines (per game-hour). 0.25 = 4 hours to full when miner is working.
pub const MINE_TENDED_GROWTH_RATE: f32 = 0.25;

/// Max distance from mine to continue tending (pushed away = abort + re-walk).
pub const MINE_WORK_RADIUS: f32 = 40.0;

/// Harmonic series multiplier for multi-miner productivity.
/// 1 miner = 1.0×, 2 = 1.5×, 3 = 1.83×, 4 = 2.08×.
pub fn mine_productivity_mult(worker_count: i32) -> f32 {
    let mut mult = 0.0_f32;
    for k in 1..=worker_count { mult += 1.0 / k as f32; }
    mult
}

/// Minimum distance from any settlement center to place a gold mine.
pub const MINE_MIN_SETTLEMENT_DIST: f32 = 300.0;

/// Minimum distance between gold mines.
pub const MINE_MIN_SPACING: f32 = 400.0;

/// Default town policy radius (pixels) for auto-mining discovery around fountain.
pub const DEFAULT_MINING_RADIUS: f32 = 2000.0;

// ============================================================================
// BUILDING HP
// ============================================================================

pub const WAYPOINT_HP: f32 = 200.0;
/// Distance within which a waypoint "covers" a gold mine (AI territory logic).
pub const WAYPOINT_COVER_RADIUS: f32 = 200.0;
pub const ARCHER_HOME_HP: f32 = 150.0;
pub const FARMER_HOME_HP: f32 = 100.0;
pub const MINER_HOME_HP: f32 = 100.0;
pub const TENT_HP: f32 = 100.0;
pub const FARM_HP: f32 = 80.0;
pub const TOWN_HP: f32 = 500.0;
pub const BED_HP: f32 = 50.0;
pub const GOLD_MINE_HP: f32 = 200.0;

/// Radius for projectile-vs-building collision detection on CPU.
pub const BUILDING_HIT_RADIUS: f32 = 20.0;

// ============================================================================
// ATLAS IDS (shared between gpu.rs, render.rs, and npc_render.wgsl)
// ============================================================================

pub const ATLAS_CHAR: f32 = 0.0;
pub const ATLAS_WORLD: f32 = 1.0;
pub const ATLAS_HEAL: f32 = 2.0;
pub const ATLAS_SLEEP: f32 = 3.0;
pub const ATLAS_ARROW: f32 = 4.0;
pub const ATLAS_BUILDING_HP: f32 = 5.0;
pub const ATLAS_MINING_BAR: f32 = 6.0;
pub const ATLAS_BUILDING: f32 = 7.0;
