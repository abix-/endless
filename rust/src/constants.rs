//! Constants - Tuning parameters for the NPC system

use crate::components::{BaseAttackType, Job};
use crate::world::BuildingKind;

/// Maximum NPCs the system can handle. NPC GPU buffers are pre-allocated to this size.
pub const MAX_NPC_COUNT: usize = 100000;

/// Maximum buildings with GPU slots. Building GPU buffers are pre-allocated to this size.
pub const MAX_BUILDINGS: usize = MAX_NPC_COUNT;

/// Total entity capacity: NPCs + buildings share unified GPU collision buffers.
pub const MAX_ENTITIES: usize = MAX_NPC_COUNT + MAX_BUILDINGS;

/// Entity flag bits for unified entity_flags GPU buffer.
/// Bit 0: combat targeting enabled (archers, raiders, towers).
pub const ENTITY_FLAG_COMBAT: u32 = 1;
/// Bit 1: entity is a building (skip separation/NPC targeting in compute shader).
pub const ENTITY_FLAG_BUILDING: u32 = 2;
/// Bit 2: entity cannot be selected as a combat target (roads).
pub const ENTITY_FLAG_UNTARGETABLE: u32 = 4;

// ============================================================================
// UPGRADE STAT DEFINITIONS (used by NpcDef to declare upgradeable stats)
// ============================================================================

/// Resource types used in upgrade costs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Food,
    Gold,
}

/// Which stat an upgrade improves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UpgradeStatKind {
    // Core NPC stats
    Hp,
    Attack,
    Range,
    AttackSpeed,
    MoveSpeed,
    Stamina,
    // Special NPC stats
    Yield,
    Alert,
    Dodge,
    ProjectileSpeed,
    ProjectileLifetime,
    // Town-only stats (not NPC-driven)
    Healing,
    FountainRange,
    FountainAttackSpeed,
    FountainProjectileLife,
    Expansion,
}

/// How an upgrade's effect is displayed in the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectDisplay {
    /// +X% per level (standard multiplicative)
    Percentage,
    /// -X% cooldown reduction (reciprocal: 1/(1+n*pct))
    CooldownReduction,
    /// Binary unlock (level 0 = locked, level 1+ = unlocked)
    Unlock,
    /// +Npx per level (flat additive, displayed as pixels)
    FlatPixels(i32),
    /// +N per level (discrete integer)
    Discrete,
}

/// One upgradeable stat declaration on an NPC category.
#[derive(Clone, Copy, Debug)]
pub struct UpgradeStatDef {
    pub kind: UpgradeStatKind,
    pub pct: f32,
    pub cost: &'static [(ResourceKind, i32)],
    pub label: &'static str,
    pub short: &'static str,
    pub tooltip: &'static str,
    pub display: EffectDisplay,
    /// Prerequisite: another stat in the same category that must be at min_level.
    pub prereq_stat: Option<UpgradeStatKind>,
    pub prereq_level: u8,
    /// Whether this upgrade triggers NPC stat re-resolution.
    pub is_combat_stat: bool,
    /// Whether purchasing this triggers healing zone invalidation.
    pub invalidates_healing: bool,
    /// Whether purchasing this triggers town grid expansion.
    pub triggers_expansion: bool,
    /// Custom cost function instead of standard exponential scaling.
    pub custom_cost: bool,
}

use ResourceKind::{Food as F, Gold as G};
use UpgradeStatKind as USK;

// Helper for concise UpgradeStatDef construction
const fn usd(
    kind: UpgradeStatKind,
    pct: f32,
    cost: &'static [(ResourceKind, i32)],
    label: &'static str,
    short: &'static str,
    tooltip: &'static str,
    display: EffectDisplay,
) -> UpgradeStatDef {
    UpgradeStatDef {
        kind,
        pct,
        cost,
        label,
        short,
        tooltip,
        display,
        prereq_stat: None,
        prereq_level: 1,
        is_combat_stat: true,
        invalidates_healing: false,
        triggers_expansion: false,
        custom_cost: false,
    }
}

const fn usd_noncombat(
    kind: UpgradeStatKind,
    pct: f32,
    cost: &'static [(ResourceKind, i32)],
    label: &'static str,
    short: &'static str,
    tooltip: &'static str,
    display: EffectDisplay,
) -> UpgradeStatDef {
    UpgradeStatDef {
        kind,
        pct,
        cost,
        label,
        short,
        tooltip,
        display,
        prereq_stat: None,
        prereq_level: 1,
        is_combat_stat: false,
        invalidates_healing: false,
        triggers_expansion: false,
        custom_cost: false,
    }
}

const fn usd_req(
    kind: UpgradeStatKind,
    pct: f32,
    cost: &'static [(ResourceKind, i32)],
    label: &'static str,
    short: &'static str,
    tooltip: &'static str,
    display: EffectDisplay,
    prereq: UpgradeStatKind,
    prereq_lv: u8,
) -> UpgradeStatDef {
    UpgradeStatDef {
        kind,
        pct,
        cost,
        label,
        short,
        tooltip,
        display,
        prereq_stat: Some(prereq),
        prereq_level: prereq_lv,
        is_combat_stat: true,
        invalidates_healing: false,
        triggers_expansion: false,
        custom_cost: false,
    }
}

// Military upgrade stat defs
const MILITARY_RANGED_UPGRADES: &[UpgradeStatDef] = &[
    usd(
        USK::Hp,
        0.10,
        &[(F, 1)],
        "HP",
        "HP",
        "+10% HP per level",
        EffectDisplay::Percentage,
    ),
    usd(
        USK::Attack,
        0.10,
        &[(F, 1)],
        "Attack",
        "Atk",
        "+10% damage per level",
        EffectDisplay::Percentage,
    ),
    usd(
        USK::Range,
        0.05,
        &[(G, 1)],
        "Detection Range",
        "Det",
        "+5% detection range per level",
        EffectDisplay::Percentage,
    ),
    usd(
        USK::AttackSpeed,
        0.08,
        &[(F, 1)],
        "Attack Speed",
        "AtkSpd",
        "-8% attack cooldown per level",
        EffectDisplay::CooldownReduction,
    ),
    usd(
        USK::MoveSpeed,
        0.05,
        &[(F, 1)],
        "Move Speed",
        "MvSpd",
        "+5% movement speed per level",
        EffectDisplay::Percentage,
    ),
    usd_req(
        USK::Alert,
        0.10,
        &[(G, 1)],
        "Alert",
        "Alert",
        "+10% alert radius per level",
        EffectDisplay::Percentage,
        USK::MoveSpeed,
        1,
    ),
    usd_req(
        USK::Stamina,
        0.10,
        &[(F, 1)],
        "Stamina",
        "Stam",
        "-10% energy drain per level",
        EffectDisplay::CooldownReduction,
        USK::MoveSpeed,
        1,
    ),
    usd_req(
        USK::Dodge,
        0.0,
        &[(G, 20)],
        "Dodge",
        "Dodge",
        "Unlocks projectile dodging",
        EffectDisplay::Unlock,
        USK::MoveSpeed,
        5,
    ),
    usd(
        USK::ProjectileSpeed,
        0.08,
        &[(G, 1)],
        "Arrow Speed",
        "ASpd",
        "+8% arrow speed per level",
        EffectDisplay::Percentage,
    ),
    usd(
        USK::ProjectileLifetime,
        0.08,
        &[(G, 1)],
        "Arrow Range",
        "ARng",
        "+8% arrow flight distance per level",
        EffectDisplay::Percentage,
    ),
];

const MILITARY_MELEE_UPGRADES: &[UpgradeStatDef] = &[
    usd(
        USK::Hp,
        0.10,
        &[(F, 1)],
        "HP",
        "HP",
        "+10% HP per level",
        EffectDisplay::Percentage,
    ),
    usd(
        USK::Attack,
        0.10,
        &[(F, 1)],
        "Attack",
        "Atk",
        "+10% damage per level",
        EffectDisplay::Percentage,
    ),
    usd(
        USK::AttackSpeed,
        0.08,
        &[(F, 1)],
        "Attack Speed",
        "AtkSpd",
        "-8% attack cooldown per level",
        EffectDisplay::CooldownReduction,
    ),
    usd(
        USK::MoveSpeed,
        0.05,
        &[(F, 1)],
        "Move Speed",
        "MvSpd",
        "+5% movement speed per level",
        EffectDisplay::Percentage,
    ),
    usd_req(
        USK::Alert,
        0.10,
        &[(G, 1)],
        "Alert",
        "Alert",
        "+10% alert radius per level",
        EffectDisplay::Percentage,
        USK::MoveSpeed,
        1,
    ),
    usd_req(
        USK::Stamina,
        0.10,
        &[(F, 1)],
        "Stamina",
        "Stam",
        "-10% energy drain per level",
        EffectDisplay::CooldownReduction,
        USK::MoveSpeed,
        1,
    ),
    usd_req(
        USK::Dodge,
        0.0,
        &[(G, 20)],
        "Dodge",
        "Dodge",
        "Unlocks projectile dodging",
        EffectDisplay::Unlock,
        USK::MoveSpeed,
        5,
    ),
];

const FARMER_UPGRADES: &[UpgradeStatDef] = &[
    usd(
        USK::Yield,
        0.15,
        &[(F, 1)],
        "Yield",
        "Yield",
        "+15% food production per level",
        EffectDisplay::Percentage,
    ),
    usd(
        USK::Hp,
        0.20,
        &[(F, 1)],
        "HP",
        "HP",
        "+20% farmer HP per level",
        EffectDisplay::Percentage,
    ),
    usd(
        USK::MoveSpeed,
        0.05,
        &[(F, 1)],
        "Move Speed",
        "MvSpd",
        "+5% farmer speed per level",
        EffectDisplay::Percentage,
    ),
    usd_req(
        USK::Stamina,
        0.10,
        &[(F, 1)],
        "Stamina",
        "Stam",
        "-10% energy drain per level",
        EffectDisplay::CooldownReduction,
        USK::MoveSpeed,
        1,
    ),
];

const MINER_UPGRADES: &[UpgradeStatDef] = &[
    usd(
        USK::Hp,
        0.20,
        &[(F, 1)],
        "HP",
        "HP",
        "+20% miner HP per level",
        EffectDisplay::Percentage,
    ),
    usd(
        USK::MoveSpeed,
        0.05,
        &[(F, 1)],
        "Move Speed",
        "MvSpd",
        "+5% miner speed per level",
        EffectDisplay::Percentage,
    ),
    usd_req(
        USK::Stamina,
        0.10,
        &[(F, 1)],
        "Stamina",
        "Stam",
        "-10% energy drain per level",
        EffectDisplay::CooldownReduction,
        USK::MoveSpeed,
        1,
    ),
    usd_noncombat(
        USK::Yield,
        0.15,
        &[(G, 1)],
        "Yield",
        "Yield",
        "+15% gold yield per level",
        EffectDisplay::Percentage,
    ),
];

// Town upgrades (not NPC-driven, appended to registry as "Town" branch)
pub const TOWN_UPGRADES: &[UpgradeStatDef] = &[
    UpgradeStatDef {
        kind: USK::Healing,
        pct: 0.20,
        cost: &[(F, 1)],
        label: "Healing",
        short: "Heal",
        tooltip: "+20% HP regen at fountain",
        display: EffectDisplay::Percentage,
        prereq_stat: None,
        prereq_level: 1,
        is_combat_stat: false,
        invalidates_healing: true,
        triggers_expansion: false,
        custom_cost: false,
    },
    UpgradeStatDef {
        kind: USK::FountainRange,
        pct: 0.0,
        cost: &[(G, 1)],
        label: "Fountain Range",
        short: "FRng",
        tooltip: "+24px fountain range per level",
        display: EffectDisplay::FlatPixels(24),
        prereq_stat: Some(USK::Healing),
        prereq_level: 1,
        is_combat_stat: false,
        invalidates_healing: true,
        triggers_expansion: false,
        custom_cost: false,
    },
    UpgradeStatDef {
        kind: USK::FountainAttackSpeed,
        pct: 0.08,
        cost: &[(G, 1)],
        label: "Fountain Atk Speed",
        short: "FAS",
        tooltip: "-8% fountain cooldown per level",
        display: EffectDisplay::CooldownReduction,
        prereq_stat: Some(USK::FountainRange),
        prereq_level: 1,
        is_combat_stat: false,
        invalidates_healing: false,
        triggers_expansion: false,
        custom_cost: false,
    },
    UpgradeStatDef {
        kind: USK::FountainProjectileLife,
        pct: 0.08,
        cost: &[(G, 1)],
        label: "Fountain Proj Life",
        short: "FPL",
        tooltip: "+8% fountain projectile life per level",
        display: EffectDisplay::Percentage,
        prereq_stat: Some(USK::FountainRange),
        prereq_level: 1,
        is_combat_stat: false,
        invalidates_healing: false,
        triggers_expansion: false,
        custom_cost: false,
    },
    UpgradeStatDef {
        kind: USK::Expansion,
        pct: 0.0,
        cost: &[(F, 1), (G, 1)],
        label: "Expansion",
        short: "Area",
        tooltip: "+1 buildable radius per level",
        display: EffectDisplay::Discrete,
        prereq_stat: None,
        prereq_level: 1,
        is_combat_stat: false,
        invalidates_healing: false,
        triggers_expansion: true,
        custom_cost: true,
    },
];

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

/// What kind of item an NPC can carry or drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ItemKind {
    Food,
    Gold,
}

/// Loot dropped when an NPC dies.
#[derive(Clone, Copy, Debug)]
pub struct LootDrop {
    pub item: ItemKind,
    pub min: i32,
    pub max: i32,
}

/// Complete NPC type definition — one entry per Job variant.
#[derive(Clone, Copy, Debug)]
pub struct NpcDef {
    pub job: Job,
    pub label: &'static str,
    pub label_plural: &'static str,
    pub sprite: (f32, f32),
    /// Sprite atlas ID (0.0 = character atlas, ATLAS_BOAT = boat atlas, etc.)
    pub atlas: f32,
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
    /// True for raider town units (menu groups under "Raider Towns"), false for village units.
    pub is_raider_unit: bool,
    /// Default count per town in world gen.
    pub default_count: usize,
    /// Upgrade branch name. NPCs with the same category share upgrades. None = no upgrades (e.g. Raider).
    pub upgrade_category: Option<&'static str>,
    /// Which stats this NPC type can upgrade. Defines the upgrade branch content.
    pub upgrade_stats: &'static [UpgradeStatDef],
    /// Possible loot drops when killed — one is picked deterministically per death.
    pub loot_drop: &'static [LootDrop],
}

pub const NPC_REGISTRY: &[NpcDef] = &[
    NpcDef {
        job: Job::Farmer,
        label: "Farmer",
        label_plural: "Farmers",
        sprite: (1.0, 6.0),
        atlas: 0.0,
        color: (0.0, 1.0, 0.0, 1.0),
        base_hp: 60.0,
        base_damage: 0.0,
        base_speed: 100.0,
        default_attack_type: BaseAttackType::Melee,
        attack_override: None,
        is_patrol_unit: false,
        is_military: false,
        has_energy: true,
        has_attack_timer: false,
        weapon: None,
        helmet: None,
        stealer: false,
        leash_range: None,
        ui_color: (80, 200, 80),
        home_building: BuildingKind::FarmerHome,
        is_raider_unit: false,
        default_count: 2,
        upgrade_category: Some("Farmer"),
        upgrade_stats: FARMER_UPGRADES,
        loot_drop: &[LootDrop {
            item: ItemKind::Food,
            min: 1,
            max: 2,
        }],
    },
    NpcDef {
        job: Job::Archer,
        label: "Archer",
        label_plural: "Archers",
        sprite: (0.0, 0.0),
        atlas: 0.0,
        color: (0.0, 0.0, 1.0, 1.0),
        base_hp: 80.0,
        base_damage: 15.0,
        base_speed: 100.0,
        default_attack_type: BaseAttackType::Ranged,
        attack_override: None,
        is_patrol_unit: true,
        is_military: true,
        has_energy: true,
        has_attack_timer: true,
        weapon: Some(EQUIP_SWORD),
        helmet: Some(EQUIP_HELMET),
        stealer: false,
        leash_range: None,
        ui_color: (80, 100, 220),
        home_building: BuildingKind::ArcherHome,
        is_raider_unit: false,
        default_count: 4,
        upgrade_category: Some("Archer"),
        upgrade_stats: MILITARY_RANGED_UPGRADES,
        loot_drop: &[
            LootDrop {
                item: ItemKind::Food,
                min: 1,
                max: 2,
            },
            LootDrop {
                item: ItemKind::Gold,
                min: 0,
                max: 1,
            },
        ],
    },
    NpcDef {
        job: Job::Raider,
        label: "Raider",
        label_plural: "Raiders",
        sprite: (0.0, 6.0),
        atlas: 0.0,
        color: (1.0, 0.0, 0.0, 1.0),
        base_hp: 120.0,
        base_damage: 15.0,
        base_speed: 115.0,
        default_attack_type: BaseAttackType::Melee,
        attack_override: None,
        is_patrol_unit: false,
        is_military: true,
        has_energy: true,
        has_attack_timer: true,
        weapon: Some(EQUIP_SWORD),
        helmet: None,
        stealer: true,
        leash_range: Some(400.0),
        ui_color: (220, 80, 80),
        home_building: BuildingKind::Tent,
        is_raider_unit: true,
        default_count: 1,
        upgrade_category: None,
        upgrade_stats: &[],
        loot_drop: &[
            LootDrop {
                item: ItemKind::Food,
                min: 1,
                max: 2,
            },
            LootDrop {
                item: ItemKind::Gold,
                min: 0,
                max: 1,
            },
        ],
    },
    NpcDef {
        job: Job::Fighter,
        label: "Fighter",
        label_plural: "Fighters",
        sprite: (1.0, 9.0),
        atlas: 0.0,
        color: (1.0, 1.0, 0.0, 1.0),
        base_hp: 150.0,
        base_damage: 22.5,
        base_speed: 85.0,
        default_attack_type: BaseAttackType::Melee,
        attack_override: None,
        is_patrol_unit: true,
        is_military: true,
        has_energy: true,
        has_attack_timer: true,
        weapon: None,
        helmet: None,
        stealer: false,
        leash_range: None,
        ui_color: (220, 220, 80),
        home_building: BuildingKind::FighterHome,
        is_raider_unit: false,
        default_count: 0,
        upgrade_category: Some("Fighter"),
        upgrade_stats: MILITARY_MELEE_UPGRADES,
        loot_drop: &[
            LootDrop {
                item: ItemKind::Food,
                min: 1,
                max: 2,
            },
            LootDrop {
                item: ItemKind::Gold,
                min: 0,
                max: 1,
            },
        ],
    },
    NpcDef {
        job: Job::Miner,
        label: "Miner",
        label_plural: "Miners",
        sprite: (1.0, 6.0),
        atlas: 0.0,
        color: (0.6, 0.4, 0.2, 1.0),
        base_hp: 80.0,
        base_damage: 0.0,
        base_speed: 100.0,
        default_attack_type: BaseAttackType::Melee,
        attack_override: None,
        is_patrol_unit: false,
        is_military: false,
        has_energy: true,
        has_attack_timer: false,
        weapon: None,
        helmet: None,
        stealer: false,
        leash_range: None,
        ui_color: (160, 110, 60),
        home_building: BuildingKind::MinerHome,
        is_raider_unit: false,
        default_count: 0,
        upgrade_category: Some("Miner"),
        upgrade_stats: MINER_UPGRADES,
        loot_drop: &[LootDrop {
            item: ItemKind::Gold,
            min: 1,
            max: 2,
        }],
    },
    NpcDef {
        job: Job::Crossbow,
        label: "Crossbow",
        label_plural: "Crossbows",
        sprite: (0.0, 0.0),
        atlas: 0.0,
        color: (0.4, 0.0, 0.8, 1.0),
        base_hp: 70.0,
        base_damage: 25.0,
        base_speed: 85.0,
        default_attack_type: BaseAttackType::Ranged,
        attack_override: Some(AttackTypeStats {
            range: 150.0,
            cooldown: 2.0,
            projectile_speed: 150.0,
            projectile_lifetime: 1.5,
        }),
        is_patrol_unit: true,
        is_military: true,
        has_energy: true,
        has_attack_timer: true,
        weapon: Some(EQUIP_SWORD),
        helmet: Some(EQUIP_HELMET),
        stealer: false,
        leash_range: None,
        ui_color: (140, 60, 220),
        home_building: BuildingKind::CrossbowHome,
        is_raider_unit: false,
        default_count: 0,
        upgrade_category: Some("Crossbow"),
        upgrade_stats: MILITARY_RANGED_UPGRADES,
        loot_drop: &[
            LootDrop {
                item: ItemKind::Food,
                min: 1,
                max: 2,
            },
            LootDrop {
                item: ItemKind::Gold,
                min: 0,
                max: 1,
            },
        ],
    },
    NpcDef {
        job: Job::Boat,
        label: "Boat",
        label_plural: "Boats",
        sprite: (0.0, 0.0),
        atlas: ATLAS_BOAT,
        color: (1.0, 1.0, 1.0, 1.0),
        base_hp: 100.0,
        base_damage: 0.0,
        base_speed: BOAT_SPEED,
        default_attack_type: BaseAttackType::Melee,
        attack_override: None,
        is_patrol_unit: false,
        is_military: false,
        has_energy: false,
        has_attack_timer: false,
        weapon: None,
        helmet: None,
        stealer: false,
        leash_range: None,
        ui_color: (180, 140, 80),
        home_building: BuildingKind::Fountain,
        is_raider_unit: false,
        default_count: 0,
        upgrade_category: None,
        upgrade_stats: &[],
        loot_drop: &[LootDrop {
            item: ItemKind::Food,
            min: 1,
            max: 3,
        }],
    },
];

/// Look up NPC definition by job. Panics if job not in registry.
pub fn npc_def(job: Job) -> &'static NpcDef {
    NPC_REGISTRY
        .iter()
        .find(|d| d.job == job)
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
pub const PROJECTILE_HIT_HALF_WIDTH: f32 = 4.0; // perpendicular to travel

/// Per-entity hitbox half-sizes (added to projectile hitbox via Minkowski sum).
/// NPC body is ~16x16 centered in 32x32 tile; buildings fill the full 32x32 tile.
pub const NPC_HITBOX_HALF: [f32; 2] = [8.0, 8.0];
pub const BUILDING_HITBOX_HALF: [f32; 2] = [16.0, 16.0];

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
pub const BOAT_SPEED: f32 = 150.0;

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
    range: 400.0,
    damage: 15.0,
    cooldown: 1.5,
    proj_speed: 350.0,
    proj_lifetime: 1.5,
};

pub const TOWER_STATS: TowerStats = TowerStats {
    range: 100.0,
    damage: 15.0,
    cooldown: 1.5,
    proj_speed: 100.0,
    proj_lifetime: 1.5,
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

/// Seconds (at 1x speed) for a newly placed building to finish construction.
pub const BUILDING_CONSTRUCT_SECS: f32 = 10.0;

/// Speed multiplier for NPCs walking on road tiles.
pub const ROAD_SPEED_MULT: f32 = 1.5;

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
    &[(F, 2), (G, 1)], // wooden → stone
    &[(F, 4), (G, 2)], // stone → fortified
];
/// Extra atlas layers per auto-tile kind (NS, 4 corners, cross, 4 T-junctions = 10).
pub const AUTOTILE_EXTRA_PER_KIND: usize = 10;

/// Auto-tile variant indices (0 = base/E-W layer at tileset_index, 1-10 = appended extras).
pub const AUTOTILE_EW: u16 = 0;
pub const AUTOTILE_NS: u16 = 1;
pub const AUTOTILE_BL: u16 = 2;  // BR src(0°) → BL on screen
pub const AUTOTILE_BR: u16 = 3;  // BL(90°) → BR on screen
pub const AUTOTILE_TR: u16 = 4;  // TL(180°) → TR on screen
pub const AUTOTILE_TL: u16 = 5;  // TR(270°) → TL on screen
pub const AUTOTILE_CROSS: u16 = 6;
pub const AUTOTILE_T_OPEN_N: u16 = 7;
pub const AUTOTILE_T_OPEN_W: u16 = 8;
pub const AUTOTILE_T_OPEN_S: u16 = 9;
pub const AUTOTILE_T_OPEN_E: u16 = 10;

/// Number of building kinds with autotile enabled.
pub fn autotile_kind_count() -> usize {
    BUILDING_REGISTRY.iter().filter(|d| d.autotile).count()
}

/// Total extra atlas layers for all auto-tiled kinds.
pub fn autotile_total_extra_layers() -> usize {
    autotile_kind_count() * AUTOTILE_EXTRA_PER_KIND
}

/// Get the autotile order index (0, 1, 2...) for a building kind among all autotile kinds.
/// Returns None if the kind doesn't use autotile.
pub fn autotile_order(kind: BuildingKind) -> Option<usize> {
    let mut order = 0;
    for def in BUILDING_REGISTRY {
        if def.kind == kind {
            return if def.autotile { Some(order) } else { None };
        }
        if def.autotile {
            order += 1;
        }
    }
    None
}

/// Compute the atlas column for an auto-tile variant.
/// Variant 0 (E-W) uses the building's base tileset index.
/// Variants 1-10 use appended extra layers.
pub fn autotile_col(kind: BuildingKind, variant: u16) -> f32 {
    if variant == 0 {
        return tileset_index(kind) as f32;
    }
    let order = autotile_order(kind).unwrap_or(0);
    let extra_base = BUILDING_REGISTRY.len() + order * AUTOTILE_EXTRA_PER_KIND;
    (extra_base as u16 + variant - 1) as f32
}

// Legacy aliases for wall constants (used in save.rs wall_level sprite updates)
pub const WALL_EXTRA_LAYERS: usize = AUTOTILE_EXTRA_PER_KIND;

/// Tended growth rate for mines (per game-hour). 0.25 = 4 hours to full when miner is working.
pub const MINE_TENDED_GROWTH_RATE: f32 = 0.25;

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
    /// Snap to world grid (waypoints, fountains, gold mines).
    Wilderness,
}

/// Special action when a building is placed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OnPlace {
    None,
    /// Initialize farm growth on BuildingInstance.
    InitFarmGrowth,
}

/// How a spawner building finds work/patrol targets for its NPC.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpawnBehavior {
    /// Find nearest free farm in own town (farmer).
    FindNearestFarm,
    /// Find nearest waypoint for patrol (archer, crossbow).
    FindNearestWaypoint,
    /// Use raider town faction (tent → raider).
    Raider,
    /// Use assigned mine or find nearest (miner).
    Miner,
}

/// NPC spawner definition — what kind of NPC a building produces.
#[derive(Clone, Copy, Debug)]
pub struct SpawnerDef {
    pub job: i32, // Job::from_i32 index (0=Farmer, 1=Archer, 2=Raider, 4=Miner, 5=Crossbow)
    pub attack_type: i32, // 0=melee, 1=ranged bow, 2=ranged xbow
    pub behavior: SpawnBehavior,
}

/// Factions tab column assignment for building display.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DisplayCategory {
    Hidden,
    Economy,
    Military,
    Tower,
}

/// Worksite occupancy config for buildings that NPCs can claim and work at.
#[derive(Clone, Copy, Debug)]
pub struct WorksiteDef {
    pub max_occupants: i32,
    pub drift_radius: f32,
    pub upgrade_job: &'static str,
    pub harvest_item: ItemKind,
    pub town_scoped: bool,
}

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
    pub tooltip: &'static str,
    pub player_buildable: bool,
    pub raider_buildable: bool,
    pub placement: PlacementMode,
    pub is_tower: bool,
    pub tower_stats: Option<TowerStats>,
    pub on_place: OnPlace,
    pub spawner: Option<SpawnerDef>,
    /// Save key in JSON (None for Fountain which uses towns vec).
    pub save_key: Option<&'static str>,
    /// Whether this kind uses unit_homes BTreeMap storage.
    pub is_unit_home: bool,
    /// Worksite config (None = not a worksite NPCs can occupy).
    pub worksite: Option<WorksiteDef>,
    /// True = uses 4-neighbor auto-tiling (requires TileSpec::External sprite strip).
    pub autotile: bool,
}

impl BuildingDef {
    /// Loot dropped when this building is destroyed: half the build cost as food.
    pub fn loot_drop(&self) -> Option<LootDrop> {
        let amount = self.cost / 2;
        if amount > 0 {
            Some(LootDrop {
                item: ItemKind::Food,
                min: amount,
                max: amount,
            })
        } else {
            None
        }
    }
}

/// Single source of truth for all building types.
/// Order must match tileset strip (index = tileset_index).
pub const BUILDING_REGISTRY: &[BuildingDef] = &[
    // 0: Fountain (town center, auto-shoots)
    BuildingDef {
        kind: BuildingKind::Fountain,
        display: DisplayCategory::Hidden,
        tile: TileSpec::Single(50, 9),
        hp: 500.0,
        cost: 0,
        label: "Fountain",
        help: "Town center",
        tooltip: "",
        player_buildable: false,
        raider_buildable: false,
        placement: PlacementMode::Wilderness,
        is_tower: true,
        tower_stats: Some(FOUNTAIN_TOWER),
        on_place: OnPlace::None,
        spawner: None,
        save_key: None,
        is_unit_home: false,
        worksite: None,
        autotile: false,
    },
    // 1: Bed
    BuildingDef {
        kind: BuildingKind::Bed,
        display: DisplayCategory::Hidden,
        tile: TileSpec::Single(15, 2),
        hp: 50.0,
        cost: 0,
        label: "Bed",
        help: "NPC rest spot",
        tooltip: "",
        player_buildable: false,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("beds"),
        is_unit_home: false,
        worksite: None,
        autotile: false,
    },
    // 2: Waypoint
    BuildingDef {
        kind: BuildingKind::Waypoint,
        display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/waypoint.png"),
        hp: 200.0,
        cost: 1,
        label: "Waypoint",
        help: "Patrol waypoint",
        tooltip: "Archers patrol between waypoints to guard\nyour territory. Place outside town to extend\npatrol coverage. HP: 200",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::Wilderness,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("waypoints"),
        is_unit_home: false,
        worksite: None,
        autotile: false,
    },
    // 3: Farm
    BuildingDef {
        kind: BuildingKind::Farm,
        display: DisplayCategory::Economy,
        tile: TileSpec::Quad([(2, 15), (4, 15), (2, 17), (4, 17)]),
        hp: 80.0,
        cost: 2,
        label: "Farm",
        help: "Grows food over time",
        tooltip: "Grows food passively (0.08/hr). Farmers tend\nit 3x faster (0.25/hr). Harvest yields 1 food.\nBuild near Farmer Homes for fast delivery. HP: 80",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::InitFarmGrowth,
        spawner: None,
        save_key: Some("farms"),
        is_unit_home: false,
        worksite: Some(WorksiteDef {
            max_occupants: 1,
            drift_radius: 20.0,
            upgrade_job: "Farmer",
            harvest_item: ItemKind::Food,
            town_scoped: true,
        }),
        autotile: false,
    },
    // 5: Farmer Home
    BuildingDef {
        kind: BuildingKind::FarmerHome,
        display: DisplayCategory::Economy,
        tile: TileSpec::External("sprites/house.png"),
        hp: 100.0,
        cost: 2,
        label: "Farmer Home",
        help: "Spawns 1 farmer",
        tooltip: "Trains 1 farmer who tends farms and carries\nfood home. 100 HP, speed 100. Respawns 12 hrs\nafter death. Build near farms for short trips.",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef {
            job: 0,
            attack_type: 0,
            behavior: SpawnBehavior::FindNearestFarm,
        }),
        save_key: Some("farmer_homes"),
        is_unit_home: true,
        worksite: None,
        autotile: false,
    },
    // 6: Archer Home
    BuildingDef {
        kind: BuildingKind::ArcherHome,
        display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/barracks.png"),
        hp: 150.0,
        cost: 4,
        label: "Archer Home",
        help: "Spawns 1 archer",
        tooltip: "Trains 1 archer — ranged defender. 100 HP,\n15 dmg, range 100, 1.5s cooldown. Patrols\nbetween waypoints. Respawns 12 hrs after death.",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef {
            job: 1,
            attack_type: 1,
            behavior: SpawnBehavior::FindNearestWaypoint,
        }),
        save_key: Some("archer_homes"),
        is_unit_home: true,
        worksite: None,
        autotile: false,
    },
    // 7: Tent (raider spawner)
    BuildingDef {
        kind: BuildingKind::Tent,
        display: DisplayCategory::Military,
        tile: TileSpec::Quad([(48, 10), (49, 10), (48, 11), (49, 11)]),
        hp: 100.0,
        cost: 3,
        label: "Tent",
        help: "Spawns 1 raider",
        tooltip: "",
        player_buildable: false,
        raider_buildable: true,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef {
            job: 2,
            attack_type: 0,
            behavior: SpawnBehavior::Raider,
        }),
        save_key: Some("tents"),
        is_unit_home: true,
        worksite: None,
        autotile: false,
    },
    // 8: Gold Mine
    BuildingDef {
        kind: BuildingKind::GoldMine,
        display: DisplayCategory::Hidden,
        tile: TileSpec::Single(43, 11),
        hp: 200.0,
        cost: 0,
        label: "Gold Mine",
        help: "Source of gold",
        tooltip: "",
        player_buildable: false,
        raider_buildable: false,
        placement: PlacementMode::Wilderness,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("gold_mines"),
        is_unit_home: false,
        worksite: Some(WorksiteDef {
            max_occupants: 5,
            drift_radius: MINE_WORK_RADIUS,
            upgrade_job: "Miner",
            harvest_item: ItemKind::Gold,
            town_scoped: false,
        }),
        autotile: false,
    },
    // 9: Miner Home
    BuildingDef {
        kind: BuildingKind::MinerHome,
        display: DisplayCategory::Economy,
        tile: TileSpec::External("sprites/miner_house.png"),
        hp: 100.0,
        cost: 4,
        label: "Miner Home",
        help: "Spawns 1 miner",
        tooltip: "Trains 1 miner who extracts gold from mines.\n5 gold per harvest (4 hr cycle). 100 HP, speed\n110. Gold funds upgrades. Respawns 12 hrs.",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef {
            job: 4,
            attack_type: 0,
            behavior: SpawnBehavior::Miner,
        }),
        save_key: Some("miner_homes"),
        is_unit_home: false,
        worksite: None,
        autotile: false,
    },
    // 10: Crossbow Home
    BuildingDef {
        kind: BuildingKind::CrossbowHome,
        display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/barracks.png"),
        hp: 150.0,
        cost: 8,
        label: "Crossbow Home",
        help: "Spawns 1 crossbow",
        tooltip: "Trains 1 crossbow — elite ranged unit. 100 HP,\n25 dmg, range 150, 2s cooldown. Highest DPS\nranged unit. Patrols waypoints. Respawns 12 hrs.",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef {
            job: 5,
            attack_type: 2,
            behavior: SpawnBehavior::FindNearestWaypoint,
        }),
        save_key: Some("crossbow_homes"),
        is_unit_home: true,
        worksite: None,
        autotile: false,
    },
    // 11: Fighter Home
    BuildingDef {
        kind: BuildingKind::FighterHome,
        display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/fighter_home.png"),
        hp: 150.0,
        cost: 5,
        label: "Fighter Home",
        help: "Spawns 1 fighter",
        tooltip: "Trains 1 fighter — melee combatant. 100 HP,\n22.5 dmg, range 50, 1s cooldown. High melee\nDPS, engages up close. Patrols waypoints.\nRespawns 12 hrs.",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef {
            job: 3,
            attack_type: 0,
            behavior: SpawnBehavior::FindNearestWaypoint,
        }),
        save_key: Some("fighter_homes"),
        is_unit_home: true,
        worksite: None,
        autotile: false,
    },
    // 12: Road
    BuildingDef {
        kind: BuildingKind::Road,
        display: DisplayCategory::Economy,
        tile: TileSpec::External("sprites/dirt_roads_131_32.png"),
        hp: 30.0,
        cost: 1,
        label: "Road",
        help: "1.5x NPC speed",
        tooltip: "NPCs move 50% faster on roads. Click-drag to\nbuild lines. Connect farms, mines, and town\ncenter for faster supply chains. HP: 30",
        player_buildable: true,
        raider_buildable: true,
        placement: PlacementMode::Wilderness,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("roads"),
        is_unit_home: false,
        worksite: None,
        autotile: true,
    },
    // 13: Wall
    BuildingDef {
        kind: BuildingKind::Wall,
        display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/wood_walls_131x32.png"),
        hp: 80.0,
        cost: 1,
        label: "Wall",
        help: "Blocks enemy movement",
        tooltip: "Defensive wall — blocks enemy NPCs from\npassing through. Click to upgrade tier.\nWooden: 80 HP, Stone: 200, Fortified: 400.",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("walls"),
        is_unit_home: false,
        worksite: None,
        autotile: true,
    },
    // 14: Tower (auto-shoots enemies)
    BuildingDef {
        kind: BuildingKind::Tower,
        display: DisplayCategory::Tower,
        tile: TileSpec::External("sprites/tower-1.png"),
        hp: 1000.0,
        cost: 40,
        label: "Tower",
        help: "Auto-attacks nearby enemies",
        tooltip: "Defensive tower — auto-shoots nearest enemy.\nSame range/damage as archer. 15 dmg, 1.5s cooldown. HP: 1000",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: true,
        tower_stats: Some(TOWER_STATS),
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("towers"),
        is_unit_home: false,
        worksite: None,
        autotile: false,
    },
];

/// Look up a building definition by kind. Panics if kind is not in registry.
pub fn building_def(kind: BuildingKind) -> &'static BuildingDef {
    BUILDING_REGISTRY
        .iter()
        .find(|d| d.kind == kind)
        .unwrap_or_else(|| panic!("no BuildingDef for {:?}", kind))
}

/// Look up the tileset index for a BuildingKind (its position in BUILDING_REGISTRY).
pub fn tileset_index(kind: BuildingKind) -> u16 {
    BUILDING_REGISTRY
        .iter()
        .position(|d| d.kind == kind)
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
pub const ATLAS_BOAT: f32 = 8.0;
