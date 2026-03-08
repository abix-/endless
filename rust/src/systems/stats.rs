//! Stat resolution, upgrades, and XP systems.
//! Stage 8: CombatConfig + resolve_combat_stats + CachedStats.
//! Stage 9: UpgradeQueue + process_upgrades_system.

use crate::components::{BaseAttackType, CachedStats, Job, Personality};
use crate::constants::{
    AttackTypeStats, EffectDisplay, FOUNTAIN_TOWER, NPC_REGISTRY, ResourceKind, TOWER_STATS,
    TOWN_UPGRADES, TowerStats, UpgradeStatDef, UpgradeStatKind, npc_def,
};
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::resources::{NpcMetaCache, NpcsByTownCache};
use crate::systemparams::{EconomyState, WorldState};
use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::LazyLock;

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
            jobs.insert(
                def.job,
                JobStats {
                    max_health: def.base_hp,
                    damage: def.base_damage,
                    speed: def.base_speed,
                },
            );
        }

        let mut attacks = HashMap::new();
        attacks.insert(
            BaseAttackType::Melee,
            AttackTypeStats {
                range: 100.0,
                cooldown: 1.0,
                projectile_speed: 400.0,
                projectile_lifetime: 0.5,
            },
        );
        attacks.insert(
            BaseAttackType::Ranged,
            AttackTypeStats {
                range: 200.0,
                cooldown: 1.5,
                projectile_speed: 200.0,
                projectile_lifetime: 1.5,
            },
        );

        Self {
            jobs,
            attacks,
            heal_rate: 5.0,
            heal_radius: 300.0,
        }
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
    pub max_level: Option<u8>,
}

/// A branch in the upgrade tree UI.
pub struct UpgradeBranch {
    pub label: &'static str,
    pub section: &'static str,     // "Economy" or "Military"
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
    pub fn count(&self) -> usize {
        self.nodes.len()
    }

    /// Look up index for a (category, stat_kind) pair. Returns None if not found.
    pub fn index(&self, category: &str, kind: UpgradeStatKind) -> Option<usize> {
        self.index_map.get(&(category, kind)).copied().or_else(|| {
            // Fallback: linear scan for categories with non-static lifetime
            self.nodes
                .iter()
                .position(|n| n.category == category && n.stat_kind == kind)
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
                label: def.label,
                short: def.short,
                tooltip: def.tooltip,
                category,
                stat_kind: def.kind,
                pct: def.pct,
                cost: def.cost,
                display: def.display,
                prereqs: Vec::new(),
                is_combat_stat: def.is_combat_stat,
                invalidates_healing: def.invalidates_healing,
                triggers_expansion: def.triggers_expansion,
                custom_cost: def.custom_cost,
                max_level: def.max_level,
            });
        }

        // Second pass: resolve prereqs (stat kind → index within this category)
        for (i, def) in stat_defs.iter().enumerate() {
            if let Some(prereq_kind) = def.prereq_stat {
                if let Some(&prereq_idx) = index_map.get(&(category, prereq_kind)) {
                    nodes[branch_start + i]
                        .prereqs
                        .push((prereq_idx, def.prereq_level));
                }
            }
        }

        // Build tree depth for UI rendering
        let mut entries: Vec<(usize, u8)> = Vec::new();
        // Compute depth: root (no prereqs) = 0, child of root = 1, grandchild = 2
        fn compute_depth(nodes: &[UpgradeNode], idx: usize, visited: &mut Vec<usize>) -> u8 {
            if visited.contains(&idx) {
                return 0;
            }
            visited.push(idx);
            if nodes[idx].prereqs.is_empty() {
                return 0;
            }
            let max_parent = nodes[idx]
                .prereqs
                .iter()
                .map(|&(pi, _)| compute_depth(nodes, pi, visited))
                .max()
                .unwrap_or(0);
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
        fn emit_tree(
            idx: usize,
            depth: u8,
            nodes: &[UpgradeNode],
            all: &[(usize, u8)],
            entries: &mut Vec<(usize, u8)>,
            emitted: &mut Vec<usize>,
        ) {
            if emitted.contains(&idx) {
                return;
            }
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
        let section = NPC_REGISTRY
            .iter()
            .find(|n| n.upgrade_category == Some(category))
            .map(|n| if n.is_military { "Military" } else { "Economy" })
            .unwrap_or("Economy");
        branches.push(UpgradeBranch {
            label: category,
            section,
            entries,
        });
    }

    // Town upgrades (appended as final branch)
    {
        let branch_start = nodes.len();
        for def in TOWN_UPGRADES {
            let idx = nodes.len();
            index_map.insert(("Town", def.kind), idx);
            nodes.push(UpgradeNode {
                label: def.label,
                short: def.short,
                tooltip: def.tooltip,
                category: "Town",
                stat_kind: def.kind,
                pct: def.pct,
                cost: def.cost,
                display: def.display,
                prereqs: Vec::new(),
                is_combat_stat: def.is_combat_stat,
                invalidates_healing: def.invalidates_healing,
                triggers_expansion: def.triggers_expansion,
                custom_cost: def.custom_cost,
                max_level: def.max_level,
            });
        }
        // Resolve Town prereqs
        for (i, def) in TOWN_UPGRADES.iter().enumerate() {
            if let Some(prereq_kind) = def.prereq_stat {
                if let Some(&prereq_idx) = index_map.get(&("Town", prereq_kind)) {
                    nodes[branch_start + i]
                        .prereqs
                        .push((prereq_idx, def.prereq_level));
                }
            }
        }
        // Build Town tree entries
        let mut entries: Vec<(usize, u8)> = Vec::new();
        let mut by_depth: Vec<(usize, u8)> = Vec::new();
        for i in 0..TOWN_UPGRADES.len() {
            let idx = branch_start + i;
            let depth = if nodes[idx].prereqs.is_empty() {
                0
            } else {
                let max_p = nodes[idx]
                    .prereqs
                    .iter()
                    .map(|&(pi, _)| if nodes[pi].prereqs.is_empty() { 0u8 } else { 1 })
                    .max()
                    .unwrap_or(0);
                max_p + 1
            };
            by_depth.push((idx, depth));
        }
        fn emit_town_tree(
            idx: usize,
            depth: u8,
            nodes: &[UpgradeNode],
            all: &[(usize, u8)],
            entries: &mut Vec<(usize, u8)>,
            emitted: &mut Vec<usize>,
        ) {
            if emitted.contains(&idx) {
                return;
            }
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
        branches.push(UpgradeBranch {
            label: "Town",
            section: "Economy",
            entries,
        });
    }

    UpgradeRegistry {
        nodes,
        branches,
        index_map,
    }
}

/// Global upgrade registry, built once from NPC_REGISTRY + TOWN_UPGRADES.
pub static UPGRADES: LazyLock<UpgradeRegistry> = LazyLock::new(build_upgrade_registry);

/// Number of upgrade slots in the current registry.
pub fn upgrade_count() -> usize {
    UPGRADES.count()
}

/// Look up upgrade node by index.
pub fn upgrade_node(idx: usize) -> &'static UpgradeNode {
    &UPGRADES.nodes[idx]
}

/// True if this town has unlocked projectile dodge (any NPC category that has Dodge).
pub fn dodge_unlocked(levels: &[u8]) -> bool {
    // Check all categories that have a Dodge upgrade
    UPGRADES.nodes.iter().enumerate().any(|(i, n)| {
        n.stat_kind == UpgradeStatKind::Dodge && levels.get(i).copied().unwrap_or(0) > 0
    })
}

/// Sum of upgrade levels for all nodes in a given category.
pub fn branch_total(levels: &[u8], category: &str) -> u32 {
    UPGRADES
        .nodes
        .iter()
        .enumerate()
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
            let now = if level == 0 {
                "-".to_string()
            } else {
                format!("+{:.0}%", lv * pct * 100.0)
            };
            let next = format!("+{:.0}%", (lv + 1.0) * pct * 100.0);
            (now, next)
        }
        EffectDisplay::CooldownReduction => {
            let now = if level == 0 {
                "-".to_string()
            } else {
                let reduction = (1.0 - 1.0 / (1.0 + lv * pct)) * 100.0;
                format!("-{:.0}%", reduction)
            };
            let next_reduction = (1.0 - 1.0 / (1.0 + (lv + 1.0) * pct)) * 100.0;
            let next = format!("-{:.0}%", next_reduction);
            (now, next)
        }
        EffectDisplay::Unlock => {
            let now = if level == 0 {
                "Locked".to_string()
            } else {
                "Unlocked".to_string()
            };
            let next = if level == 0 {
                "Unlocks".to_string()
            } else {
                "Unlocked".to_string()
            };
            (now, next)
        }
        EffectDisplay::FlatPixels(px_per_level) => {
            let now = if level == 0 {
                "-".to_string()
            } else {
                format!("+{}px", level as i32 * px_per_level)
            };
            let next = format!("+{}px", (level as i32 + 1) * px_per_level);
            (now, next)
        }
        EffectDisplay::Discrete => {
            let now = if level == 0 {
                "-".to_string()
            } else {
                format!("+{}", level)
            };
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
        self.levels
            .get(town_idx)
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
        Self {
            levels: vec![vec![0; count]; 16],
        }
    }
}

/// Upgrade purchase request message. Replaces the old UpgradeQueue resource.
/// Writers: UI left_panel, auto_upgrade_system, ai_player. Reader: process_upgrades_system.
#[derive(Message, Clone)]
pub struct UpgradeMsg {
    pub town_idx: usize,
    pub upgrade_idx: usize,
}

/// Equip an item from TownInventory onto an NPC.
/// Writers: Inventory UI. Reader: process_equip_system.
#[derive(Message, Clone)]
pub struct EquipItemMsg {
    pub npc_entity: Entity,
    pub item_id: u64,
    pub town_idx: usize,
}

/// Unequip an item from an NPC back to TownInventory.
/// Writers: Inventory UI. Reader: process_equip_system.
#[derive(Message, Clone)]
pub struct UnequipItemMsg {
    pub npc_entity: Entity,
    pub slot: crate::constants::EquipmentSlot,
    pub ring_index: u8, // 0=ring1, 1=ring2 (ignored for non-Ring)
}

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
    if xp <= 0 {
        return 0;
    }
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
    UPGRADES.nodes[idx]
        .prereqs
        .iter()
        .all(|&(pi, min_lv)| levels.get(pi).copied().unwrap_or(0) >= min_lv)
}

/// Full purchasability check: prereqs met AND can afford all costs AND below max level.
/// Single gate used by process_upgrades, auto_upgrade, AI, and UI.
pub fn upgrade_available(levels: &[u8], idx: usize, food: i32, gold: i32) -> bool {
    let lv = levels.get(idx).copied().unwrap_or(0);
    let node = &UPGRADES.nodes[idx];
    if let Some(max) = node.max_level {
        if lv >= max {
            return false;
        }
    }
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
    let missing: Vec<_> = UPGRADES.nodes[idx]
        .prereqs
        .iter()
        .filter(|&&(pi, min_lv)| levels.get(pi).copied().unwrap_or(0) < min_lv)
        .map(|&(pi, min_lv)| format!("{} Lv{}", UPGRADES.nodes[pi].label, min_lv))
        .collect();
    if missing.is_empty() {
        None
    } else {
        Some(format!("Requires: {}", missing.join(", ")))
    }
}

/// Format cost for UI display (e.g. "10+10g").
pub fn format_upgrade_cost(idx: usize, level: u8) -> String {
    let node = &UPGRADES.nodes[idx];
    if node.custom_cost {
        let (fc, gc) = expansion_cost(level);
        let mut parts = Vec::new();
        if fc > 0 { parts.push(format!("{fc} food")); }
        if gc > 0 { parts.push(format!("{gc} gold")); }
        return parts.join(", ");
    }
    let scale = upgrade_cost(level);
    node.cost
        .iter()
        .map(|&(kind, base)| {
            let total = base * scale;
            match kind {
                ResourceKind::Food => format!("{total} food"),
                ResourceKind::Gold => format!("{total} gold"),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
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
        hp_regen: FOUNTAIN_TOWER.hp_regen,
        max_hp: FOUNTAIN_TOWER.max_hp,
    }
}

/// Resolve per-tower-instance stats from base + XP level + per-instance upgrade levels.
pub fn resolve_tower_instance_stats(level: i32, upgrade_levels: &[u8]) -> TowerStats {
    let level_mult = 1.0 + level as f32 * 0.01;
    let upgrades = crate::constants::TOWER_UPGRADES;
    let get = |kind: UpgradeStatKind| -> f32 {
        for (i, def) in upgrades.iter().enumerate() {
            if def.kind == kind {
                let lv = upgrade_levels.get(i).copied().unwrap_or(0) as f32;
                if def.display == EffectDisplay::CooldownReduction {
                    return 1.0 / (1.0 + lv * def.pct);
                }
                return 1.0 + lv * def.pct;
            }
        }
        1.0
    };
    let regen_level = upgrades.iter().enumerate()
        .find(|(_, d)| d.kind == UpgradeStatKind::HpRegen)
        .map(|(i, _)| upgrade_levels.get(i).copied().unwrap_or(0) as f32)
        .unwrap_or(0.0);

    TowerStats {
        range: TOWER_STATS.range * get(UpgradeStatKind::Range) * level_mult,
        damage: TOWER_STATS.damage * get(UpgradeStatKind::Attack) * level_mult,
        cooldown: TOWER_STATS.cooldown * get(UpgradeStatKind::AttackSpeed),
        proj_speed: TOWER_STATS.proj_speed * get(UpgradeStatKind::ProjectileSpeed),
        proj_lifetime: TOWER_STATS.proj_lifetime * get(UpgradeStatKind::ProjectileLifetime),
        hp_regen: regen_level * 2.0,
        max_hp: TOWER_STATS.max_hp * get(UpgradeStatKind::Hp) * level_mult,
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
    weapon_bonus: f32,
    armor_bonus: f32,
) -> CachedStats {
    let job_base = config.jobs.get(&job).expect("missing job stats");
    let def = npc_def(job);
    let default_atk = config
        .attacks
        .get(&attack_type)
        .expect("missing attack type stats");
    let atk_base = def.attack_override.as_ref().unwrap_or(default_atk);
    let trait_mods = personality.get_stat_mods();
    let level_mult = 1.0 + level as f32 * 0.01;

    let town_idx_usize = if town_idx >= 0 {
        town_idx as usize
    } else {
        usize::MAX
    };
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
    let stamina_mult = reg.stat_mult(&town, cat, UpgradeStatKind::Stamina);
    let hp_regen_level = reg.stat_level(&town, cat, UpgradeStatKind::HpRegen) as f32;

    CachedStats {
        damage: job_base.damage * upgrade_dmg * trait_mods.damage * level_mult * (1.0 + weapon_bonus),
        range: atk_base.range * upgrade_range * trait_mods.range,
        cooldown: atk_base.cooldown * cooldown_mult * trait_mods.cooldown,
        projectile_speed: atk_base.projectile_speed * upgrade_proj_speed,
        projectile_lifetime: atk_base.projectile_lifetime * upgrade_proj_life,
        max_health: job_base.max_health * upgrade_hp * trait_mods.hp * level_mult * (1.0 + armor_bonus),
        speed: job_base.speed * upgrade_speed * trait_mods.speed,
        stamina: stamina_mult,
        hp_regen: hp_regen_level * 0.5,
        berserk_bonus: trait_mods.berserk_bonus,
    }
}

/// Re-resolve NPC stats after equipment/upgrade change. Updates CachedStats, Speed, Health (proportional), GPU.
pub fn re_resolve_npc_stats(
    entity: Entity,
    slot: usize,
    equipment: &crate::components::NpcEquipment,
    job: Job,
    attack_type: BaseAttackType,
    town_idx: i32,
    level: i32,
    personality: &Personality,
    config: &CombatConfig,
    upgrades: &TownUpgrades,
    cached_stats_q: &mut Query<&mut CachedStats>,
    speed_q: &mut Query<&mut crate::components::Speed>,
    health_q: &mut Query<&mut crate::components::Health, Without<crate::components::Building>>,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) {
    let old_max = cached_stats_q
        .get(entity)
        .map(|s| s.max_health)
        .unwrap_or(100.0);
    let new_cached = resolve_combat_stats(
        job, attack_type, town_idx, level, personality, config, upgrades,
        equipment.total_weapon_bonus(), equipment.total_armor_bonus(),
    );
    let new_speed = new_cached.speed;
    let new_max = new_cached.max_health;
    if let Ok(mut cs) = cached_stats_q.get_mut(entity) {
        *cs = new_cached;
    }
    if let Ok(mut spd) = speed_q.get_mut(entity) {
        spd.0 = new_speed;
    }
    if old_max > 0.0 && (new_max - old_max).abs() > 0.01 {
        if let Ok(mut hp) = health_q.get_mut(entity) {
            hp.0 = hp.0 * new_max / old_max;
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: slot, health: hp.0 }));
        }
    }
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetMaxHealth { idx: slot, max_health: new_max }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx: slot, speed: new_speed }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::MarkVisualDirty { idx: slot }));
}

// ============================================================================
// PROCESS UPGRADES SYSTEM
// ============================================================================

/// Drains UpgradeMsg messages, applies upgrades, re-resolves affected NPC stats.
pub fn process_upgrades_system(
    mut queue: MessageReader<UpgradeMsg>,
    mut upgrades: ResMut<TownUpgrades>,
    mut economy: EconomyState,
    npcs_by_town: Res<NpcsByTownCache>,
    config: Res<CombatConfig>,
    meta_cache: Res<NpcMetaCache>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut world_state: WorldState,
    mut cached_stats_q: Query<&mut crate::components::CachedStats>,
    mut speed_q: Query<&mut crate::components::Speed>,
    mut health_q: Query<&mut crate::components::Health, Without<crate::components::Building>>,
    attack_type_q: Query<&crate::components::BaseAttackType>,
    personality_q: Query<&crate::components::Personality>,
    equipment_q: Query<&crate::components::NpcEquipment>,
) {
    let count = upgrade_count();
    for msg in queue.read() {
        let (town_idx, upgrade_idx) = (msg.town_idx, msg.upgrade_idx);
        if upgrade_idx >= count {
            continue;
        }
        if town_idx >= upgrades.levels.len() {
            continue;
        }
        // Ensure upgrade vec is long enough
        upgrades.levels[town_idx].resize(count, 0);

        // Prereq + affordability gate
        let levels = upgrades.town_levels(town_idx);
        let mut food = economy
            .food_storage
            .food
            .get(town_idx)
            .copied()
            .unwrap_or(0);
        let mut gold = economy
            .gold_storage
            .gold
            .get(town_idx)
            .copied()
            .unwrap_or(0);
        if !upgrade_available(&levels, upgrade_idx, food, gold) {
            continue;
        }

        // Deduct cost and increment level
        let level = levels[upgrade_idx];
        deduct_upgrade_cost(upgrade_idx, level, &mut food, &mut gold);
        if let Some(f) = economy.food_storage.food.get_mut(town_idx) {
            *f = food;
        }
        if let Some(g) = economy.gold_storage.gold.get_mut(town_idx) {
            *g = gold;
        }
        upgrades.levels[town_idx][upgrade_idx] = level.saturating_add(1);

        let node = &UPGRADES.nodes[upgrade_idx];

        // Invalidate healing zone cache on radius/rate upgrades
        if node.invalidates_healing {
            world_state
                .dirty_writers
                .healing_zones
                .write(crate::messages::HealingZonesDirtyMsg);
        }

        if node.triggers_expansion {
            let _ = crate::world::expand_town_build_area(
                &mut world_state.grid,
                &mut world_state.world_data.towns,
                &world_state.entity_map,
                town_idx,
            );
            world_state
                .dirty_writers
                .terrain
                .write(crate::messages::TerrainDirtyMsg);
            continue;
        }

        // Re-resolve NPC stats if this is a combat-affecting upgrade
        if !is_combat_upgrade(upgrade_idx) {
            continue;
        }

        let Some(npc_slots) = npcs_by_town.0.get(town_idx) else {
            continue;
        };
        let slots: Vec<usize> = npc_slots.clone();
        for slot in slots {
            let Some(npc) = world_state.entity_map.get_npc(slot) else {
                continue;
            };
            let entity = npc.entity;

            let npc_level = meta_cache.0[slot].level;
            let old_max = cached_stats_q
                .get(entity)
                .map(|s| s.max_health)
                .unwrap_or(100.0);
            let pers = personality_q.get(entity).cloned().unwrap_or_default();
            let atk_type = attack_type_q
                .get(entity)
                .copied()
                .unwrap_or(crate::components::BaseAttackType::Melee);
            let (wb, ab) = equipment_q.get(entity).map(|eq| {
                (eq.total_weapon_bonus(), eq.total_armor_bonus())
            }).unwrap_or((0.0, 0.0));
            let new_cached = resolve_combat_stats(
                npc.job,
                atk_type,
                town_idx as i32,
                npc_level,
                &pers,
                &config,
                &upgrades,
                wb,
                ab,
            );
            let new_speed = new_cached.speed;
            let new_max = new_cached.max_health;
            if let Ok(mut cs) = cached_stats_q.get_mut(entity) {
                *cs = new_cached;
            }
            if let Ok(mut spd) = speed_q.get_mut(entity) {
                spd.0 = new_speed;
            }

            // Rescale HP proportionally
            if old_max > 0.0 && (new_max - old_max).abs() > 0.01 {
                if let Ok(mut hp) = health_q.get_mut(entity) {
                    hp.0 = hp.0 * new_max / old_max;
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetMaxHealth {
                        idx: slot,
                        max_health: new_max,
                    }));
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth {
                        idx: slot,
                        health: hp.0,
                    }));
                }
            }

            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed {
                idx: slot,
                speed: new_speed,
            }));
        }
    }
}

// ============================================================================
// EQUIP / UNEQUIP SYSTEM
// ============================================================================

/// Processes equip/unequip messages — moves items between TownInventory and NpcEquipment.
pub fn process_equip_system(
    mut equip_msgs: MessageReader<EquipItemMsg>,
    mut unequip_msgs: MessageReader<UnequipItemMsg>,
    mut equipment_q: Query<(
        &mut crate::components::NpcEquipment,
        &crate::components::GpuSlot,
        &Job,
        &crate::components::TownId,
        &BaseAttackType,
        &Personality,
    )>,
    mut cached_stats_q: Query<&mut CachedStats>,
    mut speed_q: Query<&mut crate::components::Speed>,
    mut health_q: Query<&mut crate::components::Health, Without<crate::components::Building>>,
    mut town_inventory: ResMut<crate::resources::TownInventory>,
    config: Res<CombatConfig>,
    upgrades: Res<TownUpgrades>,
    meta_cache: Res<NpcMetaCache>,
    entity_map: Res<crate::resources::EntityMap>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
) {
    // Equip: TownInventory → NpcEquipment
    for msg in equip_msgs.read() {
        let Some(item) = town_inventory.remove(msg.town_idx, msg.item_id) else {
            continue;
        };
        let Ok((mut eq, gpu_slot, job, town_id, atk_type, pers)) = equipment_q.get_mut(msg.npc_entity) else {
            // NPC gone — put item back
            town_inventory.add(msg.town_idx, item);
            continue;
        };
        let slot_idx = gpu_slot.0;

        // Determine target field. Ring special case: prefer empty ring1, else ring2.
        use crate::constants::EquipmentSlot;
        let target: &mut Option<crate::constants::LootItem> = match item.slot {
            EquipmentSlot::Ring => {
                if eq.ring1.is_none() { &mut eq.ring1 } else { &mut eq.ring2 }
            }
            _ => eq.slot_mut(item.slot),
        };

        // Swap out old item if present
        if let Some(old) = target.take() {
            town_inventory.add(msg.town_idx, old);
        }
        *target = Some(item);

        // Re-resolve stats
        let level = entity_map.get_npc(slot_idx).map(|n| meta_cache.0[n.slot].level).unwrap_or(0);
        re_resolve_npc_stats(
            msg.npc_entity, slot_idx, &eq, *job, *atk_type,
            town_id.0, level, pers, &config, &upgrades,
            &mut cached_stats_q, &mut speed_q, &mut health_q, &mut gpu_updates,
        );
    }

    // Unequip: NpcEquipment → TownInventory
    for msg in unequip_msgs.read() {
        let Ok((mut eq, gpu_slot, job, town_id, atk_type, pers)) = equipment_q.get_mut(msg.npc_entity) else {
            continue;
        };
        let slot_idx = gpu_slot.0;

        use crate::constants::EquipmentSlot;
        let source: &mut Option<crate::constants::LootItem> = match msg.slot {
            EquipmentSlot::Ring => {
                if msg.ring_index == 1 { &mut eq.ring2 } else { &mut eq.ring1 }
            }
            _ => eq.slot_mut(msg.slot),
        };

        let Some(item) = source.take() else {
            continue; // slot was empty
        };
        town_inventory.add(town_id.0 as usize, item);

        let level = entity_map.get_npc(slot_idx).map(|n| meta_cache.0[n.slot].level).unwrap_or(0);
        re_resolve_npc_stats(
            msg.npc_entity, slot_idx, &eq, *job, *atk_type,
            town_id.0, level, pers, &config, &upgrades,
            &mut cached_stats_q, &mut speed_q, &mut health_q, &mut gpu_updates,
        );
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
    mut queue: MessageWriter<UpgradeMsg>,
) {
    if !game_time.hour_ticked {
        return;
    }

    let count = upgrade_count();
    for (town_idx, flags) in auto.flags.iter().enumerate() {
        let levels = upgrades.town_levels(town_idx);
        let food = food_storage.food.get(town_idx).copied().unwrap_or(0);
        let gold = gold_storage.gold.get(town_idx).copied().unwrap_or(0);
        for i in 0..count.min(flags.len()) {
            if !flags[i] {
                continue;
            }
            if upgrade_available(&levels, i, food, gold) {
                queue.write(UpgradeMsg {
                    town_idx,
                    upgrade_idx: i,
                });
            }
        }
    }
}

/// Auto-buy cheapest tower upgrade each game-hour for towers with auto_upgrade enabled.
pub fn auto_tower_upgrade_system(
    game_time: Res<crate::resources::GameTime>,
    mut entity_map: ResMut<crate::resources::EntityMap>,
    mut food_storage: ResMut<crate::resources::FoodStorage>,
    mut gold_storage: ResMut<crate::resources::GoldStorage>,
) {
    if !game_time.hour_ticked {
        return;
    }
    let tower_upgrades = crate::constants::TOWER_UPGRADES;
    // Collect tower data to avoid borrow conflict on entity_map
    let towers: Vec<(usize, u32, Vec<u8>, Vec<bool>)> = entity_map
        .iter_kind(crate::world::BuildingKind::Tower)
        .filter(|i| i.auto_upgrade_flags.iter().any(|&f| f))
        .map(|i| (i.slot, i.town_idx, i.upgrade_levels.clone(), i.auto_upgrade_flags.clone()))
        .collect();

    for (slot, town_idx, upgrade_levels, auto_flags) in towers {
        let ti = town_idx as usize;
        let food = food_storage.food.get(ti).copied().unwrap_or(0);
        let gold = gold_storage.gold.get(ti).copied().unwrap_or(0);

        // Find cheapest affordable upgrade among auto-flagged stats
        let mut best: Option<(i32, usize)> = None; // (total_cost, index)
        for (i, upg) in tower_upgrades.iter().enumerate() {
            if !auto_flags.get(i).copied().unwrap_or(false) {
                continue;
            }
            let lv = upgrade_levels.get(i).copied().unwrap_or(0);
            let cost_mult = upgrade_cost(lv);
            let can_afford = upg.cost.iter().all(|(res, base)| {
                let total = base * cost_mult;
                match res {
                    crate::constants::ResourceKind::Food => food >= total,
                    crate::constants::ResourceKind::Gold => gold >= total,
                }
            });
            if can_afford {
                let total: i32 = upg.cost.iter().map(|(_, base)| base * cost_mult).sum();
                if best.map_or(true, |b| total < b.0) {
                    best = Some((total, i));
                }
            }
        }

        if let Some((_, idx)) = best {
            let upg = &tower_upgrades[idx];
            let lv = upgrade_levels.get(idx).copied().unwrap_or(0);
            let cost_mult = upgrade_cost(lv);
            // Deduct resources
            for (res, base) in upg.cost {
                let total = base * cost_mult;
                match res {
                    crate::constants::ResourceKind::Food => {
                        if let Some(f) = food_storage.food.get_mut(ti) { *f -= total; }
                    }
                    crate::constants::ResourceKind::Gold => {
                        if let Some(g) = gold_storage.gold.get_mut(ti) { *g -= total; }
                    }
                }
            }
            // Increment upgrade level
            if let Some(inst) = entity_map.get_instance_mut(slot) {
                while inst.upgrade_levels.len() <= idx {
                    inst.upgrade_levels.push(0);
                }
                inst.upgrade_levels[idx] += 1;
            }
        }
    }
}

// ============================================================================
// AUTO-EQUIP SYSTEM
// ============================================================================

/// Once per game hour, auto-equip items from TownInventory onto NPCs.
/// Picks the NPC with the lowest bonus in the matching slot (or empty slot first).
pub fn auto_equip_system(
    game_time: Res<crate::resources::GameTime>,
    town_inventory: Res<crate::resources::TownInventory>,
    equipment_q: Query<
        (Entity, &crate::components::NpcEquipment, &Job, &crate::components::TownId),
        (Without<crate::components::Building>, Without<crate::components::Dead>),
    >,
    mut equip_writer: MessageWriter<EquipItemMsg>,
) {
    if !game_time.hour_ticked {
        return;
    }

    for (town_idx, items) in town_inventory.items.iter().enumerate() {
        if items.is_empty() {
            continue;
        }

        // Collect military NPCs in this town
        let town_npcs: Vec<_> = equipment_q
            .iter()
            .filter(|(_, _, _, tid)| tid.0 == town_idx as i32)
            .collect();

        // Track items we've already queued for equip this cycle (avoid double-assign)
        let mut assigned_items: Vec<u64> = Vec::new();

        for item in items {
            if assigned_items.contains(&item.id) {
                continue;
            }

            let slot = item.slot;

            // Find best NPC candidate: empty slot first, then biggest upgrade margin
            let mut best: Option<(Entity, f32)> = None; // (entity, current_bonus)

            for &(entity, equip, job, _) in &town_npcs {
                let def = crate::constants::npc_def(*job);
                if !def.equip_slots.contains(&slot) {
                    continue;
                }

                // Get current bonus in this slot
                use crate::constants::EquipmentSlot;
                let current_bonus = match slot {
                    EquipmentSlot::Ring => {
                        // For rings, check both slots — use the lower one
                        let b1 = equip.ring1.as_ref().map(|i| i.stat_bonus).unwrap_or(0.0);
                        let b2 = equip.ring2.as_ref().map(|i| i.stat_bonus).unwrap_or(0.0);
                        b1.min(b2)
                    }
                    _ => equip.slot(slot).as_ref().map(|i| i.stat_bonus).unwrap_or(0.0),
                };

                // Must be a strict upgrade
                if item.stat_bonus <= current_bonus {
                    continue;
                }

                // Prefer NPC with lowest current bonus (distribute gear evenly)
                if best.map_or(true, |b| current_bonus < b.1) {
                    best = Some((entity, current_bonus));
                }
            }

            if let Some((entity, _)) = best {
                equip_writer.write(EquipItemMsg {
                    npc_entity: entity,
                    item_id: item.id,
                    town_idx,
                });
                assigned_items.push(item.id);
            }
        }
    }
}

// ============================================================================
// XP grant + NPC kill loot logic moved to unified death_system (health.rs)

// ============================================================================
// UNIT TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use crate::components::{BaseAttackType, Job, Personality, TraitKind, TraitInstance};

    // -- level_from_xp -------------------------------------------------------

    #[test]
    fn level_from_xp_zero() {
        assert_eq!(level_from_xp(0), 0);
    }

    #[test]
    fn level_from_xp_negative() {
        assert_eq!(level_from_xp(-100), 0);
    }

    #[test]
    fn level_from_xp_just_below_threshold() {
        // level 1 = sqrt(100/100) = 1 → need xp=100 for level 1
        assert_eq!(level_from_xp(99), 0);
    }

    #[test]
    fn level_from_xp_at_threshold() {
        assert_eq!(level_from_xp(100), 1);
    }

    #[test]
    fn level_from_xp_level_2() {
        // level 2 = sqrt(400/100) = 2
        assert_eq!(level_from_xp(400), 2);
    }

    #[test]
    fn level_from_xp_between_levels() {
        assert_eq!(level_from_xp(300), 1); // sqrt(3) = 1.73 → floor = 1
    }

    // -- upgrade_cost --------------------------------------------------------

    #[test]
    fn upgrade_cost_level_0() {
        assert_eq!(upgrade_cost(0), 10);
    }

    #[test]
    fn upgrade_cost_level_1() {
        assert_eq!(upgrade_cost(1), 20);
    }

    #[test]
    fn upgrade_cost_level_2() {
        assert_eq!(upgrade_cost(2), 40);
    }

    #[test]
    fn upgrade_cost_doubles_each_level() {
        for lv in 0..20 {
            assert_eq!(upgrade_cost(lv), 10 * (1 << lv as i32));
        }
    }

    #[test]
    fn upgrade_cost_caps_at_20() {
        // levels above 20 should be clamped
        assert_eq!(upgrade_cost(21), upgrade_cost(20));
        assert_eq!(upgrade_cost(255), upgrade_cost(20));
    }

    // -- expansion_cost ------------------------------------------------------

    #[test]
    fn expansion_cost_level_0() {
        assert_eq!(expansion_cost(0), (24, 24));
    }

    #[test]
    fn expansion_cost_level_1() {
        assert_eq!(expansion_cost(1), (32, 32));
    }

    #[test]
    fn expansion_cost_scales_linearly() {
        let (f, g) = expansion_cost(5);
        assert_eq!(f, 24 + 8 * 5);
        assert_eq!(f, g);
    }

    // -- decode_upgrade_levels -----------------------------------------------

    #[test]
    fn decode_upgrade_levels_pads_short_input() {
        let result = decode_upgrade_levels(&[1, 2]);
        assert_eq!(result.len(), upgrade_count());
        assert_eq!(result[0], 1);
        assert_eq!(result[1], 2);
        // rest should be 0
        assert!(result[2..].iter().all(|&v| v == 0));
    }

    #[test]
    fn decode_upgrade_levels_empty() {
        let result = decode_upgrade_levels(&[]);
        assert_eq!(result.len(), upgrade_count());
        assert!(result.iter().all(|&v| v == 0));
    }

    // -- upgrade_unlocked / upgrade_available --------------------------------

    #[test]
    fn upgrade_unlocked_no_prereqs() {
        // First upgrade in each branch typically has no prereqs
        let levels = vec![0u8; upgrade_count()];
        // Index 0 should have no prereqs (it's the first node)
        let node = &UPGRADES.nodes[0];
        if node.prereqs.is_empty() {
            assert!(upgrade_unlocked(&levels, 0));
        }
    }

    #[test]
    fn upgrade_unlocked_with_unmet_prereqs() {
        let levels = vec![0u8; upgrade_count()];
        // Find a node that has prereqs
        for (idx, node) in UPGRADES.nodes.iter().enumerate() {
            if !node.prereqs.is_empty() {
                assert!(!upgrade_unlocked(&levels, idx), "node {idx} should be locked with all-zero levels");
                break;
            }
        }
    }

    #[test]
    fn upgrade_unlocked_with_met_prereqs() {
        let mut levels = vec![0u8; upgrade_count()];
        // Find a node with prereqs and satisfy them
        for (idx, node) in UPGRADES.nodes.iter().enumerate() {
            if !node.prereqs.is_empty() {
                for &(pi, min_lv) in &node.prereqs {
                    levels[pi] = min_lv;
                }
                assert!(upgrade_unlocked(&levels, idx), "node {idx} should be unlocked after satisfying prereqs");
                break;
            }
        }
    }

    #[test]
    fn upgrade_available_needs_resources() {
        let mut levels = vec![0u8; upgrade_count()];
        // Find first node with no prereqs
        let idx = UPGRADES.nodes.iter().position(|n| n.prereqs.is_empty()).unwrap();
        // Ensure prereqs met but no resources
        for &(pi, min_lv) in &UPGRADES.nodes[idx].prereqs {
            levels[pi] = min_lv;
        }
        assert!(!upgrade_available(&levels, idx, 0, 0));
        // With abundant resources
        assert!(upgrade_available(&levels, idx, 100_000, 100_000));
    }

    // -- deduct_upgrade_cost -------------------------------------------------

    #[test]
    fn deduct_upgrade_cost_decrements() {
        let idx = UPGRADES.nodes.iter().position(|n| n.prereqs.is_empty()).unwrap();
        let mut food = 100_000;
        let mut gold = 100_000;
        let food_before = food;
        let gold_before = gold;
        deduct_upgrade_cost(idx, 0, &mut food, &mut gold);
        assert!(food <= food_before, "food should decrease or stay same");
        assert!(gold <= gold_before, "gold should decrease or stay same");
        assert!(food < food_before || gold < gold_before, "at least one resource should decrease");
    }

    // -- format_upgrade_cost -------------------------------------------------

    #[test]
    fn format_upgrade_cost_contains_resource_label() {
        let idx = 0;
        let s = format_upgrade_cost(idx, 0);
        assert!(s.contains("food") || s.contains("gold"), "cost string should mention resource: {s}");
    }

    // -- missing_prereqs -----------------------------------------------------

    #[test]
    fn missing_prereqs_none_when_satisfied() {
        let idx = UPGRADES.nodes.iter().position(|n| n.prereqs.is_empty()).unwrap();
        let levels = vec![0u8; upgrade_count()];
        assert!(missing_prereqs(&levels, idx).is_none());
    }

    #[test]
    fn missing_prereqs_returns_string_when_unsatisfied() {
        let levels = vec![0u8; upgrade_count()];
        for (idx, node) in UPGRADES.nodes.iter().enumerate() {
            if !node.prereqs.is_empty() {
                let msg = missing_prereqs(&levels, idx);
                assert!(msg.is_some(), "should have missing prereqs for node {idx}");
                assert!(msg.unwrap().contains("Requires:"));
                break;
            }
        }
    }

    // -- resolve_combat_stats ------------------------------------------------

    fn default_config() -> CombatConfig {
        CombatConfig::default()
    }

    fn empty_upgrades() -> TownUpgrades {
        TownUpgrades { levels: vec![vec![0u8; upgrade_count()]] }
    }

    #[test]
    fn resolve_combat_stats_archer_defaults() {
        let config = default_config();
        let upgrades = empty_upgrades();
        let personality = Personality::default();
        let stats = resolve_combat_stats(
            Job::Archer, BaseAttackType::Ranged, 0, 0,
            &personality, &config, &upgrades, 0.0, 0.0,
        );
        let job_stats = config.jobs.get(&Job::Archer).unwrap();
        // With no upgrades, no level, no equipment, no traits:
        // damage = base_damage * 1.0 * 1.0 * 1.0 * 1.0
        assert!((stats.damage - job_stats.damage).abs() < 0.01, "damage: {} vs {}", stats.damage, job_stats.damage);
        assert!((stats.max_health - job_stats.max_health).abs() < 0.01, "hp: {} vs {}", stats.max_health, job_stats.max_health);
        assert!((stats.speed - job_stats.speed).abs() < 0.01, "speed: {} vs {}", stats.speed, job_stats.speed);
        assert_eq!(stats.berserk_bonus, 0.0);
    }

    #[test]
    fn resolve_combat_stats_level_scaling() {
        let config = default_config();
        let upgrades = empty_upgrades();
        let personality = Personality::default();
        let stats_lv0 = resolve_combat_stats(
            Job::Archer, BaseAttackType::Ranged, 0, 0,
            &personality, &config, &upgrades, 0.0, 0.0,
        );
        let stats_lv10 = resolve_combat_stats(
            Job::Archer, BaseAttackType::Ranged, 0, 10,
            &personality, &config, &upgrades, 0.0, 0.0,
        );
        // level 10 = 1.10x multiplier on damage and hp
        assert!(stats_lv10.damage > stats_lv0.damage);
        let expected_ratio = 1.10;
        let actual_ratio = stats_lv10.damage / stats_lv0.damage;
        assert!((actual_ratio - expected_ratio).abs() < 0.01, "ratio: {actual_ratio}");
    }

    #[test]
    fn resolve_combat_stats_equipment_bonus() {
        let config = default_config();
        let upgrades = empty_upgrades();
        let personality = Personality::default();
        let base = resolve_combat_stats(
            Job::Archer, BaseAttackType::Ranged, 0, 0,
            &personality, &config, &upgrades, 0.0, 0.0,
        );
        let with_weapon = resolve_combat_stats(
            Job::Archer, BaseAttackType::Ranged, 0, 0,
            &personality, &config, &upgrades, 0.5, 0.0,
        );
        let with_armor = resolve_combat_stats(
            Job::Archer, BaseAttackType::Ranged, 0, 0,
            &personality, &config, &upgrades, 0.0, 0.5,
        );
        // 50% weapon bonus → 1.5x damage
        assert!((with_weapon.damage / base.damage - 1.5).abs() < 0.01);
        // 50% armor bonus → 1.5x max_health
        assert!((with_armor.max_health / base.max_health - 1.5).abs() < 0.01);
    }

    #[test]
    fn resolve_combat_stats_berserk_from_ferocity() {
        let config = default_config();
        let upgrades = empty_upgrades();
        let personality = Personality {
            trait1: Some(TraitInstance { kind: TraitKind::Ferocity, magnitude: 1.0 }),
            trait2: None,
        };
        let stats = resolve_combat_stats(
            Job::Archer, BaseAttackType::Ranged, 0, 0,
            &personality, &config, &upgrades, 0.0, 0.0,
        );
        // Ferocity m=1.0 → berserk_bonus = 0.50 * 1.0 = 0.50
        assert!((stats.berserk_bonus - 0.5).abs() < 0.01, "berserk: {}", stats.berserk_bonus);
    }

    #[test]
    fn resolve_combat_stats_timid_negative_berserk() {
        let config = default_config();
        let upgrades = empty_upgrades();
        let personality = Personality {
            trait1: Some(TraitInstance { kind: TraitKind::Ferocity, magnitude: -1.0 }),
            trait2: None,
        };
        let stats = resolve_combat_stats(
            Job::Archer, BaseAttackType::Ranged, 0, 0,
            &personality, &config, &upgrades, 0.0, 0.0,
        );
        assert!(stats.berserk_bonus < 0.0, "timid should have negative berserk: {}", stats.berserk_bonus);
    }

    // -- resolve_tower_instance_stats ----------------------------------------

    #[test]
    fn resolve_tower_instance_stats_level_0_defaults() {
        let stats = resolve_tower_instance_stats(0, &[]);
        assert!((stats.range - TOWER_STATS.range).abs() < 0.01);
        assert!((stats.damage - TOWER_STATS.damage).abs() < 0.01);
        assert!((stats.cooldown - TOWER_STATS.cooldown).abs() < 0.01);
    }

    #[test]
    fn resolve_tower_instance_stats_level_scales() {
        let stats_lv0 = resolve_tower_instance_stats(0, &[]);
        let stats_lv10 = resolve_tower_instance_stats(10, &[]);
        assert!(stats_lv10.damage > stats_lv0.damage);
        assert!(stats_lv10.range > stats_lv0.range);
    }

    // -- UpgradeRegistry::stat_mult ------------------------------------------

    #[test]
    fn stat_mult_zero_levels_returns_1() {
        let levels = vec![0u8; upgrade_count()];
        let mult = UPGRADES.stat_mult(&levels, "Military (Ranged)", UpgradeStatKind::Attack);
        assert!((mult - 1.0).abs() < 0.001, "zero upgrade should give 1.0x mult, got {mult}");
    }

    // -- auto_upgrade_system -------------------------------------------------

    #[derive(Resource, Default)]
    struct CollectedUpgrades(Vec<(usize, usize)>); // (town_idx, upgrade_idx)

    fn collect_upgrades(
        mut reader: MessageReader<UpgradeMsg>,
        mut collected: ResMut<CollectedUpgrades>,
    ) {
        for msg in reader.read() {
            collected.0.push((msg.town_idx, msg.upgrade_idx));
        }
    }

    fn setup_auto_upgrade_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(crate::resources::GameTime::default());
        app.insert_resource(TownUpgrades::default());
        app.insert_resource(crate::resources::AutoUpgrade::default());
        app.insert_resource(crate::resources::FoodStorage { food: vec![0] });
        app.insert_resource(crate::resources::GoldStorage { gold: vec![0] });
        app.insert_resource(CollectedUpgrades::default());
        app.add_message::<UpgradeMsg>();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_systems(FixedUpdate, (auto_upgrade_system, collect_upgrades).chain());
        app.update();
        app.update();
        app
    }

    #[test]
    fn auto_upgrade_skips_without_hour_tick() {
        let mut app = setup_auto_upgrade_app();
        // Enable auto for upgrade 0 but don't tick hour
        {
            let mut auto = app.world_mut().resource_mut::<crate::resources::AutoUpgrade>();
            auto.ensure_towns(1);
            auto.flags[0][0] = true;
        }
        // Give plenty of resources
        app.world_mut().resource_mut::<crate::resources::FoodStorage>().food = vec![999999];
        app.world_mut().resource_mut::<crate::resources::GoldStorage>().gold = vec![999999];
        app.update();
        let collected = app.world().resource::<CollectedUpgrades>();
        assert!(collected.0.is_empty(), "no upgrades should fire without hour_ticked");
    }

    #[test]
    fn auto_upgrade_fires_on_hour_tick() {
        let mut app = setup_auto_upgrade_app();
        {
            let mut auto = app.world_mut().resource_mut::<crate::resources::AutoUpgrade>();
            auto.ensure_towns(1);
            auto.flags[0][0] = true;
        }
        app.world_mut().resource_mut::<crate::resources::FoodStorage>().food = vec![999999];
        app.world_mut().resource_mut::<crate::resources::GoldStorage>().gold = vec![999999];
        app.world_mut().resource_mut::<crate::resources::GameTime>().hour_ticked = true;
        app.update();
        let collected = app.world().resource::<CollectedUpgrades>();
        assert!(!collected.0.is_empty(), "should fire at least one upgrade on hour tick with resources and auto enabled");
        assert_eq!(collected.0[0].0, 0, "town_idx should be 0");
        assert_eq!(collected.0[0].1, 0, "upgrade_idx should be 0");
    }

    #[test]
    fn auto_upgrade_skips_disabled_flags() {
        let mut app = setup_auto_upgrade_app();
        // All flags default to false
        app.world_mut().resource_mut::<crate::resources::FoodStorage>().food = vec![999999];
        app.world_mut().resource_mut::<crate::resources::GoldStorage>().gold = vec![999999];
        app.world_mut().resource_mut::<crate::resources::GameTime>().hour_ticked = true;
        app.update();
        let collected = app.world().resource::<CollectedUpgrades>();
        assert!(collected.0.is_empty(), "no upgrades should fire when all flags are false");
    }

    #[test]
    fn auto_upgrade_skips_unaffordable() {
        let mut app = setup_auto_upgrade_app();
        {
            let mut auto = app.world_mut().resource_mut::<crate::resources::AutoUpgrade>();
            auto.ensure_towns(1);
            auto.flags[0][0] = true;
        }
        // Zero resources — can't afford anything
        app.world_mut().resource_mut::<crate::resources::GameTime>().hour_ticked = true;
        app.update();
        let collected = app.world().resource::<CollectedUpgrades>();
        assert!(collected.0.is_empty(), "no upgrades should fire with zero resources");
    }

    // -- auto_tower_upgrade_system -------------------------------------------

    fn setup_auto_tower_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(crate::resources::GameTime::default());
        app.insert_resource(crate::resources::EntityMap::default());
        app.insert_resource(crate::resources::FoodStorage { food: vec![100] });
        app.insert_resource(crate::resources::GoldStorage { gold: vec![100] });
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_systems(FixedUpdate, auto_tower_upgrade_system);
        app.update();
        app.update();
        app
    }

    fn add_tower(app: &mut App, slot: usize, auto_flags: Vec<bool>) {
        use crate::resources::BuildingInstance;
        use crate::world::BuildingKind;
        let inst = BuildingInstance {
            kind: BuildingKind::Tower,
            position: bevy::math::Vec2::ZERO,
            town_idx: 0,
            slot,
            faction: 0,
            patrol_order: 0,
            assigned_mine: None,
            manual_mine: false,
            wall_level: 0,
            npc_uid: None,
            respawn_timer: -1.0,
            growth_ready: false,
            growth_progress: 0.0,
            occupants: 0,
            under_construction: 0.0,
            kills: 0,
            xp: 0,
            upgrade_levels: vec![0; auto_flags.len()],
            auto_upgrade_flags: auto_flags,
        };
        app.world_mut()
            .resource_mut::<crate::resources::EntityMap>()
            .add_instance(inst);
    }

    #[test]
    fn auto_tower_upgrade_skips_without_hour_tick() {
        let mut app = setup_auto_tower_app();
        let num_tower_upgrades = crate::constants::TOWER_UPGRADES.len();
        add_tower(&mut app, 5000, vec![true; num_tower_upgrades]);
        app.update();
        let em = app.world().resource::<crate::resources::EntityMap>();
        let inst = em.get_instance(5000).unwrap();
        assert!(
            inst.upgrade_levels.iter().all(|&l| l == 0),
            "no upgrades should apply without hour_ticked"
        );
    }

    #[test]
    fn auto_tower_upgrade_buys_cheapest_on_hour_tick() {
        let mut app = setup_auto_tower_app();
        let num_tower_upgrades = crate::constants::TOWER_UPGRADES.len();
        add_tower(&mut app, 5000, vec![true; num_tower_upgrades]);
        app.world_mut()
            .resource_mut::<crate::resources::GameTime>()
            .hour_ticked = true;
        app.update();
        let em = app.world().resource::<crate::resources::EntityMap>();
        let inst = em.get_instance(5000).unwrap();
        let total_upgrades: u8 = inst.upgrade_levels.iter().sum();
        assert!(
            total_upgrades > 0,
            "should buy at least one tower upgrade on hour tick"
        );
    }

    #[test]
    fn auto_tower_upgrade_deducts_resources() {
        let mut app = setup_auto_tower_app();
        let num_tower_upgrades = crate::constants::TOWER_UPGRADES.len();
        add_tower(&mut app, 5000, vec![true; num_tower_upgrades]);
        app.world_mut()
            .resource_mut::<crate::resources::GameTime>()
            .hour_ticked = true;
        app.update();
        let food = app.world().resource::<crate::resources::FoodStorage>().food[0];
        let gold = app.world().resource::<crate::resources::GoldStorage>().gold[0];
        assert!(
            food < 100 || gold < 100,
            "resources should be deducted after purchase, food={food} gold={gold}"
        );
    }

    #[test]
    fn auto_tower_upgrade_skips_disabled_flags() {
        let mut app = setup_auto_tower_app();
        let num_tower_upgrades = crate::constants::TOWER_UPGRADES.len();
        add_tower(&mut app, 5000, vec![false; num_tower_upgrades]);
        app.world_mut()
            .resource_mut::<crate::resources::GameTime>()
            .hour_ticked = true;
        app.update();
        let em = app.world().resource::<crate::resources::EntityMap>();
        let inst = em.get_instance(5000).unwrap();
        let total_upgrades: u8 = inst.upgrade_levels.iter().sum();
        assert_eq!(total_upgrades, 0, "should not upgrade when all flags are false");
    }
}
