//! Constants - Tuning parameters for the NPC system

mod buildings;
mod npcs;
mod upgrades;

pub use buildings::*;
pub use npcs::*;
pub use upgrades::*;

use bevy::reflect::Reflect;

/// Maximum NPCs the system can handle. NPC GPU buffers are pre-allocated to this size.
pub const MAX_NPC_COUNT: usize = 100000;

/// Maximum buildings with GPU slots. Building GPU buffers are pre-allocated to this size.
pub const MAX_BUILDINGS: usize = MAX_NPC_COUNT;

/// Total entity capacity: NPCs + buildings share unified GPU collision buffers.
pub const MAX_ENTITIES: usize = MAX_NPC_COUNT + MAX_BUILDINGS;

/// Universal soft cap for gameplay counters (proficiency, inventory, reputation).
/// Single constant so all caps move together if we raise the ceiling later.
pub const SOFT_CAP: usize = 9999;

/// Entity flag bits for unified entity_flags GPU buffer.
/// Bit 0: combat targeting enabled (archers, raiders, towers).
pub const ENTITY_FLAG_COMBAT: u32 = 1;
/// Bit 1: entity is a building (skip separation/NPC targeting in compute shader).
pub const ENTITY_FLAG_BUILDING: u32 = 2;
/// Bit 2: entity cannot be selected as a combat target (roads).
pub const ENTITY_FLAG_UNTARGETABLE: u32 = 4;

/// Neutral faction — friendly to everyone. Used for world-owned buildings (gold mines).
pub const FACTION_NEUTRAL: i32 = 0;
/// Player faction index (first non-neutral faction).
pub const FACTION_PLAYER: i32 = 1;
/// Sentinel town_idx for buildings not owned by any town (gold mines, etc.)
pub const TOWN_NONE: u32 = u32::MAX;

// Spatial grid lives on GPU only — see gpu.rs (256×256 cells × 128px = 32,768px coverage).

/// Distance from target at which an NPC is considered "arrived".
pub const ARRIVAL_THRESHOLD: f32 = 20.0;
/// Relaxed arrival threshold for intermediate A* waypoints (not final destination).
/// Prevents pile-up when boid separation pushes NPCs away from shared waypoints.
pub const INTERMEDIATE_ARRIVAL_THRESHOLD: f32 = 96.0;

/// Cells around each A* path cell that receive extra cost during batch accumulation (1 = 3×3 area).
pub const PATH_SPREAD_RADIUS: i32 = 1;
/// Cost added per affected cell during path accumulation. Grass=100, so +100 doubles traversal cost.
pub const PATH_SPREAD_COST: u16 = 100;

/// Floats per NPC instance in the MultiMesh buffer.
/// Transform2D (8) + Color (4) + CustomData (4) = 16
pub const FLOATS_PER_INSTANCE: usize = 16;

/// Size of push constants passed to the compute shader.
pub const PUSH_CONSTANTS_SIZE: usize = 48;

// Equipment sprite frames (column, row) — placeholder coordinates
pub const EQUIP_SWORD: (f32, f32) = (45.0, 6.0);
pub const EQUIP_HELMET: (f32, f32) = (28.0, 0.0);
pub const FOOD_SPRITE: (f32, f32) = (24.0, 9.0);
pub const GOLD_SPRITE: (f32, f32) = (41.0, 11.0);
pub const WOOD_SPRITE: (f32, f32) = (13.0, 9.0);
pub const STONE_SPRITE: (f32, f32) = (7.0, 15.0);

// Visual indicator sprites (column, row) — placeholder coordinates, verify against atlas
pub const SLEEP_SPRITE: (f32, f32) = (24.0, 7.0);
pub const HEAL_SPRITE: (f32, f32) = (23.0, 0.0);

// Distinct colors for raider factions (warm/aggressive palette)
pub const RAIDER_COLORS: [(f32, f32, f32); 10] = [
    (1.0, 0.0, 0.0), // Red
    (1.0, 0.5, 0.0), // Orange
    (1.0, 0.0, 1.0), // Magenta
    (0.5, 0.0, 1.0), // Purple
    (1.0, 1.0, 0.0), // Yellow
    (0.6, 0.3, 0.0), // Brown
    (1.0, 0.4, 0.7), // Pink
    (0.7, 0.0, 0.0), // Dark red
    (1.0, 0.8, 0.0), // Gold
    (0.6, 0.0, 0.4), // Dark magenta
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

/// Ticks an archer waits at a post before moving to next.
pub const ARCHER_PATROL_WAIT: u32 = 60;

/// Energy threshold to wake up from resting.
pub const ENERGY_WAKE_THRESHOLD: f32 = 90.0;

/// Energy threshold to stop working and seek rest.
pub const ENERGY_TIRED_THRESHOLD: f32 = 30.0;

/// Energy threshold below which NPCs consider eating (emergency only).
pub const ENERGY_EAT_THRESHOLD: f32 = 10.0;

/// HP per tick that a Mason restores to a damaged building.
pub const MASON_REPAIR_RATE: f32 = 2.0;

/// Search radius (pixels) for Mason to find damaged buildings.
pub const MASON_SEARCH_RADIUS: f32 = 6400.0;

// ============================================================================
// UTILITY AI ACTION SCORES
// ============================================================================

/// Base score for working (doing job).
pub const SCORE_WORK_BASE: f32 = 40.0;

/// Base score for wandering (idle movement).
pub const SCORE_WANDER_BASE: f32 = 10.0;

/// Multiplier for eat score (energy-based, slightly higher than rest).
pub const SCORE_EAT_MULT: f32 = 1.5;

/// Multiplier for rest score (energy-based).
pub const SCORE_REST_MULT: f32 = 1.0;

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

/// Cow growth rate (no farmer needed, grows day and night).
pub const COW_GROWTH_RATE: f32 = 0.12;

/// Food consumed per game hour while cows are growing (net cost).
pub const COW_FOOD_COST_PER_HOUR: i32 = 1;

/// Food produced per cow harvest cycle (higher than crops to offset cost).
pub const COW_HARVEST_YIELD: i32 = 3;

/// Maximum farms for item MultiMesh slot allocation.
pub const MAX_FARMS: usize = 500;

// ============================================================================
// PROJECTILE CONSTANTS
// ============================================================================

/// Maximum projectiles the system can handle.
pub const MAX_PROJECTILES: usize = 50000;

/// Oriented rectangle hitbox for arrow projectiles.
pub const PROJECTILE_HIT_HALF_LENGTH: f32 = 24.0; // along travel direction
pub const PROJECTILE_HIT_HALF_WIDTH: f32 = 8.0; // perpendicular to travel

/// Per-entity hitbox half-sizes (added to projectile hitbox via Minkowski sum).
/// NPC body is ~32x32 centered in 64x64 tile; buildings fill the full 64x64 tile.
pub const NPC_HITBOX_HALF: [f32; 2] = [16.0, 16.0];
pub const BUILDING_HITBOX_HALF: [f32; 2] = [32.0, 32.0];

/// Floats per projectile instance in MultiMesh buffer.
pub const PROJ_FLOATS_PER_INSTANCE: usize = 12;

/// Size of push constants for projectile compute shader.
pub const PROJ_PUSH_CONSTANTS_SIZE: usize = 32;

// ============================================================================
// RAIDER CONSTANTS
// ============================================================================

/// Food gained per game hour from passive foraging.
pub const RAIDER_FORAGE_RATE: i32 = 1;

/// Food cost to spawn one raider.
pub const RAIDER_SPAWN_COST: i32 = 5;

/// Hours between respawn attempts.
pub const RAIDER_RESPAWN_HOURS: f32 = 2.0;

/// Maximum raiders per town.
pub const RAIDER_MAX_POP: i32 = 500;

/// Minimum raiders needed to form a raid group.
pub const RAID_GROUP_SIZE: i32 = 3;

/// Villager population per raider town (1 raider town per 20 villagers).
pub const VILLAGERS_PER_RAIDER: i32 = 20;

// ============================================================================
// MIGRATION CONSTANTS
// ============================================================================

/// Game hours between migration trigger checks.
pub const RAIDER_SPAWN_CHECK_HOURS: f32 = 12.0;

/// Maximum dynamically-spawned raider towns.
pub const MAX_RAIDER_TOWNS: usize = 20;

/// Distance from a town at which migrating settlers settle (~5s walk at 100px/s).
pub const RAIDER_SETTLE_RADIUS: f32 = 500.0;

/// Boat movement speed (px/s) — faster than NPC walk (100px/s).
pub const BOAT_SPEED: f32 = 300.0;

/// Minimum raiders in a migrating group.
pub const MIGRATION_BASE_SIZE: usize = 3;

/// Game-hours delay before a replacement AI spawns in endless mode.
pub const ENDLESS_RESPAWN_DELAY_HOURS: f32 = 4.0;

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

/// Game hours before a dead NPC respawns from its building.
pub const SPAWNER_RESPAWN_HOURS: f32 = 12.0;

/// Town building grid spacing in pixels (matches WorldGrid cell_size for 1:1 alignment).
pub const TOWN_GRID_SPACING: f32 = 64.0;

/// Base grid extent: rows/cols from -4 to +3 = 8x8 starting area.
pub const BASE_GRID_MIN: i32 = -4;
pub const BASE_GRID_MAX: i32 = 3;

/// Maximum grid extent (rows/cols -49 to +50 = 100x100).
pub const MAX_GRID_EXTENT: i32 = 49;

// ============================================================================
// BUILDING TOWER STATS
// ============================================================================

/// Combat stats for a tower building (any building kind that auto-shoots).
#[derive(Clone, Copy, Debug)]
pub struct TowerStats {
    pub range: f32,
    pub damage: f32,
    pub cooldown: f32,
    pub proj_speed: f32,
    pub proj_lifetime: f32,
    pub hp_regen: f32,
    pub max_hp: f32,
}

pub const FOUNTAIN_TOWER: TowerStats = TowerStats {
    range: 800.0,
    damage: 15.0,
    cooldown: 1.5,
    proj_speed: 700.0,
    proj_lifetime: 1.5,
    hp_regen: 0.0,
    max_hp: 5000.0,
};

pub const BOW_TOWER_STATS: TowerStats = TowerStats {
    range: 200.0,
    damage: 15.0,
    cooldown: 1.5,
    proj_speed: 200.0,
    proj_lifetime: 1.5,
    hp_regen: 0.0,
    max_hp: 1000.0,
};

pub const CROSSBOW_TOWER_STATS: TowerStats = TowerStats {
    range: 250.0,
    damage: 25.0,
    cooldown: 2.0,
    proj_speed: 300.0,
    proj_lifetime: 1.5,
    hp_regen: 0.0,
    max_hp: 1200.0,
};

pub const CATAPULT_TOWER_STATS: TowerStats = TowerStats {
    range: 350.0,
    damage: 50.0,
    cooldown: 4.0,
    proj_speed: 150.0,
    proj_lifetime: 2.5,
    hp_regen: 0.0,
    max_hp: 800.0,
};

pub const GUARD_TOWER_STATS: TowerStats = TowerStats {
    range: 300.0,
    damage: 20.0,
    cooldown: 1.5,
    proj_speed: 250.0,
    proj_lifetime: 1.5,
    hp_regen: 0.0,
    max_hp: 1500.0,
};

// ============================================================================
// SQUAD CONSTANTS
// ============================================================================

/// Maximum number of player-controlled squads.
pub const MAX_SQUADS: usize = 10;

// ============================================================================
// NPC SKILLS / PROFICIENCY
// ============================================================================

pub const FARMING_SKILL_RATE: f32 = 0.02;
pub const COMBAT_SKILL_RATE: f32 = 1.0;
pub const DODGE_SKILL_RATE: f32 = 0.5;
pub const MAX_PROFICIENCY: f32 = SOFT_CAP as f32;

/// Default real-time seconds between AI decisions.
pub const DEFAULT_AI_INTERVAL: f32 = 5.0;

// ============================================================================
// GOLD MINE CONSTANTS
// ============================================================================

/// Gold extracted per harvest cycle (mine becomes Ready → miner takes this much).
pub const MINE_EXTRACT_PER_CYCLE: i32 = 5;

/// Seconds (at 1x speed) for a newly placed building to finish construction.
pub const BUILDING_CONSTRUCT_SECS: f32 = 10.0;

/// Tile flags bitfield (1 u32 per world grid cell in tile_flags GPU buffer).
/// Terrain bits (0-4): base terrain from Biome, set every rebuild.
pub const TILE_GRASS: u32 = 1; // bit 0
pub const TILE_FOREST: u32 = 2; // bit 1
pub const TILE_WATER: u32 = 4; // bit 2
pub const TILE_ROCK: u32 = 8; // bit 3
pub const TILE_DIRT: u32 = 16; // bit 4
/// Building bits (5+): OR'd on top of terrain.
pub const TILE_ROAD: u32 = 32; // bit 5 — 1.5x NPC speed
pub const TILE_WALL: u32 = 64; // bit 6 — blocks enemy faction NPCs
pub const WALL_FACTION_SHIFT: u32 = 8; // bits 8-11 encode wall owner faction
pub const WALL_FACTION_MASK: u32 = 0xF; // 4 bits = 16 factions

/// Per-tier wall HP values (indexed by wall_level - 1).
pub const WALL_TIER_HP: [f32; 3] = [80.0, 200.0, 400.0];
/// Per-tier wall names.
pub const WALL_TIER_NAMES: [&str; 3] = ["Wooden Palisade", "Stone Wall", "Fortified Wall"];
/// Cost to upgrade wall from tier N to tier N+1: (tier_index, &[(resource, amount)]).
pub const WALL_UPGRADE_COSTS: [&[(ResourceKind, i32)]; 2] = [
    &[(ResourceKind::Food, 2), (ResourceKind::Gold, 1)], // wooden → stone
    &[(ResourceKind::Food, 4), (ResourceKind::Gold, 2)], // stone → fortified
];

/// Tended growth rate for mines (per game-hour). 0.25 = 4 hours to full when miner is working.
pub const MINE_TENDED_GROWTH_RATE: f32 = 0.25;

/// Chop progress per game-hour. 0.5 = 2 hours for a woodcutter to fell a tree.
pub const TREE_CHOP_RATE: f32 = 0.5;

/// Quarry progress per game-hour. 0.33 = ~3 hours for a quarrier to break a rock.
pub const ROCK_QUARRY_RATE: f32 = 0.33;

/// Max distance from mine to continue tending (pushed away = abort + re-walk).
pub const MINE_WORK_RADIUS: f32 = 40.0;

/// Harmonic series multiplier for multi-miner productivity.
/// 1 miner = 1.0×, 2 = 1.5×, 3 = 1.83×, 4 = 2.08×.
pub fn mine_productivity_mult(worker_count: i32) -> f32 {
    let mut mult = 0.0_f32;
    for k in 1..=worker_count {
        mult += 1.0 / k as f32;
    }
    mult
}

/// Minimum distance from any settlement center to place a gold mine.
pub const MINE_MIN_SETTLEMENT_DIST: f32 = 300.0;

/// Minimum distance between gold mines.
pub const MINE_MIN_SPACING: f32 = 400.0;

/// Default town policy radius (pixels) for auto-mining discovery around fountain.
pub const DEFAULT_MINING_RADIUS: f32 = 2000.0;

/// Max items in TownEquipment per town. Excess pruned hourly (lowest value first -> gold).
pub const TOWN_EQUIPMENT_CAP: usize = SOFT_CAP;

// ============================================================================
// TOWN REGISTRY — single source of truth for all town types
// ============================================================================

/// What kind of faction this is — determines AI behavior and UI treatment.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, Reflect,
)]
pub enum FactionKind {
    Neutral,
    Player,
    AiBuilder,
    AiRaider,
}

/// Town type identity. Replaces implicit `is_raider: bool` branching.
#[derive(
    Clone, Copy, PartialEq, Eq, Hash, Debug, Reflect, serde::Serialize, serde::Deserialize,
)]
pub enum TownKind {
    Player,
    AiBuilder,
    AiRaider,
}

impl TownKind {
    pub fn faction_kind(self) -> FactionKind {
        match self {
            TownKind::Player => FactionKind::Player,
            TownKind::AiBuilder => FactionKind::AiBuilder,
            TownKind::AiRaider => FactionKind::AiRaider,
        }
    }
}

/// Complete town type definition — one entry per TownKind variant.
#[derive(Clone, Copy, Debug)]
pub struct TownDef {
    pub kind: TownKind,
    pub label: &'static str,
    pub is_raider: bool,
}

pub const TOWN_REGISTRY: &[TownDef] = &[
    TownDef {
        kind: TownKind::Player,
        label: "Village",
        is_raider: false,
    },
    TownDef {
        kind: TownKind::AiBuilder,
        label: "Settlement",
        is_raider: false,
    },
    TownDef {
        kind: TownKind::AiRaider,
        label: "Raider Camp",
        is_raider: true,
    },
];

/// Look up town definition by kind. Panics if kind not in registry.
pub fn town_def(kind: TownKind) -> &'static TownDef {
    TOWN_REGISTRY
        .iter()
        .find(|d| d.kind == kind)
        .unwrap_or_else(|| panic!("no TownDef for {:?}", kind))
}

// ============================================================================
// ATLAS IDS (shared between gpu.rs, render.rs, and npc_render.wgsl)
// ============================================================================

pub const ATLAS_BUILDING: f32 = 7.0;
pub const ATLAS_BOAT: f32 = 8.0;

// ============================================================================
// UNIT TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Job;
    use crate::world::BuildingKind;

    // -- roll_loot_item ------------------------------------------------------

    #[test]
    fn roll_loot_item_deterministic() {
        let a = roll_loot_item(1, 42);
        let b = roll_loot_item(1, 42);
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.rarity, b.rarity);
        assert!((a.stat_bonus - b.stat_bonus).abs() < f32::EPSILON);
        assert_eq!(a.name, b.name);
    }

    #[test]
    fn roll_loot_item_different_seeds_differ() {
        let a = roll_loot_item(1, 42);
        let b = roll_loot_item(1, 9999);
        // Different seeds should produce different items (extremely unlikely to collide)
        assert!(
            a.kind != b.kind || a.rarity != b.rarity || a.name != b.name,
            "different seeds should usually produce different items"
        );
    }

    #[test]
    fn roll_loot_item_stat_bonus_in_rarity_range() {
        for seed in 0..100 {
            let item = roll_loot_item(1, seed);
            let (min, max) = item.rarity.stat_range();
            assert!(
                item.stat_bonus >= min && item.stat_bonus <= max,
                "seed {seed}: bonus {} outside [{min}, {max}] for {:?}",
                item.stat_bonus,
                item.rarity
            );
        }
    }

    // -- Rarity --------------------------------------------------------------

    #[test]
    fn rarity_stat_ranges_ordered() {
        let rarities = [Rarity::Common, Rarity::Uncommon, Rarity::Rare, Rarity::Epic];
        for [lo, hi] in rarities.array_windows() {
            let (_, max_lower) = lo.stat_range();
            let (min_upper, _) = hi.stat_range();
            assert!(
                min_upper >= max_lower,
                "{:?} max {} should be <= {:?} min {}",
                lo,
                max_lower,
                hi,
                min_upper
            );
        }
    }

    #[test]
    fn rarity_gold_costs_increase() {
        let rarities = [Rarity::Common, Rarity::Uncommon, Rarity::Rare, Rarity::Epic];
        for [lo, hi] in rarities.array_windows() {
            assert!(
                hi.gold_cost() > lo.gold_cost(),
                "{:?} cost {} should be > {:?} cost {}",
                hi,
                hi.gold_cost(),
                lo,
                lo.gold_cost()
            );
        }
    }

    #[test]
    fn rarity_labels_non_empty() {
        for r in [Rarity::Common, Rarity::Uncommon, Rarity::Rare, Rarity::Epic] {
            assert!(!r.label().is_empty());
        }
    }

    // -- mine_productivity_mult ----------------------------------------------

    #[test]
    fn mine_productivity_zero_workers() {
        // 0 workers = 0.0 (harmonic series sum of 0 terms)
        assert!((mine_productivity_mult(0) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mine_productivity_one_worker() {
        assert!((mine_productivity_mult(1) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mine_productivity_diminishing_returns() {
        // Each additional worker adds less than the previous
        let m2 = mine_productivity_mult(2);
        let m3 = mine_productivity_mult(3);
        let m4 = mine_productivity_mult(4);
        let gain_2 = m2 - mine_productivity_mult(1);
        let gain_3 = m3 - m2;
        let gain_4 = m4 - m3;
        assert!(gain_3 < gain_2, "3rd worker should add less than 2nd");
        assert!(gain_4 < gain_3, "4th worker should add less than 3rd");
    }

    // -- npc_def (registry coverage) -----------------------------------------

    #[test]
    fn all_jobs_have_npc_def() {
        let jobs = [
            Job::Farmer,
            Job::Archer,
            Job::Raider,
            Job::Fighter,
            Job::Miner,
            Job::Crossbow,
            Job::Boat,
        ];
        for job in jobs {
            let def = npc_def(job);
            assert!(def.base_hp > 0.0, "{:?} should have positive base HP", job);
            assert!(
                def.base_speed > 0.0,
                "{:?} should have positive base speed",
                job
            );
        }
    }

    // -- building_def (registry coverage) ------------------------------------

    #[test]
    fn all_building_kinds_have_def() {
        let kinds = [
            BuildingKind::Fountain,
            BuildingKind::Waypoint,
            BuildingKind::Farm,
            BuildingKind::FarmerHome,
            BuildingKind::ArcherHome,
            BuildingKind::Tent,
            BuildingKind::GoldMine,
            BuildingKind::MinerHome,
            BuildingKind::CrossbowHome,
            BuildingKind::FighterHome,
            BuildingKind::Road,
            BuildingKind::StoneRoad,
            BuildingKind::MetalRoad,
            BuildingKind::Wall,
            BuildingKind::BowTower,
            BuildingKind::CrossbowTower,
            BuildingKind::CatapultTower,
            BuildingKind::Merchant,
            BuildingKind::Casino,
            BuildingKind::TreeNode,
            BuildingKind::RockNode,
            BuildingKind::LumberMill,
            BuildingKind::Quarry,
            BuildingKind::MasonHome,
            BuildingKind::Gate,
            BuildingKind::GuardTower,
        ];
        for kind in kinds {
            let def = building_def(kind);
            assert!(!def.label.is_empty(), "{:?} should have a label", kind);
        }
    }

    // -- raider_faction_color ------------------------------------------------

    #[test]
    fn raider_faction_color_wraps() {
        let c1 = raider_faction_color(1);
        let c11 = raider_faction_color(11); // should wrap to same as 1
        assert_eq!(c1, c11);
    }

    #[test]
    fn raider_faction_color_no_panic_edge_cases() {
        raider_faction_color(0);
        raider_faction_color(-1);
        raider_faction_color(100);
    }

    // -- autotile helpers ----------------------------------------------------

    #[test]
    fn autotile_kind_count_positive() {
        assert!(autotile_kind_count() > 0);
    }

    #[test]
    fn autotile_order_wall_exists() {
        assert!(autotile_order(BuildingKind::Wall).is_some());
    }

    #[test]
    fn autotile_order_farm_none() {
        assert!(
            autotile_order(BuildingKind::Farm).is_none(),
            "farms don't autotile"
        );
    }
}
