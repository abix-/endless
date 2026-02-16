//! Stat resolution, upgrades, and XP systems.
//! Stage 8: CombatConfig + resolve_combat_stats + CachedStats.
//! Stage 9: UpgradeQueue + process_upgrades_system + xp_grant_system.

use std::collections::HashMap;
use bevy::prelude::*;
use crate::components::{Job, BaseAttackType, CachedStats, Personality, Dead, LastHitBy, Health, Speed, NpcIndex, TownId, Faction};
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::resources::{NpcEntityMap, NpcMetaCache, NpcsByTownCache, FoodStorage, FactionStats, CombatLog, CombatEventKind, GameTime, SystemTimings, DirtyFlags};

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

/// Per-attack-type weapon stats. Determines "how does this NPC fight?"
#[derive(Clone, Debug)]
pub struct AttackTypeStats {
    pub range: f32,
    pub cooldown: f32,
    pub projectile_speed: f32,
    pub projectile_lifetime: f32,
}

/// Central combat configuration. All NPC stats resolve from this.
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
        // All jobs: 100 HP, 100 speed. Damage varies.
        jobs.insert(Job::Archer, JobStats { max_health: 100.0, damage: 15.0, speed: 100.0 });
        jobs.insert(Job::Raider, JobStats { max_health: 100.0, damage: 15.0, speed: 100.0 });
        jobs.insert(Job::Farmer, JobStats { max_health: 100.0, damage: 0.0, speed: 100.0 });
        jobs.insert(Job::Miner, JobStats { max_health: 100.0, damage: 0.0, speed: 100.0 });
        jobs.insert(Job::Fighter, JobStats { max_health: 100.0, damage: 15.0, speed: 100.0 });

        let mut attacks = HashMap::new();
        attacks.insert(BaseAttackType::Melee, AttackTypeStats {
            range: 150.0, cooldown: 1.0, projectile_speed: 500.0, projectile_lifetime: 0.5,
        });
        attacks.insert(BaseAttackType::Ranged, AttackTypeStats {
            range: 300.0, cooldown: 1.5, projectile_speed: 200.0, projectile_lifetime: 3.0,
        });

        Self { jobs, attacks, heal_rate: 5.0, heal_radius: 150.0 }
    }
}

// ============================================================================
// TOWN UPGRADES
// ============================================================================

pub const UPGRADE_COUNT: usize = 16;

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
    HealingRate = 13, FountainRadius = 14, TownArea = 15,
}

pub const UPGRADE_PCT: [f32; UPGRADE_COUNT] = [
    0.10, 0.10, 0.05,  // military: hp, attack, range
    0.08, 0.05, 0.10,  // attack speed (cooldown), military move speed, alert radius
    0.0,               // dodge (unlock)
    0.15, 0.20, 0.05,  // farm yield, farmer hp, farmer move speed
    0.20, 0.05, 0.15,  // miner hp, miner move speed, gold yield
    0.20, 0.0, 0.0,    // healing rate, fountain radius (flat), town area (discrete)
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

    // Town (13-15)
    // 13: Healing — root
    UpgradeNode { label: "Healing",      short: "Heal",   tooltip: "+20% HP regen at fountain",      category: "Town",     cost: &[(F, 1)], prereqs: &[] },
    // 14: Fountain — requires Healing Lv1
    UpgradeNode { label: "Fountain",     short: "Fount",  tooltip: "+24px fountain range per level",  category: "Town",    cost: &[(G, 1)], prereqs: &[prereq(13, 1)] },
    // 15: Expansion — root, custom slot-based cost
    UpgradeNode { label: "Expansion",    short: "Area",   tooltip: "+1 buildable radius per level",  category: "Town",     cost: &[(F, 1), (G, 1)], prereqs: &[] },
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
        (14, 1),  // Fountain (req Healing)
        (15, 0),  // Expansion (root)
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
        0 | 1 | 2 | 4 | 5 | 7 | 8 | 9 | 10 | 11 | 12 | 13 => {
            let now = if level == 0 { "\u{2014}".to_string() } else { format!("+{:.0}%", lv * pct * 100.0) };
            let next = format!("+{:.0}%", (lv + 1.0) * pct * 100.0);
            (now, next)
        }
        // Reciprocal: attack cooldown reduction (idx 3)
        3 => {
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
        // Flat: Fountain +24px per level (idx 14)
        14 => {
            let now = if level == 0 { "\u{2014}".to_string() } else { format!("+{}px", level as i32 * 24) };
            let next = format!("+{}px", (level as i32 + 1) * 24);
            (now, next)
        }
        // Discrete: Expansion +1 radius per level (idx 15)
        15 => {
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

/// Which upgrades require NPC stat re-resolution (combat-affecting).
fn is_combat_upgrade(idx: usize) -> bool {
    matches!(idx,
        0 | 1 | 2 | 3 | 4 | // Military: HP, Attack, Range, AttackSpeed, MoveSpeed
        8 | 9 |              // Farmer: HP, MoveSpeed
        10 | 11              // Miner: HP, MoveSpeed
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
    let atk_base = config.attacks.get(&attack_type).expect("missing attack type stats");
    let (trait_damage, trait_hp, trait_speed, _trait_yield) = personality.get_stat_multipliers();
    let level_mult = 1.0 + level as f32 * 0.01;

    let town_idx_usize = if town_idx >= 0 { town_idx as usize } else { usize::MAX };
    let town = upgrades.levels.get(town_idx_usize).copied().unwrap_or([0; UPGRADE_COUNT]);

    let upgrade_hp = match job {
        Job::Archer | Job::Raider | Job::Fighter => 1.0 + town[UpgradeType::MilitaryHp as usize] as f32 * UPGRADE_PCT[0],
        Job::Farmer => 1.0 + town[UpgradeType::FarmerHp as usize] as f32 * UPGRADE_PCT[8],
        Job::Miner  => 1.0 + town[UpgradeType::MinerHp as usize] as f32 * UPGRADE_PCT[10],
    };
    let upgrade_dmg = match job {
        Job::Archer | Job::Raider | Job::Fighter => 1.0 + town[UpgradeType::MilitaryAttack as usize] as f32 * UPGRADE_PCT[1],
        _ => 1.0,
    };
    let upgrade_range = match job {
        Job::Archer | Job::Raider | Job::Fighter => 1.0 + town[UpgradeType::MilitaryRange as usize] as f32 * UPGRADE_PCT[2],
        _ => 1.0,
    };
    let upgrade_speed = match job {
        Job::Archer | Job::Raider | Job::Fighter => 1.0 + town[UpgradeType::MilitaryMoveSpeed as usize] as f32 * UPGRADE_PCT[4],
        Job::Farmer => 1.0 + town[UpgradeType::FarmerMoveSpeed as usize] as f32 * UPGRADE_PCT[9],
        Job::Miner  => 1.0 + town[UpgradeType::MinerMoveSpeed as usize] as f32 * UPGRADE_PCT[11],
    };
    let cooldown_mult = 1.0 / (1.0 + town[UpgradeType::AttackSpeed as usize] as f32 * UPGRADE_PCT[3]);

    CachedStats {
        damage: job_base.damage * upgrade_dmg * trait_damage * level_mult,
        range: atk_base.range * upgrade_range,
        cooldown: atk_base.cooldown * cooldown_mult,
        projectile_speed: atk_base.projectile_speed,
        projectile_lifetime: atk_base.projectile_lifetime,
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
    mut food_storage: ResMut<FoodStorage>,
    mut gold_storage: ResMut<crate::resources::GoldStorage>,
    npcs_by_town: Res<NpcsByTownCache>,
    npc_map: Res<NpcEntityMap>,
    config: Res<CombatConfig>,
    meta_cache: Res<NpcMetaCache>,
    mut npc_query: Query<(&NpcIndex, &Job, &TownId, &BaseAttackType, &Personality, &mut Health, &mut CachedStats, &mut Speed), Without<Dead>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut world_grid: ResMut<crate::world::WorldGrid>,
    world_data: Res<crate::world::WorldData>,
    mut town_grids: ResMut<crate::world::TownGrids>,
    mut dirty: ResMut<DirtyFlags>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("process_upgrades");
    for (town_idx, upgrade_idx) in queue.0.drain(..) {
        if upgrade_idx >= UPGRADE_COUNT { continue; }
        if town_idx >= upgrades.levels.len() { continue; }

        // Prereq + affordability gate
        let levels = upgrades.town_levels(town_idx);
        let mut food = food_storage.food.get(town_idx).copied().unwrap_or(0);
        let mut gold = gold_storage.gold.get(town_idx).copied().unwrap_or(0);
        if !upgrade_available(&levels, upgrade_idx, food, gold) { continue; }

        // Deduct cost and increment level
        let level = levels[upgrade_idx];
        deduct_upgrade_cost(upgrade_idx, level, &mut food, &mut gold);
        if let Some(f) = food_storage.food.get_mut(town_idx) { *f = food; }
        if let Some(g) = gold_storage.gold.get_mut(town_idx) { *g = gold; }
        upgrades.levels[town_idx][upgrade_idx] = level.saturating_add(1);

        // Invalidate healing zone cache on radius/rate upgrades
        if upgrade_idx == UpgradeType::HealingRate as usize || upgrade_idx == UpgradeType::FountainRadius as usize {
            dirty.healing_zones = true;
        }

        if upgrade_idx == UpgradeType::TownArea as usize {
            if let Some(grid_idx) = town_grids.grids.iter().position(|g| g.town_data_idx == town_idx) {
                let _ = crate::world::expand_town_build_area(
                    &mut world_grid,
                    &world_data.towns,
                    &mut town_grids,
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
            combat_log.push(CombatEventKind::LevelUp,
                game_time.day(), game_time.hour(), game_time.minute(),
                format!("{} '{}' reached Lv.{}", job_str, name, new_level));
        }
    }
}
