//! AI player system — autonomous opponents that build and upgrade like the player.
//! Each AI has a personality (Aggressive/Balanced/Economic) that influences weighted
//! random decisions — same pattern as NPC behavior scoring.
//!
//! Slot selection: economy buildings (farms, houses, barracks) prefer inner slots
//! (closest to center). Guard posts prefer outer slots (farthest from center) with
//! minimum spacing of 5 grid slots between posts.

use std::collections::VecDeque;

use bevy::prelude::*;
use bevy::ecs::system::SystemParam;
use rand::Rng;

use crate::constants::*;
use crate::resources::*;
use crate::systemparams::WorldState;
use crate::world::{self, Building, WorldData, WorldGrid};
use crate::systems::stats::{UpgradeQueue, TownUpgrades, upgrade_node, upgrade_available, UPGRADE_COUNT};

/// Mutable world resources needed for AI building. Bundled to stay under Bevy's 16-param limit.
#[derive(SystemParam)]
pub struct AiBuildRes<'w> {
    world: WorldState<'w>,
    food_storage: ResMut<'w, FoodStorage>,
    upgrade_queue: ResMut<'w, UpgradeQueue>,
}

/// Minimum Manhattan distance between waypoints on the town grid.
const MIN_WAYPOINT_SPACING: i32 = 5;

#[derive(Resource)]
pub struct AiPlayerConfig {
    pub decision_interval: f32,
}

impl Default for AiPlayerConfig {
    fn default() -> Self { Self { decision_interval: DEFAULT_AI_INTERVAL } }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AiKind { Raider, Builder }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AiPersonality { Aggressive, Balanced, Economic }

/// All possible AI actions, scored and picked via weighted random.
#[derive(Clone, Copy, Debug)]
enum AiAction {
    BuildFarm,
    BuildFarmerHome,
    BuildArcherHome,
    BuildWaypoint,
    BuildTent,
    BuildMinerHome,
    Upgrade(usize), // upgrade index into UPGRADE_PCT
}

impl AiPersonality {
    pub fn name(self) -> &'static str {
        match self {
            Self::Aggressive => "Aggressive",
            Self::Balanced => "Balanced",
            Self::Economic => "Economic",
        }
    }

    /// Minimum food the AI hoards before spending.
    pub fn food_reserve(self) -> i32 {
        match self {
            Self::Aggressive => 0,
            Self::Balanced => 10,
            Self::Economic => 30,
        }
    }

    /// Town policies tuned per personality.
    pub fn default_policies(self) -> PolicySet {
        match self {
            Self::Aggressive => PolicySet {
                archer_aggressive: true,
                archer_leash: false,
                farmer_fight_back: true,
                prioritize_healing: false,
                archer_flee_hp: 0.0,
                farmer_flee_hp: 0.30,
                ..PolicySet::default()
            },
            Self::Balanced => PolicySet::default(),
            Self::Economic => PolicySet {
                archer_leash: true,
                prioritize_healing: true,
                archer_flee_hp: 0.25,
                farmer_flee_hp: 0.50,
                ..PolicySet::default()
            },
        }
    }

    /// Base weights for building types: (farm, house, barracks, waypoint)
    fn building_weights(self) -> (f32, f32, f32, f32) {
        match self {
            Self::Aggressive => (10.0, 10.0, 30.0, 20.0),
            Self::Balanced   => (20.0, 20.0, 15.0, 10.0),
            Self::Economic   => (30.0, 25.0,  5.0,  5.0),
        }
    }

    /// Barracks target count relative to houses.
    fn archer_home_target(self, houses: usize) -> usize {
        match self {
            Self::Aggressive => houses.max(1),
            Self::Balanced   => (houses / 2).max(1),
            Self::Economic   => 1 + houses / 3,
        }
    }

    /// Miner home target count relative to houses.
    fn miner_home_target(self, houses: usize) -> usize {
        match self {
            Self::Aggressive => (houses / 4).max(1),
            Self::Balanced   => (houses / 2).max(1),
            Self::Economic   => houses.max(1),
        }
    }

    /// Upgrade weights indexed by UpgradeType discriminant (16 entries).
    /// Only entries with weight > 0 are scored.
    fn upgrade_weights(self, kind: AiKind) -> [f32; UPGRADE_COUNT] {
        match kind {
            //                             MHP MAt MRn AS  MMS Alt Ddg FYd FHP FMS mHP mMS GYd Hel Fnt Exp
            AiKind::Raider => match self {
                Self::Economic =>         [4., 4., 0., 4., 6., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 2.],
                _ =>                      [4., 6., 2., 6., 4., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 2.],
            },
            AiKind::Builder => match self {
                Self::Aggressive =>       [6., 8., 4., 6., 4., 0., 0., 2., 1., 0., 1., 0., 1., 1., 0., 8.],
                Self::Balanced =>         [5., 5., 2., 4., 3., 0., 0., 5., 3., 1., 3., 1., 2., 3., 0., 10.],
                Self::Economic =>         [3., 2., 1., 2., 2., 0., 0., 8., 5., 2., 5., 2., 4., 5., 0., 12.],
            },
        }
    }
}

/// Weighted random selection from scored actions.
fn weighted_pick(scores: &[(AiAction, f32)]) -> Option<AiAction> {
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

pub struct AiPlayer {
    pub town_data_idx: usize,
    pub grid_idx: usize,
    pub kind: AiKind,
    pub personality: AiPersonality,
    pub last_actions: VecDeque<String>,
    pub active: bool,
}

const MAX_ACTION_HISTORY: usize = 3;

#[derive(Resource, Default)]
pub struct AiPlayerState {
    pub players: Vec<AiPlayer>,
}

// ============================================================================
// SLOT SELECTION
// ============================================================================

/// Find best empty slot closest to town center (for economy buildings).
fn find_inner_slot(
    tg: &world::TownGrid, center: Vec2, grid: &WorldGrid,
) -> Option<(i32, i32)> {
    let mut best: Option<((i32, i32), i32)> = None;
    let (min_row, max_row, min_col, max_col) = world::build_bounds(tg);
    for r in min_row..=max_row {
        for c in min_col..=max_col {
            if r == 0 && c == 0 { continue; }
            let pos = world::town_grid_to_world(center, r, c);
            let (gc, gr) = grid.world_to_grid(pos);
            if grid.cell(gc, gr).map(|cl| cl.building.is_none()) != Some(true) { continue; }
            let dist_sq = r * r + c * c;
            if best.map_or(true, |(_, d)| dist_sq < d) {
                best = Some(((r, c), dist_sq));
            }
        }
    }
    best.map(|(slot, _)| slot)
}

/// Find outermost empty slot at least MIN_WAYPOINT_SPACING from all existing waypoints.
fn find_waypoint_slot(
    tg: &world::TownGrid, center: Vec2, grid: &WorldGrid, world_data: &WorldData, ti: u32,
) -> Option<(i32, i32)> {
    // Existing waypoint grid positions for this town
    let existing: Vec<(i32, i32)> = world_data.waypoints.iter()
        .filter(|gp| gp.town_idx == ti && gp.position.x > -9000.0)
        .map(|gp| world::world_to_town_grid(center, gp.position))
        .collect();

    let mut best: Option<((i32, i32), i32)> = None;
    let (min_row, max_row, min_col, max_col) = world::build_bounds(tg);
    for r in min_row..=max_row {
        for c in min_col..=max_col {
            if r == 0 && c == 0 { continue; }
            let pos = world::town_grid_to_world(center, r, c);
            let (gc, gr) = grid.world_to_grid(pos);
            if grid.cell(gc, gr).map(|cl| cl.building.is_none()) != Some(true) { continue; }
            // Skip slots too close to existing waypoints
            let too_close = existing.iter().any(|&(er, ec)| {
                (r - er).abs() + (c - ec).abs() < MIN_WAYPOINT_SPACING
            });
            if too_close { continue; }
            let dist_sq = r * r + c * c;
            if best.map_or(true, |(_, d)| dist_sq > d) {
                best = Some(((r, c), dist_sq));
            }
        }
    }
    best.map(|(slot, _)| slot)
}

/// Count empty buildable slots in a town grid.
fn count_empty_slots(tg: &world::TownGrid, center: Vec2, grid: &WorldGrid) -> i32 {
    let (min_row, max_row, min_col, max_col) = world::build_bounds(tg);
    let mut count = 0;
    for r in min_row..=max_row {
        for c in min_col..=max_col {
            if r == 0 && c == 0 { continue; }
            let pos = world::town_grid_to_world(center, r, c);
            let (gc, gr) = grid.world_to_grid(pos);
            if grid.cell(gc, gr).map(|cl| cl.building.is_none()) == Some(true) {
                count += 1;
            }
        }
    }
    count
}

/// Check if any empty slot exists in the town grid.
fn has_empty_slot(tg: &world::TownGrid, center: Vec2, grid: &WorldGrid) -> bool {
    let (min_row, max_row, min_col, max_col) = world::build_bounds(tg);
    for r in min_row..=max_row {
        for c in min_col..=max_col {
            if r == 0 && c == 0 { continue; }
            let pos = world::town_grid_to_world(center, r, c);
            let (gc, gr) = grid.world_to_grid(pos);
            if grid.cell(gc, gr).map(|cl| cl.building.is_none()) == Some(true) {
                return true;
            }
        }
    }
    false
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
    mut timer: Local<f32>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("ai_decision");
    *timer += time.delta_secs();
    if *timer < config.decision_interval { return; }
    *timer = 0.0;

    for pi in 0..ai_state.players.len() {
        let player = &ai_state.players[pi];
        if !player.active { continue; }
        let tdi = player.town_data_idx;
        let food = res.food_storage.food.get(tdi).copied().unwrap_or(0);
        let reserve = player.personality.food_reserve();
        if food <= reserve { continue; }

        let center = res.world.world_data.towns.get(tdi).map(|t| t.center).unwrap_or_default();
        let town_name = res.world.world_data.towns.get(tdi).map(|t| t.name.clone()).unwrap_or_default();
        let pname = player.personality.name();
        let ti = tdi as u32;

        let alive = |pos: Vec2, idx: u32| idx == ti && pos.x > -9000.0;
        let farms = res.world.world_data.farms.iter().filter(|f| alive(f.position, f.town_idx)).count();
        let houses = res.world.world_data.farmer_homes.iter().filter(|h| alive(h.position, h.town_idx)).count();
        let barracks = res.world.world_data.archer_homes.iter().filter(|b| alive(b.position, b.town_idx)).count();
        let waypoints = res.world.world_data.waypoints.iter().filter(|g| alive(g.position, g.town_idx)).count();
        let mine_shafts = res.world.world_data.miner_homes.iter().filter(|ms| alive(ms.position, ms.town_idx)).count();

        let has_slots = res.world.town_grids.grids.get(player.grid_idx)
            .map(|tg| has_empty_slot(tg, center, &res.world.grid))
            .unwrap_or(false);

        let slot_fullness = res.world.town_grids.grids.get(player.grid_idx)
            .map(|tg| {
                let (min_r, max_r, min_c, max_c) = world::build_bounds(tg);
                let total = ((max_r - min_r + 1) * (max_c - min_c + 1) - 1) as f32; // -1 for center
                let empty = count_empty_slots(tg, center, &res.world.grid);
                1.0 - empty as f32 / total.max(1.0)
            })
            .unwrap_or(0.0);

        // Score all eligible actions
        let mut scores: Vec<(AiAction, f32)> = Vec::with_capacity(8);

        match player.kind {
            AiKind::Raider => {
                // Tents (only building raiders make)
                if has_slots && food >= building_cost(BuildKind::Tent) {
                    scores.push((AiAction::BuildTent, 30.0));
                }
            }
            AiKind::Builder => {
                let (fw, hw, bw, gw) = player.personality.building_weights();
                let bt = player.personality.archer_home_target(houses);

                if has_slots {
                    // Need factors: 1.0 base + deficit (higher when behind target ratio)
                    let farm_need = 1.0 + (houses as f32 - farms as f32).max(0.0);
                    let house_need = 1.0 + (farms as f32 - houses as f32).max(0.0);
                    let barracks_need = if barracks < bt { 1.0 + (bt - barracks) as f32 } else { 0.5 };
                    let gp_need = if waypoints < barracks { 1.0 + (barracks - waypoints) as f32 } else { 0.5 };

                    if food >= building_cost(BuildKind::Farm) { scores.push((AiAction::BuildFarm, fw * farm_need)); }
                    if food >= building_cost(BuildKind::FarmerHome) { scores.push((AiAction::BuildFarmerHome, hw * house_need)); }
                    if food >= building_cost(BuildKind::ArcherHome) { scores.push((AiAction::BuildArcherHome, bw * barracks_need)); }
                    if food >= building_cost(BuildKind::Waypoint) { scores.push((AiAction::BuildWaypoint, gw * gp_need)); }
                    let ms_target = player.personality.miner_home_target(houses);
                    if mine_shafts < ms_target && food >= building_cost(BuildKind::MinerHome) {
                        let ms_need = 1.0 + (ms_target - mine_shafts) as f32;
                        scores.push((AiAction::BuildMinerHome, hw * ms_need));
                    }
                }
            }
        }

        // Upgrades
        let uw = player.personality.upgrade_weights(player.kind);
        let levels = upgrades.town_levels(tdi);
        let gold = gold_storage.gold.get(tdi).copied().unwrap_or(0);
        for (idx, &weight) in uw.iter().enumerate() {
            if weight <= 0.0 { continue; }
            if !upgrade_available(&levels, idx, food, gold) { continue; }
            let mut w = weight;
            // Expansion (idx 15) urgency scales with slot fullness
            if idx == 15 {
                if slot_fullness > 0.7 {
                    w *= 2.0 + 4.0 * (slot_fullness - 0.7) / 0.3;
                }
                if !has_slots {
                    w *= 10.0;
                }
            }
            scores.push((AiAction::Upgrade(idx), w));
        }

        // Pick and execute
        let Some(action) = weighted_pick(&scores) else { continue };
        let label = execute_action(
            action, ti, tdi, center, waypoints, &mut res,
            player.grid_idx, *difficulty,
        );
        if let Some(what) = label {
            log_ai(&mut combat_log, &game_time, &town_name, pname, &what);
            let actions = &mut ai_state.players[pi].last_actions;
            if actions.len() >= MAX_ACTION_HISTORY { actions.pop_front(); }
            actions.push_back(what);
        }
    }
}

/// Try to build a standard building at the nearest inner slot.
fn try_build_inner(
    building: Building, cost: i32, label: &str,
    tdi: usize, center: Vec2, res: &mut AiBuildRes, grid_idx: usize,
) -> Option<String> {
    let tg = res.world.town_grids.grids.get(grid_idx)?;
    let (row, col) = find_inner_slot(tg, center, &res.world.grid)?;
    let ok = world::build_and_pay(&mut res.world.grid, &mut res.world.world_data, &mut res.world.farm_states,
        &mut res.food_storage, &mut res.world.spawner_state, &mut res.world.building_hp,
        building, tdi, row, col, center, cost);
    if ok { res.world.dirty.building_grid = true; }
    ok.then_some(format!("built {label}"))
}

/// Execute the chosen action, returning a log label on success.
fn execute_action(
    action: AiAction, ti: u32, tdi: usize, center: Vec2, waypoints: usize,
    res: &mut AiBuildRes, grid_idx: usize, _difficulty: Difficulty,
) -> Option<String> {
    match action {
        AiAction::BuildTent => try_build_inner(
            Building::Tent { town_idx: ti }, building_cost(BuildKind::Tent), "tent",
            tdi, center, res, grid_idx),
        AiAction::BuildFarm => try_build_inner(
            Building::Farm { town_idx: ti }, building_cost(BuildKind::Farm), "farm",
            tdi, center, res, grid_idx),
        AiAction::BuildFarmerHome => try_build_inner(
            Building::FarmerHome { town_idx: ti }, building_cost(BuildKind::FarmerHome), "farmer home",
            tdi, center, res, grid_idx),
        AiAction::BuildArcherHome => try_build_inner(
            Building::ArcherHome { town_idx: ti }, building_cost(BuildKind::ArcherHome), "archer home",
            tdi, center, res, grid_idx),
        AiAction::BuildMinerHome => try_build_inner(
            Building::MinerHome { town_idx: ti }, building_cost(BuildKind::MinerHome), "miner home",
            tdi, center, res, grid_idx),
        AiAction::BuildWaypoint => {
            let cost = building_cost(BuildKind::Waypoint);
            let tg = res.world.town_grids.grids.get(grid_idx)?;
            let (row, col) = find_waypoint_slot(tg, center, &res.world.grid, &res.world.world_data, ti)?;
            let ok = world::build_and_pay(&mut res.world.grid, &mut res.world.world_data, &mut res.world.farm_states,
                &mut res.food_storage, &mut res.world.spawner_state, &mut res.world.building_hp,
                Building::Waypoint { town_idx: ti, patrol_order: waypoints as u32 },
                tdi, row, col, center, cost);
            if ok {
                res.world.dirty.patrols = true;
                res.world.dirty.building_grid = true;
                res.world.dirty.waypoint_slots = true;
            }
            ok.then_some("built waypoint".into())
        }
        AiAction::Upgrade(idx) => {
            res.upgrade_queue.0.push((tdi, idx));
            let name = upgrade_node(idx).label;
            Some(format!("upgraded {name}"))
        }
    }
}


fn log_ai(log: &mut CombatLog, gt: &GameTime, town: &str, personality: &str, what: &str) {
    log.push(CombatEventKind::Ai, gt.day(), gt.hour(), gt.minute(),
        format!("{} [{}] {}", town, personality, what));
}
