//! Stat resolution, upgrades, and XP systems.
//! Stage 8: CombatConfig + resolve_combat_stats + CachedStats.
//! Stage 9: UpgradeQueue + process_upgrades_system + xp_grant_system.

use std::collections::HashMap;
use bevy::prelude::*;
use crate::components::{Job, BaseAttackType, CachedStats, Personality, Dead, LastHitBy, Health, Speed, NpcIndex, TownId, Faction};
use crate::constants::{FOUNTAIN_TOWER, TowerStats, NPC_REGISTRY, npc_def, AttackTypeStats};
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::resources::{NpcEntityMap, NpcMetaCache, NpcsByTownCache, FactionStats, CombatLog, CombatEventKind, GameTime, SystemTimings};
use crate::systemparams::{EconomyState, WorldState};

// ============================================================================
// COMBAT CONFIG (replaces scattered constants)
// ============================================================================

/// Per-job identity stats. Determines "what kind of NPC is this?"
#[derive(Clone, Debug)]
pub struct JobStats {
    pub max_health: f32,
    pub damage: f32,
    pub speed: f32,
}

/// Central combat configuration. All NPC stats resolve from this.
/// Base job stats populated from NPC_REGISTRY.
#[derive(Resource)]
pub struct CombatConfig {
    pub jobs: HashMap<Job, JobStats>,
    pub attacks: HashMap<BaseAttackType, AttackTypeStats>,
    pub heal_rate: f32,
    pub heal_radius: f32,
}

impl Default for CombatConfig {
    fn default() -> Self {
        let mut jobs = HashMap::new();
        for def in NPC_REGISTRY {
            jobs.insert(def.job, JobStats {
                max_health: def.base_hp,
                damage: def.base_damage,
                speed: def.base_speed,
            });
        }

        let mut attacks = HashMap::new();
        attacks.insert(BaseAttackType::Melee, AttackTypeStats {
            range: 50.0, cooldown: 1.0, projectile_speed: 200.0, projectile_lifetime: 0.5,
        });
        attacks.insert(BaseAttackType::Ranged, AttackTypeStats {
            range: 100.0, cooldown: 1.5, projectile_speed: 100.0, projectile_lifetime: 1.5,
        });

        Self { jobs, attacks, heal_rate: 5.0, heal_radius: 150.0 }
    }
}

// ============================================================================
// TOWN UPGRADES
// ============================================================================

pub const UPGRADE_COUNT: usize = 25;

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum UpgradeType {
    // Military (applies to Archer + Raider)
    MilitaryHp = 0, MilitaryAttack = 1, MilitaryRange = 2, AttackSpeed = 3,
    MilitaryMoveSpeed = 4, AlertRadius = 5, Dodge = 6,
    // Farmer
    FarmYield = 7, FarmerHp = 8, FarmerMoveSpeed = 9,
    // Miner
    MinerHp = 10, MinerMoveSpeed = 11, GoldYield = 12,
    // Town
    HealingRate = 13, FountainRange = 14, FountainAttackSpeed = 15, FountainProjectileLife = 16, TownArea = 17,
    // Arrow (applies to Archer + Raider + Fighter ranged)
    ProjectileSpeed = 18, ProjectileLifetime = 19,
    // Crossbow (separate branch from Military)
    CrossbowHp = 20, CrossbowAttack = 21, CrossbowRange = 22, CrossbowAttackSpeed = 23, CrossbowMoveSpeed = 24,
}

pub const UPGRADE_PCT: [f32; UPGRADE_COUNT] = [
    0.10, 0.10, 0.05,  // military: hp, attack, range
    0.08, 0.05, 0.10,  // attack speed (cooldown), military move speed, alert radius
    0.0,               // dodge (unlock)
    0.15, 0.20, 0.05,  // farm yield, farmer hp, farmer move speed
    0.20, 0.05, 0.15,  // miner hp, miner move speed, gold yield
    0.20, 0.0, 0.08, 0.08, 0.0, // healing rate, fountain range (flat), fountain atk speed, fountain projectile life, town area (discrete)
    0.08, 0.08,         // arrow projectile speed, arrow projectile lifetime
    0.10, 0.10, 0.05, 0.08, 0.05, // crossbow: hp, attack, range, attack speed (cooldown), move speed
];

// ============================================================================
// UPGRADE REGISTRY (single source of truth for all upgrade metadata)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind { Food, Gold }

#[derive(Clone, Copy, Debug)]
pub struct UpgradePrereq {
    pub upgrade: usize,
    pub min_level: u8,
}

pub struct UpgradeNode {
    pub label: &'static str,
    pub short: &'static str,
    pub tooltip: &'static str,
    pub category: &'static str,
    pub cost: &'static [(ResourceKind, i32)],
    pub prereqs: &'static [UpgradePrereq],
}

use ResourceKind::{Food as F, Gold as G};
const fn prereq(upgrade: usize, min_level: u8) -> UpgradePrereq {
    UpgradePrereq { upgrade, min_level }
}

pub const UPGRADE_REGISTRY: [UpgradeNode; UPGRADE_COUNT] = [
    // Military (0-6): applies to Archer + Raider
    // 0: HP — root
    UpgradeNode { label: "HP",           short: "HP",     tooltip: "+10% HP per level",              category: "Military", cost: &[(F, 1)], prereqs: &[] },
    // 1: Attack — root
    UpgradeNode { label: "Attack",       short: "Atk",    tooltip: "+10% damage per level",          category: "Military", cost: &[(F, 1)], prereqs: &[] },
    // 2: Range — requires Attack Lv1
    UpgradeNode { label: "Range",        short: "Rng",    tooltip: "+5% attack range per level",     category: "Military", cost: &[(G, 1)], prereqs: &[prereq(1, 1)] },
    // 3: Attack Speed — requires Attack Lv1
    UpgradeNode { label: "Attack Speed", short: "AtkSpd", tooltip: "-8% attack cooldown per level",  category: "Military", cost: &[(F, 1)], prereqs: &[prereq(1, 1)] },
    // 4: Move Speed — root
    UpgradeNode { label: "Move Speed",   short: "MvSpd",  tooltip: "+5% movement speed per level",   category: "Military", cost: &[(F, 1)], prereqs: &[] },
    // 5: Alert — requires Move Speed Lv1
    UpgradeNode { label: "Alert",        short: "Alert",  tooltip: "+10% alert radius per level",    category: "Military", cost: &[(G, 1)], prereqs: &[prereq(4, 1)] },
    // 6: Dodge — requires Move Speed Lv5
    UpgradeNode { label: "Dodge",        short: "Dodge",  tooltip: "Unlocks projectile dodging",     category: "Military", cost: &[(G, 20)], prereqs: &[prereq(4, 5)] },

    // Farmer (7-9)
    // 7: Yield — root
    UpgradeNode { label: "Yield",        short: "Yield",  tooltip: "+15% food production per level", category: "Farmer",   cost: &[(F, 1)], prereqs: &[] },
    // 8: HP — root
    UpgradeNode { label: "HP",           short: "HP",     tooltip: "+20% farmer HP per level",       category: "Farmer",   cost: &[(F, 1)], prereqs: &[] },
    // 9: Move Speed — root
    UpgradeNode { label: "Move Speed",   short: "MvSpd",  tooltip: "+5% farmer speed per level",     category: "Farmer",   cost: &[(F, 1)], prereqs: &[] },

    // Miner (10-12)
    // 10: HP — root
    UpgradeNode { label: "HP",           short: "HP",     tooltip: "+20% miner HP per level",        category: "Miner",    cost: &[(F, 1)], prereqs: &[] },
    // 11: Move Speed — root
    UpgradeNode { label: "Move Speed",   short: "MvSpd",  tooltip: "+5% miner speed per level",      category: "Miner",    cost: &[(F, 1)], prereqs: &[] },
    // 12: Yield — root
    UpgradeNode { label: "Yield",        short: "Yield",  tooltip: "+15% gold yield per level",      category: "Miner",    cost: &[(G, 1)], prereqs: &[] },

    // Town (13-17)
    // 13: Healing — root
    UpgradeNode { label: "Healing",      short: "Heal",   tooltip: "+20% HP regen at fountain",      category: "Town",     cost: &[(F, 1)], prereqs: &[] },
    // 14: Fountain Range — requires Healing Lv1
    UpgradeNode { label: "Fountain Range", short: "FRng", tooltip: "+24px fountain range per level", category: "Town", cost: &[(G, 1)], prereqs: &[prereq(13, 1)] },
    // 15: Fountain Attack Speed — requires Fountain Range Lv1
    UpgradeNode { label: "Fountain Atk Speed", short: "FAS", tooltip: "-8% fountain cooldown per level", category: "Town", cost: &[(G, 1)], prereqs: &[prereq(14, 1)] },
    // 16: Fountain Projectile Life — requires Fountain Range Lv1
    UpgradeNode { label: "Fountain Proj Life", short: "FPL", tooltip: "+8% fountain projectile life per level", category: "Town", cost: &[(G, 1)], prereqs: &[prereq(14, 1)] },
    // 17: Expansion — root, custom slot-based cost
    UpgradeNode { label: "Expansion",    short: "Area",   tooltip: "+1 buildable radius per level",  category: "Town",     cost: &[(F, 1), (G, 1)], prereqs: &[] },

    // Arrow (18-19): applies to military ranged attacks
    // 18: Arrow Speed — requires Range Lv1
    UpgradeNode { label: "Arrow Speed",  short: "ASpd",   tooltip: "+8% arrow speed per level",      category: "Military", cost: &[(G, 1)], prereqs: &[prereq(2, 1)] },
    // 19: Arrow Range — requires Range Lv1
    UpgradeNode { label: "Arrow Range",  short: "ARng",   tooltip: "+8% arrow flight distance per level", category: "Military", cost: &[(G, 1)], prereqs: &[prereq(2, 1)] },

    // Crossbow (20-24): separate upgrade branch
    // 20: HP — root
    UpgradeNode { label: "HP",           short: "HP",     tooltip: "+10% crossbow HP per level",        category: "Crossbow", cost: &[(F, 2)], prereqs: &[] },
    // 21: Attack — root
    UpgradeNode { label: "Attack",       short: "Atk",    tooltip: "+10% crossbow damage per level",    category: "Crossbow", cost: &[(F, 2)], prereqs: &[] },
    // 22: Range — requires Attack Lv1
    UpgradeNode { label: "Range",        short: "Rng",    tooltip: "+5% crossbow range per level",      category: "Crossbow", cost: &[(G, 2)], prereqs: &[prereq(21, 1)] },
    // 23: Attack Speed — requires Attack Lv1
    UpgradeNode { label: "Attack Speed", short: "AtkSpd", tooltip: "-8% crossbow cooldown per level",   category: "Crossbow", cost: &[(F, 2)], prereqs: &[prereq(21, 1)] },
    // 24: Move Speed — root
    UpgradeNode { label: "Move Speed",   short: "MvSpd",  tooltip: "+5% crossbow speed per level",      category: "Crossbow", cost: &[(F, 2)], prereqs: &[] },
];

/// True if this town has unlocked projectile dodge.
pub fn dodge_unlocked(levels: &[u8; UPGRADE_COUNT]) -> bool {
    levels[UpgradeType::Dodge as usize] > 0
}

/// Look up upgrade metadata by index.
pub fn upgrade_node(idx: usize) -> &'static UpgradeNode {
    &UPGRADE_REGISTRY[idx]
}

/// Render order for the upgrade tree UI. Each entry: (branch_label, &[(upgrade_index, depth)]).
/// Depth controls indentation. Nodes within a branch are listed in tree traversal order.
pub const UPGRADE_RENDER_ORDER: &[(&str, &[(usize, u8)])] = &[
    ("Military", &[
        (1, 0),   // Attack (root)
        (2, 1),   // Range (req Attack)
        (18, 2),  // Arrow Speed (req Range)
        (19, 2),  // Arrow Range (req Range)
        (3, 1),   // Attack Speed (req Attack)
        (4, 0),   // Move Speed (root)
        (5, 1),   // Alert (req Move Speed)
        (6, 1),   // Dodge (req Move Speed Lv5)
        (0, 0),   // HP (standalone root)
    ]),
    ("Farmer", &[
        (7, 0),   // Yield (root)
        (8, 0),   // HP (root)
        (9, 0),   // Move Speed (root)
    ]),
    ("Miner", &[
        (10, 0),  // HP (root)
        (11, 0),  // Move Speed (root)
        (12, 0),  // Yield (root)
    ]),
    ("Town", &[
        (13, 0),  // Healing (root)
        (14, 1),  // Fountain Range (req Healing)
        (15, 2),  // Fountain Attack Speed (req Fountain Range)
        (16, 2),  // Fountain Projectile Life (req Fountain Range)
        (17, 0),  // Expansion (root)
    ]),
    ("Crossbow", &[
        (21, 0),  // Attack (root)
        (22, 1),  // Range (req Attack)
        (23, 1),  // Attack Speed (req Attack)
        (24, 0),  // Move Speed (root)
        (20, 0),  // HP (standalone root)
    ]),
];

/// Sum of upgrade levels for all nodes in a given category.
pub fn branch_total(levels: &[u8; UPGRADE_COUNT], category: &str) -> u32 {
    UPGRADE_REGISTRY.iter().enumerate()
        .filter(|(_, n)| n.category == category)
        .map(|(i, _)| levels[i] as u32)
        .sum()
}

/// Effect summary for a given upgrade at its current level.
/// Returns (now_text, next_text) for display in the upgrade UI.
pub fn upgrade_effect_summary(idx: usize, level: u8) -> (String, String) {
    let pct = UPGRADE_PCT[idx];
    let lv = level as f32;
    match idx {
        // Multiplicative: +X% per level
        0 | 1 | 2 | 4 | 5 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 16 | 18 | 19 | 20 | 21 | 22 | 24 => {
            let now = if level == 0 { "\u{2014}".to_string() } else { format!("+{:.0}%", lv * pct * 100.0) };
            let next = format!("+{:.0}%", (lv + 1.0) * pct * 100.0);
            (now, next)
        }
        // Reciprocal: cooldown reduction (idx 3 military, idx 15 fountain, idx 23 crossbow)
        3 | 15 | 23 => {
            let now = if level == 0 { "\u{2014}".to_string() } else {
                let reduction = (1.0 - 1.0 / (1.0 + lv * pct)) * 100.0;
                format!("-{:.0}%", reduction)
            };
            let next_reduction = (1.0 - 1.0 / (1.0 + (lv + 1.0) * pct)) * 100.0;
            let next = format!("-{:.0}%", next_reduction);
            (now, next)
        }
        // Unlock: Dodge (idx 6)
        6 => {
            let now = if level == 0 { "Locked".to_string() } else { "Unlocked".to_string() };
            let next = if level == 0 { "Unlocks dodge".to_string() } else { "Unlocked".to_string() };
            (now, next)
        }
        // Flat: Fountain Range +24px per level (idx 14)
        14 => {
            let now = if level == 0 { "\u{2014}".to_string() } else { format!("+{}px", level as i32 * 24) };
            let next = format!("+{}px", (level as i32 + 1) * 24);
            (now, next)
        }
        // Discrete: Expansion +1 radius per level (idx 17)
        17 => {
            let now = if level == 0 { "\u{2014}".to_string() } else { format!("+{}", level) };
            let next = format!("+{}", level + 1);
            (now, next)
        }
        _ => ("\u{2014}".to_string(), "\u{2014}".to_string()),
    }
}

/// Per-town upgrade levels.
#[derive(Resource)]
pub struct TownUpgrades {
    pub levels: Vec<[u8; UPGRADE_COUNT]>,
}

impl TownUpgrades {
    pub fn town_levels(&self, town_idx: usize) -> [u8; UPGRADE_COUNT] {
        self.levels.get(town_idx).copied().unwrap_or([0; UPGRADE_COUNT])
    }
}

impl Default for TownUpgrades {
    fn default() -> Self {
        Self { levels: vec![[0; UPGRADE_COUNT]; 16] }
    }
}

/// Queue of upgrade purchase requests from UI. Drained by process_upgrades_system.
#[derive(Resource, Default)]
pub struct UpgradeQueue(pub Vec<(usize, usize)>); // (town_idx, upgrade_index)

// ============================================================================
// HELPERS
// ============================================================================

/// Decode persisted upgrade levels into the current upgrade layout.
/// Supports migration from legacy 18-slot layout to current 20-slot layout.
pub fn decode_upgrade_levels(raw: &[u8]) -> [u8; UPGRADE_COUNT] {
    let mut arr = [0u8; UPGRADE_COUNT];
    if raw.len() == 18 {
        // Legacy mapping:
        // 0..14 unchanged, 15 TownArea -> 17, 16 ArrowSpeed -> 18, 17 ArrowLife -> 19.
        for i in 0..=14 { arr[i] = raw[i]; }
        arr[UpgradeType::TownArea as usize] = raw[15];
        arr[UpgradeType::ProjectileSpeed as usize] = raw[16];
        arr[UpgradeType::ProjectileLifetime as usize] = raw[17];
        return arr;
    }
    for (i, &val) in raw.iter().enumerate().take(UPGRADE_COUNT) {
        arr[i] = val;
    }
    arr
}

/// Decode persisted auto-upgrade flags into the current upgrade layout.
/// Supports migration from legacy 18-slot layout to current 20-slot layout.
pub fn decode_auto_upgrade_flags(raw: &[bool]) -> [bool; UPGRADE_COUNT] {
    let mut arr = [false; UPGRADE_COUNT];
    if raw.len() == 18 {
        for i in 0..=14 { arr[i] = raw[i]; }
        arr[UpgradeType::TownArea as usize] = raw[15];
        arr[UpgradeType::ProjectileSpeed as usize] = raw[16];
        arr[UpgradeType::ProjectileLifetime as usize] = raw[17];
        return arr;
    }
    for (i, &val) in raw.iter().enumerate().take(UPGRADE_COUNT) {
        arr[i] = val;
    }
    arr
}

/// Derive level from XP: level = floor(sqrt(xp / 100))
pub fn level_from_xp(xp: i32) -> i32 {
    if xp <= 0 { return 0; }
    (xp as f32 / 100.0).sqrt().floor() as i32
}

/// Upgrade cost scale factor: base 10, doubles each level. Caps at level 20 to avoid overflow.
pub fn upgrade_cost(level: u8) -> i32 {
    let clamped = (level as u32).min(20);
    10 * (1_i32 << clamped)
}

/// Custom cost for TownArea/Expansion: proportional to new building slots unlocked.
/// Each level adds (24 + 8*level) new slots. Cost = base_per_slot * new_slots.
pub fn expansion_cost(level: u8) -> (i32, i32) {
    let new_slots = 24 + 8 * level as i32;
    (new_slots, new_slots) // food, gold
}

/// Check if all prerequisites for an upgrade are met.
pub fn upgrade_unlocked(levels: &[u8; UPGRADE_COUNT], idx: usize) -> bool {
    UPGRADE_REGISTRY[idx].prereqs.iter().all(|p| levels[p.upgrade] >= p.min_level)
}

/// Full purchasability check: prereqs met AND can afford all costs.
/// Single gate used by process_upgrades, auto_upgrade, AI, and UI.
pub fn upgrade_available(levels: &[u8; UPGRADE_COUNT], idx: usize, food: i32, gold: i32) -> bool {
    upgrade_unlocked(levels, idx) && can_afford_upgrade(idx, levels[idx], food, gold)
}

/// Check if a town can afford an upgrade at the given level.
fn can_afford_upgrade(idx: usize, level: u8, food: i32, gold: i32) -> bool {
    if idx == UpgradeType::TownArea as usize {
        let (fc, gc) = expansion_cost(level);
        return food >= fc && gold >= gc;
    }
    let scale = upgrade_cost(level);
    UPGRADE_REGISTRY[idx].cost.iter().all(|&(kind, base)| {
        let total = base * scale;
        match kind {
            ResourceKind::Food => food >= total,
            ResourceKind::Gold => gold >= total,
        }
    })
}

/// Deduct upgrade cost from storages. Caller must verify upgrade_available first.
pub fn deduct_upgrade_cost(idx: usize, level: u8, food: &mut i32, gold: &mut i32) {
    if idx == UpgradeType::TownArea as usize {
        let (fc, gc) = expansion_cost(level);
        *food -= fc;
        *gold -= gc;
        return;
    }
    let scale = upgrade_cost(level);
    for &(kind, base) in UPGRADE_REGISTRY[idx].cost {
        let total = base * scale;
        match kind {
            ResourceKind::Food => *food -= total,
            ResourceKind::Gold => *gold -= total,
        }
    }
}

/// Format missing prereqs as human-readable string.
pub fn missing_prereqs(levels: &[u8; UPGRADE_COUNT], idx: usize) -> Option<String> {
    let missing: Vec<_> = UPGRADE_REGISTRY[idx].prereqs.iter()
        .filter(|p| levels[p.upgrade] < p.min_level)
        .map(|p| format!("{} Lv{}", UPGRADE_REGISTRY[p.upgrade].label, p.min_level))
        .collect();
    if missing.is_empty() { None } else { Some(format!("Requires: {}", missing.join(", "))) }
}

/// Format cost for UI display (e.g. "10+10g").
pub fn format_upgrade_cost(idx: usize, level: u8) -> String {
    if idx == UpgradeType::TownArea as usize {
        let (fc, gc) = expansion_cost(level);
        return format!("{fc}+{gc}g");
    }
    let scale = upgrade_cost(level);
    UPGRADE_REGISTRY[idx].cost.iter()
        .map(|&(kind, base)| {
            let total = base * scale;
            match kind {
                ResourceKind::Food => format!("{total}"),
                ResourceKind::Gold => format!("{total}g"),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Resolve town tower stats from base constants + town upgrades.
pub fn resolve_town_tower_stats(levels: &[u8; UPGRADE_COUNT]) -> TowerStats {
    let cooldown_mult = 1.0 / (1.0 + levels[UpgradeType::FountainAttackSpeed as usize] as f32 * UPGRADE_PCT[UpgradeType::FountainAttackSpeed as usize]);
    let proj_life_mult = 1.0 + levels[UpgradeType::FountainProjectileLife as usize] as f32 * UPGRADE_PCT[UpgradeType::FountainProjectileLife as usize];
    let radius_bonus = levels[UpgradeType::FountainRange as usize] as f32 * 24.0;

    TowerStats {
        range: FOUNTAIN_TOWER.range + radius_bonus,
        damage: FOUNTAIN_TOWER.damage,
        cooldown: FOUNTAIN_TOWER.cooldown * cooldown_mult,
        proj_speed: FOUNTAIN_TOWER.proj_speed,
        proj_lifetime: FOUNTAIN_TOWER.proj_lifetime * proj_life_mult,
    }
}

/// Which upgrades require NPC stat re-resolution (combat-affecting).
fn is_combat_upgrade(idx: usize) -> bool {
    matches!(idx,
        0 | 1 | 2 | 3 | 4 | // Military: HP, Attack, Range, AttackSpeed, MoveSpeed
        8 | 9 |              // Farmer: HP, MoveSpeed
        10 | 11 |            // Miner: HP, MoveSpeed
        18 | 19 |            // Arrow: ProjectileSpeed, ProjectileLifetime
        20 | 21 | 22 | 23 | 24 // Crossbow: HP, Attack, Range, AttackSpeed, MoveSpeed
    )
}

// ============================================================================
// STAT RESOLVER
// ============================================================================

/// Resolve final NPC stats from config, upgrades, level, and personality.
/// Cached on entity as CachedStats. Re-resolved on spawn, upgrade, or level-up.
pub fn resolve_combat_stats(
    job: Job,
    attack_type: BaseAttackType,
    town_idx: i32,
    level: i32,
    personality: &Personality,
    config: &CombatConfig,
    upgrades: &TownUpgrades,
) -> CachedStats {
    let job_base = config.jobs.get(&job).expect("missing job stats");
    let def = npc_def(job);
    let default_atk = config.attacks.get(&attack_type).expect("missing attack type stats");
    let atk_base = def.attack_override.as_ref().unwrap_or(default_atk);
    let (trait_damage, trait_hp, trait_speed, _trait_yield) = personality.get_stat_multipliers();
    let level_mult = 1.0 + level as f32 * 0.01;

    let town_idx_usize = if town_idx >= 0 { town_idx as usize } else { usize::MAX };
    let town = upgrades.levels.get(town_idx_usize).copied().unwrap_or([0; UPGRADE_COUNT]);

    let upgrade_hp = match job {
        Job::Archer | Job::Raider | Job::Fighter => 1.0 + town[UpgradeType::MilitaryHp as usize] as f32 * UPGRADE_PCT[0],
        Job::Crossbow => 1.0 + town[UpgradeType::CrossbowHp as usize] as f32 * UPGRADE_PCT[20],
        Job::Farmer => 1.0 + town[UpgradeType::FarmerHp as usize] as f32 * UPGRADE_PCT[8],
        Job::Miner  => 1.0 + town[UpgradeType::MinerHp as usize] as f32 * UPGRADE_PCT[10],
    };
    let upgrade_dmg = match job {
        Job::Archer | Job::Raider | Job::Fighter => 1.0 + town[UpgradeType::MilitaryAttack as usize] as f32 * UPGRADE_PCT[1],
        Job::Crossbow => 1.0 + town[UpgradeType::CrossbowAttack as usize] as f32 * UPGRADE_PCT[21],
        _ => 1.0,
    };
    let upgrade_range = match job {
        Job::Archer | Job::Raider | Job::Fighter => 1.0 + town[UpgradeType::MilitaryRange as usize] as f32 * UPGRADE_PCT[2],
        Job::Crossbow => 1.0 + town[UpgradeType::CrossbowRange as usize] as f32 * UPGRADE_PCT[22],
        _ => 1.0,
    };
    let upgrade_speed = match job {
        Job::Archer | Job::Raider | Job::Fighter => 1.0 + town[UpgradeType::MilitaryMoveSpeed as usize] as f32 * UPGRADE_PCT[4],
        Job::Crossbow => 1.0 + town[UpgradeType::CrossbowMoveSpeed as usize] as f32 * UPGRADE_PCT[24],
        Job::Farmer => 1.0 + town[UpgradeType::FarmerMoveSpeed as usize] as f32 * UPGRADE_PCT[9],
        Job::Miner  => 1.0 + town[UpgradeType::MinerMoveSpeed as usize] as f32 * UPGRADE_PCT[11],
    };
    let cooldown_mult = match job {
        Job::Crossbow => 1.0 / (1.0 + town[UpgradeType::CrossbowAttackSpeed as usize] as f32 * UPGRADE_PCT[23]),
        _ => 1.0 / (1.0 + town[UpgradeType::AttackSpeed as usize] as f32 * UPGRADE_PCT[3]),
    };
    let upgrade_proj_speed = match job {
        Job::Archer | Job::Raider | Job::Fighter | Job::Crossbow => 1.0 + town[UpgradeType::ProjectileSpeed as usize] as f32 * UPGRADE_PCT[UpgradeType::ProjectileSpeed as usize],
        _ => 1.0,
    };
    let upgrade_proj_life = match job {
        Job::Archer | Job::Raider | Job::Fighter | Job::Crossbow => 1.0 + town[UpgradeType::ProjectileLifetime as usize] as f32 * UPGRADE_PCT[UpgradeType::ProjectileLifetime as usize],
        _ => 1.0,
    };

    CachedStats {
        damage: job_base.damage * upgrade_dmg * trait_damage * level_mult,
        range: atk_base.range * upgrade_range,
        cooldown: atk_base.cooldown * cooldown_mult,
        projectile_speed: atk_base.projectile_speed * upgrade_proj_speed,
        projectile_lifetime: atk_base.projectile_lifetime * upgrade_proj_life,
        max_health: job_base.max_health * upgrade_hp * trait_hp * level_mult,
        speed: job_base.speed * upgrade_speed * trait_speed,
    }
}

// ============================================================================
// PROCESS UPGRADES SYSTEM
// ============================================================================

/// Drains UpgradeQueue, applies upgrades, re-resolves affected NPC stats.
pub fn process_upgrades_system(
    mut queue: ResMut<UpgradeQueue>,
    mut upgrades: ResMut<TownUpgrades>,
    mut economy: EconomyState,
    npcs_by_town: Res<NpcsByTownCache>,
    npc_map: Res<NpcEntityMap>,
    config: Res<CombatConfig>,
    meta_cache: Res<NpcMetaCache>,
    mut npc_query: Query<(&NpcIndex, &Job, &TownId, &BaseAttackType, &Personality, &mut Health, &mut CachedStats, &mut Speed), Without<Dead>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut world_state: WorldState,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("process_upgrades");
    for (town_idx, upgrade_idx) in queue.0.drain(..) {
        if upgrade_idx >= UPGRADE_COUNT { continue; }
        if town_idx >= upgrades.levels.len() { continue; }

        // Prereq + affordability gate
        let levels = upgrades.town_levels(town_idx);
        let mut food = economy.food_storage.food.get(town_idx).copied().unwrap_or(0);
        let mut gold = economy.gold_storage.gold.get(town_idx).copied().unwrap_or(0);
        if !upgrade_available(&levels, upgrade_idx, food, gold) { continue; }

        // Deduct cost and increment level
        let level = levels[upgrade_idx];
        deduct_upgrade_cost(upgrade_idx, level, &mut food, &mut gold);
        if let Some(f) = economy.food_storage.food.get_mut(town_idx) { *f = food; }
        if let Some(g) = economy.gold_storage.gold.get_mut(town_idx) { *g = gold; }
        upgrades.levels[town_idx][upgrade_idx] = level.saturating_add(1);

        // Invalidate healing zone cache on radius/rate upgrades
        if upgrade_idx == UpgradeType::HealingRate as usize || upgrade_idx == UpgradeType::FountainRange as usize {
            world_state.dirty.healing_zones = true;
        }

        if upgrade_idx == UpgradeType::TownArea as usize {
            if let Some(grid_idx) = world_state.town_grids.grids.iter().position(|g| g.town_data_idx == town_idx) {
                let _ = crate::world::expand_town_build_area(
                    &mut world_state.grid,
                    &world_state.world_data.towns,
                    &mut world_state.town_grids,
                    grid_idx,
                );
            }
            continue;
        }

        // Re-resolve NPC stats if this is a combat-affecting upgrade
        if !is_combat_upgrade(upgrade_idx) { continue; }

        let Some(npc_slots) = npcs_by_town.0.get(town_idx) else { continue };
        for &slot in npc_slots {
            let Some(&entity) = npc_map.0.get(&slot) else { continue };
            let Ok((npc_idx, job, _town_id, atk_type, personality, mut health, mut cached, mut speed)) = npc_query.get_mut(entity) else { continue };

            let npc_level = meta_cache.0[npc_idx.0].level;
            let old_max = cached.max_health;
            *cached = resolve_combat_stats(*job, *atk_type, town_idx as i32, npc_level, personality, &config, &upgrades);
            speed.0 = cached.speed;

            // Rescale HP proportionally
            if old_max > 0.0 && (cached.max_health - old_max).abs() > 0.01 {
                health.0 = health.0 * cached.max_health / old_max;
            }

            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx: npc_idx.0, speed: cached.speed }));
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: npc_idx.0, health: health.0 }));
        }
    }
}

// ============================================================================
// AUTO-UPGRADE SYSTEM
// ============================================================================

/// Once per game hour, queues upgrades for any auto-enabled slots that are affordable.
pub fn auto_upgrade_system(
    game_time: Res<crate::resources::GameTime>,
    auto: Res<crate::resources::AutoUpgrade>,
    upgrades: Res<TownUpgrades>,
    food_storage: Res<crate::resources::FoodStorage>,
    gold_storage: Res<crate::resources::GoldStorage>,
    mut queue: ResMut<UpgradeQueue>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("auto_upgrade");
    if !game_time.hour_ticked { return; }

    for (town_idx, flags) in auto.flags.iter().enumerate() {
        let levels = upgrades.town_levels(town_idx);
        let food = food_storage.food.get(town_idx).copied().unwrap_or(0);
        let gold = gold_storage.gold.get(town_idx).copied().unwrap_or(0);
        for (i, &enabled) in flags.iter().enumerate() {
            if !enabled { continue; }
            if upgrade_available(&levels, i, food, gold) {
                queue.0.push((town_idx, i));
            }
        }
    }
}

// ============================================================================
// XP GRANT SYSTEM
// ============================================================================

/// Grant XP to killers when NPCs die. Runs between death_system and death_cleanup_system.
pub fn xp_grant_system(
    dead_query: Query<(&NpcIndex, Option<&LastHitBy>), With<Dead>>,
    mut killer_query: Query<(&NpcIndex, &Job, &TownId, &BaseAttackType, &Personality, &mut Health, &mut CachedStats, &mut Speed, &Faction), Without<Dead>>,
    npc_map: Res<NpcEntityMap>,
    mut npc_meta: ResMut<NpcMetaCache>,
    mut faction_stats: ResMut<FactionStats>,
    config: Res<CombatConfig>,
    upgrades: Res<TownUpgrades>,
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("xp_grant");
    for (_dead_idx, last_hit) in dead_query.iter() {
        let Some(last_hit) = last_hit else { continue };
        if last_hit.0 < 0 { continue; }
        let killer_slot = last_hit.0 as usize;

        let Some(&killer_entity) = npc_map.0.get(&killer_slot) else { continue };
        let Ok((npc_idx, job, town_id, atk_type, personality, mut health, mut cached, mut speed, killer_faction)) = killer_query.get_mut(killer_entity) else { continue };

        faction_stats.inc_kills(killer_faction.0);
        let idx = npc_idx.0;
        let meta = &mut npc_meta.0[idx];
        let old_xp = meta.xp;
        meta.xp += 100;
        let old_level = level_from_xp(old_xp);
        let new_level = level_from_xp(meta.xp);
        meta.level = new_level;

        if new_level > old_level {
            // Re-resolve stats with new level
            let old_max = cached.max_health;
            *cached = resolve_combat_stats(*job, *atk_type, town_id.0, new_level, personality, &config, &upgrades);
            speed.0 = cached.speed;

            // Rescale HP proportionally
            if old_max > 0.0 {
                health.0 = health.0 * cached.max_health / old_max;
            }

            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx, speed: cached.speed }));
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health: health.0 }));

            // Combat log
            let name = &meta.name;
            let job_str = crate::job_name(meta.job);
            combat_log.push(CombatEventKind::LevelUp, killer_faction.0,
                game_time.day(), game_time.hour(), game_time.minute(),
                format!("{} '{}' reached Lv.{}", job_str, name, new_level));
        }
    }
}
