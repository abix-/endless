//! AI player system — autonomous opponents that build and upgrade like the player.

use bevy::prelude::*;

use crate::constants::*;
use crate::resources::*;
use crate::world::{self, Building, WorldData, WorldGrid, TownGrids};
use crate::systems::stats::{UpgradeQueue, UpgradeType, TownUpgrades, upgrade_cost};

#[derive(Resource)]
pub struct AiPlayerConfig {
    pub decision_interval: f32,
}

impl Default for AiPlayerConfig {
    fn default() -> Self { Self { decision_interval: DEFAULT_AI_INTERVAL } }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AiKind { Raider, Builder }

pub struct AiPlayer {
    pub town_data_idx: usize,
    pub grid_idx: usize,
    pub kind: AiKind,
}

#[derive(Resource, Default)]
pub struct AiPlayerState {
    pub players: Vec<AiPlayer>,
}

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
        if food <= 0 { continue; }

        let center = world_data.towns.get(tdi).map(|t| t.center).unwrap_or_default();
        let town_name = world_data.towns.get(tdi).map(|t| t.name.clone()).unwrap_or_default();
        let ti = tdi as u32;

        let alive = |pos: Vec2, idx: u32| idx == ti && pos.x > -9000.0;
        let farms = world_data.farms.iter().filter(|f| alive(f.position, f.town_idx)).count();
        let houses = world_data.houses.iter().filter(|h| alive(h.position, h.town_idx)).count();
        let barracks = world_data.barracks.iter().filter(|b| alive(b.position, b.town_idx)).count();
        let guard_posts = world_data.guard_posts.iter().filter(|g| alive(g.position, g.town_idx)).count();

        let empty_slot = town_grids.grids.get(player.grid_idx)
            .and_then(|tg| {
                tg.unlocked.iter().find(|&&(r, c)| {
                    if r == 0 && c == 0 { return false; }
                    let pos = world::town_grid_to_world(center, r, c);
                    let (gc, gr) = grid.world_to_grid(pos);
                    grid.cell(gc, gr).map(|c| c.building.is_none()).unwrap_or(false)
                }).copied()
            });

        // Phase 1: Build (first affordable wins)
        if let Some((row, col)) = empty_slot {
            let built = match player.kind {
                AiKind::Raider => {
                    try_build(&mut grid, &mut world_data, &mut farm_states, &mut food_storage, &mut spawner_state,
                        Building::Tent { town_idx: ti }, 2, tdi, row, col, center, TENT_BUILD_COST)
                        .then(|| "built tent")
                }
                AiKind::Builder => {
                    if farms < houses && food >= FARM_BUILD_COST {
                        try_build(&mut grid, &mut world_data, &mut farm_states, &mut food_storage, &mut spawner_state,
                            Building::Farm { town_idx: ti }, -1, tdi, row, col, center, FARM_BUILD_COST)
                            .then(|| "built farm")
                    } else if houses <= farms && food >= HOUSE_BUILD_COST {
                        try_build(&mut grid, &mut world_data, &mut farm_states, &mut food_storage, &mut spawner_state,
                            Building::House { town_idx: ti }, 0, tdi, row, col, center, HOUSE_BUILD_COST)
                            .then(|| "built house")
                    } else if (barracks == 0 || barracks < houses / 2) && food >= BARRACKS_BUILD_COST {
                        try_build(&mut grid, &mut world_data, &mut farm_states, &mut food_storage, &mut spawner_state,
                            Building::Barracks { town_idx: ti }, 1, tdi, row, col, center, BARRACKS_BUILD_COST)
                            .then(|| "built barracks")
                    } else if guard_posts < barracks && food >= GUARD_POST_BUILD_COST {
                        try_build(&mut grid, &mut world_data, &mut farm_states, &mut food_storage, &mut spawner_state,
                            Building::GuardPost { town_idx: ti, patrol_order: guard_posts as u32 }, -1, tdi, row, col, center, GUARD_POST_BUILD_COST)
                            .then(|| "built guard post")
                    } else { None }
                }
            };
            if let Some(what) = built {
                log_ai(&mut combat_log, &game_time, &town_name, what);
                continue;
            }
        }

        // Phase 2: Unlock slot if no empties
        if empty_slot.is_none() {
            if let Some(tg) = town_grids.grids.get(player.grid_idx) {
                let adjacent = world::get_adjacent_locked_slots(tg);
                if let Some(&(row, col)) = adjacent.first() {
                    let f = food_storage.food.get(tdi).copied().unwrap_or(0);
                    if f >= SLOT_UNLOCK_COST {
                        if let Some(f) = food_storage.food.get_mut(tdi) { *f -= SLOT_UNLOCK_COST; }
                        if let Some(tg) = town_grids.grids.get_mut(player.grid_idx) {
                            tg.unlocked.insert((row, col));
                        }
                        log_ai(&mut combat_log, &game_time, &town_name, "unlocked slot");
                        continue;
                    }
                }
            }
        }

        // Phase 3: Upgrades
        let to_try: &[UpgradeType] = match player.kind {
            AiKind::Raider => &[UpgradeType::AttackSpeed, UpgradeType::MoveSpeed],
            AiKind::Builder => &[UpgradeType::GuardHealth, UpgradeType::GuardAttack,
                UpgradeType::FarmYield, UpgradeType::AttackSpeed, UpgradeType::MoveSpeed],
        };
        for &ut in to_try {
            let idx = ut as usize;
            let level = upgrades.levels.get(tdi).map(|l| l[idx]).unwrap_or(0);
            if food >= upgrade_cost(level) {
                upgrade_queue.0.push((tdi, idx));
                log_ai(&mut combat_log, &game_time, &town_name, &format!("upgraded {:?}", ut));
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

fn log_ai(log: &mut CombatLog, gt: &GameTime, town: &str, what: &str) {
    log.push(CombatEventKind::Ai, gt.day(), gt.hour(), gt.minute(),
        format!("AI: {} {}", town, what));
}
