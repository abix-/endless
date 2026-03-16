//! Stat resolution, upgrades, and XP systems.
//! Stage 8: CombatConfig + resolve_combat_stats + CachedStats.
//! Stage 9: UpgradeQueue + process_upgrades_system.

use crate::components::{BaseAttackType, CachedStats, Job, Personality};
use crate::constants::{
    AttackTypeStats, EffectDisplay, FOUNTAIN_TOWER, NPC_REGISTRY, ResourceKind, TOWN_UPGRADES,
    TowerStats, UpgradeStatDef, UpgradeStatKind, npc_def,
};
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::systemparams::{EconomyState, WorldState};
use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::LazyLock;

// ============================================================================
// COMBAT CONFIG (replaces scattered constants)
// ============================================================================

/// Central combat configuration. Attack defaults + healing constants.
/// Per-job base stats (hp, damage, speed) live on NpcDef in NPC_REGISTRY.
#[derive(Resource)]
pub struct CombatConfig {
    pub attacks: HashMap<BaseAttackType, AttackTypeStats>,
    pub heal_rate: f32,
    pub heal_radius: f32,
}

impl Default for CombatConfig {
    fn default() -> Self {
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
                let boost = (1.0 - 1.0 / (1.0 + lv * pct)) * 100.0;
                format!("+{:.0}%", boost)
            };
            let next_boost = (1.0 - 1.0 / (1.0 + (lv + 1.0) * pct)) * 100.0;
            let next = format!("+{:.0}%", next_boost);
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
    pub slot: crate::constants::ItemKind,
    pub ring_index: u8, // 0=ring1, 1=ring2 (ignored for non-Ring)
}

/// Request immediate auto-equip planning for a town, optionally scoped to one NPC.
/// Writers: Armory UI. Reader: auto_equip_system.
#[derive(Message, Clone)]
pub struct AutoEquipNowMsg {
    pub town_idx: usize,
    pub npc_entity: Option<Entity>,
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
            ResourceKind::Wood | ResourceKind::Stone => false,
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
            ResourceKind::Wood | ResourceKind::Stone => {}
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
        if fc > 0 {
            parts.push(format!("{fc} food"));
        }
        if gc > 0 {
            parts.push(format!("{gc} gold"));
        }
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
                ResourceKind::Wood => format!("{total} wood"),
                ResourceKind::Stone => format!("{total} stone"),
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
pub fn resolve_tower_instance_stats(
    base: &TowerStats,
    level: i32,
    upgrade_levels: &[u8],
) -> TowerStats {
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
    let regen_level = upgrades
        .iter()
        .enumerate()
        .find(|(_, d)| d.kind == UpgradeStatKind::HpRegen)
        .map(|(i, _)| upgrade_levels.get(i).copied().unwrap_or(0) as f32)
        .unwrap_or(0.0);

    TowerStats {
        range: base.range * get(UpgradeStatKind::Range) * level_mult,
        damage: base.damage * get(UpgradeStatKind::Attack) * level_mult,
        cooldown: base.cooldown * get(UpgradeStatKind::AttackSpeed),
        proj_speed: base.proj_speed * get(UpgradeStatKind::ProjectileSpeed),
        proj_lifetime: base.proj_lifetime * get(UpgradeStatKind::ProjectileLifetime),
        hp_regen: regen_level * 2.0,
        max_hp: base.max_hp * get(UpgradeStatKind::Hp) * level_mult,
    }
}

/// Which upgrades require NPC stat re-resolution (combat-affecting).
fn is_combat_upgrade(idx: usize) -> bool {
    UPGRADES.nodes[idx].is_combat_stat
}

/// Convert proficiency (0..MAX_PROFICIENCY) to a multiplier.
/// 0 = 1.0x, MAX/2 = 1.25x, MAX = 1.5x.
pub fn proficiency_mult(value: f32) -> f32 {
    use crate::constants::MAX_PROFICIENCY;
    1.0 + (value.clamp(0.0, MAX_PROFICIENCY) / MAX_PROFICIENCY) * 0.5
}

// ============================================================================
// STAT RESOLVER
// ============================================================================

/// Resolve final NPC stats from config, upgrades, level, and personality.
/// Cached on entity as CachedStats. Re-resolved on spawn, upgrade, or level-up.
pub fn resolve_combat_stats(
    job: Job,
    attack_type: BaseAttackType,
    _town_idx: i32,
    level: i32,
    personality: &Personality,
    config: &CombatConfig,
    town_levels: &[u8],
    weapon_bonus: f32,
    armor_bonus: f32,
    prof_combat: f32,
) -> CachedStats {
    let def = npc_def(job);
    let default_atk = config
        .attacks
        .get(&attack_type)
        .expect("missing attack type stats");
    let atk_base = def.attack_override.as_ref().unwrap_or(default_atk);
    let trait_mods = personality.get_stat_mods();
    let level_mult = 1.0 + level as f32 * 0.01;

    let town = town_levels;
    let reg = &*UPGRADES;

    // Use NpcDef.upgrade_category to look up all upgrades dynamically
    let cat = def.upgrade_category.unwrap_or("");
    let upgrade_hp = reg.stat_mult(town, cat, UpgradeStatKind::Hp);
    let upgrade_dmg = reg.stat_mult(town, cat, UpgradeStatKind::Attack);
    let upgrade_range = reg.stat_mult(town, cat, UpgradeStatKind::Range);
    let upgrade_speed = reg.stat_mult(town, cat, UpgradeStatKind::MoveSpeed);
    let cooldown_mult = reg.stat_mult(town, cat, UpgradeStatKind::AttackSpeed);
    let upgrade_proj_speed = reg.stat_mult(town, cat, UpgradeStatKind::ProjectileSpeed);
    let upgrade_proj_life = reg.stat_mult(town, cat, UpgradeStatKind::ProjectileLifetime);
    let stamina_mult = reg.stat_mult(town, cat, UpgradeStatKind::Stamina);
    let hp_regen_level = reg.stat_level(town, cat, UpgradeStatKind::HpRegen) as f32;

    let prof_mult = proficiency_mult(prof_combat);
    CachedStats {
        damage: def.base_damage
            * upgrade_dmg
            * trait_mods.damage
            * level_mult
            * (1.0 + weapon_bonus)
            * prof_mult,
        range: atk_base.range * upgrade_range * trait_mods.range,
        cooldown: atk_base.cooldown * cooldown_mult * trait_mods.cooldown / prof_mult,
        projectile_speed: atk_base.projectile_speed * upgrade_proj_speed,
        projectile_lifetime: atk_base.projectile_lifetime * upgrade_proj_life,
        max_health: def.base_hp * upgrade_hp * trait_mods.hp * level_mult * (1.0 + armor_bonus),
        speed: def.base_speed * upgrade_speed * trait_mods.speed,
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
    town_levels: &[u8],
    prof_combat: f32,
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
        job,
        attack_type,
        town_idx,
        level,
        personality,
        config,
        town_levels,
        equipment.total_weapon_bonus(),
        equipment.total_armor_bonus(),
        prof_combat,
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
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth {
                idx: slot,
                health: hp.0,
            }));
        }
    }
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetMaxHealth {
        idx: slot,
        max_health: new_max,
    }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed {
        idx: slot,
        speed: new_speed,
    }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::MarkVisualDirty { idx: slot }));
}

// ============================================================================
// PROCESS UPGRADES SYSTEM
// ============================================================================

/// Drains UpgradeMsg messages, applies upgrades, re-resolves affected NPC stats.
pub fn process_upgrades_system(
    mut queue: MessageReader<UpgradeMsg>,
    mut economy: EconomyState,
    config: Res<CombatConfig>,
    npc_stats_q: Query<&crate::components::NpcStats>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut world_state: WorldState,
    mut cached_stats_q: Query<&mut crate::components::CachedStats>,
    mut speed_q: Query<&mut crate::components::Speed>,
    mut health_q: Query<&mut crate::components::Health, Without<crate::components::Building>>,
    attack_type_q: Query<&crate::components::BaseAttackType>,
    personality_q: Query<&crate::components::Personality>,
    equipment_q: Query<&crate::components::NpcEquipment>,
    skills_q: Query<&crate::components::NpcSkills>,
) {
    let count = upgrade_count();
    for msg in queue.read() {
        let (town_idx, upgrade_idx) = (msg.town_idx, msg.upgrade_idx);
        if upgrade_idx >= count {
            continue;
        }
        // Prereq + affordability gate
        let levels = economy.towns.upgrade_levels(town_idx as i32);
        let mut food = economy.towns.food(town_idx as i32);
        let mut gold = economy.towns.gold(town_idx as i32);
        if !upgrade_available(&levels, upgrade_idx, food, gold) {
            continue;
        }

        // Deduct cost and increment level
        let level = levels[upgrade_idx];
        deduct_upgrade_cost(upgrade_idx, level, &mut food, &mut gold);
        if let Some(mut f) = economy.towns.food_mut(town_idx as i32) {
            f.0 = food;
        }
        if let Some(mut g) = economy.towns.gold_mut(town_idx as i32) {
            g.0 = gold;
        }
        if let Some(mut u) = economy.towns.upgrades_mut(town_idx as i32) {
            if upgrade_idx < u.0.len() {
                u.0[upgrade_idx] = level.saturating_add(1);
            }
        }

        let node = &UPGRADES.nodes[upgrade_idx];

        // Invalidate healing zone cache on radius/rate upgrades
        if node.invalidates_healing {
            world_state
                .dirty_writers
                .healing_zones
                .write(crate::messages::HealingZonesDirtyMsg);
        }

        if node.triggers_expansion {
            let mut al = economy.towns.area_level(town_idx as i32);
            let _ = crate::world::expand_town_build_area(
                &mut world_state.grid,
                &world_state.world_data.towns,
                &world_state.entity_map,
                town_idx,
                &mut al,
            );
            economy.towns.set_area_level(town_idx as i32, al);
            // Rebuild buildability with updated area levels
            let n = world_state.world_data.towns.len();
            let area_levels: Vec<i32> =
                (0..n).map(|i| economy.towns.area_level(i as i32)).collect();
            world_state.grid.sync_town_buildability(
                &world_state.world_data.towns,
                &area_levels,
                &world_state.entity_map,
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

        let town_levels = economy.towns.upgrade_levels(town_idx as i32);
        let slots: Vec<usize> = world_state
            .entity_map
            .slots_for_town(town_idx as i32)
            .to_vec();
        for slot in slots {
            let Some(npc) = world_state.entity_map.get_npc(slot) else {
                continue;
            };
            let entity = npc.entity;

            let npc_level = npc_stats_q
                .get(entity)
                .map(|s| level_from_xp(s.xp))
                .unwrap_or(0);
            let old_max = cached_stats_q
                .get(entity)
                .map(|s| s.max_health)
                .unwrap_or(100.0);
            let pers = personality_q.get(entity).cloned().unwrap_or_default();
            let atk_type = attack_type_q
                .get(entity)
                .copied()
                .unwrap_or(crate::components::BaseAttackType::Melee);
            let (wb, ab) = equipment_q
                .get(entity)
                .map(|eq| (eq.total_weapon_bonus(), eq.total_armor_bonus()))
                .unwrap_or((0.0, 0.0));
            let prof_c = skills_q.get(entity).map(|s| s.combat).unwrap_or(0.0);
            let new_cached = resolve_combat_stats(
                npc.job,
                atk_type,
                town_idx as i32,
                npc_level,
                &pers,
                &config,
                &town_levels,
                wb,
                ab,
                prof_c,
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
    config: Res<CombatConfig>,
    mut town_access: crate::systemparams::TownAccess,
    npc_stats_q: Query<&crate::components::NpcStats>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    skills_q: Query<&crate::components::NpcSkills>,
) {
    // Equip: TownEquipment -> NpcEquipment
    for msg in equip_msgs.read() {
        let item = {
            let Some(mut eq) = town_access.equipment_mut(msg.town_idx as i32) else {
                continue;
            };
            let Some(pos) = eq.0.iter().position(|i| i.id == msg.item_id) else {
                continue;
            };
            eq.0.swap_remove(pos)
        };
        let Ok((mut eq, gpu_slot, job, town_id, atk_type, pers)) =
            equipment_q.get_mut(msg.npc_entity)
        else {
            // NPC gone — put item back
            if let Some(mut teq) = town_access.equipment_mut(msg.town_idx as i32) {
                teq.0.push(item);
            }
            continue;
        };
        let slot_idx = gpu_slot.0;

        // Determine target field. Ring special case: prefer empty ring1, else ring2.
        use crate::constants::ItemKind;
        let target: &mut Option<crate::constants::LootItem> = match item.kind {
            ItemKind::Ring => {
                if eq.ring1.is_none() {
                    &mut eq.ring1
                } else {
                    &mut eq.ring2
                }
            }
            _ => eq.slot_mut(item.kind),
        };

        // Swap out old item if present
        if let Some(old) = target.take() {
            if let Some(mut teq) = town_access.equipment_mut(msg.town_idx as i32) {
                teq.0.push(old);
            }
        }
        *target = Some(item);

        // Re-resolve stats
        let level = npc_stats_q
            .get(msg.npc_entity)
            .map(|s| level_from_xp(s.xp))
            .unwrap_or(0);
        let tl = town_access.upgrade_levels(town_id.0);
        let prof_c = skills_q
            .get(msg.npc_entity)
            .map(|s| s.combat)
            .unwrap_or(0.0);
        re_resolve_npc_stats(
            msg.npc_entity,
            slot_idx,
            &eq,
            *job,
            *atk_type,
            town_id.0,
            level,
            pers,
            &config,
            &tl,
            prof_c,
            &mut cached_stats_q,
            &mut speed_q,
            &mut health_q,
            &mut gpu_updates,
        );
    }

    // Unequip: NpcEquipment → TownEquipment
    for msg in unequip_msgs.read() {
        let Ok((mut eq, gpu_slot, job, town_id, atk_type, pers)) =
            equipment_q.get_mut(msg.npc_entity)
        else {
            continue;
        };
        let slot_idx = gpu_slot.0;

        use crate::constants::ItemKind;
        let source: &mut Option<crate::constants::LootItem> = match msg.slot {
            ItemKind::Ring => {
                if msg.ring_index == 1 {
                    &mut eq.ring2
                } else {
                    &mut eq.ring1
                }
            }
            _ => eq.slot_mut(msg.slot),
        };

        let Some(item) = source.take() else {
            continue; // slot was empty
        };
        if let Some(mut teq) = town_access.equipment_mut(town_id.0) {
            teq.0.push(item);
        }

        let level = npc_stats_q
            .get(msg.npc_entity)
            .map(|s| level_from_xp(s.xp))
            .unwrap_or(0);
        let tl = town_access.upgrade_levels(town_id.0);
        let prof_c = skills_q
            .get(msg.npc_entity)
            .map(|s| s.combat)
            .unwrap_or(0.0);
        re_resolve_npc_stats(
            msg.npc_entity,
            slot_idx,
            &eq,
            *job,
            *atk_type,
            town_id.0,
            level,
            pers,
            &config,
            &tl,
            prof_c,
            &mut cached_stats_q,
            &mut speed_q,
            &mut health_q,
            &mut gpu_updates,
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
    town_access: crate::systemparams::TownAccess,
    mut queue: MessageWriter<UpgradeMsg>,
) {
    if !game_time.hour_ticked {
        return;
    }

    let count = upgrade_count();
    for (town_idx, flags) in auto.flags.iter().enumerate() {
        let ti = town_idx as i32;
        let levels = town_access.upgrade_levels(ti);
        let raw_food = town_access.food(ti);
        let raw_gold = town_access.gold(ti);
        let (rf, rg) = town_access
            .policy(ti)
            .map(|p| (p.reserve_food, p.reserve_gold))
            .unwrap_or((0, 0));
        let food = (raw_food - rf).max(0);
        let gold = (raw_gold - rg).max(0);
        for (i, &enabled) in flags.iter().enumerate().take(count) {
            if !enabled {
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
    mut towers_q: Query<(
        &crate::components::GpuSlot,
        &crate::components::TownId,
        &mut crate::components::TowerBuildingState,
    )>,
    mut town_access: crate::systemparams::TownAccess,
    _entity_map: Res<crate::resources::EntityMap>,
) {
    if !game_time.hour_ticked {
        return;
    }
    let tower_upgrades = crate::constants::TOWER_UPGRADES;

    for (_gpu_slot, town_id, mut tower) in &mut towers_q {
        if !tower.auto_upgrade_flags.iter().any(|&f| f) {
            continue;
        }
        let ti = town_id.0;
        let (rf, rg) = town_access
            .policy(ti)
            .map(|p| (p.reserve_food, p.reserve_gold))
            .unwrap_or((0, 0));
        let food = (town_access.food(ti) - rf).max(0);
        let gold = (town_access.gold(ti) - rg).max(0);

        // Find cheapest affordable upgrade among auto-flagged stats
        let mut best: Option<(i32, usize)> = None;
        for (i, upg) in tower_upgrades.iter().enumerate() {
            if !tower.auto_upgrade_flags.get(i).copied().unwrap_or(false) {
                continue;
            }
            let lv = tower.upgrade_levels.get(i).copied().unwrap_or(0);
            let cost_mult = upgrade_cost(lv);
            let can_afford = upg.cost.iter().all(|(res, base)| {
                let total = base * cost_mult;
                match res {
                    crate::constants::ResourceKind::Food => food >= total,
                    crate::constants::ResourceKind::Gold => gold >= total,
                    crate::constants::ResourceKind::Wood
                    | crate::constants::ResourceKind::Stone => false,
                }
            });
            if can_afford {
                let total: i32 = upg.cost.iter().map(|(_, base)| base * cost_mult).sum();
                if best.is_none_or(|b| total < b.0) {
                    best = Some((total, i));
                }
            }
        }

        if let Some((_, idx)) = best {
            let upg = &tower_upgrades[idx];
            let lv = tower.upgrade_levels.get(idx).copied().unwrap_or(0);
            let cost_mult = upgrade_cost(lv);
            for (res, base) in upg.cost {
                let total = base * cost_mult;
                match res {
                    crate::constants::ResourceKind::Food => {
                        if let Some(mut f) = town_access.food_mut(ti) {
                            f.0 -= total;
                        }
                    }
                    crate::constants::ResourceKind::Gold => {
                        if let Some(mut g) = town_access.gold_mut(ti) {
                            g.0 -= total;
                        }
                    }
                    crate::constants::ResourceKind::Wood
                    | crate::constants::ResourceKind::Stone => {}
                }
            }
            // Increment upgrade level on ECS component
            while tower.upgrade_levels.len() <= idx {
                tower.upgrade_levels.push(0);
            }
            tower.upgrade_levels[idx] += 1;
        }
    }
}

// ============================================================================
// AUTO-EQUIP SYSTEM
// ============================================================================

/// Once per game hour, auto-equip items from TownEquipment onto NPCs.
/// Picks the NPC with the lowest bonus in the matching slot (or empty slot first).
pub fn auto_equip_system(
    game_time: Res<crate::resources::GameTime>,
    town_access: crate::systemparams::TownAccess,
    equipment_q: Query<
        (
            Entity,
            &crate::components::NpcEquipment,
            &Job,
            &crate::components::TownId,
        ),
        (
            Without<crate::components::Building>,
            Without<crate::components::Dead>,
        ),
    >,
    mut auto_now: MessageReader<AutoEquipNowMsg>,
    mut equip_writer: MessageWriter<EquipItemMsg>,
    world_data: Res<crate::world::WorldData>,
) {
    let manual_requests: Vec<AutoEquipNowMsg> = auto_now.read().cloned().collect();
    if !game_time.hour_ticked && manual_requests.is_empty() {
        return;
    }

    let mut run_for_scope = |town_idx: usize, only_entity: Option<Entity>| {
        let Some(items) = town_access.equipment(town_idx as i32) else {
            return;
        };
        if items.is_empty() {
            return;
        }

        // Collect military NPCs in this town
        let town_npcs: Vec<_> = equipment_q
            .iter()
            .filter(|(entity, _, _, tid)| {
                tid.0 == town_idx as i32 && only_entity.is_none_or(|target| *entity == target)
            })
            .collect();
        if town_npcs.is_empty() {
            return;
        }

        // Track items we've already queued for equip this cycle (avoid double-assign)
        let mut assigned_items: Vec<u64> = Vec::new();

        for item in &items {
            if assigned_items.contains(&item.id) {
                continue;
            }

            let slot = item.kind;

            // Find best NPC candidate: empty slot first, then biggest upgrade margin
            let mut best: Option<(Entity, f32)> = None; // (entity, current_bonus)

            for &(entity, equip, job, _) in &town_npcs {
                let def = crate::constants::npc_def(*job);
                if !def.equip_slots.contains(&slot) {
                    continue;
                }

                // Get current bonus in this slot
                use crate::constants::ItemKind;
                let current_bonus = match slot {
                    ItemKind::Ring => {
                        // For rings, check both slots — use the lower one
                        let b1 = equip.ring1.as_ref().map(|i| i.stat_bonus).unwrap_or(0.0);
                        let b2 = equip.ring2.as_ref().map(|i| i.stat_bonus).unwrap_or(0.0);
                        b1.min(b2)
                    }
                    _ => equip
                        .slot(slot)
                        .as_ref()
                        .map(|i| i.stat_bonus)
                        .unwrap_or(0.0),
                };

                // Must be a strict upgrade
                if item.stat_bonus <= current_bonus {
                    continue;
                }

                // Prefer NPC with lowest current bonus (distribute gear evenly)
                if best.is_none_or(|b| current_bonus < b.1) {
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
    };

    if game_time.hour_ticked {
        for town_idx in 0..world_data.towns.len() {
            run_for_scope(town_idx, None);
        }
    }

    for req in manual_requests {
        run_for_scope(req.town_idx, req.npc_entity);
    }
}

// ============================================================================
// XP grant + NPC kill loot logic moved to unified death_system (health.rs)

// ============================================================================
// TOWN EQUIPMENT PRUNING
// ============================================================================

/// Prune excess TownEquipment hourly. Removes lowest-value items (by rarity then stat_bonus)
/// and converts them to gold. Prevents unbounded inventory growth at scale.
pub fn prune_town_equipment_system(
    game_time: Res<crate::resources::GameTime>,
    mut town_access: crate::systemparams::TownAccess,
    world_data: Res<crate::world::WorldData>,
) {
    if !game_time.hour_ticked {
        return;
    }
    let cap = crate::constants::TOWN_EQUIPMENT_CAP;
    for i in 0..world_data.towns.len() {
        let ti = i as i32;
        let Some(mut eq) = town_access.equipment_mut(ti) else {
            continue;
        };
        if eq.0.len() <= cap {
            continue;
        }
        // Sort: lowest rarity gold_cost first, then lowest stat_bonus
        eq.0.sort_by(|a, b| {
            a.rarity.gold_cost().cmp(&b.rarity.gold_cost()).then(
                a.stat_bonus
                    .partial_cmp(&b.stat_bonus)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });
        let excess = eq.0.len() - cap;
        eq.0.drain(..excess);
        // Convert pruned items to gold (1 gold each)
        if let Some(mut gold) = town_access.gold_mut(ti) {
            gold.0 += excess as i32;
        }
    }
}

// ============================================================================
// UNIT TESTS
// ============================================================================

#[cfg(test)]
mod tests;
