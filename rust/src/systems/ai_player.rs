//! AI player system — autonomous opponents that build and upgrade like the player.
//! Each AI has a personality (Aggressive/Balanced/Economic) that influences weighted
//! random decisions — same pattern as NPC behavior scoring.
//!
//! Slot selection: economy buildings (farms, houses, barracks) prefer inner slots
//! (closest to center). Guard posts target the perimeter around controlled buildings
//! with minimum spacing of 5 grid slots between posts.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::prelude::*;
use bevy::ecs::system::SystemParam;
use rand::Rng;

use crate::constants::*;
use crate::resources::*;
use crate::systemparams::WorldState;
use crate::components::{Dead, Job, NpcIndex, SquadUnit, TownId};
use crate::world::{self, BuildingKind, WorldData, WorldGrid, BuildingSpatialGrid};
use crate::systems::stats::{UpgradeQueue, TownUpgrades, upgrade_node, upgrade_available, upgrade_unlocked, upgrade_cost, UPGRADES};
use crate::constants::UpgradeStatKind;

// Rust orientation notes for readers coming from PowerShell:
// - `Option<T>` is Rust's explicit nullable type (`Some(value)` or `None`).
// - `match` is a safe, exhaustive switch. The compiler ensures all cases are handled.
// - `&T` means "borrowed reference" (read-only by default). `&mut T` is mutable borrow.
// - Iterator chains (`iter().filter().map().collect()`) are the Rust equivalent of pipeline transforms.
// - The compiler enforces aliasing rules at compile time, replacing many runtime null/state checks.

/// Mutable world resources needed for AI building. Bundled to stay under Bevy's 16-param limit.
#[derive(SystemParam)]
pub struct AiBuildRes<'w> {
    world: WorldState<'w>,
    food_storage: ResMut<'w, FoodStorage>,
    upgrade_queue: ResMut<'w, UpgradeQueue>,
    policies: ResMut<'w, TownPolicies>,
}

/// Alive buildings for a town as `(row, col)` grid slots. Returns an iterator.
/// Single source of truth for the alive + town ownership + coordinate conversion pipeline.
macro_rules! town_building_slots {
    ($list:expr, $ti:expr, $center:expr) => {
        $list.iter()
            .filter(|b| b.town_idx == $ti && world::is_alive(b.position))
            .map(|b| world::world_to_town_grid($center, b.position))
    }
}

/// Minimum grid-step distance between waypoints on the town grid
/// (counting only up/down/left/right steps, not diagonals).
const MIN_WAYPOINT_SPACING: i32 = 5;
/// Patrol posts sit one slot outside controlled buildings.
const TERRITORY_PERIMETER_PADDING: i32 = 1;
const DEFAULT_MINING_RADIUS: f32 = 300.0;
const MINING_RADIUS_STEP: f32 = 300.0;
const MAX_MINING_RADIUS: f32 = 5000.0;
/// Hard ceiling on miners per mine, regardless of personality target.
const MAX_MINERS_PER_MINE: usize = 5;

/// Minimum grid-step distance from `candidate` to any existing waypoint for this town.
/// Returns `i32::MAX` if no waypoints exist.
fn min_waypoint_spacing(
    grid: &WorldGrid,
    world_data: &WorldData,
    town_idx: u32,
    candidate: Vec2,
) -> i32 {
    // Distance metric here is "grid steps" (taxicab):
    // |row_a - row_b| + |col_a - col_b|.
    // That means diagonal movement is counted as two steps.
    let (cc, cr) = grid.world_to_grid(candidate);
    world_data.waypoints().iter()
        .filter(|w| w.town_idx == town_idx && world::is_alive(w.position))
        .map(|w| {
            let (wc, wr) = grid.world_to_grid(w.position);
            (cc as i32 - wc as i32).abs() + (cr as i32 - wr as i32).abs()
        })
        .min()
        .unwrap_or(i32::MAX)
}

fn waypoint_spacing_ok(
    grid: &WorldGrid, world_data: &WorldData, town_idx: u32, candidate: Vec2,
) -> bool {
    // Small helper so call sites read like English: "is spacing OK?"
    min_waypoint_spacing(grid, world_data, town_idx, candidate) >= MIN_WAYPOINT_SPACING
}

fn recalc_waypoint_patrol_order_clockwise(
    world_data: &mut WorldData,
    town_idx: u32,
) {
    // Rebuild patrol order from geometry, not history:
    // sort all living waypoints of this town by angle around town center.
    // This guarantees stable clockwise ordering after add/remove operations.
    let Some(center) = world_data.towns.get(town_idx as usize).map(|t| t.center) else { return; };

    let mut ids: Vec<usize> = world_data.waypoints().iter().enumerate()
        .filter(|(_, w)| w.town_idx == town_idx && world::is_alive(w.position))
        .map(|(i, _)| i)
        .collect();

    // Clockwise around town center, starting at north (+Y).
    ids.sort_by(|&a, &b| {
        let pa = world_data.waypoints()[a].position - center;
        let pb = world_data.waypoints()[b].position - center;
        // Convert vector to angle using atan2 so we can sort by rotation.
        // We use (x,y) ordering intentionally to make 0 point at +Y ("north")
        // for this game's patrol convention.
        let mut aa = pa.x.atan2(pa.y);
        let mut ab = pb.x.atan2(pb.y);
        // atan2 returns [-pi, pi]. Shift to [0, 2pi) for clean clockwise sort.
        if aa < 0.0 { aa += std::f32::consts::TAU; }
        if ab < 0.0 { ab += std::f32::consts::TAU; }
        // Tie-breaker: if two waypoints share same angle, nearer one comes first.
        // `length_squared()` avoids sqrt and preserves ordering.
        aa.partial_cmp(&ab).unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| pa.length_squared().partial_cmp(&pb.length_squared()).unwrap_or(std::cmp::Ordering::Equal))
    });

    for (order, &idx) in ids.iter().enumerate() {
        world_data.waypoints_mut()[idx].patrol_order = order as u32;
    }
}

#[derive(Clone, Default)]
struct AiTownSnapshot {
    center: Vec2,
    empty_slots: Vec<(i32, i32)>,
    farms: HashSet<(i32, i32)>,
    farmer_homes: HashSet<(i32, i32)>,
    archer_homes: HashSet<(i32, i32)>,
    crossbow_homes: HashSet<(i32, i32)>,
    miner_homes: HashSet<(i32, i32)>,
}

/// Single definition of the four building types that constitute owned territory.
/// Adding a new territory-defining building? Add it here — both paths expand from this.
macro_rules! territory_building_sets {
    (snapshot $snap:expr) => {
        // Snapshot path: already normalized into HashSets.
        $snap.farms.iter()
            .chain(&$snap.farmer_homes)
            .chain(&$snap.archer_homes)
            .chain(&$snap.crossbow_homes)
            .chain(&$snap.miner_homes)
            .copied()
    };
    (world $wd:expr, $ti:expr, $center:expr) => {
        // World path: derive slots from live world arrays using the same conversion pipeline.
        town_building_slots!($wd.farms(), $ti, $center)
            .chain(town_building_slots!($wd.get(BuildingKind::FarmerHome), $ti, $center))
            .chain(town_building_slots!($wd.get(BuildingKind::ArcherHome), $ti, $center))
            .chain(town_building_slots!($wd.get(BuildingKind::CrossbowHome), $ti, $center))
            .chain(town_building_slots!($wd.miner_homes(), $ti, $center))
    };
}

impl AiTownSnapshot {
    // Snapshot utility: union all territory-defining building slots into one set.
    fn all_building_slots(&self) -> HashSet<(i32, i32)> {
        territory_building_sets!(snapshot self).collect()
    }
}

// Fallback utility: produce the same territory set directly from world state.
// Keep behavior equivalent to `AiTownSnapshot::all_building_slots`.
fn all_building_slots_from_world(
    world_data: &WorldData, ti: u32, center: Vec2,
) -> HashSet<(i32, i32)> {
    territory_building_sets!(world world_data, ti, center).collect()
}

#[derive(Default)]
pub struct AiTownSnapshotCache {
    towns: HashMap<usize, AiTownSnapshot>,
}

#[derive(Resource)]
pub struct AiPlayerConfig {
    pub decision_interval: f32,
}

impl Default for AiPlayerConfig {
    fn default() -> Self { Self { decision_interval: DEFAULT_AI_INTERVAL } }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AiKind { Raider, Builder }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AiPersonality { Aggressive, Balanced, Economic }

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
            Self::Economic => 2,
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
                mining_radius: DEFAULT_MINING_RADIUS,
                ..PolicySet::default()
            },
            Self::Balanced => PolicySet {
                mining_radius: DEFAULT_MINING_RADIUS,
                ..PolicySet::default()
            },
            Self::Economic => PolicySet {
                archer_leash: true,
                prioritize_healing: true,
                archer_flee_hp: 0.25,
                farmer_flee_hp: 0.50,
                mining_radius: DEFAULT_MINING_RADIUS,
                ..PolicySet::default()
            },
        }
    }

    /// Food desire thresholds for toggling eat_food policy.
    /// Returns (disable_above, reenable_below) — higher food_desire = more food stress.
    fn eat_food_desire_thresholds(self) -> (f32, f32) {
        match self {
            Self::Aggressive => (0.4, 0.2),
            Self::Balanced   => (0.6, 0.3),
            Self::Economic   => (0.8, 0.5),
        }
    }

    /// Base weights for building types: (farm, house, barracks, waypoint)
    fn building_weights(self) -> (f32, f32, f32, f32) {
        // Relative urgency for (farm, farmer home, archer home, waypoint).
        // These are multipliers, not hard guarantees.
        match self {
            Self::Aggressive => (10.0, 10.0, 30.0, 20.0),
            Self::Balanced   => (20.0, 20.0, 15.0, 10.0),
            Self::Economic   => (15.0, 12.0,  8.0,  5.0),
        }
    }

    /// Barracks target count relative to houses.
    pub fn archer_home_target(self, houses: usize) -> usize {
        // Desired military housing ratio relative to civilian farmer homes.
        match self {
            Self::Aggressive => houses.max(1),
            // Balanced aims for about half as many archer homes as farmer homes.
            Self::Balanced   => (houses / 2).max(1),
            // Economic keeps military lighter: about one archer home per 3 farmer homes.
            Self::Economic   => 1 + houses / 3,
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

    /// Desired miners per discovered gold mine in policy radius.
    fn miners_per_mine_target(self) -> usize {
        // Economic personality invests the most in mining saturation.
        match self {
            Self::Aggressive => 1,
            Self::Balanced => 2,
            Self::Economic => 4,
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
            Self::Balanced   => 0.35,
            Self::Economic   => 0.20,
        }
    }

    /// Baseline mining desire even without gold-costing upgrades.
    pub fn base_mining_desire(self) -> f32 {
        match self {
            Self::Aggressive => 0.0,
            Self::Balanced   => 0.1,
            Self::Economic   => 0.3,
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
            Self::Aggressive => 2,
            Self::Balanced => 3,
            Self::Economic => 6,
        }
    }

    /// True if (row, col) is reserved for road placement in this personality's pattern.
    /// Economic: 4x4 grid, Balanced: 3x3 grid, Aggressive: cardinal axes from center.
    fn is_road_slot(self, row: i32, col: i32) -> bool {
        if row == 0 && col == 0 { return false; } // center is never a road slot
        match self {
            Self::Aggressive => row == 0 || col == 0,
            Self::Balanced => row.rem_euclid(3) == 0 || col.rem_euclid(3) == 0,
            Self::Economic => row.rem_euclid(4) == 0 || col.rem_euclid(4) == 0,
        }
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
                let (hp, atk, rng, aspd, mspd, exp) = match self {
                    Self::Economic =>  (4., 4., 0., 4., 6., 2.),
                    _ =>               (4., 6., 2., 6., 4., 2.),
                };
                for cat in ["Archer", "Fighter"] {
                    set(cat, UpgradeStatKind::Hp, hp);
                    set(cat, UpgradeStatKind::Attack, atk);
                    set(cat, UpgradeStatKind::Range, rng);
                    set(cat, UpgradeStatKind::AttackSpeed, aspd);
                    set(cat, UpgradeStatKind::MoveSpeed, mspd);
                }
                set("Town", UpgradeStatKind::Expansion, exp);
            }
            AiKind::Builder => {
                // Builder AI upgrades everything
                let (aggr, bal, econ) = (
                    // Archer/Fighter
                    (6., 8., 4., 6., 4., 3., 3.),
                    (5., 5., 2., 4., 3., 2., 2.),
                    (3., 2., 1., 2., 2., 1., 1.),
                );
                let m = match self { Self::Aggressive => aggr, Self::Balanced => bal, Self::Economic => econ };
                for cat in ["Archer", "Fighter"] {
                    set(cat, UpgradeStatKind::Hp, m.0);
                    set(cat, UpgradeStatKind::Attack, m.1);
                    set(cat, UpgradeStatKind::Range, m.2);
                    set(cat, UpgradeStatKind::AttackSpeed, m.3);
                    set(cat, UpgradeStatKind::MoveSpeed, m.4);
                    set(cat, UpgradeStatKind::ProjectileSpeed, m.5);
                    set(cat, UpgradeStatKind::ProjectileLifetime, m.6);
                }

                // Crossbow
                let x = match self { Self::Aggressive => (5., 7., 3., 5., 3.), Self::Balanced => (4., 4., 2., 3., 2.), Self::Economic => (2., 2., 1., 1., 1.) };
                set("Crossbow", UpgradeStatKind::Hp, x.0);
                set("Crossbow", UpgradeStatKind::Attack, x.1);
                set("Crossbow", UpgradeStatKind::Range, x.2);
                set("Crossbow", UpgradeStatKind::AttackSpeed, x.3);
                set("Crossbow", UpgradeStatKind::MoveSpeed, x.4);

                // Farmer
                let f = match self { Self::Aggressive => (2., 1., 0.), Self::Balanced => (5., 3., 1.), Self::Economic => (8., 5., 2.) };
                set("Farmer", UpgradeStatKind::Yield, f.0);
                set("Farmer", UpgradeStatKind::Hp, f.1);
                set("Farmer", UpgradeStatKind::MoveSpeed, f.2);

                // Miner
                let mn = match self { Self::Aggressive => (1., 0., 1.), Self::Balanced => (3., 1., 2.), Self::Economic => (5., 2., 4.) };
                set("Miner", UpgradeStatKind::Hp, mn.0);
                set("Miner", UpgradeStatKind::MoveSpeed, mn.1);
                set("Miner", UpgradeStatKind::Yield, mn.2);

                // Town
                let t = match self { Self::Aggressive => (1., 5., 6., 5., 8.), Self::Balanced => (3., 5., 4., 4., 10.), Self::Economic => (5., 4., 3., 3., 12.) };
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
    sorted.iter().take(n)
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
    if total <= 0.0 { return None; }
    let roll = rand::rng().random_range(0.0..total);
    let mut acc = 0.0;
    for &(action, score) in scores {
        acc += score;
        if roll < acc { return Some(action); }
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
        barracks.saturating_sub(waypoints) as f32 / barracks as f32
    } else {
        0.0
    };
    // Barracks deficit is primary military pressure; waypoint coverage secondary;
    // threat from GPU spatial grid adds direct enemy presence signal.
    let mut military_desire = (barracks_gap * 0.75 + waypoint_gap * 0.25 + threat).clamp(0.0, 1.0);

    // Population ratio correction: dampen food_desire when military is underweight.
    // Only applies once the town has enough NPCs to meaningfully measure ratios.
    let total_pop = civilians + military;
    if total_pop >= 10 {
        let actual_ratio = military as f32 / total_pop as f32;
        let target = personality.target_military_ratio();
        if actual_ratio < target {
            // ratio_health: 0.0 = no military at all, 1.0 = at or above target
            let ratio_health = (actual_ratio / target).min(1.0);
            food_desire *= ratio_health;
            military_desire = (military_desire + (1.0 - ratio_health)).clamp(0.0, 1.0);
        }
    }

    DesireState { food_desire, military_desire, gold_desire: 0.0, economy_desire: 0.0 }
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
        if w <= 0.0 { continue; }
        if !upgrade_unlocked(levels, idx) { continue; }
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
    /// Target building identity (kind + index). Validated alive each cycle.
    pub target_kind: Option<BuildingKind>,
    pub target_index: usize,
    /// Seconds remaining before retarget is allowed.
    pub cooldown: f32,
}

/// Attack vs reserve role for multi-squad personalities.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SquadRole { Attack, Reserve, Idle }

pub struct AiPlayer {
    pub town_data_idx: usize,
    pub grid_idx: usize,
    pub kind: AiKind,
    pub personality: AiPersonality,
    pub last_actions: VecDeque<(String, i32, i32)>,
    pub active: bool,
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
const AI_ATTACK_SEARCH_RADIUS: f32 = 5000.0;

#[derive(Resource, Default)]
pub struct AiPlayerState {
    pub players: Vec<AiPlayer>,
}

// ============================================================================
// SLOT SELECTION
// ============================================================================

/// Find best empty slot closest to town center (for economy buildings).
/// Excludes road pattern slots for the given personality.
fn find_inner_slot(
    tg: &world::TownGrid, center: Vec2, grid: &WorldGrid, personality: AiPersonality,
) -> Option<(i32, i32)> {
    // Deterministic fallback placement policy:
    // choose the empty non-road slot closest to town center.
    world::empty_slots(tg, center, grid).into_iter()
        .filter(|&(r, c)| !personality.is_road_slot(r, c))
        .min_by_key(|&(r, c)| r * r + c * c)
}

fn build_town_snapshot(
    world_data: &WorldData,
    grid: &WorldGrid,
    tg: &world::TownGrid,
    town_data_idx: usize,
    personality: AiPersonality,
) -> Option<AiTownSnapshot> {
    // Build one cached view of this town used during this AI tick.
    // Purpose: avoid recomputing per-building slot sets repeatedly while scoring.
    let town = world_data.towns.get(town_data_idx)?;
    let center = town.center;
    let ti = town_data_idx as u32;

    let farms = town_building_slots!(world_data.farms(), ti, center).collect();
    let farmer_homes = town_building_slots!(world_data.get(BuildingKind::FarmerHome), ti, center).collect();
    let archer_homes = town_building_slots!(world_data.get(BuildingKind::ArcherHome), ti, center).collect();
    let crossbow_homes = town_building_slots!(world_data.get(BuildingKind::CrossbowHome), ti, center).collect();
    let miner_homes = town_building_slots!(world_data.miner_homes(), ti, center).collect();
    // Exclude road pattern slots so non-road buildings never pick them
    let empty_slots = world::empty_slots(tg, center, grid).into_iter()
        .filter(|&(r, c)| !personality.is_road_slot(r, c))
        .collect();

    Some(AiTownSnapshot {
        center,
        empty_slots,
        farms,
        farmer_homes,
        archer_homes,
        crossbow_homes,
        miner_homes,
    })
}

fn pick_best_empty_slot<F>(snapshot: &AiTownSnapshot, mut score: F) -> Option<(i32, i32)>
where
    F: FnMut((i32, i32)) -> i32,
{
    // Generic scoring function:
    // `F: FnMut(...)` means caller can pass any closure/function that scores one slot.
    // We keep the "best so far" as Option<(slot, score)> to handle empty input safely.
    let mut best: Option<((i32, i32), i32)> = None;
    for &slot in &snapshot.empty_slots {
        let s = score(slot);
        if best.map_or(true, |(_, bs)| s > bs) {
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

fn count_neighbors(snapshot: &AiTownSnapshot, slot: (i32, i32)) -> NeighborCounts {
    // 3x3 neighborhood scan around the candidate slot.
    // This is a common scoring primitive reused by multiple building scorers.
    let (r, c) = slot;
    let mut nc = NeighborCounts { edge_farms: 0, diag_farms: 0, farmer_homes: 0, archer_homes: 0, crossbow_homes: 0 };
    for dr in -1..=1 {
        for dc in -1..=1 {
            if dr == 0 && dc == 0 { continue; }
            let n = (r + dr, c + dc);
            if snapshot.farms.contains(&n) {
                if dr == 0 || dc == 0 { nc.edge_farms += 1; } else { nc.diag_farms += 1; }
            }
            if snapshot.farmer_homes.contains(&n) { nc.farmer_homes += 1; }
            if snapshot.archer_homes.contains(&n) { nc.archer_homes += 1; }
            if snapshot.crossbow_homes.contains(&n) { nc.crossbow_homes += 1; }
        }
    }
    nc
}

fn farm_slot_score(snapshot: &AiTownSnapshot, slot: (i32, i32)) -> i32 {
    let (r, c) = slot;
    let nc = count_neighbors(snapshot, slot);
    // Base preference:
    // - reward adjacency to farms (stronger for orthogonal neighbors than diagonal)
    // - mildly reward being near farmer homes (keeps food production near workers)
    let mut score = nc.edge_farms * 24 + nc.diag_farms * 12 + nc.farmer_homes * 8;

    // Shape bonus:
    // Check all four 2x2 blocks that could include this candidate slot.
    // If placing here would complete a dense farm block, reward heavily.
    // This encourages compact agricultural clusters instead of sparse scatter.
    let two_by_two = [(0, 0), (-1, 0), (0, -1), (-1, -1)];
    for (or, oc) in two_by_two {
        let r0 = r + or;
        let c0 = c + oc;
        let block = [(r0, c0), (r0 + 1, c0), (r0, c0 + 1), (r0 + 1, c0 + 1)];
        let existing = block.iter()
            .filter(|&&b| b != slot && snapshot.farms.contains(&b))
            .count();
        if existing == 3 {
            score += 120;
        } else if existing == 2 {
            score += 30;
        }
    }

    // Line bonus:
    // Extra reward when candidate touches multiple orthogonal farms,
    // which tends to create contiguous rows/columns with better density.
    if nc.edge_farms >= 2 { score += 30; }

    // Bootstrap rule:
    // For the very first farms, bias toward the town center so early layout
    // starts compact before local-cluster signals become available.
    if snapshot.farms.is_empty() {
        let radial = r * r + c * c;
        score -= radial / 2;
    }
    score
}

fn balanced_farm_ray_score(snapshot: &AiTownSnapshot, slot: (i32, i32)) -> i32 {
    // Balanced-personality farm pattern:
    // prefer straight rays on cardinal axes from town center, with continuity bonus.
    let (r, c) = slot;
    // `radial` is squared distance from town center in grid space.
    // Squared distance is cheaper than sqrt and good enough for ranking.
    let radial = r * r + c * c;
    let on_axis = r == 0 || c == 0;
    // High base score for axis slots, strong penalty for off-axis.
    // Additional `-radial*4` keeps growth close-in before extending outward.
    let mut score = if on_axis { 500 - radial * 4 } else { -300 - radial };

    if on_axis {
        if r == 0 && c != 0 {
            let step = if c > 0 { 1 } else { -1 };
            // Big reward if this extends an existing chain from center outward.
            if snapshot.farms.contains(&(0, c - step)) { score += 220; }
            // Smaller reward for having the next slot already filled.
            if snapshot.farms.contains(&(0, c + step)) { score += 40; }
        } else if c == 0 && r != 0 {
            let step = if r > 0 { 1 } else { -1 };
            if snapshot.farms.contains(&(r - step, 0)) { score += 220; }
            if snapshot.farms.contains(&(r + step, 0)) { score += 40; }
        }
    }

    score
}

fn farmer_home_border_score(snapshot: &AiTownSnapshot, slot: (i32, i32)) -> i32 {
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
    nc.edge_farms * 90 + nc.diag_farms * 35 + nc.farmer_homes * 10 + nc.archer_homes * 5 + nc.crossbow_homes * 5
}

fn balanced_house_side_score(snapshot: &AiTownSnapshot, slot: (i32, i32)) -> i32 {
    // Balanced-personality housing pattern:
    // farms tend to form straight "rays" from town center (north/south/east/west).
    // This scorer places farmer houses to the SIDE of those rays instead of on top of them.
    let (r, c) = slot;
    let mut score = 0i32;
    let on_axis = r == 0 || c == 0;
    if on_axis {
        // Penalize slots on the center axes (row 0 or col 0).
        // Reason: we want those lanes mostly for farms and movement, not houses.
        score -= 120;
    }

    for &(fr, fc) in &snapshot.farms {
        // Side-of-ray bonus:
        // - If farm is on vertical ray (col=0), reward houses at (farm_row, +/-1).
        // - If farm is on horizontal ray (row=0), reward houses at (+/-1, farm_col).
        // This creates "farm in lane, houses on both shoulders" layout.
        if fc == 0 && fr != 0 {
            if slot == (fr, 1) || slot == (fr, -1) {
                score += 260;
            }
        } else if fr == 0 && fc != 0 {
            if slot == (1, fc) || slot == (-1, fc) {
                score += 260;
            }
        }

        // Adjacency bonus:
        // grid_steps = |row_delta| + |col_delta| (up/down/left/right step count).
        // If exactly 1, this house touches a farm edge, which is desirable.
        let grid_steps = (r - fr).abs() + (c - fc).abs();
        if grid_steps == 1 {
            score += 20;
        }
    }

    for &(hr, hc) in &snapshot.farmer_homes {
        // Anti-clumping:
        // - Massive penalty for overlap (distance 0) to prevent duplicate placement.
        // - Small penalty for direct adjacency (distance 1) to keep spacing readable.
        let d = (r - hr).abs() + (c - hc).abs();
        if d == 0 {
            score -= 200;
        } else if d == 1 {
            score -= 25;
        }
    }

    score
}

fn archer_fill_score(snapshot: &AiTownSnapshot, slot: (i32, i32)) -> i32 {
    // Archer homes act as defensive fillers:
    // prefer being near economic core, avoid over-clumping with other archer homes.
    let nc = count_neighbors(snapshot, slot);
    let near_farms = nc.edge_farms + nc.diag_farms;
    // Archers should protect economic core, but not stack on top of each other.
    let mut score = near_farms * 40 + nc.farmer_homes * 35 - nc.archer_homes * 20 - nc.crossbow_homes * 20;
    // Extra bonus for dense "value zone" (many farms/homes nearby).
    if near_farms + nc.farmer_homes >= 4 { score += 60; }
    score
}

fn miner_toward_mine_score(mine_positions: &[Vec2], center: Vec2, slot: (i32, i32)) -> i32 {
    // Miner homes should move toward global mine availability.
    // With no mines, fallback to center-biased placement.
    if mine_positions.is_empty() {
        let (r, c) = slot;
        // No mine targets: fallback to center preference.
        return -(r * r + c * c);
    }
    let wp = world::town_grid_to_world(center, slot.0, slot.1);
    // `best` = squared distance to nearest mine from this candidate slot.
    let best = mine_positions.iter()
        .map(|m| (wp - *m).length_squared())
        .fold(f32::INFINITY, f32::min);
    let radial = slot.0 * slot.0 + slot.1 * slot.1;
    // Lower distance should rank higher, so return negative cost.
    // Add small center bias via `-radial` as tie-breaker.
    -(best as i32) - radial
}

/// Find outermost empty slot at least MIN_WAYPOINT_SPACING from all existing waypoints.
fn find_waypoint_slot(
    tg: &world::TownGrid, center: Vec2, grid: &WorldGrid, world_data: &WorldData, ti: u32,
) -> Option<(i32, i32)> {
    // Waypoint placement policy (in-town fallback):
    // 1) compute perimeter around owned territory
    // 2) discard occupied/unbuildable/too-close candidates
    // 3) choose candidate maximizing spacing, then radial distance
    let occupied = controlled_territory_slots(None, world_data, center, ti);
    if occupied.is_empty() { return None; }
    let perimeter = territory_perimeter_slots(&occupied, tg);
    if perimeter.is_empty() { return None; }

    let mut best: Option<((i32, i32), i32, i32)> = None;
    for &(r, c) in &perimeter {
        if r == 0 && c == 0 { continue; }
        let pos = world::town_grid_to_world(center, r, c);
        let (gc, gr) = grid.world_to_grid(pos);
        if grid.cell(gc, gr).map(|cl| cl.building.is_none()) != Some(true) { continue; }
        let min_spacing = min_waypoint_spacing(grid, world_data, ti, pos);
        if min_spacing < MIN_WAYPOINT_SPACING { continue; }

        let radial = r * r + c * c;
        if best.map_or(true, |(_, best_spacing, best_radial)| {
            // Primary objective: maximize spacing from existing waypoints.
            // Secondary objective (tie-break): choose farther-out perimeter slot.
            min_spacing > best_spacing || (min_spacing == best_spacing && radial > best_radial)
        }) {
            best = Some(((r, c), min_spacing, radial));
        }
    }
    best.map(|(slot, _, _)| slot)
}

/// Grid slots controlled by this town's owned buildings.
/// Uses snapshot if available, otherwise scans WorldData.
fn controlled_territory_slots(
    snapshot: Option<&AiTownSnapshot>,
    world_data: &WorldData,
    center: Vec2,
    ti: u32,
) -> HashSet<(i32, i32)> {
    // `Option<&AiTownSnapshot>` lets caller pass a cache when available.
    // Pattern:
    // - fast path: use snapshot (already precomputed)
    // - fallback: compute from world state
    if let Some(snap) = snapshot {
        return snap.all_building_slots();
    }
    all_building_slots_from_world(world_data, ti, center)
}

/// Candidate perimeter slots around controlled buildings, clamped to buildable town grid.
fn territory_perimeter_slots(
    occupied: &HashSet<(i32, i32)>, tg: &world::TownGrid,
) -> HashSet<(i32, i32)> {
    // Convert occupied territory slots into a one-cell perimeter ring.
    let mut out = HashSet::new();
    let dirs = [(-1, 0), (1, 0), (0, -1), (0, 1)];

    for &(r, c) in occupied {
        for (dr, dc) in dirs {
            let nr = r + dr * TERRITORY_PERIMETER_PADDING;
            let nc = c + dc * TERRITORY_PERIMETER_PADDING;
            if occupied.contains(&(nr, nc)) { continue; }
            if !world::is_slot_buildable(tg, nr, nc) { continue; }
            out.insert((nr, nc));
        }
    }
    out
}

fn sync_town_perimeter_waypoints(
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    farm_states: &mut GrowthStates,
    spawner_state: &mut SpawnerState,
    building_hp: &mut BuildingHpState,
    slot_alloc: &mut SlotAllocator,
    building_slots: &mut BuildingSlotMap,
    combat_log: &mut CombatLog,
    game_time: &GameTime,
    town_grids: &world::TownGrids,
    town_data_idx: usize,
) -> usize {
    // Maintenance pass:
    // if town territory changed, prune in-town waypoints no longer on perimeter.
    // Wilderness waypoints are preserved.
    let Some(town) = world_data.towns.get(town_data_idx) else { return 0; };
    let Some(tg) = town_grids.grids.iter().find(|g| g.town_data_idx == town_data_idx) else { return 0; };
    let center = town.center;
    let ti = town_data_idx as u32;

    let occupied = controlled_territory_slots(None, world_data, center, ti);
    if occupied.is_empty() { return 0; }
    let perimeter = territory_perimeter_slots(&occupied, tg);
    if perimeter.is_empty() { return 0; }

    let mut prune_slots: Vec<(i32, i32)> = Vec::new();
    for wp in world_data.waypoints() {
        if wp.town_idx != ti || !world::is_alive(wp.position) { continue; }
        let slot = world::world_to_town_grid(center, wp.position);
        // Preserve wilderness/mine outposts: only prune waypoints inside town build area.
        if !world::is_slot_buildable(tg, slot.0, slot.1) { continue; }
        if !perimeter.contains(&slot) {
            prune_slots.push(slot);
        }
    }

    let mut removed = 0usize;
    for (row, col) in prune_slots {
        if world::destroy_building(
            grid, world_data, farm_states, spawner_state, building_hp,
            slot_alloc, building_slots, combat_log, game_time,
            row, col, center, "waypoint pruned (perimeter shifted)",
        ).is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        recalc_waypoint_patrol_order_clockwise(world_data, ti);
    }
    removed
}

/// Dirty-flag-gated maintenance: keep in-town patrol waypoints on the building-driven perimeter.
pub fn sync_patrol_perimeter_system(
    mut world: WorldState,
    ai_state: Res<AiPlayerState>,
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    timings: Res<SystemTimings>,
) {
    // Dirty-flag system: only runs when perimeter-affecting state changed.
    let _t = timings.scope("sync_patrol_perimeter");
    if !world.dirty.patrol_perimeter { return; }
    world.dirty.patrol_perimeter = false;

    let mut town_ids: HashSet<usize> = HashSet::new();
    for p in ai_state.players.iter().filter(|p| p.active) {
        town_ids.insert(p.town_data_idx);
    }

    let mut removed_total = 0usize;
    for town_idx in town_ids {
        removed_total += sync_town_perimeter_waypoints(
            &mut world.grid,
            &mut world.world_data,
            &mut world.farm_states,
            &mut world.spawner_state,
            &mut world.building_hp,
            &mut world.slot_alloc,
            &mut world.building_slots,
            &mut combat_log,
            &game_time,
            &world.town_grids,
            town_idx,
        );
    }

    if removed_total > 0 {
        world.dirty.patrols = true;
        world.dirty.building_grid = true;
    }
}

// ============================================================================
// MINE ANALYSIS (single-pass over gold_mines per AI tick)
// ============================================================================

/// Pre-computed mine stats for one AI town. Built once per tick, used by scoring + execution.
struct MineAnalysis {
    in_radius: usize,
    outside_radius: usize,
    uncovered: Vec<Vec2>,
    /// Closest uncovered mine to town center (for wilderness waypoint placement).
    nearest_uncovered: Option<Vec2>,
    /// All alive mine positions on the map (for miner home slot scoring).
    all_positions: Vec<Vec2>,
}

fn analyze_mines(world_data: &WorldData, center: Vec2, ti: u32, mining_radius: f32) -> MineAnalysis {
    // Single-pass analysis over alive gold mines.
    // We derive multiple outputs from one loop to avoid duplicate scans:
    // - count in/out of radius
    // - uncovered mines
    // - nearest uncovered mine
    // - all mine positions (for miner-home placement scoring)
    // Compare squared distances to avoid sqrt in hot loops.
    let radius_sq = mining_radius * mining_radius;
    let friendly: Vec<Vec2> = world_data.waypoints().iter()
        .filter(|w| w.town_idx == ti && world::is_alive(w.position))
        .map(|w| w.position)
        .collect();

    let mut in_radius = 0usize;
    let mut outside_radius = 0usize;
    let mut uncovered = Vec::new();
    let mut all_positions = Vec::new();

    for m in world_data.gold_mines() {
        if !world::is_alive(m.position) { continue; }
        all_positions.push(m.position);
        if (m.position - center).length_squared() <= radius_sq {
            in_radius += 1;
        } else {
            outside_radius += 1;
        }
        // A mine is "covered" if any friendly waypoint is within cover radius.
        if !friendly.iter().any(|wp| (*wp - m.position).length() < WAYPOINT_COVER_RADIUS) {
            uncovered.push(m.position);
        }
    }

    // Choose the uncovered mine nearest to town center for likely next waypoint target.
    let nearest_uncovered = uncovered.iter()
        .min_by(|a: &&Vec2, b: &&Vec2| a.distance(center).partial_cmp(&b.distance(center)).unwrap())
        .copied();

    MineAnalysis { in_radius, outside_radius, uncovered, nearest_uncovered, all_positions }
}

/// Per-tick derived context for one AI town. Built once before scoring/execution.
struct TownContext {
    center: Vec2,
    ti: u32,
    tdi: usize,
    grid_idx: usize,
    food: i32,
    has_slots: bool,
    slot_fullness: f32,
    mines: Option<MineAnalysis>,
}

impl TownContext {
    fn build(
        tdi: usize, grid_idx: usize, food: i32,
        snapshot: Option<&AiTownSnapshot>,
        res: &AiBuildRes, kind: AiKind, mining_radius: f32,
    ) -> Option<Self> {
        // Constructor centralizes per-tick derived values.
        // Returning `Option<Self>` avoids panics if required world/town data is missing.
        let center = snapshot.map(|s| s.center)
            .or_else(|| res.world.world_data.towns.get(tdi).map(|t| t.center))?;
        let ti = tdi as u32;
        let empty_count = snapshot.map(|s| s.empty_slots.len())
            .or_else(|| res.world.town_grids.grids.get(grid_idx)
                .map(|tg| world::empty_slots(tg, center, &res.world.grid).len()))
            .unwrap_or(0);
        let slot_fullness = res.world.town_grids.grids.get(grid_idx)
            .map(|tg| {
                let (min_r, max_r, min_c, max_c) = world::build_bounds(tg);
                // Total candidate build slots inside town bounds, minus center tile.
                let total = ((max_r - min_r + 1) * (max_c - min_c + 1) - 1) as f32;
                // Normalize occupancy to [0,1] where 1.0 means "effectively full".
                1.0 - empty_count as f32 / total.max(1.0)
            })
            .unwrap_or(0.0);
        let mines = match kind {
            // Builder AIs use mine analysis for scoring/execution.
            AiKind::Builder => Some(analyze_mines(&res.world.world_data, center, ti, mining_radius)),
            // Raider AIs don't use mining logic.
            AiKind::Raider => None,
        };
        Some(Self { center, ti, tdi, grid_idx, food, has_slots: empty_count > 0, slot_fullness, mines })
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
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    difficulty: Res<Difficulty>,
    gpu_state: Res<GpuReadState>,
    pop_stats: Res<PopulationStats>,
    mut timer: Local<f32>,
    mut snapshots: Local<AiTownSnapshotCache>,
    timings: Res<SystemTimings>,
    settings: Res<crate::settings::UserSettings>,
) {
    // System timing gate:
    // runs every `decision_interval`, not every frame.
    let _t = timings.scope("ai_decision");
    *timer += game_time.delta(&time);
    if *timer < config.decision_interval { return; }
    *timer = 0.0;

    let snapshot_dirty = res.world.dirty.building_grid || res.world.dirty.mining || res.world.dirty.patrol_perimeter;
    if snapshot_dirty {
        snapshots.towns.clear();
    }

    for pi in 0..ai_state.players.len() {
        // Two-step style common in Rust ECS:
        // 1) gather immutable state and score actions
        // 2) perform one mutating action
        let player = &ai_state.players[pi];
        if !player.active { continue; }
        let tdi = player.town_data_idx;
        let personality = player.personality;
        let kind = player.kind;
        let grid_idx = player.grid_idx;
        let _ = player; // end immutable borrow — mutable access needed later
        if !snapshots.towns.contains_key(&tdi) {
            if let Some(tg) = res.world.town_grids.grids.get(grid_idx) {
                if let Some(snap) = build_town_snapshot(&res.world.world_data, &res.world.grid, tg, tdi, personality) {
                    snapshots.towns.insert(tdi, snap);
                }
            }
        }

        let food = res.food_storage.food.get(tdi).copied().unwrap_or(0);
        let spawner_count = res.world.spawner_state.0.iter()
            .filter(|s| world::is_alive(s.position))
            .filter(|s| s.town_idx == tdi as i32)
            .filter(|s| s.is_population_spawner())
            .count() as i32;
        let reserve = personality.food_reserve_per_spawner() * spawner_count;
        // Food reserve rule: if town is at/below reserve, skip spending this tick.
        if food <= reserve { continue; }
        // Desire signals are computed once below and reused by action + upgrade scoring.
        let mining_radius = res.policies.policies.get(tdi)
            .map(|p| p.mining_radius)
            .unwrap_or(DEFAULT_MINING_RADIUS);
        let Some(ctx) = TownContext::build(
            tdi, grid_idx, food,
            snapshots.towns.get(&tdi), &res, kind, mining_radius,
        ) else { continue };

        let town_name = res.world.world_data.towns.get(tdi).map(|t| t.name.clone()).unwrap_or_default();
        let pname = personality.name();

        let counts = res.world.world_data.building_counts(ctx.ti);
        let bc = |k: BuildingKind| counts.get(&k).copied().unwrap_or(0);
        let farms = bc(BuildingKind::Farm);
        let houses = bc(BuildingKind::FarmerHome);
        let barracks = bc(BuildingKind::ArcherHome);
        let xbow_homes = bc(BuildingKind::CrossbowHome);
        let waypoints = bc(BuildingKind::Waypoint);
        let mine_shafts = bc(BuildingKind::MinerHome);
        let total_military_homes = barracks + xbow_homes;
        let faction = res.world.world_data.towns.get(tdi).map(|t| t.faction).unwrap_or(0);
        // Threat signal from GPU spatial grid: fountain's enemy count from readback.
        let threat = res.world.building_slots.get_slot(BuildingKind::Fountain, tdi)
            .and_then(|slot| gpu_state.threat_counts.get(slot).copied())
            .map(|packed| {
                let enemies = (packed >> 16) as f32;
                (enemies / 10.0).min(1.0)
            })
            .unwrap_or(0.0);
        // Count alive civilians vs military for this town from PopulationStats.
        let town_key = tdi as i32;
        let pop_alive = |job: Job| pop_stats.0.get(&(job as i32, town_key)).map(|p| p.alive).unwrap_or(0).max(0) as usize;
        let civilians = pop_alive(Job::Farmer) + pop_alive(Job::Miner);
        let military = pop_alive(Job::Archer) + pop_alive(Job::Fighter) + pop_alive(Job::Crossbow);
        let mut desires = desire_state(personality, food, reserve, houses, total_military_homes, waypoints, threat, civilians, military);

        // Gold desire: driven by cheapest gold-costing upgrade the AI wants but can't afford.
        let uw = personality.upgrade_weights(kind);
        let levels = upgrades.town_levels(tdi);
        let gold = gold_storage.gold.get(tdi).copied().unwrap_or(0);
        let cheapest_gold = cheapest_gold_upgrade_cost(&uw, &levels, gold);
        desires.gold_desire = if cheapest_gold > 0 {
            ((1.0 - gold as f32 / cheapest_gold as f32) * personality.gold_desire_mult()).clamp(0.0, 1.0)
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
                log_ai(&mut combat_log, &game_time, faction, &town_name, pname,
                    &format!("eat_food → {state} (food_desire={:.2})", desires.food_desire));
            }
        }

        // ================================================================
        // Phase 1: Score and execute a BUILDING action
        // ================================================================
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
                let bt = personality.archer_home_target(houses);
                let ht = personality.farmer_home_target(farms);
                let mines = ctx.mines.as_ref().unwrap();
                let miners_per_mine = personality.miners_per_mine_target().min(MAX_MINERS_PER_MINE);
                let ms_target = mines.in_radius * miners_per_mine;
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
                        0.0
                    };
                    let barracks_need = if barracks_deficit > 0 {
                        desires.military_desire * barracks_deficit as f32
                    } else {
                        desires.military_desire * 0.5
                    };

                    if ctx.food >= building_cost(BuildingKind::Farm) { build_scores.push((AiAction::BuildFarm, fw * farm_need)); }
                    if ctx.food >= building_cost(BuildingKind::FarmerHome) { build_scores.push((AiAction::BuildFarmerHome, hw * house_need)); }
                    if ctx.food >= building_cost(BuildingKind::ArcherHome) { build_scores.push((AiAction::BuildArcherHome, bw * barracks_need)); }
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
                        build_scores.push((AiAction::BuildMinerHome, hw * ms_need));
                    } else if miner_deficit == 0 && mines.outside_radius > 0 {
                        let expand_need = desires.gold_desire * mines.outside_radius as f32;
                        build_scores.push((AiAction::ExpandMiningRadius, personality.expand_mining_weight() * expand_need));
                    }
                }

                if ctx.food >= building_cost(BuildingKind::Waypoint) {
                    // Prefer uncovered mine support; otherwise maintain patrol coverage parity.
                    let uncovered = mines.uncovered.len();
                    if uncovered > 0 {
                        let mine_need = desires.military_desire * uncovered as f32;
                        build_scores.push((AiAction::BuildWaypoint, gw * mine_need));
                    } else if waypoints < total_military_homes {
                        let gp_need = desires.military_desire * (total_military_homes - waypoints) as f32;
                        if ctx.has_slots {
                            build_scores.push((AiAction::BuildWaypoint, gw * gp_need));
                        }
                    }
                }

                // Roads: build grid-pattern roads around economy buildings
                let rw = personality.road_weight();
                if rw > 0.0 && ctx.food >= building_cost(BuildingKind::Road) * 4 {
                    let roads = bc(BuildingKind::Road);
                    let economy_buildings = farms + houses + mine_shafts;
                    // Want roughly 1 road per 2 economy buildings
                    let road_need = economy_buildings.saturating_sub(roads / 2);
                    if road_need > 0 {
                        build_scores.push((AiAction::BuildRoads, rw * road_need as f32));
                    }
                }
            }
        }

        let debug = settings.debug_ai_decisions;
        // Retry loop: if picked action fails, remove it and re-pick from remaining.
        let mut build_succeeded = false;
        loop {
            let Some(action) = weighted_pick(&build_scores) else { break };
            let label = execute_action(
                action, &ctx, &mut res,
                snapshots.towns.get(&tdi), personality, *difficulty,
            );
            if let Some(what) = label {
                snapshots.towns.remove(&tdi);
                log_ai(&mut combat_log, &game_time, faction, &town_name, pname, &what);
                let actions = &mut ai_state.players[pi].last_actions;
                if actions.len() >= MAX_ACTION_HISTORY { actions.pop_front(); }
                actions.push_back((what, game_time.day(), game_time.hour()));
                build_succeeded = true;
                break;
            }
            // Action failed — log and remove this variant from candidates
            if debug {
                let msg = format!("[dbg] {} FAILED ({})", action.label(), format_top_scores(&build_scores, 4));
                let actions = &mut ai_state.players[pi].last_actions;
                if actions.len() >= MAX_ACTION_HISTORY { actions.pop_front(); }
                actions.push_back((msg, game_time.day(), game_time.hour()));
            }
            let failed = std::mem::discriminant(&action);
            build_scores.retain(|(a, _)| std::mem::discriminant(a) != failed);
        }
        if !build_succeeded && debug {
            if build_scores.is_empty() {
                let actions = &mut ai_state.players[pi].last_actions;
                if actions.len() >= MAX_ACTION_HISTORY { actions.pop_front(); }
                actions.push_back(("[dbg] no build candidates".into(), game_time.day(), game_time.hour()));
            }
        }

        // ================================================================
        // Phase 2: Score and execute an UPGRADE action (if food/gold remain)
        // ================================================================
        let food_after = res.food_storage.food.get(tdi).copied().unwrap_or(0);
        let gold_after = gold_storage.gold.get(tdi).copied().unwrap_or(0);
        if food_after > reserve {
            let mut upgrade_scores: Vec<(AiAction, f32)> = Vec::with_capacity(8);
            for (idx, &weight) in uw.iter().enumerate() {
                if weight <= 0.0 { continue; }
                if !upgrade_available(&levels, idx, food_after, gold_after) { continue; }
                // Fill slots first: only expansion upgrades allowed while town has empty slots
                let is_expansion = UPGRADES.nodes[idx].triggers_expansion;
                if ctx.has_slots && !is_expansion { continue; }
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
                    action, &ctx, &mut res,
                    snapshots.towns.get(&tdi), personality, *difficulty,
                );
                if label.is_some() {
                    snapshots.towns.remove(&tdi);
                }
                if let Some(what) = label {
                    log_ai(&mut combat_log, &game_time, faction, &town_name, pname, &what);
                    let actions = &mut ai_state.players[pi].last_actions;
                    if actions.len() >= MAX_ACTION_HISTORY { actions.pop_front(); }
                    actions.push_back((what, game_time.day(), game_time.hour()));
                } else if debug {
                    let name = if let AiAction::Upgrade(idx) = action { upgrade_node(idx).label } else { action.label() };
                    let msg = format!("[dbg] upgrade {} FAILED", name);
                    let actions = &mut ai_state.players[pi].last_actions;
                    if actions.len() >= MAX_ACTION_HISTORY { actions.pop_front(); }
                    actions.push_back((msg, game_time.day(), game_time.hour()));
                }
            }
        }
    }
}

fn try_build_at_slot(
    kind: BuildingKind,
    cost: i32,
    label: &str,
    tdi: usize,
    center: Vec2,
    res: &mut AiBuildRes,
    row: i32,
    col: i32,
) -> Option<String> {
    // Thin wrapper around world::build_and_pay that returns a user-facing log label.
    let ok = world::build_and_pay(
        &mut res.world.grid,
        &mut res.world.world_data,
        &mut res.world.farm_states,
        &mut res.food_storage,
        &mut res.world.spawner_state,
        &mut res.world.building_hp,
        &mut res.world.slot_alloc,
        &mut res.world.building_slots,
        &mut res.world.dirty,
        kind,
        tdi,
        row,
        col,
        center,
        cost,
    );
    ok.then_some(format!("built {label}"))
}

fn pick_slot_from_snapshot_or_inner(
    snapshot: Option<&AiTownSnapshot>,
    tg: &world::TownGrid,
    center: Vec2,
    grid: &WorldGrid,
    score: fn(&AiTownSnapshot, (i32, i32)) -> i32,
    personality: AiPersonality,
) -> Option<(i32, i32)> {
    // If snapshot exists, use expensive scoring over known empty slots.
    // If no snapshot/candidate, fallback to deterministic inner-slot policy.
    if let Some(snap) = snapshot {
        if let Some(slot) = pick_best_empty_slot(snap, |s| score(snap, s)) {
            return Some(slot);
        }
    }
    find_inner_slot(tg, center, grid, personality)
}

fn try_build_inner(
    kind: BuildingKind, cost: i32, label: &str,
    tdi: usize, center: Vec2, res: &mut AiBuildRes, grid_idx: usize,
    personality: AiPersonality,
) -> Option<String> {
    // Build using deterministic center-nearest slot.
    let tg = res.world.town_grids.grids.get(grid_idx)?;
    let (row, col) = find_inner_slot(tg, center, &res.world.grid, personality)?;
    try_build_at_slot(kind, cost, label, tdi, center, res, row, col)
}

fn try_build_scored(
    kind: BuildingKind, label: &str,
    tdi: usize, center: Vec2, res: &mut AiBuildRes, grid_idx: usize,
    snapshot: Option<&AiTownSnapshot>,
    score_fn: fn(&AiTownSnapshot, (i32, i32)) -> i32,
    personality: AiPersonality,
) -> Option<String> {
    // Build using snapshot-aware scoring with inner-slot fallback.
    let tg = res.world.town_grids.grids.get(grid_idx)?;
    let (row, col) = pick_slot_from_snapshot_or_inner(snapshot, tg, center, &res.world.grid, score_fn, personality)?;
    try_build_at_slot(kind, building_cost(kind), label, tdi, center, res, row, col)
}

fn try_build_miner_home(
    ctx: &TownContext, mines: &MineAnalysis, res: &mut AiBuildRes,
    snapshot: Option<&AiTownSnapshot>, personality: AiPersonality,
) -> Option<String> {
    // Miner homes are intentionally special-cased:
    // score depends on mine positions from per-tick MineAnalysis, not only local adjacency.
    let tg = res.world.town_grids.grids.get(ctx.grid_idx)?;
    let slot = if let Some(snap) = snapshot {
        pick_best_empty_slot(snap, |s| miner_toward_mine_score(&mines.all_positions, ctx.center, s))
            .or_else(|| find_inner_slot(tg, ctx.center, &res.world.grid, personality))
    } else {
        find_inner_slot(tg, ctx.center, &res.world.grid, personality)
    }?;
    try_build_at_slot(
        BuildingKind::MinerHome,
        building_cost(BuildingKind::MinerHome), "miner home",
        ctx.tdi, ctx.center, res, slot.0, slot.1,
    )
}

/// Build roads in personality-specific patterns around economy buildings.
/// Economic: 4x4 grid, Balanced: 3x3 grid, Aggressive: cardinal axes from center.
fn try_build_road_grid(
    ctx: &TownContext,
    res: &mut AiBuildRes,
    batch_size: usize,
    personality: AiPersonality,
) -> Option<String> {
    let cost = building_cost(BuildingKind::Road);
    let ti = ctx.ti;
    let center = ctx.center;
    let wd = &res.world.world_data;

    // Collect economy building positions as town grid coords
    let econ_slots: Vec<(i32, i32)> = town_building_slots!(wd.farms(), ti, center)
        .chain(town_building_slots!(wd.get(BuildingKind::FarmerHome), ti, center))
        .chain(town_building_slots!(wd.miner_homes(), ti, center))
        .collect();
    if econ_slots.is_empty() { return None; }

    // Collect existing road positions for quick lookup
    let road_slots: HashSet<(i32, i32)> = town_building_slots!(wd.get(BuildingKind::Road), ti, center).collect();

    // Generate candidate road cells on personality-specific pattern near economy buildings
    let mut candidates: HashMap<(i32, i32), i32> = HashMap::new();
    let tg = res.world.town_grids.grids.get(ctx.grid_idx);
    let (min_r, max_r, min_c, max_c) = tg.map(|g| world::build_bounds(g)).unwrap_or((-4, 3, -4, 3));

    for r in min_r..=max_r {
        for c in min_c..=max_c {
            if !personality.is_road_slot(r, c) { continue; }
            // Score by adjacency to economy buildings (distance 2 covers the 4-cell pattern gap)
            let adj = econ_slots.iter().filter(|&&(er, ec)| {
                (er - r).abs() <= 2 && (ec - c).abs() <= 2
            }).count() as i32;
            if adj > 0 {
                candidates.insert((r, c), adj);
            }
        }
    }

    // Filter out existing roads
    candidates.retain(|slot, _| !road_slots.contains(slot));

    // Sort by score (highest adjacency first), then by distance to center (closer first)
    let mut ranked: Vec<((i32, i32), i32)> = candidates.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.cmp(&a.1).then_with(|| {
            let da = a.0.0 * a.0.0 + a.0.1 * a.0.1;
            let db = b.0.0 * b.0.0 + b.0.1 * b.0.1;
            da.cmp(&db)
        })
    });

    let mut placed = 0usize;
    for &((r, c), _score) in ranked.iter().take(batch_size * 2) {
        if placed >= batch_size { break; }
        let food = res.food_storage.food.get(ctx.tdi).copied().unwrap_or(0);
        if food < cost { break; }

        let pos = world::town_grid_to_world(center, r, c);
        if world::place_wilderness_building(
            BuildingKind::Road,
            &mut res.world.grid, &mut res.world.world_data,
            &mut res.world.building_hp, &mut res.food_storage,
            &mut res.world.slot_alloc, &mut res.world.building_slots,
            ctx.tdi, pos, cost, &res.world.town_grids,
        ).is_ok() {
            placed += 1;
        }
    }

    if placed > 0 {
        res.world.dirty.mark_building_changed(BuildingKind::Road);
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
    _difficulty: Difficulty,
) -> Option<String> {
    // Action execution uses `match` on enum variant.
    // This gives explicit, compile-checked control flow per action type.
    match action {
        AiAction::BuildTent => try_build_inner(
            BuildingKind::Tent, building_cost(BuildingKind::Tent), "tent",
            ctx.tdi, ctx.center, res, ctx.grid_idx, personality),
        AiAction::BuildFarm => {
            let score = if personality == AiPersonality::Balanced { balanced_farm_ray_score } else { farm_slot_score };
            try_build_scored(BuildingKind::Farm, "farm",
                ctx.tdi, ctx.center, res, ctx.grid_idx, snapshot, score, personality)
        }
        AiAction::BuildFarmerHome => {
            let score = if personality == AiPersonality::Balanced { balanced_house_side_score } else { farmer_home_border_score };
            try_build_scored(BuildingKind::FarmerHome, "farmer home",
                ctx.tdi, ctx.center, res, ctx.grid_idx, snapshot, score, personality)
        }
        AiAction::BuildArcherHome => try_build_scored(
            BuildingKind::ArcherHome, "archer home",
            ctx.tdi, ctx.center, res, ctx.grid_idx, snapshot, archer_fill_score, personality),
        AiAction::BuildCrossbowHome => try_build_scored(
            BuildingKind::CrossbowHome, "crossbow home",
            ctx.tdi, ctx.center, res, ctx.grid_idx, snapshot, archer_fill_score, personality),
        AiAction::BuildMinerHome => {
            let Some(mines) = &ctx.mines else { return None; };
            try_build_miner_home(ctx, mines, res, snapshot, personality)
        }
        AiAction::ExpandMiningRadius => {
            // Policy action, not building placement.
            // Expands search radius for mines in fixed-size steps with max cap.
            let Some(policy) = res.policies.policies.get_mut(ctx.tdi) else { return None; };
            let old = policy.mining_radius;
            let new = (old + MINING_RADIUS_STEP).min(MAX_MINING_RADIUS);
            if new <= old {
                return None;
            }
            policy.mining_radius = new;
            res.world.dirty.mining = true;
            Some(format!("expanded mining radius to {:.0}px", new))
        }
        AiAction::BuildWaypoint => {
            // Builder-only guard: if mines are unavailable, skip action safely.
            let Some(mines) = &ctx.mines else { return None; };
            let cost = building_cost(BuildingKind::Waypoint);
            let wp_pos = mines.nearest_uncovered
                .filter(|&pos| waypoint_spacing_ok(&res.world.grid, &res.world.world_data, ctx.ti, pos))
                .or_else(|| {
                    let tg = res.world.town_grids.grids.get(ctx.grid_idx)?;
                    let (row, col) = find_waypoint_slot(tg, ctx.center, &res.world.grid, &res.world.world_data, ctx.ti)?;
                    Some(world::town_grid_to_world(ctx.center, row, col))
                });
            let Some(pos) = wp_pos else { return None; };
            // Placement is world-position based (not town-slot based),
            // so this supports both in-town and wilderness waypoint targets.
            if world::place_wilderness_building(
                world::BuildingKind::Waypoint,
                &mut res.world.grid, &mut res.world.world_data,
                &mut res.world.building_hp, &mut res.food_storage,
                &mut res.world.slot_alloc, &mut res.world.building_slots,
                ctx.tdi, pos, cost, &res.world.town_grids,
            ).is_ok() {
                recalc_waypoint_patrol_order_clockwise(&mut res.world.world_data, ctx.ti);
                res.world.dirty.mark_building_changed(world::BuildingKind::Waypoint);
                Some("built waypoint".into())
            } else {
                None
            }
        }
        AiAction::BuildRoads => {
            try_build_road_grid(ctx, res, personality.road_batch_size(), personality)
        }
        AiAction::Upgrade(idx) => {
            res.upgrade_queue.0.push((ctx.tdi, idx));
            let name = upgrade_node(idx).label;
            Some(format!("upgraded {name}"))
        }
    }
}


fn log_ai(log: &mut CombatLog, gt: &GameTime, faction: i32, town: &str, personality: &str, what: &str) {
    // Centralized AI log format so all decisions read consistently in the combat log.
    log.push(CombatEventKind::Ai, faction, gt.day(), gt.hour(), gt.minute(),
        format!("{} [{}] {}", town, personality, what));
}

// ============================================================================
// AI SQUAD COMMANDER
// ============================================================================

/// Resolve a building's position from WorldData by kind + index.
/// Returns None if index is out of bounds or building is dead.
fn resolve_building_pos(world_data: &WorldData, kind: BuildingKind, index: usize) -> Option<Vec2> {
    (crate::constants::building_def(kind).pos_town)(world_data, index).map(|(pos, _)| pos)
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
            Self::Aggressive => if attack_idx == 0 { 55 } else { 45 },
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
                    BuildingKind::Farm, BuildingKind::FarmerHome,
                    BuildingKind::ArcherHome, BuildingKind::CrossbowHome, BuildingKind::Waypoint,
                    BuildingKind::Tent, BuildingKind::MinerHome,
                ],
                Self::Balanced => &[BuildingKind::ArcherHome, BuildingKind::CrossbowHome, BuildingKind::Waypoint],
                Self::Economic => &[BuildingKind::Farm],
            },
        }
    }

    /// Broad fallback set when preferred kinds yield no target.
    /// Fountain last priority — destroy the base after clearing defenses.
    fn fallback_attack_kinds() -> &'static [BuildingKind] {
        &[
            BuildingKind::Farm, BuildingKind::FarmerHome,
            BuildingKind::ArcherHome, BuildingKind::CrossbowHome, BuildingKind::Waypoint,
            BuildingKind::Tent, BuildingKind::MinerHome,
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
    bgrid: &BuildingSpatialGrid,
    center: Vec2,
    faction: i32,
) -> Option<(BuildingKind, usize, Vec2)> {
    let mut best_d2 = f32::MAX;
    let mut result: Option<(BuildingKind, usize, Vec2)> = None;
    let r2 = AI_ATTACK_SEARCH_RADIUS * AI_ATTACK_SEARCH_RADIUS;
    bgrid.for_each_nearby(center, AI_ATTACK_SEARCH_RADIUS, |bref| {
        if bref.faction == faction || bref.faction < 0 { return; }
        if bref.kind != BuildingKind::Farm { return; }
        let dx = bref.position.x - center.x;
        let dy = bref.position.y - center.y;
        let d2 = dx * dx + dy * dy;
        if d2 <= r2 && d2 < best_d2 {
            best_d2 = d2;
            result = Some((bref.kind, bref.index, bref.position));
        }
    });
    result
}

fn pick_ai_target_unclaimed(
    bgrid: &BuildingSpatialGrid,
    center: Vec2,
    faction: i32,
    personality: AiPersonality,
    role: SquadRole,
    claimed: &HashSet<(BuildingKind, usize)>,
) -> Option<(BuildingKind, usize, Vec2)> {
    if role != SquadRole::Attack { return None; }

    let find_nearest_unclaimed = |allowed_kinds: &[BuildingKind]| -> Option<(BuildingKind, usize, Vec2)> {
        let mut best_d2 = f32::MAX;
        let mut result: Option<(BuildingKind, usize, Vec2)> = None;
        let r2 = AI_ATTACK_SEARCH_RADIUS * AI_ATTACK_SEARCH_RADIUS;
        bgrid.for_each_nearby(center, AI_ATTACK_SEARCH_RADIUS, |bref| {
            if bref.faction == faction || bref.faction < 0 { return; }
            if !allowed_kinds.contains(&bref.kind) { return; }
            if claimed.contains(&(bref.kind, bref.index)) { return; }
            let dx = bref.position.x - center.x;
            let dy = bref.position.y - center.y;
            let d2 = dx * dx + dy * dy;
            if d2 <= r2 && d2 < best_d2 {
                best_d2 = d2;
                result = Some((bref.kind, bref.index, bref.position));
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
    bgrid: Res<BuildingSpatialGrid>,
    military: Query<(&TownId, &NpcIndex), (With<SquadUnit>, Without<Dead>)>,
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    mut dirty: ResMut<DirtyFlags>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("ai_squad_commander");
    let dt = game_time.delta(&time);

    // Count alive military units per town.
    let mut units_by_town: HashMap<i32, usize> = HashMap::new();
    for (town_id, _) in military.iter() {
        *units_by_town.entry(town_id.0).or_default() += 1;
    }

    for pi in 0..ai_state.players.len() {
        let player = &ai_state.players[pi];
        if !player.active { continue; }

        let tdi = player.town_data_idx;
        let personality = player.personality;
        let kind = player.kind;
        let Some(town) = world_data.towns.get(tdi) else { continue };
        let center = town.center;
        let faction = town.faction;

        // --- Self-healing squad allocation ---
        let desired = match kind {
            AiKind::Builder => personality.desired_squad_count(),
            AiKind::Raider => 1, // single attack squad for raider towns
        };
        let owned: usize = squad_state.squads.iter()
            .filter(|s| s.owner == SquadOwner::Town(tdi))
            .count();
        if owned < desired {
            for _ in owned..desired {
                let idx = squad_state.alloc_squad(SquadOwner::Town(tdi));
                let base_cd = personality.retarget_cooldown();
                let jitter = rand::rng().random_range(0.3..1.0);
                let sq = squad_state.squads.get_mut(idx).unwrap();
                sq.wave_min_start = personality.wave_min_start(kind);
                sq.wave_retreat_below_pct = personality.wave_retreat_pct(kind);
                ai_state.players[pi].squad_cmd.insert(idx, AiSquadCmdState {
                    target_kind: None,
                    target_index: 0,
                    cooldown: base_cd * jitter,
                });
            }
        }

        // Rebuild squad_indices from ownership scan.
        rebuild_squad_indices(&mut ai_state.players[pi], &squad_state.squads);
        let squad_indices = ai_state.players[pi].squad_indices.clone();
        if squad_indices.is_empty() { continue; }

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
                            dirty.squads = true;
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
                                    .map(|i| attack_total * personality.attack_split_weight(i) / total_attack_weight)
                                    .sum();
                                attack_total.saturating_sub(allocated_before)
                            } else {
                                attack_total * personality.attack_split_weight(attack_idx) / total_attack_weight
                            }
                        }
                        SquadRole::Idle => 0,
                    };

                    if let Some(squad) = squad_state.squads.get_mut(si) {
                        if squad.target_size != new_target_size {
                            squad.target_size = new_target_size;
                            dirty.squads = true;
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
        let mut claimed_targets: HashSet<(BuildingKind, usize)> = HashSet::new();
        for &si in &squad_indices {
            let cmd = ai_state.players[pi].squad_cmd.entry(si).or_default();
            if cmd.cooldown > 0.0 { cmd.cooldown -= dt; }

            let Some(squad) = squad_state.squads.get(si) else { continue };

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
                cmd.target_kind = None;
                continue;
            }

            let member_count = squad.members.len();

            if squad.wave_active {
                // --- Wave end conditions ---
                let target_alive = cmd.target_kind
                    .and_then(|k| resolve_building_pos(&world_data, k, cmd.target_index))
                    .is_some();

                let loss_threshold = squad.wave_start_count
                    * squad.wave_retreat_below_pct / 100;
                let heavy_losses = member_count < loss_threshold.max(1);

                if !target_alive || heavy_losses {
                    // End wave — clear target, reset to gathering
                    let reason = if !target_alive { "target cleared" } else { "heavy losses" };
                    let squad = squad_state.squads.get_mut(si).unwrap();
                    squad.wave_active = false;
                    squad.target = None;
                    squad.wave_start_count = 0;
                    cmd.target_kind = None;
                    cmd.cooldown = personality.retarget_cooldown()
                        + rand::rng().random_range(-RETARGET_JITTER..RETARGET_JITTER);

                    let town_name = &town.name;
                    let pname = personality.name();
                    combat_log.push(CombatEventKind::Raid, faction, game_time.day(), game_time.hour(), game_time.minute(),
                        format!("{} [{}] wave ended ({}), {} remaining", town_name, pname, reason, member_count));
                }
            } else {
                // --- Gathering phase: wait for wave_min_start ---
                let min_start = squad.wave_min_start.max(1);
                if member_count < min_start || cmd.cooldown > 0.0 {
                    continue; // not enough members or cooldown active
                }

                // Pick target based on AI kind
                let target = match kind {
                    AiKind::Raider => pick_raider_farm_target(&bgrid, center, faction),
                    AiKind::Builder => pick_ai_target_unclaimed(
                        &bgrid, center, faction, personality, SquadRole::Attack, &claimed_targets,
                    ),
                };

                if let Some((bk, bi, pos)) = target {
                    cmd.target_kind = Some(bk);
                    cmd.target_index = bi;
                    claimed_targets.insert((bk, bi));

                    let squad = squad_state.squads.get_mut(si).unwrap();
                    squad.target = Some(pos);
                    squad.wave_active = true;
                    squad.wave_start_count = member_count;

                    let town_name = &town.name;
                    let pname = personality.name();
                    let unit_label = match kind {
                        AiKind::Raider => "raiders",
                        AiKind::Builder => "units",
                    };
                    combat_log.push_at(CombatEventKind::Raid, faction, game_time.day(), game_time.hour(), game_time.minute(),
                        format!("{} [{}] wave started: {} {} -> {}", town_name, pname, member_count, unit_label, crate::constants::building_def(bk).label),
                        Some(pos));
                }
            }
        }
    }
}

