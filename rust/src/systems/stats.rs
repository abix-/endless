//! Stat resolution system — centralizes NPC stats into CombatConfig + CachedStats.
//! Stage 8: pure refactor. All init values match previous hardcoded constants.

use std::collections::HashMap;
use bevy::prelude::*;
use crate::components::{Job, BaseAttackType, CachedStats, Personality};

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
        jobs.insert(Job::Guard, JobStats { max_health: 100.0, damage: 15.0, speed: 100.0 });
        jobs.insert(Job::Raider, JobStats { max_health: 100.0, damage: 15.0, speed: 100.0 });
        jobs.insert(Job::Farmer, JobStats { max_health: 100.0, damage: 0.0, speed: 100.0 });
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
// TOWN UPGRADES (stub for Stage 8, activated in Stage 9)
// ============================================================================

pub const UPGRADE_COUNT: usize = 14;

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum UpgradeType {
    GuardHealth = 0, GuardAttack = 1, GuardRange = 2, GuardSize = 3,
    AttackSpeed = 4, MoveSpeed = 5, AlertRadius = 6,
    FarmYield = 7, FarmerHp = 8, FarmerCap = 9, GuardCap = 10,
    HealingRate = 11, FoodEfficiency = 12, FountainRadius = 13,
}

pub const UPGRADE_PCT: [f32; UPGRADE_COUNT] = [
    0.10, 0.10, 0.05, 0.05,  // guard: health, attack, range, size
    0.08, 0.05, 0.10,         // cooldown reduction, move speed, alert radius
    0.15, 0.20, 0.0, 0.0,    // farm yield, farmer HP | farmer cap, guard cap (flat)
    0.20, 0.10, 0.0,          // healing rate, food efficiency | fountain radius (flat)
];

/// Per-town upgrade levels. All zeros in Stage 8.
#[derive(Resource)]
pub struct TownUpgrades {
    pub levels: Vec<[u8; UPGRADE_COUNT]>,
}

impl Default for TownUpgrades {
    fn default() -> Self {
        Self { levels: vec![[0; UPGRADE_COUNT]; 16] } // pre-alloc for 16 towns
    }
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

    // Town upgrade multipliers (all 1.0 in Stage 8 since levels are 0)
    let town_idx_usize = if town_idx >= 0 { town_idx as usize } else { usize::MAX };
    let town = upgrades.levels.get(town_idx_usize).copied().unwrap_or([0; UPGRADE_COUNT]);

    let upgrade_hp = match job {
        Job::Guard => 1.0 + town[UpgradeType::GuardHealth as usize] as f32 * UPGRADE_PCT[0],
        Job::Farmer => 1.0 + town[UpgradeType::FarmerHp as usize] as f32 * UPGRADE_PCT[8],
        _ => 1.0,
    };
    let upgrade_dmg = match job {
        Job::Guard => 1.0 + town[UpgradeType::GuardAttack as usize] as f32 * UPGRADE_PCT[1],
        _ => 1.0,
    };
    let upgrade_range = match job {
        Job::Guard => 1.0 + town[UpgradeType::GuardRange as usize] as f32 * UPGRADE_PCT[2],
        _ => 1.0,
    };
    let upgrade_speed = 1.0 + town[UpgradeType::MoveSpeed as usize] as f32 * UPGRADE_PCT[5];
    let cooldown_mult = 1.0 / (1.0 + town[UpgradeType::AttackSpeed as usize] as f32 * UPGRADE_PCT[4]);

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
