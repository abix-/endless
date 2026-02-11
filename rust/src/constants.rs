//! Constants - Tuning parameters for the NPC system

/// Maximum NPCs the system can handle. Buffers are pre-allocated to this size.
pub const MAX_NPC_COUNT: usize = 50000;

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
pub const SPRITE_GUARD: (f32, f32) = (0.0, 11.0);
pub const SPRITE_RAIDER: (f32, f32) = (0.0, 6.0);
pub const SPRITE_FIGHTER: (f32, f32) = (7.0, 0.0);

/// Size of push constants passed to the compute shader.
pub const PUSH_CONSTANTS_SIZE: usize = 48;

// Equipment sprite frames (column, row) — placeholder coordinates
pub const EQUIP_SWORD: (f32, f32) = (0.0, 8.0);
pub const EQUIP_HELMET: (f32, f32) = (7.0, 9.0);
pub const FOOD_SPRITE: (f32, f32) = (6.0, 8.0);

// Visual indicator sprites (column, row) — placeholder coordinates, verify against atlas
pub const SLEEP_SPRITE: (f32, f32) = (51.0, 0.0);
pub const HEAL_SPRITE: (f32, f32) = (23.0, 0.0);

// Distinct colors for raider factions (warm/aggressive palette)
pub const RAIDER_COLORS: [(f32, f32, f32); 10] = [
    (1.0, 0.5, 0.5),   // Red tint
    (1.0, 0.7, 0.4),   // Orange tint
    (1.0, 0.5, 0.7),   // Magenta tint
    (0.8, 0.5, 1.0),   // Purple tint
    (1.0, 0.9, 0.4),   // Yellow tint
    (0.9, 0.6, 0.5),   // Brown tint
    (1.0, 0.6, 0.7),   // Pink tint
    (0.8, 0.4, 0.4),   // Dark red tint
    (1.0, 0.8, 0.5),   // Gold tint
    (0.8, 0.4, 0.6),   // Dark magenta tint
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

/// Ticks a guard waits at a post before moving to next.
pub const GUARD_PATROL_WAIT: u32 = 60;

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

/// Collision detection radius for projectile hits.
pub const PROJECTILE_HIT_RADIUS: f32 = 10.0;

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
// STARVATION CONSTANTS
// ============================================================================

/// Max HP multiplier when starving (50% of normal).
pub const STARVING_HP_CAP: f32 = 0.5;

/// Speed multiplier when starving (50% of normal).
pub const STARVING_SPEED_MULT: f32 = 0.5;

// ============================================================================
// BUILDING SYSTEM CONSTANTS
// ============================================================================

/// Food cost to build a farm.
pub const FARM_BUILD_COST: i32 = 1;

/// Food cost to build a guard post.
pub const GUARD_POST_BUILD_COST: i32 = 1;

/// Food cost to unlock one adjacent grid slot.
pub const SLOT_UNLOCK_COST: i32 = 1;

/// Food cost to build a hut (supports 1 farmer).
pub const HUT_BUILD_COST: i32 = 1;

/// Food cost to build a barracks (supports 1 guard).
pub const BARRACKS_BUILD_COST: i32 = 1;

/// Food cost to build a tent (supports 1 raider).
pub const TENT_BUILD_COST: i32 = 1;

/// Game hours before a dead NPC respawns from its building.
pub const SPAWNER_RESPAWN_HOURS: f32 = 12.0;

/// Town building grid spacing in pixels (matches WorldGrid cell_size for 1:1 alignment).
pub const TOWN_GRID_SPACING: f32 = 32.0;

/// Base grid extent: rows/cols from -2 to +3 = 6x6 starting area.
pub const BASE_GRID_MIN: i32 = -2;
pub const BASE_GRID_MAX: i32 = 3;

/// Maximum grid extent (rows/cols -49 to +50 = 100x100).
pub const MAX_GRID_EXTENT: i32 = 49;

// ============================================================================
// GUARD POST TURRET CONSTANTS
// ============================================================================

/// Detection range for guard post auto-attack.
pub const GUARD_POST_RANGE: f32 = 250.0;

/// Damage per turret projectile.
pub const GUARD_POST_DAMAGE: f32 = 8.0;

/// Seconds between turret shots.
pub const GUARD_POST_COOLDOWN: f32 = 3.0;

/// Turret projectile speed (pixels/sec).
pub const GUARD_POST_PROJ_SPEED: f32 = 300.0;

/// Turret projectile lifetime (seconds).
pub const GUARD_POST_PROJ_LIFETIME: f32 = 1.5;
