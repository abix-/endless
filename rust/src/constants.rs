//! Constants - Tuning parameters for the NPC system

use bevy::prelude::Vec2;
use crate::components::{Job, BaseAttackType};
use crate::world::{BuildingKind, WorldData, PlacedBuilding, is_alive};
use crate::resources::BuildingHpState;
use serde_json::Value as JsonValue;

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

// ============================================================================
// NPC REGISTRY — single source of truth for all NPC types
// ============================================================================

/// Per-attack-type stats (range, cooldown, projectile behavior).
#[derive(Clone, Copy, Debug)]
pub struct AttackTypeStats {
    pub range: f32,
    pub cooldown: f32,
    pub projectile_speed: f32,
    pub projectile_lifetime: f32,
}

/// Complete NPC type definition — one entry per Job variant.
#[derive(Clone, Copy, Debug)]
pub struct NpcDef {
    pub job: Job,
    pub label: &'static str,
    pub label_plural: &'static str,
    pub sprite: (f32, f32),
    pub color: (f32, f32, f32, f32),
    // Base combat stats
    pub base_hp: f32,
    pub base_damage: f32,
    pub base_speed: f32,
    pub default_attack_type: BaseAttackType,
    /// Per-job attack override (e.g. crossbow has different range/cooldown than generic Ranged).
    pub attack_override: Option<AttackTypeStats>,
    // Classification
    pub is_patrol_unit: bool,
    pub is_military: bool,
    // Spawn component flags
    pub has_energy: bool,
    pub has_attack_timer: bool,
    pub weapon: Option<(f32, f32)>,
    pub helmet: Option<(f32, f32)>,
    pub stealer: bool,
    pub leash_range: Option<f32>,
    /// UI text color for roster/panels (softer than GPU sprite `color`).
    pub ui_color: (u8, u8, u8),
    /// Which building this NPC type spawns from (for world gen & menu).
    pub home_building: BuildingKind,
    /// True for raider-camp units (menu groups under "Raider Camps"), false for village units.
    pub is_camp_unit: bool,
    /// Default count per town/camp in world gen.
    pub default_count: usize,
}

pub const NPC_REGISTRY: &[NpcDef] = &[
    NpcDef {
        job: Job::Farmer, label: "Farmer", label_plural: "Farmers",
        sprite: (1.0, 6.0), color: (0.0, 1.0, 0.0, 1.0),
        base_hp: 100.0, base_damage: 0.0, base_speed: 100.0,
        default_attack_type: BaseAttackType::Melee, attack_override: None,
        is_patrol_unit: false, is_military: false,
        has_energy: true, has_attack_timer: false,
        weapon: None, helmet: None, stealer: false, leash_range: None,
        ui_color: (80, 200, 80),
        home_building: BuildingKind::FarmerHome, is_camp_unit: false, default_count: 2,
    },
    NpcDef {
        job: Job::Archer, label: "Archer", label_plural: "Archers",
        sprite: (0.0, 0.0), color: (0.0, 0.0, 1.0, 1.0),
        base_hp: 100.0, base_damage: 15.0, base_speed: 100.0,
        default_attack_type: BaseAttackType::Ranged, attack_override: None,
        is_patrol_unit: true, is_military: true,
        has_energy: true, has_attack_timer: true,
        weapon: Some(EQUIP_SWORD), helmet: Some(EQUIP_HELMET), stealer: false, leash_range: None,
        ui_color: (80, 100, 220),
        home_building: BuildingKind::ArcherHome, is_camp_unit: false, default_count: 4,
    },
    NpcDef {
        job: Job::Raider, label: "Raider", label_plural: "Raiders",
        sprite: (0.0, 6.0), color: (1.0, 0.0, 0.0, 1.0),
        base_hp: 100.0, base_damage: 15.0, base_speed: 100.0,
        default_attack_type: BaseAttackType::Melee, attack_override: None,
        is_patrol_unit: false, is_military: true,
        has_energy: true, has_attack_timer: true,
        weapon: Some(EQUIP_SWORD), helmet: None, stealer: true, leash_range: Some(400.0),
        ui_color: (220, 80, 80),
        home_building: BuildingKind::Tent, is_camp_unit: true, default_count: 1,
    },
    NpcDef {
        job: Job::Fighter, label: "Fighter", label_plural: "Fighters",
        sprite: (1.0, 9.0), color: (1.0, 1.0, 0.0, 1.0),
        base_hp: 100.0, base_damage: 22.5, base_speed: 100.0,
        default_attack_type: BaseAttackType::Melee,
        attack_override: None,
        is_patrol_unit: true, is_military: true,
        has_energy: true, has_attack_timer: true,
        weapon: None, helmet: None, stealer: false, leash_range: None,
        ui_color: (220, 220, 80),
        home_building: BuildingKind::FighterHome, is_camp_unit: false, default_count: 0,
    },
    NpcDef {
        job: Job::Miner, label: "Miner", label_plural: "Miners",
        sprite: (1.0, 6.0), color: (0.6, 0.4, 0.2, 1.0),
        base_hp: 100.0, base_damage: 0.0, base_speed: 100.0,
        default_attack_type: BaseAttackType::Melee, attack_override: None,
        is_patrol_unit: false, is_military: false,
        has_energy: true, has_attack_timer: false,
        weapon: None, helmet: None, stealer: false, leash_range: None,
        ui_color: (160, 110, 60),
        home_building: BuildingKind::MinerHome, is_camp_unit: false, default_count: 0,
    },
    NpcDef {
        job: Job::Crossbow, label: "Crossbow", label_plural: "Crossbows",
        sprite: (0.0, 0.0), color: (0.4, 0.0, 0.8, 1.0),
        base_hp: 100.0, base_damage: 25.0, base_speed: 100.0,
        default_attack_type: BaseAttackType::Ranged,
        attack_override: Some(AttackTypeStats { range: 150.0, cooldown: 2.0, projectile_speed: 150.0, projectile_lifetime: 1.5 }),
        is_patrol_unit: true, is_military: true,
        has_energy: true, has_attack_timer: true,
        weapon: Some(EQUIP_SWORD), helmet: Some(EQUIP_HELMET), stealer: false, leash_range: None,
        ui_color: (140, 60, 220),
        home_building: BuildingKind::CrossbowHome, is_camp_unit: false, default_count: 0,
    },
];

/// Look up NPC definition by job. Panics if job not in registry.
pub fn npc_def(job: Job) -> &'static NpcDef {
    NPC_REGISTRY.iter().find(|d| d.job == job)
        .unwrap_or_else(|| panic!("no NpcDef for {:?}", job))
}

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
}

pub const FOUNTAIN_TOWER: TowerStats = TowerStats {
    range: 400.0, damage: 15.0, cooldown: 1.5, proj_speed: 350.0, proj_lifetime: 1.5,
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

/// Distance within which a waypoint "covers" a gold mine (AI territory logic).
pub const WAYPOINT_COVER_RADIUS: f32 = 200.0;

/// Radius for projectile-vs-building collision detection on CPU.
pub const BUILDING_HIT_RADIUS: f32 = 20.0;

// ============================================================================
// BUILDING REGISTRY — single source of truth for all building definitions
// ============================================================================

/// Tile specification: single 16x16 sprite or 2x2 composite of four 16x16 sprites.
#[derive(Clone, Copy, Debug)]
pub enum TileSpec {
    Single(u32, u32),
    Quad([(u32, u32); 4]),  // [TL, TR, BL, BR]
    External(&'static str), // asset path, e.g. "sprites/house.png"
}

/// How a building is placed on the map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlacementMode {
    /// Snap to town grid (farms, homes, beds, tents).
    TownGrid,
    /// Snap to world grid (waypoints, fountains, camps, gold mines).
    Wilderness,
}

/// Special action when a building is placed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OnPlace {
    None,
    /// Push to GrowthStates (farms).
    InitFarmGrowth,
}

/// How a spawner building finds work/patrol targets for its NPC.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpawnBehavior {
    /// Find nearest free farm in own town (farmer).
    FindNearestFarm,
    /// Find nearest waypoint for patrol (archer, crossbow).
    FindNearestWaypoint,
    /// Use camp faction (tent → raider).
    CampRaider,
    /// Use assigned mine or find nearest (miner).
    Miner,
}

/// NPC spawner definition — what kind of NPC a building produces.
#[derive(Clone, Copy, Debug)]
pub struct SpawnerDef {
    pub job: i32,           // Job::from_i32 index (0=Farmer, 1=Archer, 2=Raider, 4=Miner, 5=Crossbow)
    pub attack_type: i32,   // 0=melee, 1=ranged bow, 2=ranged xbow
    pub behavior: SpawnBehavior,
}

/// Factions tab column assignment for building display.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DisplayCategory { Hidden, Economy, Military }

/// Complete building definition — one entry per BuildingKind.
/// Index in BUILDING_REGISTRY = tileset index for GPU rendering.
#[derive(Clone, Copy, Debug)]
pub struct BuildingDef {
    pub kind: BuildingKind,
    pub display: DisplayCategory,
    pub tile: TileSpec,
    pub hp: f32,
    pub cost: i32,
    pub label: &'static str,
    pub help: &'static str,
    pub player_buildable: bool,
    pub camp_buildable: bool,
    pub placement: PlacementMode,
    pub is_tower: bool,
    pub tower_stats: Option<TowerStats>,
    pub on_place: OnPlace,
    pub spawner: Option<SpawnerDef>,
    /// Total slot count for this kind in WorldData (including tombstoned).
    pub len: fn(&WorldData) -> usize,
    /// Get (position, town_idx) for building at index. None if tombstoned or out of range.
    pub pos_town: fn(&WorldData, usize) -> Option<(Vec2, u32)>,
    /// Count alive buildings of this kind for a given town_idx.
    pub count_for_town: fn(&WorldData, u32) -> usize,
    /// Immutable access to this kind's HP vec.
    pub hps: fn(&BuildingHpState) -> &[f32],
    /// Mutable access to this kind's HP vec.
    pub hps_mut: fn(&mut BuildingHpState) -> &mut Vec<f32>,
    /// Save key in JSON (None for Fountain/Camp which share towns vec).
    pub save_key: Option<&'static str>,
    /// Serialize this kind's WorldData vec to JSON.
    pub save_vec: fn(&WorldData) -> JsonValue,
    /// Deserialize JSON into this kind's WorldData vec.
    pub load_vec: fn(&mut WorldData, JsonValue),
    /// Whether this kind uses unit_homes BTreeMap storage.
    pub is_unit_home: bool,
    /// Push a new building into WorldData at position with town_idx.
    pub place: fn(&mut WorldData, Vec2, u32),
    /// Tombstone (soft-delete) a building near the given position.
    pub tombstone: fn(&mut WorldData, Vec2),
    /// Find index of building near position.
    pub find_index: fn(&WorldData, Vec2) -> Option<usize>,
}

/// Single source of truth for all building types.
/// Order must match tileset strip (index = tileset_index).
pub const BUILDING_REGISTRY: &[BuildingDef] = &[
    // 0: Fountain (town center, auto-shoots)
    BuildingDef {
        kind: BuildingKind::Fountain, display: DisplayCategory::Hidden,
        tile: TileSpec::Single(50, 9),
        hp: 500.0, cost: 0,
        label: "Fountain", help: "Town center",
        player_buildable: false, camp_buildable: false,
        placement: PlacementMode::Wilderness,
        is_tower: true, tower_stats: Some(FOUNTAIN_TOWER),
        on_place: OnPlace::None, spawner: None,
        len: |wd| wd.towns.len(),
        pos_town: |wd, i| wd.towns.get(i).map(|t| (t.center, i as u32)),
        count_for_town: |wd, ti| if wd.towns.get(ti as usize).map(|t| t.sprite_type == 0).unwrap_or(false) { 1 } else { 0 },
        hps: |hp| &hp.towns,
        hps_mut: |hp| &mut hp.towns,
        save_key: None, save_vec: |_| JsonValue::Null, load_vec: |_, _| {},
        is_unit_home: false,
        place: |_, _, _| {},
        tombstone: |_, _| {},
        find_index: |_, _| None,
    },
    // 1: Bed
    BuildingDef {
        kind: BuildingKind::Bed, display: DisplayCategory::Hidden,
        tile: TileSpec::Single(15, 2),
        hp: 50.0, cost: 0,
        label: "Bed", help: "NPC rest spot",
        player_buildable: false, camp_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false, tower_stats: None,
        on_place: OnPlace::None, spawner: None,
        len: |wd| wd.get(BuildingKind::Bed).len(),
        pos_town: |wd, i| wd.get(BuildingKind::Bed).get(i).filter(|b| is_alive(b.position)).map(|b| (b.position, b.town_idx)),
        count_for_town: |wd, ti| wd.get(BuildingKind::Bed).iter().filter(|b| is_alive(b.position) && b.town_idx == ti).count(),
        hps: |hp| hp.hps.get(&BuildingKind::Bed).map(|v| v.as_slice()).unwrap_or(&[]),
        hps_mut: |hp| hp.hps.entry(BuildingKind::Bed).or_default(),
        save_key: Some("beds"),
        save_vec: |wd| serde_json::to_value(wd.get(BuildingKind::Bed)).unwrap(),
        load_vec: |wd, v| { wd.buildings.insert(BuildingKind::Bed, serde_json::from_value(v).unwrap_or_default()); },
        is_unit_home: false,
        place: |wd, pos, ti| wd.get_mut(BuildingKind::Bed).push(PlacedBuilding::new(pos, ti)),
        tombstone: |wd, pos| { if let Some(b) = wd.get_mut(BuildingKind::Bed).iter_mut().find(|b| (b.position - pos).length() < 1.0) { b.position = Vec2::new(-99999.0, -99999.0); } },
        find_index: |wd, pos| wd.get(BuildingKind::Bed).iter().position(|b| (b.position - pos).length() < 1.0),
    },
    // 2: Waypoint
    BuildingDef {
        kind: BuildingKind::Waypoint, display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/waypoint.png"),
        hp: 200.0, cost: 1,
        label: "Waypoint", help: "Patrol waypoint",
        player_buildable: true, camp_buildable: false,
        placement: PlacementMode::Wilderness,
        is_tower: false, tower_stats: None,
        on_place: OnPlace::None, spawner: None,
        len: |wd| wd.get(BuildingKind::Waypoint).len(),
        pos_town: |wd, i| wd.get(BuildingKind::Waypoint).get(i).filter(|b| is_alive(b.position)).map(|b| (b.position, b.town_idx)),
        count_for_town: |wd, ti| wd.get(BuildingKind::Waypoint).iter().filter(|b| is_alive(b.position) && b.town_idx == ti).count(),
        hps: |hp| hp.hps.get(&BuildingKind::Waypoint).map(|v| v.as_slice()).unwrap_or(&[]),
        hps_mut: |hp| hp.hps.entry(BuildingKind::Waypoint).or_default(),
        save_key: Some("waypoints"),
        save_vec: |wd| serde_json::to_value(wd.get(BuildingKind::Waypoint)).unwrap(),
        load_vec: |wd, v| { wd.buildings.insert(BuildingKind::Waypoint, serde_json::from_value(v).unwrap_or_default()); },
        is_unit_home: false,
        place: |wd, pos, ti| wd.get_mut(BuildingKind::Waypoint).push(PlacedBuilding::new(pos, ti)),
        tombstone: |wd, pos| { if let Some(b) = wd.get_mut(BuildingKind::Waypoint).iter_mut().find(|b| (b.position - pos).length() < 1.0) { b.position = Vec2::new(-99999.0, -99999.0); } },
        find_index: |wd, pos| wd.get(BuildingKind::Waypoint).iter().position(|b| (b.position - pos).length() < 1.0),
    },
    // 3: Farm
    BuildingDef {
        kind: BuildingKind::Farm, display: DisplayCategory::Economy,
        tile: TileSpec::Quad([(2, 15), (4, 15), (2, 17), (4, 17)]),
        hp: 80.0, cost: 2,
        label: "Farm", help: "Grows food over time",
        player_buildable: true, camp_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false, tower_stats: None,
        on_place: OnPlace::InitFarmGrowth, spawner: None,
        len: |wd| wd.get(BuildingKind::Farm).len(),
        pos_town: |wd, i| wd.get(BuildingKind::Farm).get(i).filter(|b| is_alive(b.position)).map(|b| (b.position, b.town_idx)),
        count_for_town: |wd, ti| wd.get(BuildingKind::Farm).iter().filter(|b| is_alive(b.position) && b.town_idx == ti).count(),
        hps: |hp| hp.hps.get(&BuildingKind::Farm).map(|v| v.as_slice()).unwrap_or(&[]),
        hps_mut: |hp| hp.hps.entry(BuildingKind::Farm).or_default(),
        save_key: Some("farms"),
        save_vec: |wd| serde_json::to_value(wd.get(BuildingKind::Farm)).unwrap(),
        load_vec: |wd, v| { wd.buildings.insert(BuildingKind::Farm, serde_json::from_value(v).unwrap_or_default()); },
        is_unit_home: false,
        place: |wd, pos, ti| wd.get_mut(BuildingKind::Farm).push(PlacedBuilding::new(pos, ti)),
        tombstone: |wd, pos| { if let Some(b) = wd.get_mut(BuildingKind::Farm).iter_mut().find(|b| (b.position - pos).length() < 1.0) { b.position = Vec2::new(-99999.0, -99999.0); } },
        find_index: |wd, pos| wd.get(BuildingKind::Farm).iter().position(|b| (b.position - pos).length() < 1.0),
    },
    // 4: Camp (raider town center)
    BuildingDef {
        kind: BuildingKind::Camp, display: DisplayCategory::Hidden,
        tile: TileSpec::Quad([(46, 10), (47, 10), (46, 11), (47, 11)]),
        hp: 500.0, cost: 0,
        label: "Camp", help: "Raider camp center",
        player_buildable: false, camp_buildable: false,
        placement: PlacementMode::Wilderness,
        is_tower: false, tower_stats: None,
        on_place: OnPlace::None, spawner: None,
        len: |wd| wd.towns.len(),
        pos_town: |wd, i| wd.towns.get(i).map(|t| (t.center, i as u32)),
        count_for_town: |wd, ti| if wd.towns.get(ti as usize).map(|t| t.sprite_type == 1).unwrap_or(false) { 1 } else { 0 },
        hps: |hp| &hp.towns,
        hps_mut: |hp| &mut hp.towns,
        save_key: None, save_vec: |_| JsonValue::Null, load_vec: |_, _| {},
        is_unit_home: false,
        place: |_, _, _| {},
        tombstone: |_, _| {},
        find_index: |_, _| None,
    },
    // 5: Farmer Home
    BuildingDef {
        kind: BuildingKind::FarmerHome, display: DisplayCategory::Economy,
        tile: TileSpec::External("sprites/house.png"),
        hp: 100.0, cost: 2,
        label: "Farmer Home", help: "Spawns 1 farmer",
        player_buildable: true, camp_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false, tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef { job: 0, attack_type: 0, behavior: SpawnBehavior::FindNearestFarm }),
        len: |wd| wd.get(BuildingKind::FarmerHome).len(),
        pos_town: |wd, i| wd.get(BuildingKind::FarmerHome).get(i).filter(|b| is_alive(b.position)).map(|b| (b.position, b.town_idx)),
        count_for_town: |wd, ti| wd.get(BuildingKind::FarmerHome).iter().filter(|b| is_alive(b.position) && b.town_idx == ti).count(),
        hps: |hp| hp.hps.get(&BuildingKind::FarmerHome).map(|v| v.as_slice()).unwrap_or(&[]),
        hps_mut: |hp| hp.hps.entry(BuildingKind::FarmerHome).or_default(),
        save_key: Some("farmer_homes"),
        save_vec: |wd| serde_json::to_value(wd.get(BuildingKind::FarmerHome)).unwrap(),
        load_vec: |wd, v| { wd.buildings.insert(BuildingKind::FarmerHome, serde_json::from_value(v).unwrap_or_default()); },
        is_unit_home: true,
        place: |wd, pos, ti| wd.get_mut(BuildingKind::FarmerHome).push(PlacedBuilding::new(pos, ti)),
        tombstone: |wd, pos| { if let Some(b) = wd.get_mut(BuildingKind::FarmerHome).iter_mut().find(|b| (b.position - pos).length() < 1.0) { b.position = Vec2::new(-99999.0, -99999.0); } },
        find_index: |wd, pos| wd.get(BuildingKind::FarmerHome).iter().position(|b| (b.position - pos).length() < 1.0),
    },
    // 6: Archer Home
    BuildingDef {
        kind: BuildingKind::ArcherHome, display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/barracks.png"),
        hp: 150.0, cost: 4,
        label: "Archer Home", help: "Spawns 1 archer",
        player_buildable: true, camp_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false, tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef { job: 1, attack_type: 1, behavior: SpawnBehavior::FindNearestWaypoint }),
        len: |wd| wd.get(BuildingKind::ArcherHome).len(),
        pos_town: |wd, i| wd.get(BuildingKind::ArcherHome).get(i).filter(|b| is_alive(b.position)).map(|b| (b.position, b.town_idx)),
        count_for_town: |wd, ti| wd.get(BuildingKind::ArcherHome).iter().filter(|b| is_alive(b.position) && b.town_idx == ti).count(),
        hps: |hp| hp.hps.get(&BuildingKind::ArcherHome).map(|v| v.as_slice()).unwrap_or(&[]),
        hps_mut: |hp| hp.hps.entry(BuildingKind::ArcherHome).or_default(),
        save_key: Some("archer_homes"),
        save_vec: |wd| serde_json::to_value(wd.get(BuildingKind::ArcherHome)).unwrap(),
        load_vec: |wd, v| { wd.buildings.insert(BuildingKind::ArcherHome, serde_json::from_value(v).unwrap_or_default()); },
        is_unit_home: true,
        place: |wd, pos, ti| wd.get_mut(BuildingKind::ArcherHome).push(PlacedBuilding::new(pos, ti)),
        tombstone: |wd, pos| { if let Some(b) = wd.get_mut(BuildingKind::ArcherHome).iter_mut().find(|b| (b.position - pos).length() < 1.0) { b.position = Vec2::new(-99999.0, -99999.0); } },
        find_index: |wd, pos| wd.get(BuildingKind::ArcherHome).iter().position(|b| (b.position - pos).length() < 1.0),
    },
    // 7: Tent (raider spawner)
    BuildingDef {
        kind: BuildingKind::Tent, display: DisplayCategory::Military,
        tile: TileSpec::Quad([(48, 10), (49, 10), (48, 11), (49, 11)]),
        hp: 100.0, cost: 3,
        label: "Tent", help: "Spawns 1 raider",
        player_buildable: false, camp_buildable: true,
        placement: PlacementMode::TownGrid,
        is_tower: false, tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef { job: 2, attack_type: 0, behavior: SpawnBehavior::CampRaider }),
        len: |wd| wd.get(BuildingKind::Tent).len(),
        pos_town: |wd, i| wd.get(BuildingKind::Tent).get(i).filter(|b| is_alive(b.position)).map(|b| (b.position, b.town_idx)),
        count_for_town: |wd, ti| wd.get(BuildingKind::Tent).iter().filter(|b| is_alive(b.position) && b.town_idx == ti).count(),
        hps: |hp| hp.hps.get(&BuildingKind::Tent).map(|v| v.as_slice()).unwrap_or(&[]),
        hps_mut: |hp| hp.hps.entry(BuildingKind::Tent).or_default(),
        save_key: Some("tents"),
        save_vec: |wd| serde_json::to_value(wd.get(BuildingKind::Tent)).unwrap(),
        load_vec: |wd, v| { wd.buildings.insert(BuildingKind::Tent, serde_json::from_value(v).unwrap_or_default()); },
        is_unit_home: true,
        place: |wd, pos, ti| wd.get_mut(BuildingKind::Tent).push(PlacedBuilding::new(pos, ti)),
        tombstone: |wd, pos| { if let Some(b) = wd.get_mut(BuildingKind::Tent).iter_mut().find(|b| (b.position - pos).length() < 1.0) { b.position = Vec2::new(-99999.0, -99999.0); } },
        find_index: |wd, pos| wd.get(BuildingKind::Tent).iter().position(|b| (b.position - pos).length() < 1.0),
    },
    // 8: Gold Mine
    BuildingDef {
        kind: BuildingKind::GoldMine, display: DisplayCategory::Hidden,
        tile: TileSpec::Single(43, 11),
        hp: 200.0, cost: 0,
        label: "Gold Mine", help: "Source of gold",
        player_buildable: false, camp_buildable: false,
        placement: PlacementMode::Wilderness,
        is_tower: false, tower_stats: None,
        on_place: OnPlace::None, spawner: None,
        len: |wd| wd.get(BuildingKind::GoldMine).len(),
        pos_town: |wd, i| wd.get(BuildingKind::GoldMine).get(i).filter(|b| is_alive(b.position)).map(|b| (b.position, 0)),
        count_for_town: |wd, _| wd.get(BuildingKind::GoldMine).iter().filter(|b| is_alive(b.position)).count(),
        hps: |hp| hp.hps.get(&BuildingKind::GoldMine).map(|v| v.as_slice()).unwrap_or(&[]),
        hps_mut: |hp| hp.hps.entry(BuildingKind::GoldMine).or_default(),
        save_key: Some("gold_mines"),
        save_vec: |wd| serde_json::to_value(wd.get(BuildingKind::GoldMine)).unwrap(),
        load_vec: |wd, v| {
            wd.buildings.insert(BuildingKind::GoldMine,
                serde_json::from_value::<Vec<PlacedBuilding>>(v.clone())
                    .unwrap_or_else(|_| {
                        serde_json::from_value::<Vec<[f32; 2]>>(v)
                            .unwrap_or_default()
                            .into_iter()
                            .map(|[x, y]| PlacedBuilding::new(Vec2::new(x, y), 0))
                            .collect()
                    }));
        },
        is_unit_home: false,
        place: |wd, pos, _| wd.get_mut(BuildingKind::GoldMine).push(PlacedBuilding::new(pos, 0)),
        tombstone: |wd, pos| { if let Some(b) = wd.get_mut(BuildingKind::GoldMine).iter_mut().find(|b| (b.position - pos).length() < 1.0) { b.position = Vec2::new(-99999.0, -99999.0); } },
        find_index: |wd, pos| wd.get(BuildingKind::GoldMine).iter().position(|b| (b.position - pos).length() < 1.0),
    },
    // 9: Miner Home
    BuildingDef {
        kind: BuildingKind::MinerHome, display: DisplayCategory::Economy,
        tile: TileSpec::External("sprites/miner_house.png"),
        hp: 100.0, cost: 4,
        label: "Miner Home", help: "Spawns 1 miner",
        player_buildable: true, camp_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false, tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef { job: 4, attack_type: 0, behavior: SpawnBehavior::Miner }),
        len: |wd| wd.get(BuildingKind::MinerHome).len(),
        pos_town: |wd, i| wd.get(BuildingKind::MinerHome).get(i).filter(|b| is_alive(b.position)).map(|b| (b.position, b.town_idx)),
        count_for_town: |wd, ti| wd.get(BuildingKind::MinerHome).iter().filter(|b| is_alive(b.position) && b.town_idx == ti).count(),
        hps: |hp| hp.hps.get(&BuildingKind::MinerHome).map(|v| v.as_slice()).unwrap_or(&[]),
        hps_mut: |hp| hp.hps.entry(BuildingKind::MinerHome).or_default(),
        save_key: Some("miner_homes"),
        save_vec: |wd| serde_json::to_value(wd.get(BuildingKind::MinerHome)).unwrap(),
        load_vec: |wd, v| { wd.buildings.insert(BuildingKind::MinerHome, serde_json::from_value(v).unwrap_or_default()); },
        is_unit_home: false,
        place: |wd, pos, ti| wd.get_mut(BuildingKind::MinerHome).push(PlacedBuilding::new(pos, ti)),
        tombstone: |wd, pos| { if let Some(b) = wd.get_mut(BuildingKind::MinerHome).iter_mut().find(|b| (b.position - pos).length() < 1.0) { b.position = Vec2::new(-99999.0, -99999.0); } },
        find_index: |wd, pos| wd.get(BuildingKind::MinerHome).iter().position(|b| (b.position - pos).length() < 1.0),
    },
    // 10: Crossbow Home
    BuildingDef {
        kind: BuildingKind::CrossbowHome, display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/barracks.png"),
        hp: 150.0, cost: 8,
        label: "Crossbow Home", help: "Spawns 1 crossbow",
        player_buildable: true, camp_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false, tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef { job: 5, attack_type: 2, behavior: SpawnBehavior::FindNearestWaypoint }),
        len: |wd| wd.get(BuildingKind::CrossbowHome).len(),
        pos_town: |wd, i| wd.get(BuildingKind::CrossbowHome).get(i).filter(|b| is_alive(b.position)).map(|b| (b.position, b.town_idx)),
        count_for_town: |wd, ti| wd.get(BuildingKind::CrossbowHome).iter().filter(|b| is_alive(b.position) && b.town_idx == ti).count(),
        hps: |hp| hp.hps.get(&BuildingKind::CrossbowHome).map(|v| v.as_slice()).unwrap_or(&[]),
        hps_mut: |hp| hp.hps.entry(BuildingKind::CrossbowHome).or_default(),
        save_key: Some("crossbow_homes"),
        save_vec: |wd| serde_json::to_value(wd.get(BuildingKind::CrossbowHome)).unwrap(),
        load_vec: |wd, v| { wd.buildings.insert(BuildingKind::CrossbowHome, serde_json::from_value(v).unwrap_or_default()); },
        is_unit_home: true,
        place: |wd, pos, ti| wd.get_mut(BuildingKind::CrossbowHome).push(PlacedBuilding::new(pos, ti)),
        tombstone: |wd, pos| { if let Some(b) = wd.get_mut(BuildingKind::CrossbowHome).iter_mut().find(|b| (b.position - pos).length() < 1.0) { b.position = Vec2::new(-99999.0, -99999.0); } },
        find_index: |wd, pos| wd.get(BuildingKind::CrossbowHome).iter().position(|b| (b.position - pos).length() < 1.0),
    },
    // 11: Fighter Home
    BuildingDef {
        kind: BuildingKind::FighterHome, display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/fighter_home.png"),
        hp: 150.0, cost: 5,
        label: "Fighter Home", help: "Spawns 1 fighter",
        player_buildable: true, camp_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false, tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef { job: 3, attack_type: 0, behavior: SpawnBehavior::FindNearestWaypoint }),
        len: |wd| wd.get(BuildingKind::FighterHome).len(),
        pos_town: |wd, i| wd.get(BuildingKind::FighterHome).get(i).filter(|b| is_alive(b.position)).map(|b| (b.position, b.town_idx)),
        count_for_town: |wd, ti| wd.get(BuildingKind::FighterHome).iter().filter(|b| is_alive(b.position) && b.town_idx == ti).count(),
        hps: |hp| hp.hps.get(&BuildingKind::FighterHome).map(|v| v.as_slice()).unwrap_or(&[]),
        hps_mut: |hp| hp.hps.entry(BuildingKind::FighterHome).or_default(),
        save_key: Some("fighter_homes"),
        save_vec: |wd| serde_json::to_value(wd.get(BuildingKind::FighterHome)).unwrap(),
        load_vec: |wd, v| { wd.buildings.insert(BuildingKind::FighterHome, serde_json::from_value(v).unwrap_or_default()); },
        is_unit_home: true,
        place: |wd, pos, ti| wd.get_mut(BuildingKind::FighterHome).push(PlacedBuilding::new(pos, ti)),
        tombstone: |wd, pos| { if let Some(b) = wd.get_mut(BuildingKind::FighterHome).iter_mut().find(|b| (b.position - pos).length() < 1.0) { b.position = Vec2::new(-99999.0, -99999.0); } },
        find_index: |wd, pos| wd.get(BuildingKind::FighterHome).iter().position(|b| (b.position - pos).length() < 1.0),
    },
];

/// Look up a building definition by kind. Panics if kind is not in registry.
pub fn building_def(kind: BuildingKind) -> &'static BuildingDef {
    BUILDING_REGISTRY.iter().find(|d| d.kind == kind)
        .unwrap_or_else(|| panic!("no BuildingDef for {:?}", kind))
}

/// Look up the tileset index for a BuildingKind (its position in BUILDING_REGISTRY).
pub fn tileset_index(kind: BuildingKind) -> u16 {
    BUILDING_REGISTRY.iter().position(|d| d.kind == kind)
        .unwrap_or_else(|| panic!("no tileset index for {:?}", kind)) as u16
}

/// Food cost to build a building. Returns 0 for non-buildable types.
pub fn building_cost(kind: BuildingKind) -> i32 {
    building_def(kind).cost
}

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
