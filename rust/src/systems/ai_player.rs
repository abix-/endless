//! AI player system — autonomous opponents that build and upgrade like the player.
//! Each AI has a personality (Aggressive/Balanced/Economic) that drives build order,
//! upgrade priorities, town policies, and food management.
//!
//! Slot selection: economy buildings (farms, houses, barracks) prefer inner slots
//! (closest to center). Guard posts prefer outer slots (farthest from center) with
//! minimum spacing of 5 grid slots between posts.

use bevy::prelude::*;

use crate::constants::*;
use crate::resources::*;
use crate::world::{self, Building, WorldData, WorldGrid, TownGrids};
use crate::systems::stats::{UpgradeQueue, UpgradeType, TownUpgrades, upgrade_cost};

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

/// What the AI decided to build (before slot selection).
#[derive(Clone, Copy)]
enum BuildChoice { Farm, House, Barracks, GuardPost }

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

    /// Decide what to build based on personality priorities and current counts.
    /// Returns None if nothing should be built (ratios satisfied or can't afford).
    fn pick_building_type(
        self, farms: usize, houses: usize, barracks: usize, guard_posts: usize, food: i32,
    ) -> Option<BuildChoice> {
        match self {
            // Aggressive: military first — barracks, guard posts, then economy
            Self::Aggressive => {
                if barracks < houses.max(1) && food >= BARRACKS_BUILD_COST { return Some(BuildChoice::Barracks); }
                if guard_posts < barracks && food >= GUARD_POST_BUILD_COST { return Some(BuildChoice::GuardPost); }
                if farms < houses && food >= FARM_BUILD_COST { return Some(BuildChoice::Farm); }
                if houses <= farms && food >= HOUSE_BUILD_COST { return Some(BuildChoice::House); }
                None
            }
            // Balanced: economy and military in tandem
            Self::Balanced => {
                if farms < houses && food >= FARM_BUILD_COST { return Some(BuildChoice::Farm); }
                if houses <= farms && food >= HOUSE_BUILD_COST { return Some(BuildChoice::House); }
                if (barracks == 0 || barracks < houses / 2) && food >= BARRACKS_BUILD_COST { return Some(BuildChoice::Barracks); }
                if guard_posts < barracks && food >= GUARD_POST_BUILD_COST { return Some(BuildChoice::GuardPost); }
                None
            }
            // Economic: farms first, minimal military
            Self::Economic => {
                if farms <= houses && food >= FARM_BUILD_COST { return Some(BuildChoice::Farm); }
                if houses < farms && food >= HOUSE_BUILD_COST { return Some(BuildChoice::House); }
                if barracks < 1 + houses / 3 && food >= BARRACKS_BUILD_COST { return Some(BuildChoice::Barracks); }
                if guard_posts < barracks && food >= GUARD_POST_BUILD_COST { return Some(BuildChoice::GuardPost); }
                None
            }
        }
    }

    /// Upgrade priority list for Builder AI.
    fn builder_upgrades(self) -> &'static [UpgradeType] {
        match self {
            Self::Aggressive => &[
                UpgradeType::GuardAttack, UpgradeType::AttackSpeed,
                UpgradeType::GuardHealth, UpgradeType::MoveSpeed, UpgradeType::GuardRange,
            ],
            Self::Balanced => &[
                UpgradeType::GuardHealth, UpgradeType::GuardAttack,
                UpgradeType::FarmYield, UpgradeType::AttackSpeed, UpgradeType::MoveSpeed,
            ],
            Self::Economic => &[
                UpgradeType::FarmYield, UpgradeType::FarmerHp,
                UpgradeType::HealingRate, UpgradeType::GuardHealth, UpgradeType::GuardAttack,
            ],
        }
    }

    /// Upgrade priority list for Raider AI.
    fn raider_upgrades(self) -> &'static [UpgradeType] {
        match self {
            Self::Economic => &[UpgradeType::MoveSpeed, UpgradeType::AttackSpeed],
            _ => &[UpgradeType::AttackSpeed, UpgradeType::MoveSpeed],
        }
    }
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

/// One decision per AI per interval tick. Accumulates real time via Local timer.
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
) {
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

        // Phase 1: Build — decide WHAT, then pick WHERE
        if has_slots {
            let built = match player.kind {
                AiKind::Raider => {
                    if player.personality == AiPersonality::Economic && food < 20 {
                        None
                    } else if let Some(tg) = town_grids.grids.get(player.grid_idx) {
                        // Raiders use inner slots for tents (cluster around camp)
                        find_inner_slot(tg, center, &grid).and_then(|(row, col)| {
                            try_build(&mut grid, &mut world_data, &mut farm_states, &mut food_storage, &mut spawner_state,
                                Building::Tent { town_idx: ti }, 2, tdi, row, col, center, TENT_BUILD_COST)
                                .then_some("built tent")
                        })
                    } else { None }
                }
                AiKind::Builder => {
                    if let Some(choice) = player.personality.pick_building_type(farms, houses, barracks, guard_posts, food) {
                        if let Some(tg) = town_grids.grids.get(player.grid_idx) {
                            // Guard posts → outermost slot with spacing; everything else → innermost
                            let slot = match choice {
                                BuildChoice::GuardPost => find_guard_post_slot(tg, center, &grid, &world_data, ti),
                                _ => find_inner_slot(tg, center, &grid),
                            };
                            slot.and_then(|(row, col)| {
                                let (building, spawner_kind, cost, label) = match choice {
                                    BuildChoice::Farm => (Building::Farm { town_idx: ti }, -1, FARM_BUILD_COST, "built farm"),
                                    BuildChoice::House => (Building::House { town_idx: ti }, 0, HOUSE_BUILD_COST, "built house"),
                                    BuildChoice::Barracks => (Building::Barracks { town_idx: ti }, 1, BARRACKS_BUILD_COST, "built barracks"),
                                    BuildChoice::GuardPost => (
                                        Building::GuardPost { town_idx: ti, patrol_order: guard_posts as u32 },
                                        -1, GUARD_POST_BUILD_COST, "built guard post",
                                    ),
                                };
                                try_build(&mut grid, &mut world_data, &mut farm_states, &mut food_storage, &mut spawner_state,
                                    building, spawner_kind, tdi, row, col, center, cost)
                                    .then_some(label)
                            })
                        } else { None }
                    } else { None }
                }
            };
            if let Some(what) = built {
                log_ai(&mut combat_log, &game_time, &town_name, pname, what);
                continue;
            }
        }

        // Phase 2: Unlock slot if no empties
        if !has_slots {
            if let Some(tg) = town_grids.grids.get(player.grid_idx) {
                let adjacent = world::get_adjacent_locked_slots(tg);
                if let Some(&(row, col)) = adjacent.first() {
                    let f = food_storage.food.get(tdi).copied().unwrap_or(0);
                    if f >= SLOT_UNLOCK_COST {
                        if let Some(f) = food_storage.food.get_mut(tdi) { *f -= SLOT_UNLOCK_COST; }
                        // Set terrain to dirt at the unlocked slot
                        let slot_pos = world::town_grid_to_world(center, row, col);
                        let (gc, gr) = grid.world_to_grid(slot_pos);
                        if let Some(cell) = grid.cell_mut(gc, gr) {
                            cell.terrain = world::Biome::Dirt;
                        }
                        if let Some(tg) = town_grids.grids.get_mut(player.grid_idx) {
                            tg.unlocked.insert((row, col));
                        }
                        log_ai(&mut combat_log, &game_time, &town_name, pname, "unlocked slot");
                        continue;
                    }
                }
            }
        }

        // Phase 3: Upgrades (personality-driven priority)
        let to_try = match player.kind {
            AiKind::Raider => player.personality.raider_upgrades(),
            AiKind::Builder => player.personality.builder_upgrades(),
        };
        for &ut in to_try {
            let idx = ut as usize;
            let level = upgrades.levels.get(tdi).map(|l| l[idx]).unwrap_or(0);
            if food >= upgrade_cost(level) {
                upgrade_queue.0.push((tdi, idx));
                log_ai(&mut combat_log, &game_time, &town_name, pname, &format!("upgraded {:?}", ut));
                break;
            }
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
