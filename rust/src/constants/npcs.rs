//! NPC registry, activity registry, equipment/loot types and generation.

use bevy::reflect::Reflect;
use crate::components::{ActivityKind, BaseAttackType, Distraction, Job};
use crate::world::BuildingKind;
use super::upgrades::*;

/// Per-attack-type stats (range, cooldown, projectile behavior).
#[derive(Clone, Copy, Debug)]
pub struct AttackTypeStats {
    pub range: f32,
    pub cooldown: f32,
    pub projectile_speed: f32,
    pub projectile_lifetime: f32,
}

/// What kind of item an NPC can carry or drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect, serde::Serialize, serde::Deserialize)]
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

// ============================================================================
// EQUIPMENT & LOOT TYPES
// ============================================================================

/// Equipment slot on an NPC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect, serde::Serialize, serde::Deserialize)]
pub enum EquipmentSlot {
    // Sprite-visible slots
    Helm,
    Armor,
    Weapon,
    Shield,
    // Stat-only slots
    Gloves,
    Boots,
    Belt,
    Amulet,
    Ring,
}

/// All equipment slot variants for iteration.
pub const ALL_EQUIPMENT_SLOTS: &[EquipmentSlot] = &[
    EquipmentSlot::Helm,
    EquipmentSlot::Armor,
    EquipmentSlot::Weapon,
    EquipmentSlot::Shield,
    EquipmentSlot::Gloves,
    EquipmentSlot::Boots,
    EquipmentSlot::Belt,
    EquipmentSlot::Amulet,
    EquipmentSlot::Ring,
];

/// Rarity tier for loot items.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect, serde::Serialize, serde::Deserialize)]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Epic,
}

impl Rarity {
    pub fn gold_cost(self) -> i32 {
        match self {
            Self::Common => 25,
            Self::Uncommon => 75,
            Self::Rare => 200,
            Self::Epic => 500,
        }
    }

    pub fn color(self) -> (u8, u8, u8) {
        match self {
            Self::Common => (255, 255, 255),
            Self::Uncommon => (30, 200, 30),
            Self::Rare => (60, 120, 255),
            Self::Epic => (180, 60, 255),
        }
    }

    /// Stat bonus range (min%, max%) for this rarity.
    pub fn stat_range(self) -> (f32, f32) {
        match self {
            Self::Common => (0.05, 0.10),
            Self::Uncommon => (0.10, 0.20),
            Self::Rare => (0.20, 0.35),
            Self::Epic => (0.35, 0.50),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Common => "Common",
            Self::Uncommon => "Uncommon",
            Self::Rare => "Rare",
            Self::Epic => "Epic",
        }
    }
}

/// Rarity roll weights (out of 100).
const RARITY_WEIGHTS: [(Rarity, u32); 4] = [
    (Rarity::Common, 60),
    (Rarity::Uncommon, 25),
    (Rarity::Rare, 12),
    (Rarity::Epic, 3),
];

/// A concrete equipment item with stats.
#[derive(Clone, Debug, Reflect, serde::Serialize, serde::Deserialize)]
pub struct LootItem {
    pub id: u64,
    pub slot: EquipmentSlot,
    pub rarity: Rarity,
    /// Fractional stat bonus (e.g. 0.15 = +15% damage or HP).
    pub stat_bonus: f32,
    /// Atlas sprite (col, row).
    pub sprite: (f32, f32),
    pub name: String,
}

/// Sprite options per slot (atlas col, row). Visible slots have distinct sprites.
const WEAPON_SPRITES: &[(f32, f32)] = &[(45.0, 6.0), (46.0, 6.0), (47.0, 6.0), (44.0, 6.0)];
const ARMOR_SPRITES: &[(f32, f32)] = &[(40.0, 0.0), (41.0, 0.0), (42.0, 0.0)];
const HELM_SPRITES: &[(f32, f32)] = &[(28.0, 0.0), (29.0, 0.0), (30.0, 0.0)];
const SHIELD_SPRITES: &[(f32, f32)] = &[(43.0, 6.0), (44.0, 7.0), (45.0, 7.0)];

/// Name generation tables per slot.
const ITEM_PREFIXES: &[&str] = &["Iron", "Steel", "Bronze", "Silver", "Dark", "Ancient", "Blessed"];
const WEAPON_NAMES: &[&str] = &["Sword", "Axe", "Spear", "Mace", "Blade"];
const ARMOR_NAMES: &[&str] = &["Chainmail", "Plate", "Leather", "Brigandine", "Cuirass"];
const HELM_NAMES: &[&str] = &["Helm", "Crown", "Circlet", "Coif", "Casque"];
const SHIELD_NAMES: &[&str] = &["Shield", "Buckler", "Kite Shield", "Pavise", "Targe"];
const GLOVE_NAMES: &[&str] = &["Gauntlets", "Bracers", "Grips", "Wraps", "Vambraces"];
const BOOT_NAMES: &[&str] = &["Greaves", "Boots", "Sabatons", "Treads", "Striders"];
const BELT_NAMES: &[&str] = &["Belt", "Sash", "Girdle", "Cord", "Binding"];
const AMULET_NAMES: &[&str] = &["Amulet", "Pendant", "Talisman", "Charm", "Medallion"];
const RING_NAMES: &[&str] = &["Ring", "Band", "Signet", "Loop", "Circle"];

fn slot_names(slot: EquipmentSlot) -> &'static [&'static str] {
    match slot {
        EquipmentSlot::Weapon => WEAPON_NAMES,
        EquipmentSlot::Armor => ARMOR_NAMES,
        EquipmentSlot::Helm => HELM_NAMES,
        EquipmentSlot::Shield => SHIELD_NAMES,
        EquipmentSlot::Gloves => GLOVE_NAMES,
        EquipmentSlot::Boots => BOOT_NAMES,
        EquipmentSlot::Belt => BELT_NAMES,
        EquipmentSlot::Amulet => AMULET_NAMES,
        EquipmentSlot::Ring => RING_NAMES,
    }
}

/// Roll a random loot item using deterministic seed.
pub fn roll_loot_item(id: u64, seed: u32) -> LootItem {
    // Rarity roll
    let rarity_roll = seed % 100;
    let mut cumulative = 0u32;
    let mut rarity = Rarity::Common;
    for &(r, weight) in &RARITY_WEIGHTS {
        cumulative += weight;
        if rarity_roll < cumulative {
            rarity = r;
            break;
        }
    }

    // Slot roll (uniform across all 9 slot types)
    let slot_seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
    let slot = ALL_EQUIPMENT_SLOTS[slot_seed as usize % ALL_EQUIPMENT_SLOTS.len()];

    // Stat bonus within rarity range
    let stat_seed = slot_seed.wrapping_mul(1103515245).wrapping_add(12345);
    let (min_stat, max_stat) = rarity.stat_range();
    let t = (stat_seed % 1000) as f32 / 1000.0;
    let stat_bonus = min_stat + t * (max_stat - min_stat);

    // Sprite from curated list (visible slots) or sentinel (stat-only slots)
    let sprite_seed = stat_seed.wrapping_mul(1103515245).wrapping_add(12345);
    let sprite = match slot {
        EquipmentSlot::Weapon => WEAPON_SPRITES[sprite_seed as usize % WEAPON_SPRITES.len()],
        EquipmentSlot::Armor => ARMOR_SPRITES[sprite_seed as usize % ARMOR_SPRITES.len()],
        EquipmentSlot::Helm => HELM_SPRITES[sprite_seed as usize % HELM_SPRITES.len()],
        EquipmentSlot::Shield => SHIELD_SPRITES[sprite_seed as usize % SHIELD_SPRITES.len()],
        _ => (-1.0, 0.0), // stat-only slots have no sprite
    };

    // Name
    let name_seed = sprite_seed.wrapping_mul(1103515245).wrapping_add(12345);
    let names = slot_names(slot);
    let prefix = ITEM_PREFIXES[name_seed as usize % ITEM_PREFIXES.len()];
    let base = names[(name_seed >> 8) as usize % names.len()];
    let name = format!("{} {}", prefix, base);

    LootItem {
        id,
        slot,
        rarity,
        stat_bonus,
        sprite,
        name,
    }
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
    /// Chance (0.0–1.0) this NPC type drops equipment when killed.
    pub equipment_drop_rate: f32,
    /// Which equipment slots this NPC type can equip (military: Weapon+Armor, others: none).
    pub equip_slots: &'static [EquipmentSlot],
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
        equipment_drop_rate: 0.0,
        equip_slots: &[],
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
        equipment_drop_rate: 0.0,
        equip_slots: ALL_EQUIPMENT_SLOTS,
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
        base_speed: 110.0,
        default_attack_type: BaseAttackType::Melee,
        attack_override: None,
        is_patrol_unit: false,
        is_military: true,
        has_energy: true,
        has_attack_timer: true,
        stealer: true,
        leash_range: Some(800.0),
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
        equipment_drop_rate: 0.30,
        equip_slots: ALL_EQUIPMENT_SLOTS,
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
        equipment_drop_rate: 0.0,
        equip_slots: ALL_EQUIPMENT_SLOTS,
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
        equipment_drop_rate: 0.0,
        equip_slots: &[],
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
            range: 300.0,
            cooldown: 2.0,
            projectile_speed: 300.0,
            projectile_lifetime: 1.5,
        }),
        is_patrol_unit: true,
        is_military: true,
        has_energy: true,
        has_attack_timer: true,
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
        equipment_drop_rate: 0.0,
        equip_slots: ALL_EQUIPMENT_SLOTS,
    },
    NpcDef {
        job: Job::Boat,
        label: "Boat",
        label_plural: "Boats",
        sprite: (0.0, 0.0),
        atlas: super::ATLAS_BOAT,
        color: (1.0, 1.0, 1.0, 1.0),
        base_hp: 100.0,
        base_damage: 0.0,
        base_speed: super::BOAT_SPEED,
        default_attack_type: BaseAttackType::Melee,
        attack_override: None,
        is_patrol_unit: false,
        is_military: false,
        has_energy: false,
        has_attack_timer: false,
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
        equipment_drop_rate: 0.0,
        equip_slots: &[],
    },
];

/// Look up NPC definition by job. Panics if job not in registry.
pub fn npc_def(job: Job) -> &'static NpcDef {
    NPC_REGISTRY
        .iter()
        .find(|d| d.job == job)
        .unwrap_or_else(|| panic!("no NpcDef for {:?}", job))
}

// ── Activity registry ─────────────────────────────────────────────

pub struct ActivityDef {
    pub activity: ActivityKind,
    pub label: &'static str,
    pub distraction: Distraction,
    pub sleep_visual: bool,
    pub is_restful: bool,
    pub is_working: bool,
}

pub const ACTIVITY_REGISTRY: &[ActivityDef] = &[
    ActivityDef { activity: ActivityKind::Idle,        label: "Idle",         distraction: Distraction::ByEnemy,  sleep_visual: false, is_restful: false, is_working: false },
    ActivityDef { activity: ActivityKind::Work,        label: "Working",      distraction: Distraction::ByDamage, sleep_visual: false, is_restful: false, is_working: true },
    ActivityDef { activity: ActivityKind::Patrol,      label: "Patrol",       distraction: Distraction::ByEnemy,  sleep_visual: false, is_restful: false, is_working: false },
    ActivityDef { activity: ActivityKind::SquadAttack, label: "Squad Attack", distraction: Distraction::ByEnemy,  sleep_visual: false, is_restful: false, is_working: false },
    ActivityDef { activity: ActivityKind::Rest,        label: "Resting",      distraction: Distraction::None,     sleep_visual: true,  is_restful: true,  is_working: false },
    ActivityDef { activity: ActivityKind::Heal,        label: "Healing",      distraction: Distraction::None,     sleep_visual: false, is_restful: true,  is_working: false },
    ActivityDef { activity: ActivityKind::Wander,      label: "Wandering",    distraction: Distraction::ByEnemy,  sleep_visual: false, is_restful: false, is_working: false },
    ActivityDef { activity: ActivityKind::Raid,        label: "Raiding",      distraction: Distraction::ByEnemy,  sleep_visual: false, is_restful: false, is_working: false },
    ActivityDef { activity: ActivityKind::ReturnLoot,  label: "Returning",    distraction: Distraction::None,     sleep_visual: false, is_restful: false, is_working: false },
    ActivityDef { activity: ActivityKind::Mine,        label: "Mining",       distraction: Distraction::ByDamage, sleep_visual: false, is_restful: false, is_working: true },
];

pub fn activity_def(kind: ActivityKind) -> &'static ActivityDef {
    ACTIVITY_REGISTRY.iter().find(|d| d.activity == kind).unwrap()
}
