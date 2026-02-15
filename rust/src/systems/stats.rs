//! Stat resolution, upgrades, and XP systems.
//! Stage 8: CombatConfig + resolve_combat_stats + CachedStats.
//! Stage 9: UpgradeQueue + process_upgrades_system + xp_grant_system.

use std::collections::HashMap;
use bevy::prelude::*;
use crate::components::{Job, BaseAttackType, CachedStats, Personality, Dead, LastHitBy, Health, Speed, NpcIndex, TownId};
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::resources::{NpcEntityMap, NpcMetaCache, NpcsByTownCache, FoodStorage, CombatLog, CombatEventKind, GameTime, SystemTimings};

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

pub const UPGRADE_COUNT: usize = 13;

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum UpgradeType {
    GuardHealth = 0, GuardAttack = 1, GuardRange = 2, GuardSize = 3,
    AttackSpeed = 4, MoveSpeed = 5, AlertRadius = 6,
    FarmYield = 7, FarmerHp = 8,
    HealingRate = 9, FoodEfficiency = 10, FountainRadius = 11, TownArea = 12,
}

pub const UPGRADE_PCT: [f32; UPGRADE_COUNT] = [
    0.10, 0.10, 0.05, 0.05,  // guard: health, attack, range, size
    0.08, 0.05, 0.10,         // cooldown reduction, move speed, alert radius
    0.15, 0.20,               // farm yield, farmer HP
    0.20, 0.10, 0.0, 0.0,     // healing rate, food efficiency, fountain radius (flat), town area (discrete)
];

// ============================================================================
// UPGRADE REGISTRY (single source of truth for all upgrade metadata)
// ============================================================================

pub struct UpgradeNode {
    pub label: &'static str,
    pub short: &'static str,
    pub tooltip: &'static str,
    pub category: &'static str,
}

pub const UPGRADE_REGISTRY: [UpgradeNode; UPGRADE_COUNT] = [
    UpgradeNode { label: "Guard Health",    short: "G.HP",    tooltip: "+10% guard HP per level",                  category: "Guard" },
    UpgradeNode { label: "Guard Attack",    short: "G.Atk",   tooltip: "+10% guard damage per level",              category: "Guard" },
    UpgradeNode { label: "Guard Range",     short: "G.Rng",   tooltip: "+5% guard attack range per level",         category: "Guard" },
    UpgradeNode { label: "Guard Size",      short: "G.Size",  tooltip: "+5% guard size per level",                 category: "Guard" },
    UpgradeNode { label: "Attack Speed",    short: "AtkSpd",  tooltip: "-8% attack cooldown per level",            category: "Guard" },
    UpgradeNode { label: "Move Speed",      short: "MvSpd",   tooltip: "+5% movement speed per level",             category: "Guard" },
    UpgradeNode { label: "Alert Radius",    short: "Alert",   tooltip: "+10% alert radius per level",              category: "Guard" },
    UpgradeNode { label: "Farm Yield",      short: "FarmY",   tooltip: "+15% food production per level",           category: "Farm" },
    UpgradeNode { label: "Farmer HP",       short: "F.HP",    tooltip: "+20% farmer HP per level",                 category: "Farm" },
    UpgradeNode { label: "Healing Rate",    short: "Heal",    tooltip: "+20% HP regen at fountain per level",      category: "Town" },
    UpgradeNode { label: "Food Efficiency", short: "FoodEff", tooltip: "10% chance per level to not consume food", category: "Town" },
    UpgradeNode { label: "Fountain Radius", short: "Fount",   tooltip: "+24px fountain healing range per level",   category: "Town" },
    UpgradeNode { label: "Town Area",       short: "Area",    tooltip: "+1 buildable radius per level",            category: "Town" },
];

/// Look up upgrade metadata by index.
pub fn upgrade_node(idx: usize) -> &'static UpgradeNode {
    &UPGRADE_REGISTRY[idx]
}

/// Per-town upgrade levels.
#[derive(Resource)]
pub struct TownUpgrades {
    pub levels: Vec<[u8; UPGRADE_COUNT]>,
}

impl Default for TownUpgrades {
    fn default() -> Self {
        Self { levels: vec![[0; UPGRADE_COUNT]; 16] } // pre-alloc for 16 towns
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

/// Upgrade cost: base 10, doubles each level. Caps at level 20 to avoid overflow.
pub fn upgrade_cost(level: u8) -> i32 {
    let clamped = (level as u32).min(20);
    10 * (1_i32 << clamped)
}

/// Which upgrades require NPC stat re-resolution (combat-affecting).
fn is_combat_upgrade(idx: usize) -> bool {
    matches!(idx,
        0 | 1 | 2 | 3 | // GuardHealth, GuardAttack, GuardRange, GuardSize
        4 | 5 |          // AttackSpeed, MoveSpeed
        8                // FarmerHp
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
        Job::Guard => 1.0 + town[UpgradeType::GuardHealth as usize] as f32 * UPGRADE_PCT[0],
        Job::Farmer | Job::Miner => 1.0 + town[UpgradeType::FarmerHp as usize] as f32 * UPGRADE_PCT[8],
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

// ============================================================================
// PROCESS UPGRADES SYSTEM
// ============================================================================

/// Drains UpgradeQueue, applies upgrades, re-resolves affected NPC stats.
pub fn process_upgrades_system(
    mut queue: ResMut<UpgradeQueue>,
    mut upgrades: ResMut<TownUpgrades>,
    mut food_storage: ResMut<FoodStorage>,
    npcs_by_town: Res<NpcsByTownCache>,
    npc_map: Res<NpcEntityMap>,
    config: Res<CombatConfig>,
    meta_cache: Res<NpcMetaCache>,
    mut npc_query: Query<(&NpcIndex, &Job, &TownId, &BaseAttackType, &Personality, &mut Health, &mut CachedStats, &mut Speed), Without<Dead>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut world_grid: ResMut<crate::world::WorldGrid>,
    world_data: Res<crate::world::WorldData>,
    mut town_grids: ResMut<crate::world::TownGrids>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("process_upgrades");
    for (town_idx, upgrade_idx) in queue.0.drain(..) {
        if upgrade_idx >= UPGRADE_COUNT { continue; }
        if town_idx >= upgrades.levels.len() { continue; }

        let level = upgrades.levels[town_idx][upgrade_idx];
        let cost = upgrade_cost(level);

        // Check food
        let food = food_storage.food.get(town_idx).copied().unwrap_or(0);
        if food < cost { continue; }

        // Deduct food and increment level
        if let Some(f) = food_storage.food.get_mut(town_idx) {
            *f -= cost;
        }
        upgrades.levels[town_idx][upgrade_idx] = level.saturating_add(1);

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
    mut queue: ResMut<UpgradeQueue>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("auto_upgrade");
    if !game_time.hour_ticked { return; }

    for (town_idx, flags) in auto.flags.iter().enumerate() {
        let food = food_storage.food.get(town_idx).copied().unwrap_or(0);
        let levels = upgrades.levels.get(town_idx).copied().unwrap_or([0; UPGRADE_COUNT]);
        for (i, &enabled) in flags.iter().enumerate() {
            if enabled && food >= upgrade_cost(levels[i]) {
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
    mut killer_query: Query<(&NpcIndex, &Job, &TownId, &BaseAttackType, &Personality, &mut Health, &mut CachedStats, &mut Speed), Without<Dead>>,
    npc_map: Res<NpcEntityMap>,
    mut npc_meta: ResMut<NpcMetaCache>,
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
        let Ok((npc_idx, job, town_id, atk_type, personality, mut health, mut cached, mut speed)) = killer_query.get_mut(killer_entity) else { continue };

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
