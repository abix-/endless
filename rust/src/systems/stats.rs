//! Stat resolution, upgrades, and XP systems.
//! Stage 8: CombatConfig + resolve_combat_stats + CachedStats.
//! Stage 9: UpgradeQueue + process_upgrades_system + xp_grant_system.

use std::collections::HashMap;
use std::sync::LazyLock;
use bevy::prelude::*;
use crate::components::{Job, BaseAttackType, CachedStats, CombatState, Personality, Dead, LastHitBy, Health, Speed, NpcIndex, TownId, Faction, Activity, Home};
use crate::constants::{FOUNTAIN_TOWER, TowerStats, NPC_REGISTRY, npc_def, AttackTypeStats, UpgradeStatKind, UpgradeStatDef, ResourceKind, EffectDisplay, TOWN_UPGRADES, ItemKind};
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
// DYNAMIC UPGRADE REGISTRY (built from NPC_REGISTRY + TOWN_UPGRADES)
// ============================================================================

/// A single upgrade entry in the registry (built at init).
pub struct UpgradeNode {
    pub label: &'static str,
    pub short: &'static str,
    pub tooltip: &'static str,
    pub category: &'static str,
    pub stat_kind: UpgradeStatKind,
    pub pct: f32,
    pub cost: &'static [(ResourceKind, i32)],
    pub display: EffectDisplay,
    pub prereqs: Vec<(usize, u8)>, // (prereq_index, min_level)
    pub is_combat_stat: bool,
    pub invalidates_healing: bool,
    pub triggers_expansion: bool,
    pub custom_cost: bool,
}

/// A branch in the upgrade tree UI.
pub struct UpgradeBranch {
    pub label: &'static str,
    pub section: &'static str, // "Economy" or "Military"
    pub entries: Vec<(usize, u8)>, // (node_index, depth)
}

/// The complete upgrade registry, built once at startup.
pub struct UpgradeRegistry {
    pub nodes: Vec<UpgradeNode>,
    pub branches: Vec<UpgradeBranch>,
    /// (category, stat_kind) → index into nodes
    pub index_map: HashMap<(&'static str, UpgradeStatKind), usize>,
}

impl UpgradeRegistry {
    /// Number of upgrade slots.
    pub fn count(&self) -> usize { self.nodes.len() }

    /// Look up index for a (category, stat_kind) pair. Returns None if not found.
    pub fn index(&self, category: &str, kind: UpgradeStatKind) -> Option<usize> {
        self.index_map.get(&(category, kind)).copied()
            .or_else(|| {
                // Fallback: linear scan for categories with non-static lifetime
                self.nodes.iter().position(|n| n.category == category && n.stat_kind == kind)
            })
    }

    /// Get the multiplier for a (category, stat) at a town's upgrade levels.
    /// Returns 1.0 if the upgrade doesn't exist or level is 0.
    pub fn stat_mult(&self, levels: &[u8], category: &str, kind: UpgradeStatKind) -> f32 {
        if let Some(idx) = self.index(category, kind) {
            let lv = levels.get(idx).copied().unwrap_or(0) as f32;
            let pct = self.nodes[idx].pct;
            match self.nodes[idx].display {
                EffectDisplay::CooldownReduction => 1.0 / (1.0 + lv * pct),
                _ => 1.0 + lv * pct,
            }
        } else {
            1.0
        }
    }

    /// Get raw level for a (category, stat) pair.
    pub fn stat_level(&self, levels: &[u8], category: &str, kind: UpgradeStatKind) -> u8 {
        if let Some(idx) = self.index(category, kind) {
            levels.get(idx).copied().unwrap_or(0)
        } else {
            0
        }
    }
}

/// Build the upgrade registry from NPC_REGISTRY + TOWN_UPGRADES.
fn build_upgrade_registry() -> UpgradeRegistry {
    let mut nodes: Vec<UpgradeNode> = Vec::new();
    let mut branches: Vec<UpgradeBranch> = Vec::new();
    let mut index_map: HashMap<(&'static str, UpgradeStatKind), usize> = HashMap::new();

    // Collect unique NPC categories in order of first appearance
    let mut seen_categories: Vec<&'static str> = Vec::new();
    for npc in NPC_REGISTRY {
        if let Some(cat) = npc.upgrade_category {
            if !seen_categories.contains(&cat) {
                seen_categories.push(cat);
            }
        }
    }

    // For each category, collect union of upgrade stats across all NPCs in that category
    for &category in &seen_categories {
        let mut stat_defs: Vec<&'static UpgradeStatDef> = Vec::new();
        for npc in NPC_REGISTRY {
            if npc.upgrade_category == Some(category) {
                for def in npc.upgrade_stats {
                    if !stat_defs.iter().any(|d| d.kind == def.kind) {
                        stat_defs.push(def);
                    }
                }
            }
        }

        let branch_start = nodes.len();
        // First pass: create nodes (without prereqs)
        for def in &stat_defs {
            let idx = nodes.len();
            index_map.insert((category, def.kind), idx);
            nodes.push(UpgradeNode {
                label: def.label, short: def.short, tooltip: def.tooltip,
                category, stat_kind: def.kind, pct: def.pct, cost: def.cost,
                display: def.display, prereqs: Vec::new(),
                is_combat_stat: def.is_combat_stat,
                invalidates_healing: def.invalidates_healing,
                triggers_expansion: def.triggers_expansion,
                custom_cost: def.custom_cost,
            });
        }

        // Second pass: resolve prereqs (stat kind → index within this category)
        for (i, def) in stat_defs.iter().enumerate() {
            if let Some(prereq_kind) = def.prereq_stat {
                if let Some(&prereq_idx) = index_map.get(&(category, prereq_kind)) {
                    nodes[branch_start + i].prereqs.push((prereq_idx, def.prereq_level));
                }
            }
        }

        // Build tree depth for UI rendering
        let mut entries: Vec<(usize, u8)> = Vec::new();
        // Compute depth: root (no prereqs) = 0, child of root = 1, grandchild = 2
        fn compute_depth(nodes: &[UpgradeNode], idx: usize, visited: &mut Vec<usize>) -> u8 {
            if visited.contains(&idx) { return 0; }
            visited.push(idx);
            if nodes[idx].prereqs.is_empty() { return 0; }
            let max_parent = nodes[idx].prereqs.iter()
                .map(|&(pi, _)| compute_depth(nodes, pi, visited))
                .max().unwrap_or(0);
            max_parent + 1
        }

        // Group: roots first, then depth 1, then depth 2...
        // Within each depth, preserve stat_defs order (which matches NPC declaration order)
        let mut by_depth: Vec<(usize, u8)> = Vec::new();
        for i in 0..stat_defs.len() {
            let idx = branch_start + i;
            let depth = compute_depth(&nodes, idx, &mut Vec::new());
            by_depth.push((idx, depth));
        }

        // Build tree-ordered list: for each root node, emit it then its children (DFS)
        fn emit_tree(idx: usize, depth: u8, nodes: &[UpgradeNode], all: &[(usize, u8)], entries: &mut Vec<(usize, u8)>, emitted: &mut Vec<usize>) {
            if emitted.contains(&idx) { return; }
            emitted.push(idx);
            entries.push((idx, depth));
            // Find children of this node (nodes that have this idx as a prereq)
            for &(child_idx, child_depth) in all {
                if nodes[child_idx].prereqs.iter().any(|&(pi, _)| pi == idx) {
                    emit_tree(child_idx, child_depth, nodes, all, entries, emitted);
                }
            }
        }

        let mut emitted: Vec<usize> = Vec::new();
        // Start from roots (depth 0)
        for &(idx, depth) in &by_depth {
            if depth == 0 {
                emit_tree(idx, depth, &nodes, &by_depth, &mut entries, &mut emitted);
            }
        }

        // Derive section from the first NPC in this category
        let section = NPC_REGISTRY.iter()
            .find(|n| n.upgrade_category == Some(category))
            .map(|n| if n.is_military { "Military" } else { "Economy" })
            .unwrap_or("Economy");
        branches.push(UpgradeBranch { label: category, section, entries });
    }

    // Town upgrades (appended as final branch)
    {
        let branch_start = nodes.len();
        for def in TOWN_UPGRADES {
            let idx = nodes.len();
            index_map.insert(("Town", def.kind), idx);
            nodes.push(UpgradeNode {
                label: def.label, short: def.short, tooltip: def.tooltip,
                category: "Town", stat_kind: def.kind, pct: def.pct, cost: def.cost,
                display: def.display, prereqs: Vec::new(),
                is_combat_stat: def.is_combat_stat,
                invalidates_healing: def.invalidates_healing,
                triggers_expansion: def.triggers_expansion,
                custom_cost: def.custom_cost,
            });
        }
        // Resolve Town prereqs
        for (i, def) in TOWN_UPGRADES.iter().enumerate() {
            if let Some(prereq_kind) = def.prereq_stat {
                if let Some(&prereq_idx) = index_map.get(&("Town", prereq_kind)) {
                    nodes[branch_start + i].prereqs.push((prereq_idx, def.prereq_level));
                }
            }
        }
        // Build Town tree entries
        let mut entries: Vec<(usize, u8)> = Vec::new();
        let mut by_depth: Vec<(usize, u8)> = Vec::new();
        for i in 0..TOWN_UPGRADES.len() {
            let idx = branch_start + i;
            let depth = if nodes[idx].prereqs.is_empty() { 0 } else {
                let max_p = nodes[idx].prereqs.iter()
                    .map(|&(pi, _)| if nodes[pi].prereqs.is_empty() { 0u8 } else { 1 })
                    .max().unwrap_or(0);
                max_p + 1
            };
            by_depth.push((idx, depth));
        }
        fn emit_town_tree(idx: usize, depth: u8, nodes: &[UpgradeNode], all: &[(usize, u8)], entries: &mut Vec<(usize, u8)>, emitted: &mut Vec<usize>) {
            if emitted.contains(&idx) { return; }
            emitted.push(idx);
            entries.push((idx, depth));
            for &(child_idx, child_depth) in all {
                if nodes[child_idx].prereqs.iter().any(|&(pi, _)| pi == idx) {
                    emit_town_tree(child_idx, child_depth, nodes, all, entries, emitted);
                }
            }
        }
        let mut emitted: Vec<usize> = Vec::new();
        for &(idx, depth) in &by_depth {
            if depth == 0 {
                emit_town_tree(idx, depth, &nodes, &by_depth, &mut entries, &mut emitted);
            }
        }
        branches.push(UpgradeBranch { label: "Town", section: "Economy", entries });
    }

    UpgradeRegistry { nodes, branches, index_map }
}

/// Global upgrade registry, built once from NPC_REGISTRY + TOWN_UPGRADES.
pub static UPGRADES: LazyLock<UpgradeRegistry> = LazyLock::new(build_upgrade_registry);

/// Number of upgrade slots in the current registry.
pub fn upgrade_count() -> usize { UPGRADES.count() }

/// Look up upgrade node by index.
pub fn upgrade_node(idx: usize) -> &'static UpgradeNode { &UPGRADES.nodes[idx] }

/// True if this town has unlocked projectile dodge (any NPC category that has Dodge).
pub fn dodge_unlocked(levels: &[u8]) -> bool {
    // Check all categories that have a Dodge upgrade
    UPGRADES.nodes.iter().enumerate()
        .any(|(i, n)| n.stat_kind == UpgradeStatKind::Dodge && levels.get(i).copied().unwrap_or(0) > 0)
}

/// Sum of upgrade levels for all nodes in a given category.
pub fn branch_total(levels: &[u8], category: &str) -> u32 {
    UPGRADES.nodes.iter().enumerate()
        .filter(|(_, n)| n.category == category)
        .map(|(i, _)| levels.get(i).copied().unwrap_or(0) as u32)
        .sum()
}

/// Effect summary for a given upgrade at its current level.
/// Returns (now_text, next_text) for display in the upgrade UI.
pub fn upgrade_effect_summary(idx: usize, level: u8) -> (String, String) {
    let node = &UPGRADES.nodes[idx];
    let pct = node.pct;
    let lv = level as f32;

    match node.display {
        EffectDisplay::Percentage => {
            let now = if level == 0 { "-".to_string() } else { format!("+{:.0}%", lv * pct * 100.0) };
            let next = format!("+{:.0}%", (lv + 1.0) * pct * 100.0);
            (now, next)
        }
        EffectDisplay::CooldownReduction => {
            let now = if level == 0 { "-".to_string() } else {
                let reduction = (1.0 - 1.0 / (1.0 + lv * pct)) * 100.0;
                format!("-{:.0}%", reduction)
            };
            let next_reduction = (1.0 - 1.0 / (1.0 + (lv + 1.0) * pct)) * 100.0;
            let next = format!("-{:.0}%", next_reduction);
            (now, next)
        }
        EffectDisplay::Unlock => {
            let now = if level == 0 { "Locked".to_string() } else { "Unlocked".to_string() };
            let next = if level == 0 { "Unlocks".to_string() } else { "Unlocked".to_string() };
            (now, next)
        }
        EffectDisplay::FlatPixels(px_per_level) => {
            let now = if level == 0 { "-".to_string() } else { format!("+{}px", level as i32 * px_per_level) };
            let next = format!("+{}px", (level as i32 + 1) * px_per_level);
            (now, next)
        }
        EffectDisplay::Discrete => {
            let now = if level == 0 { "-".to_string() } else { format!("+{}", level) };
            let next = format!("+{}", level + 1);
            (now, next)
        }
    }
}

/// Per-town upgrade levels (dynamic size, matches UPGRADES.count()).
#[derive(Resource)]
pub struct TownUpgrades {
    pub levels: Vec<Vec<u8>>,
}

impl TownUpgrades {
    pub fn town_levels(&self, town_idx: usize) -> Vec<u8> {
        let count = upgrade_count();
        self.levels.get(town_idx)
            .map(|v| {
                let mut r = v.clone();
                r.resize(count, 0);
                r
            })
            .unwrap_or_else(|| vec![0; count])
    }

    /// Ensure the levels vec has at least `n` entries.
    pub fn ensure_towns(&mut self, n: usize) {
        let count = upgrade_count();
        while self.levels.len() < n {
            self.levels.push(vec![0; count]);
        }
        // Pad existing entries if upgrade count grew
        for v in &mut self.levels {
            v.resize(count, 0);
        }
    }
}

impl Default for TownUpgrades {
    fn default() -> Self {
        let count = upgrade_count();
        Self { levels: vec![vec![0; count]; 16] }
    }
}

/// Queue of upgrade purchase requests from UI. Drained by process_upgrades_system.
#[derive(Resource, Default)]
pub struct UpgradeQueue(pub Vec<(usize, usize)>); // (town_idx, upgrade_index)

// ============================================================================
// HELPERS
// ============================================================================

/// Decode persisted upgrade levels into the current upgrade layout.
/// Pads to current upgrade_count() if the saved data is shorter (new upgrades added).
pub fn decode_upgrade_levels(raw: &[u8]) -> Vec<u8> {
    let count = upgrade_count();
    let mut result = vec![0u8; count];
    for (i, &val) in raw.iter().enumerate().take(count) {
        result[i] = val;
    }
    result
}

/// Decode persisted auto-upgrade flags into the current upgrade layout.
/// Pads to current upgrade_count() if the saved data is shorter.
pub fn decode_auto_upgrade_flags(raw: &[bool]) -> Vec<bool> {
    let count = upgrade_count();
    let mut result = vec![false; count];
    for (i, &val) in raw.iter().enumerate().take(count) {
        result[i] = val;
    }
    result
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
pub fn upgrade_unlocked(levels: &[u8], idx: usize) -> bool {
    UPGRADES.nodes[idx].prereqs.iter().all(|&(pi, min_lv)| levels.get(pi).copied().unwrap_or(0) >= min_lv)
}

/// Full purchasability check: prereqs met AND can afford all costs.
/// Single gate used by process_upgrades, auto_upgrade, AI, and UI.
pub fn upgrade_available(levels: &[u8], idx: usize, food: i32, gold: i32) -> bool {
    let lv = levels.get(idx).copied().unwrap_or(0);
    upgrade_unlocked(levels, idx) && can_afford_upgrade(idx, lv, food, gold)
}

/// Check if a town can afford an upgrade at the given level.
fn can_afford_upgrade(idx: usize, level: u8, food: i32, gold: i32) -> bool {
    let node = &UPGRADES.nodes[idx];
    if node.custom_cost {
        let (fc, gc) = expansion_cost(level);
        return food >= fc && gold >= gc;
    }
    let scale = upgrade_cost(level);
    node.cost.iter().all(|&(kind, base)| {
        let total = base * scale;
        match kind {
            ResourceKind::Food => food >= total,
            ResourceKind::Gold => gold >= total,
        }
    })
}

/// Deduct upgrade cost from storages. Caller must verify upgrade_available first.
pub fn deduct_upgrade_cost(idx: usize, level: u8, food: &mut i32, gold: &mut i32) {
    let node = &UPGRADES.nodes[idx];
    if node.custom_cost {
        let (fc, gc) = expansion_cost(level);
        *food -= fc;
        *gold -= gc;
        return;
    }
    let scale = upgrade_cost(level);
    for &(kind, base) in node.cost {
        let total = base * scale;
        match kind {
            ResourceKind::Food => *food -= total,
            ResourceKind::Gold => *gold -= total,
        }
    }
}

/// Format missing prereqs as human-readable string.
pub fn missing_prereqs(levels: &[u8], idx: usize) -> Option<String> {
    let missing: Vec<_> = UPGRADES.nodes[idx].prereqs.iter()
        .filter(|&&(pi, min_lv)| levels.get(pi).copied().unwrap_or(0) < min_lv)
        .map(|&(pi, min_lv)| format!("{} Lv{}", UPGRADES.nodes[pi].label, min_lv))
        .collect();
    if missing.is_empty() { None } else { Some(format!("Requires: {}", missing.join(", "))) }
}

/// Format cost for UI display (e.g. "10+10g").
pub fn format_upgrade_cost(idx: usize, level: u8) -> String {
    let node = &UPGRADES.nodes[idx];
    if node.custom_cost {
        let (fc, gc) = expansion_cost(level);
        return format!("{fc}+{gc}g");
    }
    let scale = upgrade_cost(level);
    node.cost.iter()
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
pub fn resolve_town_tower_stats(levels: &[u8]) -> TowerStats {
    let reg = &*UPGRADES;
    let cooldown_mult = reg.stat_mult(levels, "Town", UpgradeStatKind::FountainAttackSpeed);
    let proj_life_mult = reg.stat_mult(levels, "Town", UpgradeStatKind::FountainProjectileLife);
    let fountain_range_lv = reg.stat_level(levels, "Town", UpgradeStatKind::FountainRange) as f32;
    let radius_bonus = fountain_range_lv * 24.0;

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
    UPGRADES.nodes[idx].is_combat_stat
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
    let town = upgrades.town_levels(town_idx_usize);
    let reg = &*UPGRADES;

    // Use NpcDef.upgrade_category to look up all upgrades dynamically
    let cat = def.upgrade_category.unwrap_or("");
    let upgrade_hp = reg.stat_mult(&town, cat, UpgradeStatKind::Hp);
    let upgrade_dmg = reg.stat_mult(&town, cat, UpgradeStatKind::Attack);
    let upgrade_range = reg.stat_mult(&town, cat, UpgradeStatKind::Range);
    let upgrade_speed = reg.stat_mult(&town, cat, UpgradeStatKind::MoveSpeed);
    let cooldown_mult = reg.stat_mult(&town, cat, UpgradeStatKind::AttackSpeed);
    let upgrade_proj_speed = reg.stat_mult(&town, cat, UpgradeStatKind::ProjectileSpeed);
    let upgrade_proj_life = reg.stat_mult(&town, cat, UpgradeStatKind::ProjectileLifetime);

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
    let count = upgrade_count();
    for (town_idx, upgrade_idx) in queue.0.drain(..) {
        if upgrade_idx >= count { continue; }
        if town_idx >= upgrades.levels.len() { continue; }
        // Ensure upgrade vec is long enough
        upgrades.levels[town_idx].resize(count, 0);

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

        let node = &UPGRADES.nodes[upgrade_idx];

        // Invalidate healing zone cache on radius/rate upgrades
        if node.invalidates_healing {
            world_state.dirty.healing_zones = true;
        }

        if node.triggers_expansion {
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

    let count = upgrade_count();
    for (town_idx, flags) in auto.flags.iter().enumerate() {
        let levels = upgrades.town_levels(town_idx);
        let food = food_storage.food.get(town_idx).copied().unwrap_or(0);
        let gold = gold_storage.gold.get(town_idx).copied().unwrap_or(0);
        for i in 0..count.min(flags.len()) {
            if !flags[i] { continue; }
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
    dead_query: Query<(&NpcIndex, &Job, Option<&LastHitBy>), With<Dead>>,
    mut killer_query: Query<(&NpcIndex, &Job, &TownId, &BaseAttackType, &Personality, &mut Health, &mut CachedStats, &mut Speed, &Faction, &mut Activity, &Home, &mut CombatState), Without<Dead>>,
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
    for (_dead_idx, dead_job, last_hit) in dead_query.iter() {
        let Some(last_hit) = last_hit else { continue };
        if last_hit.0 < 0 { continue; }
        let killer_slot = last_hit.0 as usize;

        let Some(&killer_entity) = npc_map.0.get(&killer_slot) else { continue };
        let Ok((npc_idx, job, town_id, atk_type, personality, mut health, mut cached, mut speed, killer_faction, mut activity, home, mut combat_state)) = killer_query.get_mut(killer_entity) else { continue };

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

        // Loot: killer picks up loot from dead NPC and carries it home
        let drop = &npc_def(*dead_job).loot_drop;
        let amount = if drop.min == drop.max { drop.min } else {
            drop.min + (meta.xp as i32 % (drop.max - drop.min + 1)) // deterministic spread
        };
        if amount > 0 {
            // Disengage combat — loot delivery is highest priority
            *combat_state = CombatState::None;
            // Accumulate into existing Returning loot or create new
            if matches!(&*activity, Activity::Returning { .. }) {
                activity.add_loot(drop.item, amount);
            } else {
                *activity = Activity::Returning { loot: vec![(drop.item, amount)] };
            }
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));

            let item_name = match drop.item { ItemKind::Food => "food", ItemKind::Gold => "gold" };
            let killer_name = &npc_meta.0[idx].name;
            let killer_job = crate::job_name(npc_meta.0[idx].job);
            combat_log.push(CombatEventKind::Loot, killer_faction.0,
                game_time.day(), game_time.hour(), game_time.minute(),
                format!("{} '{}' looted {} {}", killer_job, killer_name, amount, item_name));
        }
    }
}
