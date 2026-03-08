//! AI player system — autonomous opponents that build and upgrade like the player.
//! Each AI has a personality (Aggressive/Balanced/Economic) that influences weighted
//! random decisions — same pattern as NPC behavior scoring.
//!
//! Slot selection: economy buildings (farms, houses, barracks) prefer inner slots
//! (closest to center). Waypoints form a single outer ring on the perimeter of the
//! build area, placed at block corners adjacent to road intersections. When the town
//! area expands, inner waypoints are pruned to maintain one ring.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use rand::Rng;

use crate::components::{Building, Dead, Job, TownId};
use crate::constants::UpgradeStatKind;
use crate::constants::*;
use crate::resources::*;
use crate::systemparams::WorldState;
use crate::systems::stats::{
    TownUpgrades, UPGRADES, expansion_cost, upgrade_available, upgrade_cost, upgrade_node,
    upgrade_unlocked,
};
use crate::world::{self, BuildingKind, WorldData, WorldGrid};

// Rust orientation notes for readers coming from PowerShell:
// - `Option<T>` is Rust's explicit nullable type (`Some(value)` or `None`).
// - `match` is a safe, exhaustive switch. The compiler ensures all cases are handled.
// - `&T` means "borrowed reference" (read-only by default). `&mut T` is mutable borrow.
// - Iterator chains (`iter().filter().map().collect()`) are the Rust equivalent of pipeline transforms.
// - The compiler enforces aliasing rules at compile time, replacing many runtime null/state checks.

/// Mutable world resources needed for AI building. Bundled to stay under Bevy's 16-param limit.
#[derive(SystemParam)]
pub struct AiBuildRes<'w, 's> {
    world: WorldState<'w>,
    food_storage: ResMut<'w, FoodStorage>,
    upgrade_queue: MessageWriter<'w, crate::systems::stats::UpgradeMsg>,
    gpu_updates: MessageWriter<'w, crate::messages::GpuUpdateMsg>,
    policies: ResMut<'w, TownPolicies>,
    commands: Commands<'w, 's>,
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

fn recalc_waypoint_patrol_order_clockwise(
    world_data: &mut WorldData,
    entity_map: &mut EntityMap,
    town_idx: u32,
) {
    // Rebuild patrol order from geometry, not history:
    // sort all living waypoints of this town by angle around town center.
    // This guarantees stable clockwise ordering after add/remove operations.
    let Some(center) = world_data.towns.get(town_idx as usize).map(|t| t.center) else {
        return;
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

    for (order, &(slot, _)) in entries.iter().enumerate() {
        if let Some(inst) = entity_map.get_instance_mut(slot) {
            inst.patrol_order = order as u32;
        }
    }
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

/// Road layout style — randomly assigned per AI town, independent of personality.
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
    /// Returns (disable_above, reenable_below) — higher food_desire = more food stress.
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
            // Economic: slight surplus, not exponential (was farms*2 → runaway spiral).
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
    fn waypoint_ring_slots(self, area_level: i32, center: Vec2, grid: &world::WorldGrid, road_style: RoadStyle) -> Vec<(usize, usize)> {
        let (min_c, max_c, min_r, max_r) = world::build_bounds(area_level, center, grid);
        let (cc, cr) = grid.world_to_grid(center);
        const MIN_SPACING: i32 = 5;

        // Walk perimeter clockwise: top→right→bottom→left (no duplicate corners)
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
            if (last.0 as i32 - first.0 as i32).abs() + (last.1 as i32 - first.1 as i32).abs() >= MIN_SPACING {
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

/// Per-squad AI command state — independent cooldown and target memory.
#[derive(Clone, Default)]
pub struct AiSquadCmdState {
    /// Target building UID (stable identity, survives slot reuse).
    pub building_uid: Option<crate::components::EntityUid>,
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

// ============================================================================
// SLOT SELECTION
// ============================================================================

/// Find best empty slot closest to town center (for economy buildings).
/// Excludes road and waypoint pattern slots for the given personality.
fn find_inner_slot(
    town_idx: usize,
    center: Vec2,
    area_level: i32,
    grid: &WorldGrid,
    entity_map: &EntityMap,
    personality: AiPersonality,
    road_style: RoadStyle,
) -> Option<(usize, usize)> {
    let (cc, cr) = grid.world_to_grid(center);
    let wp_slots: HashSet<(usize, usize)> = personality
        .waypoint_ring_slots(area_level, center, grid, road_style)
        .into_iter()
        .collect();
    world::empty_slots(town_idx, center, grid, entity_map)
        .into_iter()
        .filter(|&(c, r)| !road_style.is_road_slot(c, r, cc, cr) && !wp_slots.contains(&(c, r)))
        .min_by_key(|&(c, r)| {
            let dc = c as i32 - cc as i32;
            let dr = r as i32 - cr as i32;
            dc * dc + dr * dr
        })
}

fn build_town_snapshot(
    world_data: &WorldData,
    entity_map: &EntityMap,
    grid: &WorldGrid,
    town_data_idx: usize,
    personality: AiPersonality,
    road_style: RoadStyle,
) -> Option<AiTownSnapshot> {
    let town = world_data.towns.get(town_data_idx)?;
    let center = town.center;
    let area_level = town.area_level;
    let ti = town_data_idx as u32;

    let (cc, cr) = grid.world_to_grid(center);
    let farms = entity_map
        .iter_kind_for_town(BuildingKind::Farm, ti)
        .map(|b| grid.world_to_grid(b.position))
        .collect();
    let farmer_homes = entity_map
        .iter_kind_for_town(BuildingKind::FarmerHome, ti)
        .map(|b| grid.world_to_grid(b.position))
        .collect();
    let archer_homes = entity_map
        .iter_kind_for_town(BuildingKind::ArcherHome, ti)
        .map(|b| grid.world_to_grid(b.position))
        .collect();
    let crossbow_homes = entity_map
        .iter_kind_for_town(BuildingKind::CrossbowHome, ti)
        .map(|b| grid.world_to_grid(b.position))
        .collect();
    let waypoint_ring = personality.waypoint_ring_slots(area_level, center, grid, road_style);
    let wp_slots: HashSet<(usize, usize)> = waypoint_ring.iter().copied().collect();
    let empty_slots = world::empty_slots(town_data_idx, center, grid, entity_map)
        .into_iter()
        .filter(|&(c, r)| !road_style.is_road_slot(c, r, cc, cr) && !wp_slots.contains(&(c, r)))
        .collect();

    Some(AiTownSnapshot {
        center,
        cc,
        cr,
        empty_slots,
        farms,
        farmer_homes,
        archer_homes,
        crossbow_homes,
        waypoint_ring,
    })
}

fn pick_best_empty_slot<F>(snapshot: &AiTownSnapshot, mut score: F) -> Option<(usize, usize)>
where
    F: FnMut((usize, usize)) -> i32,
{
    let mut best: Option<((usize, usize), i32)> = None;
    for &slot in &snapshot.empty_slots {
        let s = score(slot);
        if best.is_none_or(|(_, bs)| s > bs) {
            best = Some((slot, s));
        }
    }
    best.map(|(slot, _)| slot)
}

struct NeighborCounts {
    edge_farms: i32,
    diag_farms: i32,
    farmer_homes: i32,
    archer_homes: i32,
    crossbow_homes: i32,
}

fn count_neighbors(snapshot: &AiTownSnapshot, slot: (usize, usize)) -> NeighborCounts {
    let (col, row) = slot;
    let mut nc = NeighborCounts {
        edge_farms: 0,
        diag_farms: 0,
        farmer_homes: 0,
        archer_homes: 0,
        crossbow_homes: 0,
    };
    for dr in -1i32..=1 {
        for dc in -1i32..=1 {
            if dr == 0 && dc == 0 {
                continue;
            }
            let nc_col = col as i32 + dc;
            let nc_row = row as i32 + dr;
            if nc_col < 0 || nc_row < 0 {
                continue;
            }
            let n = (nc_col as usize, nc_row as usize);
            if snapshot.farms.contains(&n) {
                if dr == 0 || dc == 0 {
                    nc.edge_farms += 1;
                } else {
                    nc.diag_farms += 1;
                }
            }
            if snapshot.farmer_homes.contains(&n) {
                nc.farmer_homes += 1;
            }
            if snapshot.archer_homes.contains(&n) {
                nc.archer_homes += 1;
            }
            if snapshot.crossbow_homes.contains(&n) {
                nc.crossbow_homes += 1;
            }
        }
    }
    nc
}

fn farm_slot_score(snapshot: &AiTownSnapshot, slot: (usize, usize)) -> i32 {
    let (col, row) = slot;
    let nc = count_neighbors(snapshot, slot);
    let mut score = nc.edge_farms * 24 + nc.diag_farms * 12 + nc.farmer_homes * 8;

    // Shape bonus: check 2x2 blocks that could include this slot
    let ci = col as i32;
    let ri = row as i32;
    let two_by_two: [(i32, i32); 4] = [(0, 0), (-1, 0), (0, -1), (-1, -1)];
    for (oc, or) in two_by_two {
        let c0 = ci + oc;
        let r0 = ri + or;
        if c0 < 0 || r0 < 0 {
            continue;
        }
        let block = [
            (c0 as usize, r0 as usize),
            (c0 as usize + 1, r0 as usize),
            (c0 as usize, r0 as usize + 1),
            (c0 as usize + 1, r0 as usize + 1),
        ];
        let existing = block
            .iter()
            .filter(|&&b| b != slot && snapshot.farms.contains(&b))
            .count();
        if existing == 3 {
            score += 120;
        } else if existing == 2 {
            score += 30;
        }
    }

    if nc.edge_farms >= 2 {
        score += 30;
    }

    // Bootstrap: bias toward town center
    if snapshot.farms.is_empty() {
        let dc = col as i32 - snapshot.cc as i32;
        let dr = row as i32 - snapshot.cr as i32;
        let radial = dc * dc + dr * dr;
        score -= radial / 2;
    }
    score
}

fn balanced_farm_ray_score(snapshot: &AiTownSnapshot, slot: (usize, usize)) -> i32 {
    let (col, row) = slot;
    let cc = snapshot.cc as i32;
    let cr = snapshot.cr as i32;
    let dc = col as i32 - cc;
    let dr = row as i32 - cr;
    let radial = dc * dc + dr * dr;
    let on_axis = dc == 0 || dr == 0;
    let mut score = if on_axis {
        500 - radial * 4
    } else {
        -300 - radial
    };

    if on_axis {
        if dr == 0 && dc != 0 {
            // Horizontal ray: check continuity toward center
            let step = if dc > 0 { 1i32 } else { -1 };
            let prev = ((col as i32 - step) as usize, row);
            let next = ((col as i32 + step) as usize, row);
            if snapshot.farms.contains(&prev) {
                score += 220;
            }
            if snapshot.farms.contains(&next) {
                score += 40;
            }
        } else if dc == 0 && dr != 0 {
            // Vertical ray
            let step = if dr > 0 { 1i32 } else { -1 };
            let prev = (col, (row as i32 - step) as usize);
            let next = (col, (row as i32 + step) as usize);
            if snapshot.farms.contains(&prev) {
                score += 220;
            }
            if snapshot.farms.contains(&next) {
                score += 40;
            }
        }
    }

    score
}

fn farmer_home_border_score(snapshot: &AiTownSnapshot, slot: (usize, usize)) -> i32 {
    // Farmer homes should border farms; reject positions with no nearby farms.
    // Then reward stronger farm adjacency and moderate proximity to existing homes.
    let nc = count_neighbors(snapshot, slot);
    if nc.edge_farms == 0 && nc.diag_farms == 0 {
        // "Impossible" score so this candidate almost never wins:
        // i32::MIN/4 leaves headroom to avoid overflow if weights are added later.
        return i32::MIN / 4;
    }
    // Weighted linear score:
    // edge farm contact matters most, then diagonal farm contact, then home adjacency.
    nc.edge_farms * 90
        + nc.diag_farms * 35
        + nc.farmer_homes * 10
        + nc.archer_homes * 5
        + nc.crossbow_homes * 5
}

fn balanced_house_side_score(snapshot: &AiTownSnapshot, slot: (usize, usize)) -> i32 {
    let (col, row) = slot;
    let cc = snapshot.cc;
    let cr = snapshot.cr;
    let dc = col as i32 - cc as i32;
    let dr = row as i32 - cr as i32;
    let mut score = 0i32;
    let on_axis = dc == 0 || dr == 0;
    if on_axis {
        score -= 120;
    }

    for &(fc, fr) in &snapshot.farms {
        let fdc = fc as i32 - cc as i32;
        let fdr = fr as i32 - cr as i32;
        // Side-of-ray bonus: farms on vertical axis get houses at +/-1 col offset
        if fdc == 0 && fdr != 0 {
            if row == fr && (col as i32 - cc as i32 == 1 || col as i32 - cc as i32 == -1) {
                score += 260;
            }
        } else if fdr == 0 && fdc != 0 {
            if col == fc && (row as i32 - cr as i32 == 1 || row as i32 - cr as i32 == -1) {
                score += 260;
            }
        }

        let grid_steps = (col as i32 - fc as i32).abs() + (row as i32 - fr as i32).abs();
        if grid_steps == 1 {
            score += 20;
        }
    }

    for &(hc, hr) in &snapshot.farmer_homes {
        let d = (col as i32 - hc as i32).abs() + (row as i32 - hr as i32).abs();
        if d == 0 {
            score -= 200;
        } else if d == 1 {
            score -= 25;
        }
    }

    score
}

fn archer_fill_score(snapshot: &AiTownSnapshot, slot: (usize, usize)) -> i32 {
    // Archer homes act as defensive fillers:
    // prefer being near economic core, avoid over-clumping with other archer homes.
    let nc = count_neighbors(snapshot, slot);
    let near_farms = nc.edge_farms + nc.diag_farms;
    // Archers should protect economic core, but not stack on top of each other.
    let mut score =
        near_farms * 40 + nc.farmer_homes * 35 - nc.archer_homes * 20 - nc.crossbow_homes * 20;
    // Extra bonus for dense "value zone" (many farms/homes nearby).
    if near_farms + nc.farmer_homes >= 4 {
        score += 60;
    }
    score
}

fn miner_toward_mine_score(mine_positions: &[Vec2], grid: &WorldGrid, slot: (usize, usize), cc: usize, cr: usize) -> i32 {
    let dc = slot.0 as i32 - cc as i32;
    let dr = slot.1 as i32 - cr as i32;
    let radial = dc * dc + dr * dr;
    if mine_positions.is_empty() {
        return -radial;
    }
    let wp = grid.grid_to_world(slot.0, slot.1);
    let best = mine_positions
        .iter()
        .map(|m| (wp - *m).length_squared())
        .fold(f32::INFINITY, f32::min);
    -(best as i32) - radial
}

/// Find the next ideal waypoint slot from the personality's outer ring pattern.
/// Uses cached ring from snapshot if available, otherwise computes fresh.
fn find_waypoint_slot(
    area_level: i32,
    center: Vec2,
    grid: &WorldGrid,
    entity_map: &EntityMap,
    ti: u32,
    personality: AiPersonality,
    road_style: RoadStyle,
    cached_ring: Option<&[(usize, usize)]>,
) -> Option<(usize, usize)> {
    let computed;
    let ideal = match cached_ring {
        Some(ring) => ring,
        None => {
            computed = personality.waypoint_ring_slots(area_level, center, grid, road_style);
            &computed
        }
    };
    let existing: HashSet<(usize, usize)> = entity_map
        .iter_kind_for_town(BuildingKind::Waypoint, ti)
        .map(|b| grid.world_to_grid(b.position))
        .collect();

    ideal
        .iter()
        .copied()
        .filter(|slot| !existing.contains(slot))
        .find(|&(c, r)| !entity_map.has_building_at(c as i32, r as i32))
}

fn sync_town_perimeter_waypoints(
    world: &mut WorldState,
    combat_log: &mut MessageWriter<crate::messages::CombatLogMsg>,
    gpu_updates: &mut MessageWriter<crate::messages::GpuUpdateMsg>,
    damage_writer: &mut MessageWriter<crate::messages::DamageMsg>,
    game_time: &GameTime,
    town_data_idx: usize,
    personality: AiPersonality,
    road_style: RoadStyle,
) -> usize {
    // Keep exactly one perimeter ring, but only prune inner/old waypoints
    // after the new outer ring is fully established.
    let Some(town) = world.world_data.towns.get(town_data_idx) else {
        return 0;
    };
    let center = town.center;
    let area_level = town.area_level;
    let ti = town_data_idx as u32;
    let ideal_slots = personality.waypoint_ring_slots(area_level, center, &world.grid, road_style);
    if ideal_slots.is_empty() {
        return 0;
    }
    let ideal: HashSet<(usize, usize)> = ideal_slots.iter().copied().collect();

    let existing: HashSet<(usize, usize)> = world
        .entity_map
        .iter_kind_for_town(BuildingKind::Waypoint, ti)
        .map(|b| world.grid.world_to_grid(b.position))
        .collect();

    let outer_complete = ideal.iter().all(|&(c, r)| {
        existing.contains(&(c, r)) || world.entity_map.has_building_at(c as i32, r as i32)
    });
    if !outer_complete {
        return 0;
    }

    let mut prune_slots: Vec<(usize, usize)> = Vec::new();
    for &slot in &existing {
        if !ideal.contains(&slot) {
            prune_slots.push(slot);
        }
    }

    let mut removed = 0usize;
    for (col, row) in prune_slots {
        let building_gpu_slot = world
            .entity_map
            .iter_kind_for_town(BuildingKind::Waypoint, ti)
            .find(|b| world.grid.world_to_grid(b.position) == (col, row))
            .map(|b| b.slot);
        let Some(building_gpu_slot) = building_gpu_slot else {
            continue;
        };
        let Some(uid) = world.entity_map.uid_for_slot(building_gpu_slot) else {
            continue;
        };

        damage_writer.write(crate::messages::DamageMsg {
            target: uid,
            amount: f32::MAX,
            attacker: -1,
            attacker_faction: 0,
        });
        if world
            .destroy_building(
                combat_log,
                game_time,
                col,
                row,
                "waypoint pruned (not on outer ring)",
                gpu_updates,
            )
            .is_ok()
        {
            removed += 1;
        }
    }
    if removed > 0 {
        recalc_waypoint_patrol_order_clockwise(&mut world.world_data, &mut world.entity_map, ti);
    }
    removed
}

/// Dirty-flag-gated maintenance: keep in-town patrol waypoints on the building-driven perimeter.
pub fn sync_patrol_perimeter_system(
    mut world: WorldState,
    ai_state: Res<AiPlayerState>,
    mut combat_log: MessageWriter<crate::messages::CombatLogMsg>,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
    mut damage_writer: MessageWriter<crate::messages::DamageMsg>,
    game_time: Res<GameTime>,
    mut perimeter_dirty: ResMut<PerimeterSyncDirty>,
) {
    // Flag-gated system: only runs when perimeter_dirty_drain_system detected dirty messages.
    if !perimeter_dirty.0 {
        return;
    }
    perimeter_dirty.0 = false;

    let town_personalities: Vec<(usize, AiPersonality, RoadStyle)> = ai_state
        .players
        .iter()
        .filter(|p| p.active)
        .map(|p| (p.town_data_idx, p.personality, p.road_style))
        .collect();

    let mut removed_total = 0usize;
    for (town_idx, personality, road_style) in town_personalities {
        removed_total += sync_town_perimeter_waypoints(
            &mut world,
            &mut combat_log,
            &mut gpu_updates,
            &mut damage_writer,
            &game_time,
            town_idx,
            personality,
            road_style,
        );
    }

    if removed_total > 0 {
        world
            .dirty_writers
            .patrols
            .write(crate::messages::PatrolsDirtyMsg);
        world
            .dirty_writers
            .building_grid
            .write(crate::messages::BuildingGridDirtyMsg);
    }
}

// ============================================================================
// MINE ANALYSIS (single-pass over gold_mines per AI tick)
// ============================================================================

/// Pre-computed mine stats for one AI town. Built once per tick, used by scoring + execution.
struct MineAnalysis {
    in_radius: usize,
    outside_radius: usize,
    /// All alive mine positions on the map (for miner home slot scoring).
    all_positions: Vec<Vec2>,
}

fn analyze_mines(entity_map: &EntityMap, center: Vec2, mining_radius: f32) -> MineAnalysis {
    // Single-pass analysis over alive gold mines.
    let radius_sq = mining_radius * mining_radius;
    let mut in_radius = 0usize;
    let mut outside_radius = 0usize;
    let mut all_positions = Vec::new();

    for m in entity_map.iter_kind(BuildingKind::GoldMine) {
        all_positions.push(m.position);
        if (m.position - center).length_squared() <= radius_sq {
            in_radius += 1;
        } else {
            outside_radius += 1;
        }
    }

    MineAnalysis {
        in_radius,
        outside_radius,
        all_positions,
    }
}

/// Per-tick derived context for one AI town. Built once before scoring/execution.
struct TownContext {
    center: Vec2,
    ti: u32,
    tdi: usize,
    area_level: i32,
    food: i32,
    has_slots: bool,
    slot_fullness: f32,
    mines: Option<MineAnalysis>,
}

impl TownContext {
    fn build(
        tdi: usize,
        food: i32,
        snapshot: Option<&AiTownSnapshot>,
        res: &AiBuildRes,
        kind: AiKind,
        mining_radius: f32,
    ) -> Option<Self> {
        let town = res.world.world_data.towns.get(tdi)?;
        let center = snapshot.map(|s| s.center).unwrap_or(town.center);
        let area_level = town.area_level;
        let ti = tdi as u32;
        let empty_count = snapshot
            .map(|s| s.empty_slots.len())
            .unwrap_or_else(|| {
                world::empty_slots(tdi, center, &res.world.grid, &res.world.entity_map).len()
            });
        let (min_c, max_c, min_r, max_r) = world::build_bounds(area_level, center, &res.world.grid);
        let total = ((max_c - min_c + 1) * (max_r - min_r + 1) - 1) as f32;
        let slot_fullness = 1.0 - empty_count as f32 / total.max(1.0);
        let mines = match kind {
            AiKind::Builder => Some(analyze_mines(&res.world.entity_map, center, mining_radius)),
            AiKind::Raider => None,
        };
        Some(Self {
            center,
            ti,
            tdi,
            area_level,
            food,
            has_slots: empty_count > 0,
            slot_fullness,
            mines,
        })
    }
}

// ============================================================================
// AI DECISION SYSTEM
// ============================================================================

/// One decision per AI per interval tick. Scores all eligible actions, picks via weighted random.
pub fn ai_decision_system(
    time: Res<Time>,
    config: Res<AiPlayerConfig>,
    mut ai_state: ResMut<AiPlayerState>,
    mut res: AiBuildRes,
    upgrades: Res<TownUpgrades>,
    gold_storage: Res<GoldStorage>,
    mut combat_log: MessageWriter<crate::messages::CombatLogMsg>,
    game_time: Res<GameTime>,
    difficulty: Res<Difficulty>,
    gpu_state: Res<GpuReadState>,
    pop_stats: Res<PopulationStats>,
    mut timer: Local<f32>,
    mut snapshots: Local<AiTownSnapshotCache>,
    settings: Res<crate::settings::UserSettings>,
    mut snapshot_dirty: ResMut<AiSnapshotDirty>,
) {
    // System timing gate:
    // runs every `decision_interval`, not every frame.
    *timer += game_time.delta(&time);
    if *timer < config.decision_interval {
        return;
    }
    *timer = 0.0;

    let dirty = snapshot_dirty.0;
    snapshot_dirty.0 = false;
    if dirty {
        snapshots.towns.clear();
        // Recompute spawner counts per town from EntityMap
        snapshots.spawner_counts.clear();
        for inst in res.world.entity_map.iter_instances() {
            if crate::constants::building_def(inst.kind).spawner.is_some() {
                *snapshots
                    .spawner_counts
                    .entry(inst.town_idx as usize)
                    .or_default() += 1;
            }
        }
    }

    for pi in 0..ai_state.players.len() {
        // Two-step style common in Rust ECS:
        // 1) gather immutable state and score actions
        // 2) perform one mutating action
        let player = &ai_state.players[pi];
        if !player.active {
            continue;
        }
        let tdi = player.town_data_idx;
        let personality = player.personality;
        let road_style = player.road_style;
        let build_enabled = player.build_enabled;
        let upgrade_enabled = player.upgrade_enabled;
        let kind = player.kind;
        let _ = player; // end immutable borrow — mutable access needed later
        if let std::collections::hash_map::Entry::Vacant(e) = snapshots.towns.entry(tdi) {
            if let Some(snap) = build_town_snapshot(
                &res.world.world_data,
                &res.world.entity_map,
                &res.world.grid,
                tdi,
                personality,
                road_style,
            ) {
                e.insert(snap);
            }
        }

        let food = res.food_storage.food.get(tdi).copied().unwrap_or(0);
        let spawner_count = snapshots.spawner_counts.get(&tdi).copied().unwrap_or(0);
        let reserve = personality.food_reserve_per_spawner() * spawner_count;
        // Desire signals are computed once below and reused by action + upgrade scoring.
        let mining_radius = res
            .policies
            .policies
            .get(tdi)
            .map(|p| p.mining_radius)
            .unwrap_or(crate::constants::DEFAULT_MINING_RADIUS);
        let Some(ctx) = TownContext::build(
            tdi,
            food,
            snapshots.towns.get(&tdi),
            &res,
            kind,
            mining_radius,
        ) else {
            continue;
        };

        let town_name = res
            .world
            .world_data
            .towns
            .get(tdi)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        let pname = personality.name();

        // Pre-compute mine_shafts before bc closure to allow mutable borrow for bootstrap.
        let mine_shafts = res
            .world
            .entity_map
            .count_for_town(BuildingKind::MinerHome, ctx.ti);

        // Deterministic miner bootstrap: bypasses food reserve gate.
        // Ensures min miner homes are built before the town can starve its gold economy.
        if matches!(kind, AiKind::Builder)
            && ctx.has_slots
            && mine_shafts < personality.min_miner_homes()
            && food >= building_cost(BuildingKind::MinerHome)
        {
            if let Some(mines) = ctx.mines.as_ref().filter(|m| m.in_radius + m.outside_radius > 0) {
                if let Some(what) = try_build_miner_home(
                    &ctx,
                    mines,
                    &mut res,
                    snapshots.towns.get(&tdi),
                    personality,
                    road_style,
                ) {
                    snapshots.towns.remove(&tdi);
                    let faction = res
                        .world
                        .world_data
                        .towns
                        .get(tdi)
                        .map(|t| t.faction)
                        .unwrap_or(0);
                    log_ai(
                        &mut combat_log,
                        &game_time,
                        faction,
                        &town_name,
                        pname,
                        &what,
                    );
                    let actions = &mut ai_state.players[pi].last_actions;
                    if actions.len() >= MAX_ACTION_HISTORY {
                        actions.pop_front();
                    }
                    actions.push_back((what, game_time.day(), game_time.hour()));
                    continue;
                }
            }
        }

        // Hoard food for miner home: if bootstrap didn't fire (food < 4), block all spending.
        if matches!(kind, AiKind::Builder)
            && ctx.has_slots
            && mine_shafts < personality.min_miner_homes()
            && food < building_cost(BuildingKind::MinerHome)
        {
            if ctx
                .mines
                .as_ref()
                .is_some_and(|m| m.in_radius + m.outside_radius > 0)
            {
                continue;
            }
        }

        // Food reserve rule: if town is at/below reserve, skip spending this tick.
        if food <= reserve {
            continue;
        }

        let bc = |k: BuildingKind| res.world.entity_map.count_for_town(k, ctx.ti);
        let farms = bc(BuildingKind::Farm);
        let houses = bc(BuildingKind::FarmerHome);
        let barracks = bc(BuildingKind::ArcherHome);
        let xbow_homes = bc(BuildingKind::CrossbowHome);
        let waypoints = bc(BuildingKind::Waypoint);
        let total_military_homes = barracks + xbow_homes;
        let faction = res
            .world
            .world_data
            .towns
            .get(tdi)
            .map(|t| t.faction)
            .unwrap_or(0);
        // Threat signal from GPU spatial grid: fountain's enemy count from readback.
        let threat = res
            .world
            .entity_map
            .iter_kind_for_town(BuildingKind::Fountain, tdi as u32)
            .next()
            .map(|inst| inst.slot)
            .and_then(|slot| gpu_state.threat_counts.get(slot).copied())
            .map(|packed| {
                let enemies = (packed >> 16) as f32;
                (enemies / 10.0).min(1.0)
            })
            .unwrap_or(0.0);
        // Count alive civilians vs military for this town from PopulationStats.
        let town_key = tdi as i32;
        let pop_alive = |job: Job| {
            pop_stats
                .0
                .get(&(job as i32, town_key))
                .map(|p| p.alive)
                .unwrap_or(0)
                .max(0) as usize
        };
        let civilians = pop_alive(Job::Farmer) + pop_alive(Job::Miner);
        let military = pop_alive(Job::Archer) + pop_alive(Job::Fighter) + pop_alive(Job::Crossbow);
        let mut desires = desire_state(
            personality,
            food,
            reserve,
            houses + mine_shafts,
            total_military_homes,
            waypoints,
            threat,
            civilians,
            military,
        );

        // Gold desire: driven by cheapest gold-costing upgrade the AI wants but can't afford.
        let uw = personality.upgrade_weights(kind);
        let levels = upgrades.town_levels(tdi);
        let gold = gold_storage.gold.get(tdi).copied().unwrap_or(0);
        let cheapest_gold = cheapest_gold_upgrade_cost(&uw, &levels, gold);
        desires.gold_desire = if cheapest_gold > 0 {
            ((1.0 - gold as f32 / cheapest_gold as f32) * personality.gold_desire_mult())
                .clamp(0.0, 1.0)
        } else {
            personality.base_mining_desire()
        };

        // Economy desire: how much the town needs to fill its buildable area.
        // Floors other desires so building scores never collapse to zero while slots remain.
        desires.economy_desire = 1.0 - ctx.slot_fullness;
        desires.food_desire = desires.food_desire.max(desires.economy_desire);
        desires.military_desire = desires.military_desire.max(desires.economy_desire);
        desires.gold_desire = desires.gold_desire.max(desires.economy_desire);

        // --- Policy: eat_food toggle based on food desire ---
        if let Some(policy) = res.policies.policies.get_mut(tdi) {
            let (off_threshold, on_threshold) = personality.eat_food_desire_thresholds();
            let should_eat = if policy.eat_food {
                desires.food_desire < off_threshold
            } else {
                desires.food_desire < on_threshold
            };
            if should_eat != policy.eat_food {
                policy.eat_food = should_eat;
                let state = if should_eat { "on" } else { "off" };
                log_ai(
                    &mut combat_log,
                    &game_time,
                    faction,
                    &town_name,
                    pname,
                    &format!(
                        "eat_food → {state} (food_desire={:.2})",
                        desires.food_desire
                    ),
                );
            }
        }

        let debug = settings.debug_ai_decisions;

        // ================================================================
        // Phase 1: Score and execute a BUILDING action
        // ================================================================
        if build_enabled {
        let mut build_scores: Vec<(AiAction, f32)> = Vec::with_capacity(8);

        match kind {
            AiKind::Raider => {
                // Raider AI has a smaller economy action set.
                if ctx.has_slots && ctx.food >= building_cost(BuildingKind::Tent) {
                    build_scores.push((AiAction::BuildTent, 30.0));
                }
            }
            AiKind::Builder => {
                // Builder AI scores economic + military + mining expansion actions.
                let (fw, hw, bw, gw) = personality.building_weights();
                let total_civilians = houses + mine_shafts;
                let bt = personality.archer_home_target(total_civilians);
                let ht = personality.farmer_home_target(farms);
                let Some(mines) = ctx.mines.as_ref() else { continue; };
                let ms_target = ((total_civilians as f32 * personality.mining_ratio()) as usize)
                    .max(mines.in_radius) // at least 1 miner per in-radius mine
                    .min(mines.in_radius * MAX_MINERS_PER_MINE);
                let house_deficit = ht.saturating_sub(houses);
                let barracks_deficit = bt.saturating_sub(barracks);
                let miner_deficit = ms_target.saturating_sub(mine_shafts);

                if ctx.has_slots {
                    // Desire-driven need model:
                    // food_desire gates farm/house construction,
                    // military_desire gates barracks/crossbow/waypoint construction.
                    // Base personality weights set ratios within each category.
                    let farm_need = desires.food_desire * (houses as f32 - farms as f32).max(0.0);
                    let house_need = if house_deficit > 0 {
                        desires.food_desire * (house_deficit as f32).min(10.0)
                    } else {
                        desires.food_desire * 0.5 // baseline to match military's 0.5 floor
                    };
                    let barracks_need = if barracks_deficit > 0 {
                        desires.military_desire * barracks_deficit as f32
                    } else {
                        desires.military_desire * 0.5
                    };

                    if ctx.food >= building_cost(BuildingKind::Farm) {
                        build_scores.push((AiAction::BuildFarm, fw * farm_need));
                    }
                    if ctx.food >= building_cost(BuildingKind::FarmerHome) {
                        build_scores.push((AiAction::BuildFarmerHome, hw * house_need));
                    }
                    if ctx.food >= building_cost(BuildingKind::ArcherHome) {
                        build_scores.push((AiAction::BuildArcherHome, bw * barracks_need));
                    }
                    // Crossbow homes: AI builds them once it has some archer homes established
                    if barracks >= 2 && ctx.food >= building_cost(BuildingKind::CrossbowHome) {
                        let xbow_need = if xbow_homes < barracks / 2 {
                            desires.military_desire * barracks.saturating_sub(xbow_homes * 2) as f32
                        } else {
                            desires.military_desire * 0.5
                        };
                        build_scores.push((AiAction::BuildCrossbowHome, bw * 0.6 * xbow_need));
                    }
                    if miner_deficit > 0 && ctx.food >= building_cost(BuildingKind::MinerHome) {
                        let ms_need = desires.gold_desire * miner_deficit as f32;
                        // Bootstrap boost: guarantee min miner homes per personality
                        let bootstrap = if mine_shafts < personality.min_miner_homes() {
                            5.0
                        } else {
                            1.0
                        };
                        build_scores.push((AiAction::BuildMinerHome, hw * ms_need * bootstrap));
                    } else if miner_deficit == 0
                        && mines.outside_radius > 0
                        && mine_shafts >= mines.in_radius
                    {
                        let expand_need = desires.gold_desire * mines.outside_radius as f32;
                        build_scores.push((
                            AiAction::ExpandMiningRadius,
                            personality.expand_mining_weight() * expand_need,
                        ));
                    }
                }

                let perimeter_target = snapshots
                    .towns
                    .get(&tdi)
                    .map(|s| s.waypoint_ring.len())
                    .unwrap_or(total_military_homes);
                let waypoint_target = total_military_homes.max(perimeter_target);
                if ctx.food >= building_cost(BuildingKind::Waypoint) && waypoints < waypoint_target
                {
                    let gp_need = desires.military_desire * (waypoint_target - waypoints) as f32;
                    build_scores.push((AiAction::BuildWaypoint, gw * gp_need));
                }

                // Roads: build roads using the town's road style
                let rw = personality.road_weight();
                if road_style != RoadStyle::None
                    && rw > 0.0
                    && ctx.food >= building_cost(BuildingKind::Road) * 4
                {
                    let road_candidates = count_road_candidates(
                        &res.world.entity_map,
                        ctx.area_level,
                        ctx.center,
                        &res.world.grid,
                        ctx.ti,
                        road_style,
                    );
                    if road_candidates > 0 {
                        let roads = bc(BuildingKind::Road) + bc(BuildingKind::StoneRoad) + bc(BuildingKind::MetalRoad);
                        let economy_buildings = farms + houses + mine_shafts;
                        let road_need =
                            road_candidates.min(economy_buildings.saturating_sub(roads / 2));
                        if road_need > 0 {
                            build_scores.push((AiAction::BuildRoads, rw * road_need as f32));
                        }
                    }
                }
            }
        }

        // Retry loop: if picked action fails, remove it and re-pick from remaining.
        let mut build_succeeded = false;
        loop {
            let Some(action) = weighted_pick(&build_scores) else {
                break;
            };
            let label = execute_action(
                action,
                &ctx,
                &mut res,
                snapshots.towns.get(&tdi),
                personality,
                road_style,
                *difficulty,
            );
            if let Some(what) = label {
                snapshots.towns.remove(&tdi);
                log_ai(
                    &mut combat_log,
                    &game_time,
                    faction,
                    &town_name,
                    pname,
                    &what,
                );
                let actions = &mut ai_state.players[pi].last_actions;
                if actions.len() >= MAX_ACTION_HISTORY {
                    actions.pop_front();
                }
                actions.push_back((what, game_time.day(), game_time.hour()));
                build_succeeded = true;
                break;
            }
            // Action failed — log and remove this variant from candidates
            if debug {
                let msg = format!(
                    "[dbg] {} FAILED ({})",
                    action.label(),
                    format_top_scores(&build_scores, 4)
                );
                let actions = &mut ai_state.players[pi].last_actions;
                if actions.len() >= MAX_ACTION_HISTORY {
                    actions.pop_front();
                }
                actions.push_back((msg, game_time.day(), game_time.hour()));
            }
            let failed = std::mem::discriminant(&action);
            build_scores.retain(|(a, _)| std::mem::discriminant(a) != failed);
        }
        if !build_succeeded && debug {
            if build_scores.is_empty() {
                let actions = &mut ai_state.players[pi].last_actions;
                if actions.len() >= MAX_ACTION_HISTORY {
                    actions.pop_front();
                }
                actions.push_back((
                    "[dbg] no build candidates".into(),
                    game_time.day(),
                    game_time.hour(),
                ));
            }
        }

        } // build_enabled

        // ================================================================
        // Phase 2: Score and execute an UPGRADE action (if food/gold remain)
        // ================================================================
        if upgrade_enabled {
        let food_after = res.food_storage.food.get(tdi).copied().unwrap_or(0);
        let gold_after = gold_storage.gold.get(tdi).copied().unwrap_or(0);
        // Gold reservation: when no empty slots, reserve gold for expansion upgrade.
        let expansion_gold_reserve = if !ctx.has_slots {
            uw.iter()
                .enumerate()
                .filter(|&(_, &w)| w > 0.0)
                .filter(|&(idx, _)| UPGRADES.nodes[idx].triggers_expansion)
                .filter(|&(idx, _)| upgrade_unlocked(&levels, idx))
                .map(|(idx, _)| {
                    let lv = levels.get(idx).copied().unwrap_or(0);
                    let node = &UPGRADES.nodes[idx];
                    if node.custom_cost {
                        expansion_cost(lv).1
                    } else {
                        let scale = upgrade_cost(lv);
                        node.cost
                            .iter()
                            .filter(|&&(kind, _)| kind == ResourceKind::Gold)
                            .map(|&(_, base)| base * scale)
                            .sum::<i32>()
                    }
                })
                .min()
                .unwrap_or(0)
        } else {
            0
        };
        if food_after > reserve {
            let mut upgrade_scores: Vec<(AiAction, f32)> = Vec::with_capacity(8);
            for (idx, &weight) in uw.iter().enumerate() {
                if weight <= 0.0 {
                    continue;
                }
                if !upgrade_available(&levels, idx, food_after, gold_after) {
                    continue;
                }
                // Fill slots first: only expansion upgrades allowed while town has empty slots
                let is_expansion = UPGRADES.nodes[idx].triggers_expansion;
                if ctx.has_slots && !is_expansion {
                    continue;
                }
                // Hoard gold for expansion: skip non-expansion gold-costing upgrades
                // unless we have surplus gold beyond what expansion needs.
                if !ctx.has_slots && !is_expansion && expansion_gold_reserve > 0 {
                    let lv = levels.get(idx).copied().unwrap_or(0);
                    let node = &UPGRADES.nodes[idx];
                    let gold_cost: i32 = if node.custom_cost {
                        expansion_cost(lv).1
                    } else {
                        let scale = upgrade_cost(lv);
                        node.cost
                            .iter()
                            .filter(|&&(kind, _)| kind == ResourceKind::Gold)
                            .map(|&(_, base)| base * scale)
                            .sum()
                    };
                    if gold_cost > 0 && gold_after - gold_cost < expansion_gold_reserve {
                        continue;
                    }
                }
                let mut w = weight;
                if is_military_upgrade(idx) {
                    w *= 1.0 + desires.military_desire * 2.0;
                }
                if UPGRADES.nodes[idx].triggers_expansion {
                    // Delay expansion while town still has empty slots and can afford buildings.
                    // Previous check only looked at home targets — missed farms, waypoints, roads.
                    if matches!(kind, AiKind::Builder) && ctx.has_slots {
                        let cheapest = building_cost(BuildingKind::Farm)
                            .min(building_cost(BuildingKind::FarmerHome))
                            .min(building_cost(BuildingKind::ArcherHome))
                            .min(building_cost(BuildingKind::MinerHome));
                        if food_after >= cheapest {
                            continue;
                        }
                    }
                    if ctx.slot_fullness > 0.7 {
                        w *= 2.0 + 4.0 * (ctx.slot_fullness - 0.7) / 0.3;
                    }
                    if !ctx.has_slots {
                        w *= 10.0;
                    }
                }
                upgrade_scores.push((AiAction::Upgrade(idx), w));
            }

            if let Some(action) = weighted_pick(&upgrade_scores) {
                let label = execute_action(
                    action,
                    &ctx,
                    &mut res,
                    snapshots.towns.get(&tdi),
                    personality,
                    road_style,
                    *difficulty,
                );
                if label.is_some() {
                    snapshots.towns.remove(&tdi);
                }
                if let Some(what) = label {
                    log_ai(
                        &mut combat_log,
                        &game_time,
                        faction,
                        &town_name,
                        pname,
                        &what,
                    );
                    let actions = &mut ai_state.players[pi].last_actions;
                    if actions.len() >= MAX_ACTION_HISTORY {
                        actions.pop_front();
                    }
                    actions.push_back((what, game_time.day(), game_time.hour()));
                } else if debug {
                    let name = if let AiAction::Upgrade(idx) = action {
                        upgrade_node(idx).label
                    } else {
                        action.label()
                    };
                    let msg = format!("[dbg] upgrade {} FAILED", name);
                    let actions = &mut ai_state.players[pi].last_actions;
                    if actions.len() >= MAX_ACTION_HISTORY {
                        actions.pop_front();
                    }
                    actions.push_back((msg, game_time.day(), game_time.hour()));
                }
            }
        }
        } // upgrade_enabled
    }
}

fn try_build_at_slot(
    kind: BuildingKind,
    cost: i32,
    label: &str,
    tdi: usize,
    res: &mut AiBuildRes,
    col: usize,
    row: usize,
) -> Option<String> {
    let pos = res.world.grid.grid_to_world(col, row);
    res.world
        .place_building(
            &mut res.food_storage,
            kind,
            tdi,
            pos,
            cost,
            &mut res.gpu_updates,
            &mut res.commands,
        )
        .ok()
        .map(|_| format!("built {label}"))
}

fn pick_slot_from_snapshot_or_inner(
    snapshot: Option<&AiTownSnapshot>,
    town_idx: usize,
    center: Vec2,
    area_level: i32,
    grid: &WorldGrid,
    entity_map: &EntityMap,
    score: fn(&AiTownSnapshot, (usize, usize)) -> i32,
    personality: AiPersonality,
    road_style: RoadStyle,
) -> Option<(usize, usize)> {
    if let Some(snap) = snapshot {
        if let Some(slot) = pick_best_empty_slot(snap, |s| score(snap, s)) {
            return Some(slot);
        }
    }
    find_inner_slot(town_idx, center, area_level, grid, entity_map, personality, road_style)
}

fn try_build_inner(
    kind: BuildingKind,
    cost: i32,
    label: &str,
    tdi: usize,
    center: Vec2,
    res: &mut AiBuildRes,
    area_level: i32,
    personality: AiPersonality,
    road_style: RoadStyle,
) -> Option<String> {
    let (col, row) = find_inner_slot(
        tdi,
        center,
        area_level,
        &res.world.grid,
        &res.world.entity_map,
        personality,
        road_style,
    )?;
    try_build_at_slot(kind, cost, label, tdi, res, col, row)
}

fn try_build_scored(
    kind: BuildingKind,
    label: &str,
    tdi: usize,
    center: Vec2,
    res: &mut AiBuildRes,
    area_level: i32,
    snapshot: Option<&AiTownSnapshot>,
    score_fn: fn(&AiTownSnapshot, (usize, usize)) -> i32,
    personality: AiPersonality,
    road_style: RoadStyle,
) -> Option<String> {
    let (col, row) = pick_slot_from_snapshot_or_inner(
        snapshot,
        tdi,
        center,
        area_level,
        &res.world.grid,
        &res.world.entity_map,
        score_fn,
        personality,
        road_style,
    )?;
    try_build_at_slot(kind, building_cost(kind), label, tdi, res, col, row)
}

fn try_build_miner_home(
    ctx: &TownContext,
    mines: &MineAnalysis,
    res: &mut AiBuildRes,
    snapshot: Option<&AiTownSnapshot>,
    personality: AiPersonality,
    road_style: RoadStyle,
) -> Option<String> {
    // Miner homes are intentionally special-cased:
    // score depends on mine positions from per-tick MineAnalysis, not only local adjacency.
    let grid = &res.world.grid;
    let (cc, cr) = grid.world_to_grid(ctx.center);
    let slot = if let Some(snap) = snapshot {
        pick_best_empty_slot(snap, |s| {
            miner_toward_mine_score(&mines.all_positions, grid, s, cc, cr)
        })
        .or_else(|| {
            find_inner_slot(
                ctx.tdi,
                ctx.center,
                ctx.area_level,
                &res.world.grid,
                &res.world.entity_map,
                personality,
                road_style,
            )
        })
    } else {
        find_inner_slot(
            ctx.tdi,
            ctx.center,
            ctx.area_level,
            &res.world.grid,
            &res.world.entity_map,
            personality,
            road_style,
        )
    }?;
    try_build_at_slot(
        BuildingKind::MinerHome,
        building_cost(BuildingKind::MinerHome),
        "miner home",
        ctx.tdi,
        res,
        slot.0,
        slot.1,
    )
}

/// Count available road candidate slots (road-pattern slots near economy buildings, minus existing roads).
/// Used to gate road scoring so roads aren't scored when no candidates exist.
fn count_road_candidates(
    entity_map: &EntityMap,
    area_level: i32,
    center: Vec2,
    grid: &world::WorldGrid,
    ti: u32,
    road_style: RoadStyle,
) -> usize {
    let (cc, cr) = grid.world_to_grid(center);
    let econ_slots: Vec<(usize, usize)> = entity_map
        .iter_kind_for_town(BuildingKind::Farm, ti)
        .chain(entity_map.iter_kind_for_town(BuildingKind::FarmerHome, ti))
        .chain(entity_map.iter_kind_for_town(BuildingKind::MinerHome, ti))
        .map(|b| grid.world_to_grid(b.position))
        .collect();
    if econ_slots.is_empty() {
        return 0;
    }
    let road_slots: HashSet<(usize, usize)> = [BuildingKind::Road, BuildingKind::StoneRoad, BuildingKind::MetalRoad]
        .iter()
        .flat_map(|&kind| entity_map.iter_kind_for_town(kind, ti))
        .map(|b| grid.world_to_grid(b.position))
        .collect();
    let (min_c, max_c, min_r, max_r) = world::build_bounds(area_level, center, grid);
    // Cardinal: extend axes to 2× build radius for attack corridors
    let (ext_min_c, ext_max_c, ext_min_r, ext_max_r) = if road_style == RoadStyle::Cardinal {
        let half_c = cc - min_c;
        let half_r = cr - min_r;
        (cc.saturating_sub(half_c * 2), max_c + half_c, cr.saturating_sub(half_r * 2), max_r + half_r)
    } else {
        (min_c, max_c, min_r, max_r)
    };
    let mut count = 0usize;
    for row in ext_min_r..=ext_max_r {
        for col in ext_min_c..=ext_max_c {
            if !road_style.is_road_slot(col, row, cc, cr) {
                continue;
            }
            if road_slots.contains(&(col, row)) {
                continue;
            }
            if entity_map.has_building_at(col as i32, row as i32) {
                continue;
            }
            let in_bounds = col >= min_c && col <= max_c && row >= min_r && row <= max_r;
            let adj = econ_slots
                .iter()
                .any(|&(ec, er)| {
                    (ec as i32 - col as i32).abs() <= 2 && (er as i32 - row as i32).abs() <= 2
                });
            if adj || !in_bounds {
                count += 1;
            }
        }
    }
    count
}

/// Build roads around economy buildings using the town's road style.
fn try_build_road_grid(
    ctx: &TownContext,
    res: &mut AiBuildRes,
    batch_size: usize,
    road_style: RoadStyle,
) -> Option<String> {
    let cost = building_cost(BuildingKind::Road);
    let ti = ctx.ti;
    let grid = &res.world.grid;
    let (cc, cr) = grid.world_to_grid(ctx.center);
    let entity_map = &res.world.entity_map;

    let econ_slots: Vec<(usize, usize)> = entity_map
        .iter_kind_for_town(BuildingKind::Farm, ti)
        .chain(entity_map.iter_kind_for_town(BuildingKind::FarmerHome, ti))
        .chain(entity_map.iter_kind_for_town(BuildingKind::MinerHome, ti))
        .map(|b| grid.world_to_grid(b.position))
        .collect();
    if econ_slots.is_empty() {
        return None;
    }

    let road_slots: HashSet<(usize, usize)> = [BuildingKind::Road, BuildingKind::StoneRoad, BuildingKind::MetalRoad]
        .iter()
        .flat_map(|&kind| entity_map.iter_kind_for_town(kind, ti))
        .map(|b| grid.world_to_grid(b.position))
        .collect();

    let mut candidates: HashMap<(usize, usize), i32> = HashMap::new();
    let (min_c, max_c, min_r, max_r) = world::build_bounds(ctx.area_level, ctx.center, &res.world.grid);
    // Cardinal: extend axes to 2× build radius for attack corridors
    let (ext_min_c, ext_max_c, ext_min_r, ext_max_r) = if road_style == RoadStyle::Cardinal {
        let half_c = cc - min_c;
        let half_r = cr - min_r;
        (cc.saturating_sub(half_c * 2), max_c + half_c, cr.saturating_sub(half_r * 2), max_r + half_r)
    } else {
        (min_c, max_c, min_r, max_r)
    };

    for row in ext_min_r..=ext_max_r {
        for col in ext_min_c..=ext_max_c {
            if !road_style.is_road_slot(col, row, cc, cr) {
                continue;
            }
            if entity_map.has_building_at(col as i32, row as i32) {
                continue;
            }
            let in_bounds = col >= min_c && col <= max_c && row >= min_r && row <= max_r;
            let adj = econ_slots
                .iter()
                .filter(|&&(ec, er)| {
                    (ec as i32 - col as i32).abs() <= 2 && (er as i32 - row as i32).abs() <= 2
                })
                .count() as i32;
            if adj > 0 {
                candidates.insert((col, row), adj);
            } else if !in_bounds {
                candidates.insert((col, row), 1);
            }
        }
    }

    candidates.retain(|slot, _| !road_slots.contains(slot));

    // Sort by score (highest adjacency first), then by distance to center (closer first)
    let mut ranked: Vec<((usize, usize), i32)> = candidates.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.cmp(&a.1).then_with(|| {
            let da = (a.0 .0 as i32 - cc as i32).pow(2) + (a.0 .1 as i32 - cr as i32).pow(2);
            let db = (b.0 .0 as i32 - cc as i32).pow(2) + (b.0 .1 as i32 - cr as i32).pow(2);
            da.cmp(&db)
        })
    });

    let mut placed = 0usize;
    for &((col, row), _score) in ranked.iter().take(batch_size * 2) {
        if placed >= batch_size {
            break;
        }
        let food = res.food_storage.food.get(ctx.tdi).copied().unwrap_or(0);
        if food < cost {
            break;
        }

        let pos = res.world.grid.grid_to_world(col, row);
        if res
            .world
            .place_building(
                &mut res.food_storage,
                BuildingKind::Road,
                ctx.tdi,
                pos,
                cost,
                &mut res.gpu_updates,
                &mut res.commands,
            )
            .is_ok()
        {
            placed += 1;
        }
    }

    if placed > 0 {
        Some(format!("built {} roads", placed))
    } else {
        None
    }
}

/// Execute the chosen action, returning a log label on success.
fn execute_action(
    action: AiAction,
    ctx: &TownContext,
    res: &mut AiBuildRes,
    snapshot: Option<&AiTownSnapshot>,
    personality: AiPersonality,
    road_style: RoadStyle,
    _difficulty: Difficulty,
) -> Option<String> {
    // Action execution uses `match` on enum variant.
    // This gives explicit, compile-checked control flow per action type.
    match action {
        AiAction::BuildTent => try_build_inner(
            BuildingKind::Tent,
            building_cost(BuildingKind::Tent),
            "tent",
            ctx.tdi,
            ctx.center,
            res,
            ctx.area_level,
            personality,
            road_style,
        ),
        AiAction::BuildFarm => {
            let score = if personality == AiPersonality::Balanced {
                balanced_farm_ray_score
            } else {
                farm_slot_score
            };
            try_build_scored(
                BuildingKind::Farm,
                "farm",
                ctx.tdi,
                ctx.center,
                res,
                ctx.area_level,
                snapshot,
                score,
                personality,
                road_style,
            )
        }
        AiAction::BuildFarmerHome => {
            let score = if personality == AiPersonality::Balanced {
                balanced_house_side_score
            } else {
                farmer_home_border_score
            };
            try_build_scored(
                BuildingKind::FarmerHome,
                "farmer home",
                ctx.tdi,
                ctx.center,
                res,
                ctx.area_level,
                snapshot,
                score,
                personality,
                road_style,
            )
        }
        AiAction::BuildArcherHome => try_build_scored(
            BuildingKind::ArcherHome,
            "archer home",
            ctx.tdi,
            ctx.center,
            res,
            ctx.area_level,
            snapshot,
            archer_fill_score,
            personality,
            road_style,
        ),
        AiAction::BuildCrossbowHome => try_build_scored(
            BuildingKind::CrossbowHome,
            "crossbow home",
            ctx.tdi,
            ctx.center,
            res,
            ctx.area_level,
            snapshot,
            archer_fill_score,
            personality,
            road_style,
        ),
        AiAction::BuildMinerHome => {
            let Some(mines) = &ctx.mines else {
                return None;
            };
            try_build_miner_home(ctx, mines, res, snapshot, personality, road_style)
        }
        AiAction::ExpandMiningRadius => {
            // Policy action, not building placement.
            // Expands search radius for mines in fixed-size steps with max cap.
            let Some(policy) = res.policies.policies.get_mut(ctx.tdi) else {
                return None;
            };
            let old = policy.mining_radius;
            let new = (old + MINING_RADIUS_STEP).min(MAX_MINING_RADIUS);
            if new <= old {
                return None;
            }
            policy.mining_radius = new;
            res.world
                .dirty_writers
                .mining
                .write(crate::messages::MiningDirtyMsg);
            Some(format!("expanded mining radius to {:.0}px", new))
        }
        AiAction::BuildWaypoint => {
            let cost = building_cost(BuildingKind::Waypoint);
            let cached_ring = snapshot.map(|s| s.waypoint_ring.as_slice());
            let (col, row) = find_waypoint_slot(
                ctx.area_level,
                ctx.center,
                &res.world.grid,
                &res.world.entity_map,
                ctx.ti,
                personality,
                road_style,
                cached_ring,
            )?;
            let pos = res.world.grid.grid_to_world(col, row);
            if res
                .world
                .place_building(
                    &mut res.food_storage,
                    world::BuildingKind::Waypoint,
                    ctx.tdi,
                    pos,
                    cost,
                    &mut res.gpu_updates,
                    &mut res.commands,
                )
                .is_ok()
            {
                recalc_waypoint_patrol_order_clockwise(
                    &mut res.world.world_data,
                    &mut res.world.entity_map,
                    ctx.ti,
                );
                Some("built waypoint".into())
            } else {
                None
            }
        }
        AiAction::BuildRoads => {
            try_build_road_grid(ctx, res, personality.road_batch_size(), road_style)
        }
        AiAction::Upgrade(idx) => {
            res.upgrade_queue.write(crate::systems::stats::UpgradeMsg {
                town_idx: ctx.tdi,
                upgrade_idx: idx,
            });
            let name = upgrade_node(idx).label;
            Some(format!("upgraded {name}"))
        }
    }
}

fn log_ai(
    log: &mut MessageWriter<crate::messages::CombatLogMsg>,
    gt: &GameTime,
    faction: i32,
    town: &str,
    personality: &str,
    what: &str,
) {
    // Centralized AI log format so all decisions read consistently in the combat log.
    log.write(crate::messages::CombatLogMsg {
        kind: CombatEventKind::Ai,
        faction,
        day: gt.day(),
        hour: gt.hour(),
        minute: gt.minute(),
        message: format!("{} [{}] {}", town, personality, what),
        location: None,
    });
}

// ============================================================================
// AI SQUAD COMMANDER
// ============================================================================

/// Resolve a building's position by slot. Returns None if slot has no instance (dead/freed).
fn resolve_building_pos(entity_map: &EntityMap, uid: crate::components::EntityUid) -> Option<Vec2> {
    entity_map.instance_by_uid(uid).map(|inst| inst.position)
}

impl AiPersonality {
    fn retarget_cooldown(self) -> f32 {
        match self {
            Self::Aggressive => AGGRESSIVE_RETARGET_COOLDOWN,
            Self::Balanced => BALANCED_RETARGET_COOLDOWN,
            Self::Economic => ECONOMIC_RETARGET_COOLDOWN,
        }
    }

    fn attack_squad_count(self) -> usize {
        match self {
            Self::Aggressive => 2,
            Self::Balanced => 1,
            Self::Economic => 1,
        }
    }

    fn desired_squad_count(self) -> usize {
        1 + self.attack_squad_count()
    }

    /// Percent of town archers kept in squad[0] as patrol/defense.
    fn defense_share_pct(self) -> usize {
        match self {
            Self::Aggressive => 25,
            Self::Balanced => 45,
            Self::Economic => 65,
        }
    }

    /// Relative split for each attack squad (index within attack squads only).
    fn attack_split_weight(self, attack_idx: usize) -> usize {
        match self {
            Self::Aggressive => {
                if attack_idx == 0 {
                    55
                } else {
                    45
                }
            }
            Self::Balanced => 100,
            Self::Economic => 100,
        }
    }

    /// Preferred building kinds to attack, by personality and squad role.
    fn attack_kinds(self, role: SquadRole) -> &'static [BuildingKind] {
        match role {
            SquadRole::Reserve | SquadRole::Idle => &[], // non-attack squads don't attack
            SquadRole::Attack => match self {
                Self::Aggressive => &[
                    BuildingKind::Farm,
                    BuildingKind::FarmerHome,
                    BuildingKind::ArcherHome,
                    BuildingKind::CrossbowHome,
                    BuildingKind::Waypoint,
                    BuildingKind::Tent,
                    BuildingKind::MinerHome,
                ],
                Self::Balanced => &[
                    BuildingKind::ArcherHome,
                    BuildingKind::CrossbowHome,
                    BuildingKind::Waypoint,
                ],
                Self::Economic => &[BuildingKind::Farm],
            },
        }
    }

    /// Broad fallback set when preferred kinds yield no target.
    /// Fountain last priority — destroy the base after clearing defenses.
    fn fallback_attack_kinds() -> &'static [BuildingKind] {
        &[
            BuildingKind::Farm,
            BuildingKind::FarmerHome,
            BuildingKind::ArcherHome,
            BuildingKind::CrossbowHome,
            BuildingKind::Waypoint,
            BuildingKind::Tent,
            BuildingKind::MinerHome,
            BuildingKind::Fountain,
        ]
    }

    /// Minimum members before a wave can start.
    fn wave_min_start(self, kind: AiKind) -> usize {
        match kind {
            AiKind::Raider => RAID_GROUP_SIZE as usize,
            AiKind::Builder => match self {
                Self::Aggressive => 3,
                Self::Balanced => 5,
                Self::Economic => 8,
            },
        }
    }

    /// Loss threshold percent — end wave when alive drops below this % of wave_start_count.
    fn wave_retreat_pct(self, kind: AiKind) -> usize {
        match kind {
            AiKind::Raider => 30,
            AiKind::Builder => match self {
                Self::Aggressive => 25,
                Self::Balanced => 40,
                Self::Economic => 60,
            },
        }
    }
}

/// Pick nearest enemy farm as raider squad target.
fn pick_raider_farm_target(
    entity_map: &EntityMap,
    center: Vec2,
    faction: i32,
) -> Option<(BuildingKind, crate::components::EntityUid, Vec2)> {
    let mut best_d2 = f32::MAX;
    let mut result: Option<(BuildingKind, crate::components::EntityUid, Vec2)> = None;
    let r2 = AI_ATTACK_SEARCH_RADIUS * AI_ATTACK_SEARCH_RADIUS;
    entity_map.for_each_nearby(center, AI_ATTACK_SEARCH_RADIUS, |inst| {
        if inst.faction == faction || inst.faction == crate::constants::FACTION_NEUTRAL {
            return;
        }
        if inst.kind != BuildingKind::Farm {
            return;
        }
        let Some(uid) = entity_map.uid_for_slot(inst.slot) else {
            return;
        };
        let dx = inst.position.x - center.x;
        let dy = inst.position.y - center.y;
        let d2 = dx * dx + dy * dy;
        if d2 <= r2 && d2 < best_d2 {
            best_d2 = d2;
            result = Some((inst.kind, uid, inst.position));
        }
    });
    result
}

fn pick_ai_target_unclaimed(
    entity_map: &EntityMap,
    center: Vec2,
    faction: i32,
    personality: AiPersonality,
    role: SquadRole,
    claimed: &HashSet<crate::components::EntityUid>,
) -> Option<(BuildingKind, crate::components::EntityUid, Vec2)> {
    if role != SquadRole::Attack {
        return None;
    }

    let find_nearest_unclaimed = |allowed_kinds: &[BuildingKind]| -> Option<(BuildingKind, crate::components::EntityUid, Vec2)> {
        let mut best_d2 = f32::MAX;
        let mut result: Option<(BuildingKind, crate::components::EntityUid, Vec2)> = None;
        let r2 = AI_ATTACK_SEARCH_RADIUS * AI_ATTACK_SEARCH_RADIUS;
        entity_map.for_each_nearby(center, AI_ATTACK_SEARCH_RADIUS, |inst| {
            if inst.faction == faction || inst.faction == crate::constants::FACTION_NEUTRAL { return; }
            if !allowed_kinds.contains(&inst.kind) { return; }
            let Some(uid) = entity_map.uid_for_slot(inst.slot) else { return; };
            if claimed.contains(&uid) { return; }
            let dx = inst.position.x - center.x;
            let dy = inst.position.y - center.y;
            let d2 = dx * dx + dy * dy;
            if d2 <= r2 && d2 < best_d2 {
                best_d2 = d2;
                result = Some((inst.kind, uid, inst.position));
            }
        });
        result
    };

    let preferred = personality.attack_kinds(role);
    find_nearest_unclaimed(preferred)
        .or_else(|| find_nearest_unclaimed(AiPersonality::fallback_attack_kinds()))
}

/// Rebuild squad_indices for one AI player by scanning SquadState ownership.
pub fn rebuild_squad_indices(player: &mut AiPlayer, squads: &[Squad]) {
    player.squad_indices.clear();
    for (i, s) in squads.iter().enumerate() {
        if s.owner == SquadOwner::Town(player.town_data_idx) {
            player.squad_indices.push(i);
        }
    }
}

/// AI squad commander — wave-based attack cycle for both Builder and Raider AIs.
/// Sets shared squad knobs: target, target_size, patrol_enabled, rest_when_tired.
/// Wave model: gather → threshold → dispatch → detect end → reset.
pub fn ai_squad_commander_system(
    time: Res<Time>,
    mut ai_state: ResMut<AiPlayerState>,
    mut squad_state: ResMut<SquadState>,
    world_data: Res<WorldData>,
    entity_map: Res<EntityMap>,
    mut combat_log: MessageWriter<crate::messages::CombatLogMsg>,
    game_time: Res<GameTime>,
    mut squads_dirty_w: MessageWriter<crate::messages::SquadsDirtyMsg>,
    mut timer: Local<f32>,
    military_q: Query<(&Job, &TownId), (Without<Building>, Without<Dead>)>,
) {
    const AI_SQUAD_HEARTBEAT: f32 = 2.0;
    let dt = game_time.delta(&time);
    *timer += dt;
    if *timer < AI_SQUAD_HEARTBEAT {
        return;
    }
    let elapsed = *timer;
    *timer = 0.0;

    // Count alive military units per town.
    let mut units_by_town: HashMap<i32, usize> = HashMap::new();
    for (job, town_id) in military_q.iter() {
        if !job.is_military() {
            continue;
        }
        *units_by_town.entry(town_id.0).or_default() += 1;
    }

    for pi in 0..ai_state.players.len() {
        let player = &ai_state.players[pi];
        if !player.active {
            continue;
        }

        let tdi = player.town_data_idx;
        let personality = player.personality;
        let kind = player.kind;
        let Some(town) = world_data.towns.get(tdi) else {
            continue;
        };
        let center = town.center;
        let faction = town.faction;

        // --- Self-healing squad allocation ---
        let desired = match kind {
            AiKind::Builder => personality.desired_squad_count(),
            AiKind::Raider => 1, // single attack squad for raider towns
        };
        let owned: usize = squad_state
            .squads
            .iter()
            .filter(|s| s.owner == SquadOwner::Town(tdi))
            .count();
        if owned < desired {
            for _ in owned..desired {
                let idx = squad_state.alloc_squad(SquadOwner::Town(tdi));
                let base_cd = personality.retarget_cooldown();
                let jitter = rand::rng().random_range(0.3..1.0);
                let sq = squad_state.squads.get_mut(idx).expect("squad just allocated");
                sq.wave_min_start = personality.wave_min_start(kind);
                sq.wave_retreat_below_pct = personality.wave_retreat_pct(kind);
                ai_state.players[pi].squad_cmd.insert(
                    idx,
                    AiSquadCmdState {
                        building_uid: None,
                        cooldown: base_cd * jitter,
                    },
                );
            }
        }

        // Rebuild squad_indices from ownership scan.
        rebuild_squad_indices(&mut ai_state.players[pi], &squad_state.squads);
        let squad_indices = ai_state.players[pi].squad_indices.clone();
        if squad_indices.is_empty() {
            continue;
        }

        // --- Set target_size per squad ---
        let unit_count = units_by_town.get(&(tdi as i32)).copied().unwrap_or(0);

        match kind {
            AiKind::Raider => {
                // Raider towns: single squad gets all raiders
                if let Some(&si) = squad_indices.first() {
                    if let Some(squad) = squad_state.squads.get_mut(si) {
                        let new_size = unit_count;
                        if squad.target_size != new_size {
                            squad.target_size = new_size;
                            squads_dirty_w.write(crate::messages::SquadsDirtyMsg);
                        }
                        squad.patrol_enabled = false;
                        squad.rest_when_tired = false;
                    }
                }
            }
            AiKind::Builder => {
                // Builder AIs: defense + attack split
                let attack_squads = personality.attack_squad_count();
                let defense_size = unit_count * personality.defense_share_pct() / 100;
                let attack_total = unit_count.saturating_sub(defense_size);
                let total_attack_weight: usize = (0..attack_squads)
                    .map(|i| personality.attack_split_weight(i))
                    .sum::<usize>()
                    .max(1);

                for (role_idx, &si) in squad_indices.iter().enumerate() {
                    let role = if role_idx == 0 {
                        SquadRole::Reserve
                    } else if role_idx - 1 < attack_squads {
                        SquadRole::Attack
                    } else {
                        SquadRole::Idle
                    };

                    let new_target_size = match role {
                        SquadRole::Reserve => defense_size,
                        SquadRole::Attack => {
                            let attack_idx = role_idx - 1;
                            if attack_idx + 1 == attack_squads {
                                let allocated_before: usize = (0..attack_idx)
                                    .map(|i| {
                                        attack_total * personality.attack_split_weight(i)
                                            / total_attack_weight
                                    })
                                    .sum();
                                attack_total.saturating_sub(allocated_before)
                            } else {
                                attack_total * personality.attack_split_weight(attack_idx)
                                    / total_attack_weight
                            }
                        }
                        SquadRole::Idle => 0,
                    };

                    if let Some(squad) = squad_state.squads.get_mut(si) {
                        if squad.target_size != new_target_size {
                            squad.target_size = new_target_size;
                            squads_dirty_w.write(crate::messages::SquadsDirtyMsg);
                        }
                        let should_patrol = role == SquadRole::Reserve;
                        if squad.patrol_enabled != should_patrol {
                            squad.patrol_enabled = should_patrol;
                        }
                        if role != SquadRole::Attack && squad.target.is_some() {
                            squad.target = None;
                            squad.wave_active = false;
                        }
                        if !squad.rest_when_tired {
                            squad.rest_when_tired = true;
                        }
                    }
                }
            }
        }

        // --- Wave-based retarget for all attack squads ---
        let mut claimed_targets: HashSet<crate::components::EntityUid> = HashSet::new();
        for &si in &squad_indices {
            let cmd = ai_state.players[pi].squad_cmd.entry(si).or_default();
            if cmd.cooldown > 0.0 {
                cmd.cooldown -= elapsed;
            }

            let Some(squad) = squad_state.squads.get(si) else {
                continue;
            };

            // Determine if this squad is an attack squad
            let is_attack = match kind {
                AiKind::Raider => true, // raider town squads always attack
                AiKind::Builder => {
                    let role_idx = squad_indices.iter().position(|&i| i == si).unwrap_or(0);
                    let attack_squads = personality.attack_squad_count();
                    role_idx >= 1 && role_idx - 1 < attack_squads
                }
            };
            if !is_attack {
                cmd.building_uid = None;
                continue;
            }

            let member_count = squad.members.len();

            if squad.wave_active {
                // --- Wave end conditions ---
                let target_alive = cmd
                    .building_uid
                    .and_then(|uid| resolve_building_pos(&entity_map, uid))
                    .is_some();

                let loss_threshold = squad.wave_start_count * squad.wave_retreat_below_pct / 100;
                let heavy_losses = member_count < loss_threshold.max(1);

                if !target_alive || heavy_losses {
                    // End wave — clear target, reset to gathering
                    let reason = if !target_alive {
                        "target cleared"
                    } else {
                        "heavy losses"
                    };
                    let squad = squad_state.squads.get_mut(si).expect("squad index valid");
                    squad.wave_active = false;
                    squad.target = None;
                    squad.wave_start_count = 0;
                    cmd.building_uid = None;
                    cmd.cooldown = personality.retarget_cooldown()
                        + rand::rng().random_range(-RETARGET_JITTER..RETARGET_JITTER);

                    let town_name = &town.name;
                    let pname = personality.name();
                    combat_log.write(crate::messages::CombatLogMsg {
                        kind: CombatEventKind::Raid,
                        faction,
                        day: game_time.day(),
                        hour: game_time.hour(),
                        minute: game_time.minute(),
                        message: format!(
                            "{} [{}] wave ended ({}), {} remaining",
                            town_name, pname, reason, member_count
                        ),
                        location: None,
                    });
                }
            } else {
                // --- Gathering phase: wait for wave_min_start ---
                let min_start = squad.wave_min_start.max(1);
                if member_count < min_start || cmd.cooldown > 0.0 {
                    continue; // not enough members or cooldown active
                }

                // Pick target based on AI kind
                let target = match kind {
                    AiKind::Raider => pick_raider_farm_target(&entity_map, center, faction),
                    AiKind::Builder => pick_ai_target_unclaimed(
                        &entity_map,
                        center,
                        faction,
                        personality,
                        SquadRole::Attack,
                        &claimed_targets,
                    ),
                };

                if let Some((bk, uid, pos)) = target {
                    cmd.building_uid = Some(uid);
                    claimed_targets.insert(uid);

                    let squad = squad_state.squads.get_mut(si).expect("squad index valid");
                    squad.target = Some(pos);
                    squad.wave_active = true;
                    squad.wave_start_count = member_count;

                    let town_name = &town.name;
                    let pname = personality.name();
                    let unit_label = match kind {
                        AiKind::Raider => "raiders",
                        AiKind::Builder => "units",
                    };
                    combat_log.write(crate::messages::CombatLogMsg {
                        kind: CombatEventKind::Raid,
                        faction,
                        day: game_time.day(),
                        hour: game_time.hour(),
                        minute: game_time.minute(),
                        message: format!(
                            "{} [{}] wave started: {} {} -> {}",
                            town_name,
                            pname,
                            member_count,
                            unit_label,
                            crate::constants::building_def(bk).label
                        ),
                        location: Some(pos),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;

    // ── ai_dirty_drain_system ──────────────────────────────────────────

    fn setup_ai_dirty_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(GameTime::default());
        app.insert_resource(AiSnapshotDirty(false));
        app.add_message::<crate::messages::BuildingGridDirtyMsg>();
        app.add_message::<crate::messages::MiningDirtyMsg>();
        app.add_message::<crate::messages::PatrolPerimeterDirtyMsg>();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.insert_resource(SendGridDirty(false));
        app.add_systems(FixedUpdate, (send_grid_dirty, ai_dirty_drain_system).chain());
        app.update();
        app.update();
        app
    }

    #[derive(Resource, Default)]
    struct SendGridDirty(bool);

    fn send_grid_dirty(
        mut writer: MessageWriter<crate::messages::BuildingGridDirtyMsg>,
        mut flag: ResMut<SendGridDirty>,
    ) {
        if flag.0 {
            writer.write(crate::messages::BuildingGridDirtyMsg);
            flag.0 = false;
        }
    }

    #[test]
    fn ai_dirty_drain_sets_flag_on_grid_msg() {
        let mut app = setup_ai_dirty_app();
        app.insert_resource(SendGridDirty(true));
        app.update();
        let dirty = app.world().resource::<AiSnapshotDirty>();
        assert!(dirty.0, "AiSnapshotDirty should be true after grid dirty msg");
    }

    #[test]
    fn ai_dirty_drain_stays_false_without_msgs() {
        let mut app = setup_ai_dirty_app();
        app.update();
        let dirty = app.world().resource::<AiSnapshotDirty>();
        assert!(!dirty.0, "AiSnapshotDirty should stay false with no messages");
    }

    // ── perimeter_dirty_drain_system ───────────────────────────────────

    fn setup_perimeter_dirty_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(GameTime::default());
        app.insert_resource(PerimeterSyncDirty(false));
        app.add_message::<crate::messages::PatrolPerimeterDirtyMsg>();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.insert_resource(SendPerimeterDirty(false));
        app.add_systems(
            FixedUpdate,
            (send_perimeter_dirty, perimeter_dirty_drain_system).chain(),
        );
        app.update();
        app.update();
        app
    }

    #[derive(Resource, Default)]
    struct SendPerimeterDirty(bool);

    fn send_perimeter_dirty(
        mut writer: MessageWriter<crate::messages::PatrolPerimeterDirtyMsg>,
        mut flag: ResMut<SendPerimeterDirty>,
    ) {
        if flag.0 {
            writer.write(crate::messages::PatrolPerimeterDirtyMsg);
            flag.0 = false;
        }
    }

    #[test]
    fn perimeter_dirty_sets_flag_on_msg() {
        let mut app = setup_perimeter_dirty_app();
        app.insert_resource(SendPerimeterDirty(true));
        app.update();
        let dirty = app.world().resource::<PerimeterSyncDirty>();
        assert!(dirty.0, "PerimeterSyncDirty should be true after msg");
    }

    #[test]
    fn perimeter_dirty_stays_false_without_msgs() {
        let mut app = setup_perimeter_dirty_app();
        app.update();
        let dirty = app.world().resource::<PerimeterSyncDirty>();
        assert!(!dirty.0, "PerimeterSyncDirty should stay false with no msgs");
    }
}
