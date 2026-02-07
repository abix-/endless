//! Constants - Tuning parameters for the NPC system

/// Maximum NPCs the system can handle. Buffers are pre-allocated to this size.
pub const MAX_NPC_COUNT: usize = 10000;

/// Spatial grid dimensions. The world is divided into GRID_WIDTH × GRID_HEIGHT cells.
/// With CELL_SIZE=100 and 8000px world, we need 80x80 grid.
pub const GRID_WIDTH: usize = 80;
pub const GRID_HEIGHT: usize = 80;
pub const GRID_CELLS: usize = GRID_WIDTH * GRID_HEIGHT;

/// Maximum NPCs per grid cell. Increased for larger cells.
pub const MAX_PER_CELL: usize = 64;

/// Size of each grid cell in pixels.
/// Must be >= detect_range / 3 to ensure 3x3 neighbor search covers full range.
/// With detect_range=300px, cell_size=100px covers 300px (3 cells × 100px).
pub const CELL_SIZE: f32 = 100.0;

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

/// Energy restored when eating one food.
pub const ENERGY_FROM_EATING: f32 = 30.0;

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
