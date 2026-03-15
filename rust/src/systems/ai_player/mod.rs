//! AI player system -- autonomous opponents that build and upgrade like the player.
//! Each AI has a personality (Aggressive/Balanced/Economic) that influences weighted
//! random decisions -- same pattern as NPC behavior scoring.
//!
//! Slot selection: economy buildings (farms, houses, barracks) prefer inner slots
//! (closest to center). Waypoints form a single outer ring on the perimeter of the
//! build area, placed at block corners adjacent to road intersections. When the town
//! area expands, inner waypoints are pruned to maintain one ring.

mod decision;
mod mine_analysis;
mod slot_selection;
mod squad_commander;
#[cfg(test)]
mod tests;

pub use decision::ai_decision_system;
pub use slot_selection::sync_patrol_perimeter_system;
pub use squad_commander::{ai_squad_commander_system, rebuild_squad_indices};

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use rand::Rng;

use crate::components::{Building, WaypointOrder};
use crate::constants::UpgradeStatKind;
use crate::constants::*;
use crate::resources::*;
use crate::systemparams::WorldState;
use crate::systems::stats::{
    UPGRADES, expansion_cost, upgrade_available, upgrade_cost, upgrade_node, upgrade_unlocked,
};
use crate::world::{self, BuildingKind, WorldData};

/// Mutable world resources needed for AI building. Bundled to stay under Bevy's 16-param limit.
#[derive(SystemParam)]
pub struct AiBuildRes<'w, 's> {
    world: WorldState<'w>,
    upgrade_queue: MessageWriter<'w, crate::systems::stats::UpgradeMsg>,
    gpu_updates: MessageWriter<'w, crate::messages::GpuUpdateMsg>,
    commands: Commands<'w, 's>,
    waypoint_q: Query<'w, 's, &'static mut WaypointOrder, With<Building>>,
}

/// Bundled dirty-flag message readers for ai_dirty_drain_system.
/// Separate from ai_decision_system to avoid MessageReader/MessageWriter conflict
/// (ai_decision_system writes via DirtyWriters in WorldState).
#[derive(SystemParam)]
pub struct AiDirtyReaders<'w, 's> {
    pub grid: MessageReader<'w, 's, crate::messages::BuildingGridDirtyMsg>,
    pub mining: MessageReader<'w, 's, crate::messages::MiningDirtyMsg>,
    pub perimeter: MessageReader<'w, 's, crate::messages::PatrolPerimeterDirtyMsg>,
}

/// Intermediate resource: set by ai_dirty_drain_system, consumed by ai_decision_system.
#[derive(Resource, Default)]
pub struct AiSnapshotDirty(pub bool);

/// Intermediate resource: set by perimeter_dirty_drain_system, consumed by sync_patrol_perimeter_system.
#[derive(Resource, Default)]
pub struct PerimeterSyncDirty(pub bool);

/// Drain dirty messages into AiSnapshotDirty. Runs before ai_decision_system
/// so that the decision system can use DirtyWriters without conflicting.
pub fn ai_dirty_drain_system(mut dirty: ResMut<AiSnapshotDirty>, mut readers: AiDirtyReaders) {
    if readers.grid.read().count() > 0
        || readers.mining.read().count() > 0
        || readers.perimeter.read().count() > 0
    {
        dirty.0 = true;
    }
}

/// Drain PatrolPerimeterDirtyMsg for sync_patrol_perimeter_system (which also
/// has DirtyWriters via WorldState, causing a Reader/Writer conflict).
pub fn perimeter_dirty_drain_system(
    mut dirty: ResMut<PerimeterSyncDirty>,
    mut reader: MessageReader<crate::messages::PatrolPerimeterDirtyMsg>,
) {
    if reader.read().count() > 0 {
        dirty.0 = true;
    }
}

/// Alive buildings for a town as `(row, col)` grid slots. Returns an iterator.
/// Single source of truth for the alive + town ownership + coordinate conversion pipeline.
const MINING_RADIUS_STEP: f32 = 300.0;
const MAX_MINING_RADIUS: f32 = 5000.0;
/// Hard ceiling on miners per mine, regardless of personality target.
const MAX_MINERS_PER_MINE: usize = 5;

/// Initial mining radius: reaches at least the nearest gold mine, rounded up to step grid.
pub fn initial_mining_radius(entity_map: &EntityMap, center: Vec2) -> f32 {
    let nearest = entity_map
        .iter_kind(BuildingKind::GoldMine)
        .map(|m| (m.position - center).length())
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    match nearest {
        Some(dist) => (dist + 50.0).min(MAX_MINING_RADIUS),
        None => 0.0,
    }
}

/// Returns Vec<(slot, new_order)> for the caller to apply to WaypointOrder ECS components.
fn recalc_waypoint_patrol_order_clockwise(
    world_data: &mut WorldData,
    entity_map: &mut EntityMap,
    town_idx: u32,
) -> Vec<(usize, u32)> {
    // Rebuild patrol order from geometry, not history:
    // sort all living waypoints of this town by angle around town center.
    // This guarantees stable clockwise ordering after add/remove operations.
    let Some(center) = world_data.towns.get(town_idx as usize).map(|t| t.center) else {
        return Vec::new();
    };

    // Collect (slot, position) for living waypoints of this town
    let mut entries: Vec<(usize, Vec2)> = entity_map
        .iter_kind_for_town(BuildingKind::Waypoint, town_idx)
        .map(|b| (b.slot, b.position))
        .collect();

    // Clockwise around town center, starting at north (+Y).
    entries.sort_by(|a, b| {
        let pa = a.1 - center;
        let pb = b.1 - center;
        let mut aa = pa.x.atan2(pa.y);
        let mut ab = pb.x.atan2(pb.y);
        if aa < 0.0 {
            aa += std::f32::consts::TAU;
        }
        if ab < 0.0 {
            ab += std::f32::consts::TAU;
        }
        aa.partial_cmp(&ab)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                pa.length_squared()
                    .partial_cmp(&pb.length_squared())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    entries
        .iter()
        .enumerate()
        .map(|(order, &(slot, _))| (slot, order as u32))
        .collect()
}

#[derive(Clone, Default)]
struct AiTownSnapshot {
    center: Vec2,
    /// Town center in world grid coords.
    cc: usize,
    cr: usize,
    empty_slots: Vec<(usize, usize)>,
    farms: HashSet<(usize, usize)>,
    farmer_homes: HashSet<(usize, usize)>,
    archer_homes: HashSet<(usize, usize)>,
    crossbow_homes: HashSet<(usize, usize)>,
    /// Cached ideal waypoint ring positions in world grid (col, row).
    waypoint_ring: Vec<(usize, usize)>,
}

#[derive(Default)]
pub struct AiTownSnapshotCache {
    towns: HashMap<usize, AiTownSnapshot>,
    /// Cached population-spawner count per town. Recomputed on dirty.building_grid.
    spawner_counts: HashMap<usize, i32>,
}

#[derive(Resource)]
pub struct AiPlayerConfig {
    pub decision_interval: f32,
}

impl Default for AiPlayerConfig {
    fn default() -> Self {
        Self {
            decision_interval: DEFAULT_AI_INTERVAL,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AiKind {
    Raider,
    Builder,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AiPersonality {
    Aggressive,
    Balanced,
    Economic,
}

/// Road layout style -- randomly assigned per AI town, independent of personality.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoadStyle {
    None,
    Cardinal,
    Grid4,
    Grid5,
}

impl RoadStyle {
    /// True if (col, row) in world grid is reserved for road placement in this style.
    pub fn is_road_slot(self, col: usize, row: usize, cc: usize, cr: usize) -> bool {
        let dc = col as i32 - cc as i32;
        let dr = row as i32 - cr as i32;
        if dc == 0 && dr == 0 {
            return false;
        }
        match self {
            Self::None => false,
            Self::Cardinal => dr == 0 || dc == 0,
            Self::Grid4 => dr.rem_euclid(4) == 0 || dc.rem_euclid(4) == 0,
            Self::Grid5 => dr.rem_euclid(5) == 0 || dc.rem_euclid(5) == 0,
        }
    }

    pub fn random(rng: &mut impl rand::Rng) -> Self {
        const STYLES: [RoadStyle; 4] = [
            RoadStyle::None,
            RoadStyle::Cardinal,
            RoadStyle::Grid4,
            RoadStyle::Grid5,
        ];
        STYLES[rng.random_range(0..STYLES.len())]
    }
}

/// All possible AI actions, scored and picked via weighted random.
#[derive(Clone, Copy, Debug)]
enum AiAction {
    BuildFarm,
    BuildFarmerHome,
    BuildArcherHome,
    BuildCrossbowHome,
    BuildWaypoint,
    BuildTent,
    BuildMinerHome,
    BuildRoads,
    ExpandMiningRadius,
    Upgrade(usize), // upgrade index into UPGRADES registry
}

impl AiAction {
    fn label(self) -> &'static str {
        match self {
            Self::BuildFarm => "Farm",
            Self::BuildFarmerHome => "FarmerHome",
            Self::BuildArcherHome => "ArcherHome",
            Self::BuildCrossbowHome => "XbowHome",
            Self::BuildWaypoint => "Waypoint",
            Self::BuildTent => "Tent",
            Self::BuildMinerHome => "MinerHome",
            Self::BuildRoads => "Roads",
            Self::ExpandMiningRadius => "ExpandMining",
            Self::Upgrade(_) => "Upgrade",
        }
    }
}

impl AiPersonality {
    // Human-readable label for UI/logging.
    pub fn name(self) -> &'static str {
        match self {
            Self::Aggressive => "Aggressive",
            Self::Balanced => "Balanced",
            Self::Economic => "Economic",
        }
    }

    /// Food reserve per active NPC spawner for this personality.
    pub fn food_reserve_per_spawner(self) -> i32 {
        match self {
            Self::Aggressive => 0,
            Self::Balanced => 1,
            Self::Economic => 1,
        }
    }

    /// Town policies tuned per personality.
    pub fn default_policies(self) -> PolicySet {
        // Default policy baseline used when a town first gets an AI profile.
        match self {
            Self::Aggressive => PolicySet {
                archer_aggressive: true,
                archer_leash: false,
                farmer_fight_back: true,
                prioritize_healing: false,
                archer_flee_hp: 0.0,
                farmer_flee_hp: 0.30,
                mining_radius: crate::constants::DEFAULT_MINING_RADIUS,
                ..PolicySet::default()
            },
            Self::Balanced => PolicySet {
                mining_radius: crate::constants::DEFAULT_MINING_RADIUS,
                ..PolicySet::default()
            },
            Self::Economic => PolicySet {
                archer_leash: true,
                prioritize_healing: true,
                archer_flee_hp: 0.25,
                farmer_flee_hp: 0.50,
                mining_radius: crate::constants::DEFAULT_MINING_RADIUS,
                ..PolicySet::default()
            },
        }
    }

    /// Food desire thresholds for toggling eat_food policy.
    /// Returns (disable_above, reenable_below) -- higher food_desire = more food stress.
    fn eat_food_desire_thresholds(self) -> (f32, f32) {
        match self {
            Self::Aggressive => (0.4, 0.2),
            Self::Balanced => (0.6, 0.3),
            Self::Economic => (0.8, 0.5),
        }
    }

    /// Base weights for building types: (farm, house, barracks, waypoint)
    fn building_weights(self) -> (f32, f32, f32, f32) {
        // Relative urgency for (farm, farmer home, archer home, waypoint).
        // These are multipliers, not hard guarantees.
        match self {
            Self::Aggressive => (10.0, 10.0, 30.0, 20.0),
            Self::Balanced => (20.0, 20.0, 15.0, 10.0),
            Self::Economic => (15.0, 12.0, 8.0, 5.0),
        }
    }

    /// Barracks target count relative to total civilian homes (farmer + miner).
    pub fn archer_home_target(self, civilian_homes: usize) -> usize {
        match self {
            Self::Aggressive => civilian_homes.max(1),
            Self::Balanced => (civilian_homes / 2).max(1),
            Self::Economic => 1 + civilian_homes / 3,
        }
    }

    /// Farmer home target count relative to farms.
    fn farmer_home_target(self, farms: usize) -> usize {
        // Desired worker-housing ratio relative to farm count.
        match self {
            Self::Aggressive => farms.max(1),
            // Balanced tends toward ~1 farmer home per farm.
            Self::Balanced => (farms + 1).max(1),
            // Economic: slight surplus, not exponential (was farms*2 -> runaway spiral).
            Self::Economic => (farms + 2).max(1),
        }
    }

    /// Fraction of total civilian homes that should be miner homes.
    fn mining_ratio(self) -> f32 {
        match self {
            Self::Aggressive => 0.1,
            Self::Balanced => 0.2,
            Self::Economic => 0.3,
        }
    }

    /// Minimum miner homes this personality guarantees before other buildings outcompete.
    fn min_miner_homes(self) -> usize {
        match self {
            Self::Aggressive => 1,
            Self::Balanced => 2,
            Self::Economic => 3,
        }
    }

    /// Gold desire multiplier: Economic invests heavily in gold upgrades, Aggressive barely cares.
    pub fn gold_desire_mult(self) -> f32 {
        match self {
            Self::Aggressive => 0.5,
            Self::Balanced => 1.0,
            Self::Economic => 1.5,
        }
    }

    /// Target military share of total population (military / total).
    fn target_military_ratio(self) -> f32 {
        match self {
            Self::Aggressive => 0.50,
            Self::Balanced => 0.35,
            Self::Economic => 0.20,
        }
    }

    /// Baseline mining desire even without gold-costing upgrades.
    pub fn base_mining_desire(self) -> f32 {
        match self {
            Self::Aggressive => 0.0,
            Self::Balanced => 0.1,
            Self::Economic => 0.3,
        }
    }

    /// Weight for expanding mining radius (Aggressive expands eagerly, Economic conservatively).
    fn expand_mining_weight(self) -> f32 {
        // Controls how quickly personality chooses policy expansion
        // once current mine coverage is saturated.
        match self {
            Self::Aggressive => 12.0,
            Self::Balanced => 8.0,
            Self::Economic => 5.0,
        }
    }

    /// Weight for BuildRoads action.
    fn road_weight(self) -> f32 {
        match self {
            Self::Aggressive => 2.0,
            Self::Balanced => 3.0,
            Self::Economic => 8.0,
        }
    }

    /// How many road cells to place per BuildRoads tick.
    fn road_batch_size(self) -> usize {
        match self {
            Self::Aggressive => 4,
            Self::Balanced => 3,
            Self::Economic => 6,
        }
    }

    /// Compute the ideal outer ring of waypoint positions for the current build area.
    /// Walks the perimeter clockwise with corners guaranteed and min 5 Manhattan spacing.
    fn waypoint_ring_slots(
        self,
        area_level: i32,
        center: Vec2,
        grid: &world::WorldGrid,
        road_style: RoadStyle,
    ) -> Vec<(usize, usize)> {
        let (min_c, max_c, min_r, max_r) = world::build_bounds(area_level, center, grid);
        let (cc, cr) = grid.world_to_grid(center);
        const MIN_SPACING: i32 = 5;

        // Walk perimeter clockwise: top->right->bottom->left (no duplicate corners)
        // Stored as (col, row) in world grid coords
        let mut perimeter: Vec<(usize, usize)> = Vec::new();
        for c in min_c..=max_c {
            perimeter.push((c, max_r));
        }
        for r in (min_r..max_r).rev() {
            perimeter.push((max_c, r));
        }
        for c in (min_c..max_c).rev() {
            perimeter.push((c, min_r));
        }
        for r in (min_r + 1)..max_r {
            perimeter.push((min_c, r));
        }

        let corners: HashSet<(usize, usize)> = [
            (min_c, max_r),
            (max_c, max_r),
            (max_c, min_r),
            (min_c, min_r),
        ]
        .into_iter()
        .collect();

        let mut result: Vec<(usize, usize)> = Vec::new();
        for &(c, r) in &perimeter {
            if c == cc && r == cr {
                continue;
            }
            // Corners always included (even on road slots); others skip road slots
            if road_style.is_road_slot(c, r, cc, cr) && !corners.contains(&(c, r)) {
                continue;
            }
            let is_corner = corners.contains(&(c, r));
            let too_close = result.iter().any(|&(pc, pr)| {
                (c as i32 - pc as i32).abs() + (r as i32 - pr as i32).abs() < MIN_SPACING
            });
            if is_corner || !too_close {
                result.push((c, r));
            }
        }

        // Wrap-around check: drop trailing non-corner entries too close to the first
        while result.len() > 4 {
            let last = *result.last().expect("result non-empty after len check");
            let first = result[0];
            if (last.0 as i32 - first.0 as i32).abs() + (last.1 as i32 - first.1 as i32).abs()
                >= MIN_SPACING
            {
                break;
            }
            if corners.contains(&last) {
                break;
            }
            result.pop();
        }
        result
    }

    /// Upgrade weights by (category, stat_kind). Returns a Vec indexed by upgrade registry index.
    /// Only entries with weight > 0 are scored.
    pub fn upgrade_weights(self, kind: AiKind) -> Vec<f32> {
        let reg = &*UPGRADES;
        let count = reg.count();
        let mut weights = vec![0.0f32; count];

        // Helper to set weight by category + stat
        let mut set = |cat: &str, stat: UpgradeStatKind, w: f32| {
            if let Some(idx) = reg.index(cat, stat) {
                weights[idx] = w;
            }
        };

        match kind {
            AiKind::Raider => {
                // Raiders upgrade Archer + Fighter stats
                let (hp, atk, rng, aspd, mspd, stam, exp) = match self {
                    Self::Economic => (4., 4., 0., 4., 6., 3., 2.),
                    _ => (4., 6., 2., 6., 4., 3., 2.),
                };
                for cat in ["Archer", "Fighter"] {
                    set(cat, UpgradeStatKind::Hp, hp);
                    set(cat, UpgradeStatKind::Attack, atk);
                    set(cat, UpgradeStatKind::Range, rng);
                    set(cat, UpgradeStatKind::AttackSpeed, aspd);
                    set(cat, UpgradeStatKind::MoveSpeed, mspd);
                    set(cat, UpgradeStatKind::Stamina, stam);
                }
                set("Town", UpgradeStatKind::Expansion, exp);
            }
            AiKind::Builder => {
                // Builder AI upgrades everything
                let (aggr, bal, econ) = (
                    // Archer/Fighter: hp, atk, rng, aspd, mspd, pspd, plife, stam
                    (6., 8., 4., 6., 4., 3., 3., 3.),
                    (5., 5., 2., 4., 3., 2., 2., 4.),
                    (3., 2., 1., 2., 2., 1., 1., 2.),
                );
                let m = match self {
                    Self::Aggressive => aggr,
                    Self::Balanced => bal,
                    Self::Economic => econ,
                };
                for cat in ["Archer", "Fighter"] {
                    set(cat, UpgradeStatKind::Hp, m.0);
                    set(cat, UpgradeStatKind::Attack, m.1);
                    set(cat, UpgradeStatKind::Range, m.2);
                    set(cat, UpgradeStatKind::AttackSpeed, m.3);
                    set(cat, UpgradeStatKind::MoveSpeed, m.4);
                    set(cat, UpgradeStatKind::ProjectileSpeed, m.5);
                    set(cat, UpgradeStatKind::ProjectileLifetime, m.6);
                    set(cat, UpgradeStatKind::Stamina, m.7);
                }

                // Crossbow: hp, atk, rng, aspd, mspd, stam
                let x = match self {
                    Self::Aggressive => (5., 7., 3., 5., 3., 3.),
                    Self::Balanced => (4., 4., 2., 3., 2., 3.),
                    Self::Economic => (2., 2., 1., 1., 1., 1.),
                };
                set("Crossbow", UpgradeStatKind::Hp, x.0);
                set("Crossbow", UpgradeStatKind::Attack, x.1);
                set("Crossbow", UpgradeStatKind::Range, x.2);
                set("Crossbow", UpgradeStatKind::AttackSpeed, x.3);
                set("Crossbow", UpgradeStatKind::MoveSpeed, x.4);
                set("Crossbow", UpgradeStatKind::Stamina, x.5);

                // Farmer: yield, hp, mspd, stam
                let f = match self {
                    Self::Aggressive => (2., 1., 0., 1.),
                    Self::Balanced => (5., 3., 1., 3.),
                    Self::Economic => (8., 5., 2., 5.),
                };
                set("Farmer", UpgradeStatKind::Yield, f.0);
                set("Farmer", UpgradeStatKind::Hp, f.1);
                set("Farmer", UpgradeStatKind::MoveSpeed, f.2);
                set("Farmer", UpgradeStatKind::Stamina, f.3);

                // Miner: hp, mspd, yield, stam
                let mn = match self {
                    Self::Aggressive => (1., 0., 1., 0.),
                    Self::Balanced => (3., 1., 2., 2.),
                    Self::Economic => (5., 2., 4., 3.),
                };
                set("Miner", UpgradeStatKind::Hp, mn.0);
                set("Miner", UpgradeStatKind::MoveSpeed, mn.1);
                set("Miner", UpgradeStatKind::Yield, mn.2);
                set("Miner", UpgradeStatKind::Stamina, mn.3);

                // Town
                let t = match self {
                    Self::Aggressive => (1., 5., 6., 5., 8.),
                    Self::Balanced => (3., 5., 4., 4., 10.),
                    Self::Economic => (5., 4., 3., 3., 12.),
                };
                set("Town", UpgradeStatKind::Healing, t.0);
                set("Town", UpgradeStatKind::FountainRange, t.1);
                set("Town", UpgradeStatKind::FountainAttackSpeed, t.2);
                set("Town", UpgradeStatKind::FountainProjectileLife, t.3);
                set("Town", UpgradeStatKind::Expansion, t.4);
            }
        }
        weights
    }
}

/// Format top N scores for debug logging.
fn format_top_scores(scores: &[(AiAction, f32)], n: usize) -> String {
    let mut sorted: Vec<_> = scores.iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    sorted
        .iter()
        .take(n)
        .map(|(a, s)| format!("{}={:.1}", a.label(), s))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Weighted random selection from scored actions.
fn weighted_pick(scores: &[(AiAction, f32)]) -> Option<AiAction> {
    // Standard weighted-random draw:
    // 1) sum all weights
    // 2) roll in [0,total)
    // 3) walk cumulative weight until roll falls inside a bucket
    let total: f32 = scores.iter().map(|(_, s)| *s).sum();
    if total <= 0.0 {
        return None;
    }
    let roll = rand::rng().random_range(0.0..total);
    let mut acc = 0.0;
    for &(action, score) in scores {
        acc += score;
        if roll < acc {
            return Some(action);
        }
    }
    scores.last().map(|(a, _)| *a)
}

/// Desire signals used by action + upgrade scoring.
#[derive(Clone, Copy, Default)]
struct DesireState {
    food_desire: f32,     // 0.0 = comfortable, 1.0 = urgent
    military_desire: f32, // 0.0 = comfortable, 1.0 = urgent
    gold_desire: f32,     // 0.0 = gold abundant, 1.0 = urgent need for upgrades
    economy_desire: f32,  // 0.0 = town full, 1.0 = town empty (need to fill slots)
}

/// Compute food and military desire once per decision tick.
/// `civilians`/`military` are alive NPC counts for this town.
fn desire_state(
    personality: AiPersonality,
    food: i32,
    reserve: i32,
    houses: usize,
    barracks: usize,
    waypoints: usize,
    threat: f32,
    civilians: usize,
    military: usize,
) -> DesireState {
    let mut food_desire = if reserve > 0 {
        (1.0 - (food - reserve) as f32 / reserve as f32).clamp(0.0, 1.0)
    } else if food < 5 {
        0.8
    } else if food < 10 {
        0.4
    } else {
        0.0
    };

    let barracks_target = personality.archer_home_target(houses).max(1);
    let barracks_gap = barracks_target.saturating_sub(barracks) as f32 / barracks_target as f32;
    let waypoint_gap = if barracks > 0 {
        // Cap at 0.5 so waypoint deficit can't snowball military_desire
        (barracks.saturating_sub(waypoints) as f32 / barracks as f32).min(0.5)
    } else {
        0.0
    };
    // Barracks deficit is primary military pressure; waypoint coverage secondary;
    // threat from GPU spatial grid adds direct enemy presence signal.
    let mut military_desire = (barracks_gap * 0.75 + waypoint_gap * 0.25 + threat).clamp(0.0, 1.0);

    // Population ratio correction: dampen the underweight side's desire.
    // Only applies once the town has enough NPCs to meaningfully measure ratios.
    let total_pop = civilians + military;
    if total_pop >= 10 {
        let actual_ratio = military as f32 / total_pop as f32;
        let target = personality.target_military_ratio();
        if actual_ratio < target {
            // Under-military: boost military, dampen food
            let ratio_health = (actual_ratio / target).min(1.0);
            food_desire *= ratio_health;
            military_desire = (military_desire + (1.0 - ratio_health)).clamp(0.0, 1.0);
        } else if actual_ratio > target {
            // Over-military: dampen military, boost food
            let excess = ((actual_ratio - target) / (1.0 - target)).min(1.0);
            military_desire *= 1.0 - excess;
            food_desire = (food_desire + excess * 0.5).clamp(0.0, 1.0);
        }
    }

    DesireState {
        food_desire,
        military_desire,
        gold_desire: 0.0,
        economy_desire: 0.0,
    }
}

/// Shared food/military desire calculator for UI/debug surfaces.
/// Keeps diagnostics aligned with live AI logic.
pub fn debug_food_military_desire(
    personality: AiPersonality,
    food: i32,
    reserve: i32,
    houses: usize,
    barracks: usize,
    waypoints: usize,
    threat: f32,
    civilians: usize,
    military: usize,
) -> (f32, f32) {
    let d = desire_state(
        personality,
        food,
        reserve,
        houses,
        barracks,
        waypoints,
        threat,
        civilians,
        military,
    );
    (d.food_desire, d.military_desire)
}

fn is_military_upgrade(idx: usize) -> bool {
    let cat = UPGRADES.nodes[idx].category;
    cat == "Archer" || cat == "Fighter" || cat == "Crossbow"
}

/// Cheapest gold cost among upgrades the AI wants but can't afford.
/// Returns 0 if no gold-costing upgrades are wanted or all are affordable.
pub fn cheapest_gold_upgrade_cost(weights: &[f32], levels: &[u8], gold: i32) -> i32 {
    let mut cheapest = i32::MAX;
    for (idx, &w) in weights.iter().enumerate() {
        if w <= 0.0 {
            continue;
        }
        if !upgrade_unlocked(levels, idx) {
            continue;
        }
        let node = &UPGRADES.nodes[idx];
        let lv = levels.get(idx).copied().unwrap_or(0);
        let scale = upgrade_cost(lv);
        for &(kind, base) in node.cost {
            if kind == ResourceKind::Gold {
                let total = base * scale;
                if total > gold && total < cheapest {
                    cheapest = total;
                }
            }
        }
    }
    if cheapest == i32::MAX { 0 } else { cheapest }
}

/// Per-squad AI command state -- independent cooldown and target memory.
#[derive(Clone, Default)]
pub struct AiSquadCmdState {
    /// Target building UID (stable identity, survives slot reuse).
    pub building_uid: Option<Entity>,
    /// Seconds remaining before retarget is allowed.
    pub cooldown: f32,
}

/// Attack vs reserve role for multi-squad personalities.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SquadRole {
    Attack,
    Reserve,
    Idle,
}

pub struct AiPlayer {
    pub town_data_idx: usize,
    pub kind: AiKind,
    pub personality: AiPersonality,
    pub road_style: RoadStyle,
    pub last_actions: VecDeque<(String, i32, i32)>,
    pub active: bool,
    pub build_enabled: bool,
    pub upgrade_enabled: bool,
    /// Indices into SquadState.squads owned by this AI.
    pub squad_indices: Vec<usize>,
    /// Per-squad command state keyed by squad index.
    pub squad_cmd: HashMap<usize, AiSquadCmdState>,
}

const MAX_ACTION_HISTORY: usize = 20;

// Commander cooldowns (real-time seconds) per personality.
const AGGRESSIVE_RETARGET_COOLDOWN: f32 = 15.0;
const BALANCED_RETARGET_COOLDOWN: f32 = 25.0;
const ECONOMIC_RETARGET_COOLDOWN: f32 = 40.0;
const RETARGET_JITTER: f32 = 2.0;
// Search radius for enemy buildings from town center.
const AI_ATTACK_SEARCH_RADIUS: f32 = 10000.0;

#[derive(Resource, Default)]
pub struct AiPlayerState {
    pub players: Vec<AiPlayer>,
}
