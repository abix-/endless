//! Constants - Tuning parameters for the NPC system

/// Maximum NPCs the system can handle. Buffers are pre-allocated to this size.
pub const MAX_NPC_COUNT: usize = 10000;

/// Spatial grid dimensions. The world is divided into GRID_WIDTH × GRID_HEIGHT cells.
pub const GRID_WIDTH: usize = 128;
pub const GRID_HEIGHT: usize = 128;
pub const GRID_CELLS: usize = GRID_WIDTH * GRID_HEIGHT;

/// Maximum NPCs per grid cell.
pub const MAX_PER_CELL: usize = 48;

/// Size of each grid cell in pixels.
pub const CELL_SIZE: f32 = 64.0;

/// Minimum distance NPCs try to maintain from each other.
pub const SEPARATION_RADIUS: f32 = 20.0;

/// How strongly NPCs push away from neighbors.
pub const SEPARATION_STRENGTH: f32 = 50.0;

/// Distance from target at which an NPC is considered "arrived".
pub const ARRIVAL_THRESHOLD: f32 = 8.0;

/// Floats per NPC instance in the MultiMesh buffer.
pub const FLOATS_PER_INSTANCE: usize = 12;

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

/// Energy drain per tick while active.
pub const ENERGY_DRAIN_RATE: f32 = 0.02;

/// Energy recovery per tick while resting.
pub const ENERGY_RECOVER_RATE: f32 = 0.2;
