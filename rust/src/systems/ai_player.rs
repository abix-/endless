//! AI player system — autonomous opponents that build and upgrade like the player.
//! Each AI has a personality (Aggressive/Balanced/Economic) that influences weighted
//! random decisions — same pattern as NPC behavior scoring.
//!
//! Slot selection: economy buildings (farms, houses, barracks) prefer inner slots
//! (closest to center). Guard posts prefer outer slots (farthest from center) with
//! minimum spacing of 5 grid slots between posts.

use bevy::prelude::*;
use rand::Rng;

use crate::constants::*;
use crate::resources::*;
use crate::world::{self, Building, WorldData, WorldGrid, TownGrids};
use crate::systems::stats::{UpgradeQueue, TownUpgrades, upgrade_cost};

/// Minimum Manhattan distance between guard posts on the town grid.
const MIN_GUARD_POST_SPACING: i32 = 5;

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
    BuildHouse,
    BuildBarracks,
    BuildGuardPost,
    BuildTent,
    UnlockSlot,
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
                guard_aggressive: true,
                guard_leash: false,
                farmer_fight_back: true,
                prioritize_healing: false,
                guard_flee_hp: 0.0,
                farmer_flee_hp: 0.30,
                ..PolicySet::default()
            },
            Self::Balanced => PolicySet::default(),
            Self::Economic => PolicySet {
                guard_leash: true,
                prioritize_healing: true,
                guard_flee_hp: 0.25,
                farmer_flee_hp: 0.50,
                ..PolicySet::default()
            },
        }
    }

    /// Base weights for building types: (farm, house, barracks, guard_post)
    fn building_weights(self) -> (f32, f32, f32, f32) {
        match self {
            Self::Aggressive => (10.0, 10.0, 30.0, 20.0),
            Self::Balanced   => (20.0, 20.0, 15.0, 10.0),
            Self::Economic   => (30.0, 25.0,  5.0,  5.0),
        }
    }

    /// Barracks target count relative to houses.
    fn barracks_target(self, houses: usize) -> usize {
        match self {
            Self::Aggressive => houses.max(1),
            Self::Balanced   => (houses / 2).max(1),
            Self::Economic   => 1 + houses / 3,
        }
    }

    /// Upgrade weights indexed by UpgradeType discriminant (12 entries).
    /// Only entries with weight > 0 are scored.
    fn upgrade_weights(self, kind: AiKind) -> [f32; 12] {
        match kind {
            AiKind::Raider => match self {
                //                           GH  GA  GR  GS  AS  MS  AR  FY  FH  HR  FE  FR
                Self::Economic =>           [0., 0., 0., 0., 4., 6., 0., 0., 0., 0., 0., 0.],
                _ =>                        [0., 0., 0., 0., 6., 4., 0., 0., 0., 0., 0., 0.],
            },
            AiKind::Builder => match self {
                //                           GH  GA  GR  GS  AS  MS  AR  FY  FH  HR  FE  FR
                Self::Aggressive =>         [6., 8., 4., 0., 6., 4., 0., 2., 1., 1., 0., 0.],
                Self::Balanced =>           [5., 5., 2., 0., 4., 3., 0., 5., 3., 3., 0., 0.],
                Self::Economic =>           [3., 2., 1., 0., 2., 2., 0., 8., 5., 5., 0., 0.],
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
}

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
    for &(r, c) in &tg.unlocked {
        if r == 0 && c == 0 { continue; }
        let pos = world::town_grid_to_world(center, r, c);
        let (gc, gr) = grid.world_to_grid(pos);
        if grid.cell(gc, gr).map(|cl| cl.building.is_none()) != Some(true) { continue; }
        let dist_sq = r * r + c * c;
        if best.map_or(true, |(_, d)| dist_sq < d) {
            best = Some(((r, c), dist_sq));
        }
    }
    best.map(|(slot, _)| slot)
}

/// Find outermost empty slot at least MIN_GUARD_POST_SPACING from all existing guard posts.
fn find_guard_post_slot(
    tg: &world::TownGrid, center: Vec2, grid: &WorldGrid, world_data: &WorldData, ti: u32,
) -> Option<(i32, i32)> {
    // Existing guard post grid positions for this town
    let existing: Vec<(i32, i32)> = world_data.guard_posts.iter()
        .filter(|gp| gp.town_idx == ti && gp.position.x > -9000.0)
        .map(|gp| world::world_to_town_grid(center, gp.position))
        .collect();

    let mut best: Option<((i32, i32), i32)> = None;
    for &(r, c) in &tg.unlocked {
        if r == 0 && c == 0 { continue; }
        let pos = world::town_grid_to_world(center, r, c);
        let (gc, gr) = grid.world_to_grid(pos);
        if grid.cell(gc, gr).map(|cl| cl.building.is_none()) != Some(true) { continue; }
        // Skip slots too close to existing guard posts
        let too_close = existing.iter().any(|&(er, ec)| {
            (r - er).abs() + (c - ec).abs() < MIN_GUARD_POST_SPACING
        });
        if too_close { continue; }
        let dist_sq = r * r + c * c;
        if best.map_or(true, |(_, d)| dist_sq > d) {
            best = Some(((r, c), dist_sq));
        }
    }
    best.map(|(slot, _)| slot)
}

/// Check if any empty slot exists in the town grid.
fn has_empty_slot(tg: &world::TownGrid, center: Vec2, grid: &WorldGrid) -> bool {
    tg.unlocked.iter().any(|&(r, c)| {
        if r == 0 && c == 0 { return false; }
        let pos = world::town_grid_to_world(center, r, c);
        let (gc, gr) = grid.world_to_grid(pos);
        grid.cell(gc, gr).map(|cl| cl.building.is_none()) == Some(true)
    })
}

// ============================================================================
// AI DECISION SYSTEM
// ============================================================================

/// One decision per AI per interval tick. Scores all eligible actions, picks via weighted random.
pub fn ai_decision_system(
    time: Res<Time>,
    config: Res<AiPlayerConfig>,
    ai_state: Res<AiPlayerState>,
    mut grid: ResMut<WorldGrid>,
    mut world_data: ResMut<WorldData>,
    mut farm_states: ResMut<FarmStates>,
    mut food_storage: ResMut<FoodStorage>,
    mut town_grids: ResMut<TownGrids>,
    mut spawner_state: ResMut<SpawnerState>,
    mut upgrade_queue: ResMut<UpgradeQueue>,
    upgrades: Res<TownUpgrades>,
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    mut timer: Local<f32>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("ai_decision");
    *timer += time.delta_secs();
    if *timer < config.decision_interval { return; }
    *timer = 0.0;

    for player in ai_state.players.iter() {
        let tdi = player.town_data_idx;
        let food = food_storage.food.get(tdi).copied().unwrap_or(0);
        let reserve = player.personality.food_reserve();
        if food <= reserve { continue; }

        let center = world_data.towns.get(tdi).map(|t| t.center).unwrap_or_default();
        let town_name = world_data.towns.get(tdi).map(|t| t.name.clone()).unwrap_or_default();
        let pname = player.personality.name();
        let ti = tdi as u32;

        let alive = |pos: Vec2, idx: u32| idx == ti && pos.x > -9000.0;
        let farms = world_data.farms.iter().filter(|f| alive(f.position, f.town_idx)).count();
        let houses = world_data.houses.iter().filter(|h| alive(h.position, h.town_idx)).count();
        let barracks = world_data.barracks.iter().filter(|b| alive(b.position, b.town_idx)).count();
        let guard_posts = world_data.guard_posts.iter().filter(|g| alive(g.position, g.town_idx)).count();

        let has_slots = town_grids.grids.get(player.grid_idx)
            .map(|tg| has_empty_slot(tg, center, &grid))
            .unwrap_or(false);
        let can_unlock = !has_slots && town_grids.grids.get(player.grid_idx)
            .map(|tg| !world::get_adjacent_locked_slots(tg).is_empty())
            .unwrap_or(false);

        // Score all eligible actions
        let mut scores: Vec<(AiAction, f32)> = Vec::with_capacity(8);

        match player.kind {
            AiKind::Raider => {
                // Tents (only building raiders make)
                if has_slots && food >= TENT_BUILD_COST {
                    scores.push((AiAction::BuildTent, 30.0));
                }
            }
            AiKind::Builder => {
                let (fw, hw, bw, gw) = player.personality.building_weights();
                let bt = player.personality.barracks_target(houses);

                if has_slots {
                    // Need factors: 1.0 base + deficit (higher when behind target ratio)
                    let farm_need = 1.0 + (houses as f32 - farms as f32).max(0.0);
                    let house_need = 1.0 + (farms as f32 - houses as f32).max(0.0);
                    let barracks_need = if barracks < bt { 1.0 + (bt - barracks) as f32 } else { 0.5 };
                    let gp_need = if guard_posts < barracks { 1.0 + (barracks - guard_posts) as f32 } else { 0.5 };

                    if food >= FARM_BUILD_COST { scores.push((AiAction::BuildFarm, fw * farm_need)); }
                    if food >= HOUSE_BUILD_COST { scores.push((AiAction::BuildHouse, hw * house_need)); }
                    if food >= BARRACKS_BUILD_COST { scores.push((AiAction::BuildBarracks, bw * barracks_need)); }
                    if food >= GUARD_POST_BUILD_COST { scores.push((AiAction::BuildGuardPost, gw * gp_need)); }
                }
            }
        }

        // Unlock slot
        if can_unlock && food >= SLOT_UNLOCK_COST {
            scores.push((AiAction::UnlockSlot, 8.0));
        }

        // Upgrades
        let uw = player.personality.upgrade_weights(player.kind);
        for (idx, &weight) in uw.iter().enumerate() {
            if weight <= 0.0 { continue; }
            let level = upgrades.levels.get(tdi).map(|l| l[idx]).unwrap_or(0);
            if food >= upgrade_cost(level) {
                scores.push((AiAction::Upgrade(idx), weight));
            }
        }

        // Pick and execute
        let Some(action) = weighted_pick(&scores) else { continue };
        let label = execute_action(
            action, ti, tdi, center, guard_posts,
            &mut grid, &mut world_data, &mut farm_states, &mut food_storage,
            &mut town_grids, &mut spawner_state, &mut upgrade_queue,
            player.grid_idx,
        );
        if let Some(what) = label {
            log_ai(&mut combat_log, &game_time, &town_name, pname, &what);
        }
    }
}

/// Execute the chosen action, returning a log label on success.
#[allow(clippy::too_many_arguments)]
fn execute_action(
    action: AiAction, ti: u32, tdi: usize, center: Vec2, guard_posts: usize,
    grid: &mut WorldGrid, world_data: &mut WorldData, farm_states: &mut FarmStates,
    food_storage: &mut FoodStorage, town_grids: &mut TownGrids,
    spawner_state: &mut SpawnerState, upgrade_queue: &mut UpgradeQueue,
    grid_idx: usize,
) -> Option<String> {
    match action {
        AiAction::BuildTent => {
            let tg = town_grids.grids.get(grid_idx)?;
            let (row, col) = find_inner_slot(tg, center, grid)?;
            try_build(grid, world_data, farm_states, food_storage, spawner_state,
                Building::Tent { town_idx: ti }, 2, tdi, row, col, center, TENT_BUILD_COST)
                .then_some("built tent".into())
        }
        AiAction::BuildFarm => {
            let tg = town_grids.grids.get(grid_idx)?;
            let (row, col) = find_inner_slot(tg, center, grid)?;
            try_build(grid, world_data, farm_states, food_storage, spawner_state,
                Building::Farm { town_idx: ti }, -1, tdi, row, col, center, FARM_BUILD_COST)
                .then_some("built farm".into())
        }
        AiAction::BuildHouse => {
            let tg = town_grids.grids.get(grid_idx)?;
            let (row, col) = find_inner_slot(tg, center, grid)?;
            try_build(grid, world_data, farm_states, food_storage, spawner_state,
                Building::House { town_idx: ti }, 0, tdi, row, col, center, HOUSE_BUILD_COST)
                .then_some("built house".into())
        }
        AiAction::BuildBarracks => {
            let tg = town_grids.grids.get(grid_idx)?;
            let (row, col) = find_inner_slot(tg, center, grid)?;
            try_build(grid, world_data, farm_states, food_storage, spawner_state,
                Building::Barracks { town_idx: ti }, 1, tdi, row, col, center, BARRACKS_BUILD_COST)
                .then_some("built barracks".into())
        }
        AiAction::BuildGuardPost => {
            let tg = town_grids.grids.get(grid_idx)?;
            let (row, col) = find_guard_post_slot(tg, center, grid, world_data, ti)?;
            try_build(grid, world_data, farm_states, food_storage, spawner_state,
                Building::GuardPost { town_idx: ti, patrol_order: guard_posts as u32 },
                -1, tdi, row, col, center, GUARD_POST_BUILD_COST)
                .then_some("built guard post".into())
        }
        AiAction::UnlockSlot => {
            let tg = town_grids.grids.get(grid_idx)?;
            let adjacent = world::get_adjacent_locked_slots(tg);
            let &(row, col) = adjacent.first()?;
            let f = food_storage.food.get(tdi).copied().unwrap_or(0);
            if f < SLOT_UNLOCK_COST { return None; }
            if let Some(f) = food_storage.food.get_mut(tdi) { *f -= SLOT_UNLOCK_COST; }
            let slot_pos = world::town_grid_to_world(center, row, col);
            let (gc, gr) = grid.world_to_grid(slot_pos);
            if let Some(cell) = grid.cell_mut(gc, gr) {
                cell.terrain = world::Biome::Dirt;
            }
            if let Some(tg) = town_grids.grids.get_mut(grid_idx) {
                tg.unlocked.insert((row, col));
            }
            Some("unlocked slot".into())
        }
        AiAction::Upgrade(idx) => {
            upgrade_queue.0.push((tdi, idx));
            let name = match idx {
                0 => "GuardHealth", 1 => "GuardAttack", 2 => "GuardRange",
                3 => "GuardSize", 4 => "AttackSpeed", 5 => "MoveSpeed",
                6 => "AlertRadius", 7 => "FarmYield", 8 => "FarmerHp",
                9 => "HealingRate", 10 => "FoodEfficiency", 11 => "FountainRadius",
                _ => "Unknown",
            };
            Some(format!("upgraded {name}"))
        }
    }
}

/// Place building, deduct food, push spawner entry if applicable.
#[allow(clippy::too_many_arguments)]
fn try_build(
    grid: &mut WorldGrid, world_data: &mut WorldData, farm_states: &mut FarmStates,
    food_storage: &mut FoodStorage, spawner_state: &mut SpawnerState,
    building: Building, spawner_kind: i32, tdi: usize,
    row: i32, col: i32, center: Vec2, cost: i32,
) -> bool {
    if world::place_building(grid, world_data, farm_states, building, row, col, center).is_err() {
        return false;
    }
    if let Some(f) = food_storage.food.get_mut(tdi) { *f -= cost; }
    if spawner_kind >= 0 {
        let pos = world::town_grid_to_world(center, row, col);
        let (gc, gr) = grid.world_to_grid(pos);
        let snapped = grid.grid_to_world(gc, gr);
        spawner_state.0.push(SpawnerEntry {
            building_kind: spawner_kind, town_idx: tdi as i32,
            position: snapped, npc_slot: -1, respawn_timer: 0.0,
        });
    }
    true
}

fn log_ai(log: &mut CombatLog, gt: &GameTime, town: &str, personality: &str, what: &str) {
    log.push(CombatEventKind::Ai, gt.day(), gt.hour(), gt.minute(),
        format!("{} [{}] {}", town, personality, what));
}
