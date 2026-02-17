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
use crate::world::{self, Building, WorldData, WorldGrid};
use crate::systems::stats::{UpgradeQueue, TownUpgrades, upgrade_node, upgrade_available, UPGRADE_COUNT};

/// Mutable world resources needed for AI building. Bundled to stay under Bevy's 16-param limit.
#[derive(SystemParam)]
pub struct AiBuildRes<'w> {
    world: WorldState<'w>,
    food_storage: ResMut<'w, FoodStorage>,
    upgrade_queue: ResMut<'w, UpgradeQueue>,
    policies: ResMut<'w, TownPolicies>,
}

/// Minimum Manhattan distance between waypoints on the town grid.
const MIN_WAYPOINT_SPACING: i32 = 5;
/// Patrol posts sit one slot outside controlled buildings.
const TERRITORY_PERIMETER_PADDING: i32 = 1;

/// Minimum Manhattan grid-distance from `candidate` to any existing waypoint for this town.
/// Returns `i32::MAX` if no waypoints exist.
fn min_waypoint_spacing(
    grid: &WorldGrid,
    world_data: &WorldData,
    town_idx: u32,
    candidate: Vec2,
) -> i32 {
    let (cc, cr) = grid.world_to_grid(candidate);
    world_data.waypoints.iter()
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
    min_waypoint_spacing(grid, world_data, town_idx, candidate) >= MIN_WAYPOINT_SPACING
}

fn recalc_waypoint_patrol_order_clockwise(
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    town_idx: u32,
) {
    let Some(center) = world_data.towns.get(town_idx as usize).map(|t| t.center) else { return; };

    let mut ids: Vec<usize> = world_data.waypoints.iter().enumerate()
        .filter(|(_, w)| w.town_idx == town_idx && world::is_alive(w.position))
        .map(|(i, _)| i)
        .collect();

    // Clockwise around town center, starting at north (+Y).
    ids.sort_by(|&a, &b| {
        let pa = world_data.waypoints[a].position - center;
        let pb = world_data.waypoints[b].position - center;
        let mut aa = pa.x.atan2(pa.y);
        let mut ab = pb.x.atan2(pb.y);
        if aa < 0.0 { aa += std::f32::consts::TAU; }
        if ab < 0.0 { ab += std::f32::consts::TAU; }
        aa.partial_cmp(&ab).unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| pa.length_squared().partial_cmp(&pb.length_squared()).unwrap_or(std::cmp::Ordering::Equal))
    });

    for (order, &idx) in ids.iter().enumerate() {
        let pos = world_data.waypoints[idx].position;
        world_data.waypoints[idx].patrol_order = order as u32;
        let (gc, gr) = grid.world_to_grid(pos);
        if let Some(cell) = grid.cell_mut(gc, gr) {
            if let Some(Building::Waypoint { town_idx: ti, patrol_order }) = cell.building.as_mut() {
                if *ti == town_idx {
                    *patrol_order = order as u32;
                }
            }
        }
    }
}

#[derive(Clone, Default)]
struct AiTownSnapshot {
    center: Vec2,
    empty_slots: Vec<(i32, i32)>,
    farms: HashSet<(i32, i32)>,
    farmer_homes: HashSet<(i32, i32)>,
    archer_homes: HashSet<(i32, i32)>,
    miner_homes: HashSet<(i32, i32)>,
    gold_mines: Vec<Vec2>,
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
    ExpandMiningRadius,
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
        match self {
            Self::Aggressive => PolicySet {
                archer_aggressive: true,
                archer_leash: false,
                farmer_fight_back: true,
                prioritize_healing: false,
                archer_flee_hp: 0.0,
                farmer_flee_hp: 0.30,
                mining_radius: 300.0,
                ..PolicySet::default()
            },
            Self::Balanced => PolicySet {
                mining_radius: 300.0,
                ..PolicySet::default()
            },
            Self::Economic => PolicySet {
                archer_leash: true,
                prioritize_healing: true,
                archer_flee_hp: 0.25,
                farmer_flee_hp: 0.50,
                mining_radius: 300.0,
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

    /// Farmer home target count relative to farms.
    fn farmer_home_target(self, farms: usize) -> usize {
        match self {
            Self::Aggressive => farms.max(1),
            Self::Balanced => (farms + 1).max(1),
            Self::Economic => (farms * 2).max(1),
        }
    }

    /// Desired miners per discovered gold mine in policy radius.
    fn miners_per_mine_target(self) -> usize {
        match self {
            Self::Aggressive => 1,
            Self::Balanced => 2,
            Self::Economic => 4,
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
    world::empty_slots(tg, center, grid).into_iter()
        .min_by_key(|&(r, c)| r * r + c * c)
}

fn build_town_snapshot(
    world_data: &WorldData,
    grid: &WorldGrid,
    tg: &world::TownGrid,
    town_data_idx: usize,
) -> Option<AiTownSnapshot> {
    let town = world_data.towns.get(town_data_idx)?;
    let center = town.center;
    let ti = town_data_idx as u32;

    let farms = world_data.farms.iter()
        .filter(|f| f.town_idx == ti && world::is_alive(f.position))
        .map(|f| world::world_to_town_grid(center, f.position))
        .collect();
    let farmer_homes = world_data.farmer_homes.iter()
        .filter(|h| h.town_idx == ti && world::is_alive(h.position))
        .map(|h| world::world_to_town_grid(center, h.position))
        .collect();
    let archer_homes = world_data.archer_homes.iter()
        .filter(|h| h.town_idx == ti && world::is_alive(h.position))
        .map(|h| world::world_to_town_grid(center, h.position))
        .collect();
    let miner_homes = world_data.miner_homes.iter()
        .filter(|h| h.town_idx == ti && world::is_alive(h.position))
        .map(|h| world::world_to_town_grid(center, h.position))
        .collect();
    let empty_slots = world::empty_slots(tg, center, grid);

    let gold_mines = world_data.gold_mines.iter()
        .filter(|m| world::is_alive(m.position))
        .map(|m| m.position)
        .collect();

    Some(AiTownSnapshot {
        center,
        empty_slots,
        farms,
        farmer_homes,
        archer_homes,
        miner_homes,
        gold_mines,
    })
}

fn pick_best_empty_slot<F>(snapshot: &AiTownSnapshot, mut score: F) -> Option<(i32, i32)>
where
    F: FnMut((i32, i32)) -> i32,
{
    let mut best: Option<((i32, i32), i32)> = None;
    for &slot in &snapshot.empty_slots {
        let s = score(slot);
        if best.map_or(true, |(_, bs)| s > bs) {
            best = Some((slot, s));
        }
    }
    best.map(|(slot, _)| slot)
}

fn farm_slot_score(snapshot: &AiTownSnapshot, slot: (i32, i32)) -> i32 {
    let (r, c) = slot;
    let mut score = 0i32;
    let mut orth_farms = 0i32;

    for dr in -1..=1 {
        for dc in -1..=1 {
            if dr == 0 && dc == 0 { continue; }
            let n = (r + dr, c + dc);
            if snapshot.farms.contains(&n) {
                score += if dr == 0 || dc == 0 { 24 } else { 12 };
                if dr == 0 || dc == 0 { orth_farms += 1; }
            }
            if snapshot.farmer_homes.contains(&n) {
                score += 8;
            }
        }
    }

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

    if orth_farms >= 2 { score += 30; }
    if snapshot.farms.is_empty() {
        let radial = r * r + c * c;
        score -= radial / 2;
    }
    score
}

fn balanced_farm_ray_score(snapshot: &AiTownSnapshot, slot: (i32, i32)) -> i32 {
    let (r, c) = slot;
    let radial = r * r + c * c;
    let on_axis = r == 0 || c == 0;
    let mut score = if on_axis { 500 - radial * 4 } else { -300 - radial };

    if on_axis {
        if r == 0 && c != 0 {
            let step = if c > 0 { 1 } else { -1 };
            if snapshot.farms.contains(&(0, c - step)) { score += 220; }
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
    let (r, c) = slot;
    let mut edge_farm = 0i32;
    let mut diag_farm = 0i32;
    let mut near_homes = 0i32;
    let mut near_archers = 0i32;

    for dr in -1..=1 {
        for dc in -1..=1 {
            if dr == 0 && dc == 0 { continue; }
            let n = (r + dr, c + dc);
            if snapshot.farms.contains(&n) {
                if dr == 0 || dc == 0 { edge_farm += 1; } else { diag_farm += 1; }
            }
            if snapshot.farmer_homes.contains(&n) { near_homes += 1; }
            if snapshot.archer_homes.contains(&n) { near_archers += 1; }
        }
    }

    if edge_farm == 0 && diag_farm == 0 {
        return i32::MIN / 4;
    }
    edge_farm * 90 + diag_farm * 35 + near_homes * 10 + near_archers * 5
}

fn balanced_house_side_score(snapshot: &AiTownSnapshot, slot: (i32, i32)) -> i32 {
    let (r, c) = slot;
    let mut score = 0i32;
    let on_axis = r == 0 || c == 0;
    if on_axis {
        score -= 120;
    }

    for &(fr, fc) in &snapshot.farms {
        if fc == 0 && fr != 0 {
            if slot == (fr, 1) || slot == (fr, -1) {
                score += 260;
            }
        } else if fr == 0 && fc != 0 {
            if slot == (1, fc) || slot == (-1, fc) {
                score += 260;
            }
        }

        let manhattan = (r - fr).abs() + (c - fc).abs();
        if manhattan == 1 {
            score += 20;
        }
    }

    for &(hr, hc) in &snapshot.farmer_homes {
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
    let (r, c) = slot;
    let mut near_farms = 0i32;
    let mut near_homes = 0i32;
    let mut near_archers = 0i32;

    for dr in -1..=1 {
        for dc in -1..=1 {
            if dr == 0 && dc == 0 { continue; }
            let n = (r + dr, c + dc);
            if snapshot.farms.contains(&n) { near_farms += 1; }
            if snapshot.farmer_homes.contains(&n) { near_homes += 1; }
            if snapshot.archer_homes.contains(&n) { near_archers += 1; }
        }
    }

    let mut score = near_farms * 40 + near_homes * 35 - near_archers * 20;
    if near_farms + near_homes >= 4 { score += 60; }
    score
}

fn miner_toward_mine_score(snapshot: &AiTownSnapshot, slot: (i32, i32)) -> i32 {
    if snapshot.gold_mines.is_empty() {
        let (r, c) = slot;
        return -(r * r + c * c);
    }
    let wp = world::town_grid_to_world(snapshot.center, slot.0, slot.1);
    let best = snapshot.gold_mines.iter()
        .map(|m| (wp - *m).length_squared())
        .fold(f32::INFINITY, f32::min);
    let radial = slot.0 * slot.0 + slot.1 * slot.1;
    -(best as i32) - radial
}

/// Find outermost empty slot at least MIN_WAYPOINT_SPACING from all existing waypoints.
fn find_waypoint_slot(
    tg: &world::TownGrid, center: Vec2, grid: &WorldGrid, world_data: &WorldData, ti: u32,
) -> Option<(i32, i32)> {
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
    if let Some(snap) = snapshot {
        let mut slots = snap.farms.clone();
        slots.extend(&snap.farmer_homes);
        slots.extend(&snap.archer_homes);
        slots.extend(&snap.miner_homes);
        return slots;
    }
    let mut slots = HashSet::new();
    let alive_town = |pos: Vec2, town: u32| town == ti && world::is_alive(pos);
    for f in &world_data.farms { if alive_town(f.position, f.town_idx) { slots.insert(world::world_to_town_grid(center, f.position)); } }
    for h in &world_data.farmer_homes { if alive_town(h.position, h.town_idx) { slots.insert(world::world_to_town_grid(center, h.position)); } }
    for h in &world_data.archer_homes { if alive_town(h.position, h.town_idx) { slots.insert(world::world_to_town_grid(center, h.position)); } }
    for h in &world_data.miner_homes { if alive_town(h.position, h.town_idx) { slots.insert(world::world_to_town_grid(center, h.position)); } }
    slots
}

/// Candidate perimeter slots around controlled buildings, clamped to buildable town grid.
fn territory_perimeter_slots(
    occupied: &HashSet<(i32, i32)>, tg: &world::TownGrid,
) -> HashSet<(i32, i32)> {
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
    let Some(town) = world_data.towns.get(town_data_idx) else { return 0; };
    let Some(tg) = town_grids.grids.iter().find(|g| g.town_data_idx == town_data_idx) else { return 0; };
    let center = town.center;
    let ti = town_data_idx as u32;

    let occupied = controlled_territory_slots(None, world_data, center, ti);
    if occupied.is_empty() { return 0; }
    let perimeter = territory_perimeter_slots(&occupied, tg);
    if perimeter.is_empty() { return 0; }

    let mut prune_slots: Vec<(i32, i32)> = Vec::new();
    for wp in &world_data.waypoints {
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
        recalc_waypoint_patrol_order_clockwise(grid, world_data, ti);
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

/// Pre-computed mine stats for one AI town. Built once, used by scoring + execution.
struct MineAnalysis {
    in_radius: usize,
    outside_radius: usize,
    uncovered: Vec<Vec2>,
    /// Closest uncovered mine to town center (for wilderness waypoint placement).
    nearest_uncovered: Option<Vec2>,
}

fn analyze_mines(world_data: &WorldData, center: Vec2, ti: u32, mining_radius: f32) -> MineAnalysis {
    let radius_sq = mining_radius * mining_radius;
    let friendly: Vec<Vec2> = world_data.waypoints.iter()
        .filter(|w| w.town_idx == ti && world::is_alive(w.position))
        .map(|w| w.position)
        .collect();

    let mut in_radius = 0usize;
    let mut outside_radius = 0usize;
    let mut uncovered = Vec::new();

    for m in &world_data.gold_mines {
        if !world::is_alive(m.position) { continue; }
        if (m.position - center).length_squared() <= radius_sq {
            in_radius += 1;
        } else {
            outside_radius += 1;
        }
        if !friendly.iter().any(|wp| (*wp - m.position).length() < WAYPOINT_COVER_RADIUS) {
            uncovered.push(m.position);
        }
    }

    let nearest_uncovered = uncovered.iter()
        .min_by(|a, b| a.distance(center).partial_cmp(&b.distance(center)).unwrap())
        .copied();

    MineAnalysis { in_radius, outside_radius, uncovered, nearest_uncovered }
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
    mut snapshots: Local<AiTownSnapshotCache>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("ai_decision");
    *timer += time.delta_secs();
    if *timer < config.decision_interval { return; }
    *timer = 0.0;

    let snapshot_dirty = res.world.dirty.building_grid || res.world.dirty.mining || res.world.dirty.patrol_perimeter;
    if snapshot_dirty {
        snapshots.towns.clear();
    }

    for pi in 0..ai_state.players.len() {
        let player = &ai_state.players[pi];
        if !player.active { continue; }
        let tdi = player.town_data_idx;
        if !snapshots.towns.contains_key(&tdi) {
            if let Some(tg) = res.world.town_grids.grids.get(player.grid_idx) {
                if let Some(snap) = build_town_snapshot(&res.world.world_data, &res.world.grid, tg, tdi) {
                    snapshots.towns.insert(tdi, snap);
                }
            }
        }

        let food = res.food_storage.food.get(tdi).copied().unwrap_or(0);
        let spawner_count = res.world.spawner_state.0.iter()
            .filter(|s| world::is_alive(s.position))
            .filter(|s| s.town_idx == tdi as i32)
            .filter(|s| matches!(s.building_kind, 0 | 1 | 2 | 3))
            .count() as i32;
        let reserve = player.personality.food_reserve_per_spawner() * spawner_count;
        if food <= reserve { continue; }

        let center = snapshots.towns.get(&tdi)
            .map(|s| s.center)
            .or_else(|| res.world.world_data.towns.get(tdi).map(|t| t.center))
            .unwrap_or_default();
        let town_name = res.world.world_data.towns.get(tdi).map(|t| t.name.clone()).unwrap_or_default();
        let pname = player.personality.name();
        let ti = tdi as u32;

        let counts = res.world.world_data.building_counts(ti);
        let farms = counts.farms;
        let houses = counts.farmer_homes;
        let barracks = counts.archer_homes;
        let waypoints = counts.waypoints;
        let mine_shafts = counts.miner_homes;

        let has_slots = snapshots.towns.get(&tdi)
            .map(|s| !s.empty_slots.is_empty())
            .or_else(|| {
                res.world.town_grids.grids.get(player.grid_idx)
                    .map(|tg| !world::empty_slots(tg, center, &res.world.grid).is_empty())
            })
            .unwrap_or(false);

        let slot_fullness = res.world.town_grids.grids.get(player.grid_idx)
            .map(|tg| {
                let (min_r, max_r, min_c, max_c) = world::build_bounds(tg);
                let total = ((max_r - min_r + 1) * (max_c - min_c + 1) - 1) as f32; // -1 for center
                let empty = snapshots.towns.get(&tdi)
                    .map(|s| s.empty_slots.len() as i32)
                    .unwrap_or_else(|| world::empty_slots(tg, center, &res.world.grid).len() as i32);
                1.0 - empty as f32 / total.max(1.0)
            })
            .unwrap_or(0.0);

        // Score all eligible actions
        let mut scores: Vec<(AiAction, f32)> = Vec::with_capacity(8);
        let mut miner_target_for_expansion = 0usize;
        let mut cached_mines: Option<MineAnalysis> = None;

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
                let ht = player.personality.farmer_home_target(farms);
                let mining_radius = res.policies.policies.get(tdi)
                    .map(|p| p.mining_radius)
                    .unwrap_or(300.0);
                let mines = analyze_mines(&res.world.world_data, center, ti, mining_radius);
                cached_mines = Some(mines);
                let mines = cached_mines.as_ref().unwrap();
                let miners_per_mine = player.personality.miners_per_mine_target();
                let ms_target = mines.in_radius * miners_per_mine;
                miner_target_for_expansion = ms_target;
                let house_deficit = ht.saturating_sub(houses);
                let barracks_deficit = bt.saturating_sub(barracks);
                let miner_deficit = ms_target.saturating_sub(mine_shafts);

                if has_slots {
                    // Need factors: 1.0 base + deficit (higher when behind target ratio)
                    let farm_need = 1.0 + (houses as f32 - farms as f32).max(0.0);
                    let house_need = if house_deficit > 0 { 1.0 + house_deficit as f32 } else { 0.5 };
                    let barracks_need = if barracks_deficit > 0 { 1.0 + barracks_deficit as f32 } else { 0.5 };

                    if food >= building_cost(BuildKind::Farm) { scores.push((AiAction::BuildFarm, fw * farm_need)); }
                    if food >= building_cost(BuildKind::FarmerHome) { scores.push((AiAction::BuildFarmerHome, hw * house_need)); }
                    if food >= building_cost(BuildKind::ArcherHome) { scores.push((AiAction::BuildArcherHome, bw * barracks_need)); }
                    if miner_deficit > 0 && food >= building_cost(BuildKind::MinerHome) {
                        let ms_need = 1.0 + miner_deficit as f32;
                        scores.push((AiAction::BuildMinerHome, hw * ms_need));
                    } else if miner_deficit == 0 && mines.outside_radius > 0 {
                        let expand_need = 1.0 + mines.outside_radius as f32;
                        scores.push((AiAction::ExpandMiningRadius, fw * 0.6 * expand_need));
                    }
                }

                // Waypoints: wilderness placement (independent of town grid slots)
                // Score when there are uncovered mines to extend patrol toward
                if food >= building_cost(BuildKind::Waypoint) {
                    let uncovered = mines.uncovered.len();
                    if uncovered > 0 {
                        let mine_need = 1.0 + uncovered as f32;
                        scores.push((AiAction::BuildWaypoint, gw * mine_need));
                    } else if waypoints < barracks {
                        // Fallback: still need waypoints for patrol coverage even without mines
                        let gp_need = 1.0 + (barracks - waypoints) as f32;
                        if has_slots {
                            scores.push((AiAction::BuildWaypoint, gw * gp_need));
                        }
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
                if matches!(player.kind, AiKind::Builder) {
                    let ht = player.personality.farmer_home_target(farms);
                    let bt = player.personality.archer_home_target(houses);
                    let wants_more_homes = has_slots && (
                        (houses < ht && food >= building_cost(BuildKind::FarmerHome))
                            || (barracks < bt && food >= building_cost(BuildKind::ArcherHome))
                            || (mine_shafts < miner_target_for_expansion && food >= building_cost(BuildKind::MinerHome))
                    );
                    if wants_more_homes {
                        continue;
                    }
                }
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
            player.grid_idx, snapshots.towns.get(&tdi), player.personality, *difficulty,
            cached_mines.as_ref(),
        );
        if label.is_some() {
            snapshots.towns.remove(&tdi);
        }
        if let Some(what) = label {
            let faction = res.world.world_data.towns.get(tdi).map(|t| t.faction).unwrap_or(0);
            log_ai(&mut combat_log, &game_time, faction, &town_name, pname, &what);
            let actions = &mut ai_state.players[pi].last_actions;
            if actions.len() >= MAX_ACTION_HISTORY { actions.pop_front(); }
            actions.push_back(what);
        }
    }
}

fn try_build_at_slot(
    building: Building,
    cost: i32,
    label: &str,
    tdi: usize,
    center: Vec2,
    res: &mut AiBuildRes,
    row: i32,
    col: i32,
) -> Option<String> {
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
        building,
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
) -> Option<(i32, i32)> {
    if let Some(snap) = snapshot {
        if let Some(slot) = pick_best_empty_slot(snap, |s| score(snap, s)) {
            return Some(slot);
        }
    }
    find_inner_slot(tg, center, grid)
}

fn try_build_inner(
    building: Building, cost: i32, label: &str,
    tdi: usize, center: Vec2, res: &mut AiBuildRes, grid_idx: usize,
) -> Option<String> {
    let tg = res.world.town_grids.grids.get(grid_idx)?;
    let (row, col) = find_inner_slot(tg, center, &res.world.grid)?;
    try_build_at_slot(building, cost, label, tdi, center, res, row, col)
}

fn try_build_scored(
    building: Building, kind: BuildKind, label: &str,
    tdi: usize, center: Vec2, res: &mut AiBuildRes, grid_idx: usize,
    snapshot: Option<&AiTownSnapshot>,
    score_fn: fn(&AiTownSnapshot, (i32, i32)) -> i32,
) -> Option<String> {
    let tg = res.world.town_grids.grids.get(grid_idx)?;
    let (row, col) = pick_slot_from_snapshot_or_inner(snapshot, tg, center, &res.world.grid, score_fn)?;
    try_build_at_slot(building, building_cost(kind), label, tdi, center, res, row, col)
}

/// Execute the chosen action, returning a log label on success.
fn execute_action(
    action: AiAction, ti: u32, tdi: usize, center: Vec2, waypoints: usize,
    res: &mut AiBuildRes, grid_idx: usize, snapshot: Option<&AiTownSnapshot>, personality: AiPersonality, _difficulty: Difficulty,
    cached_mines: Option<&MineAnalysis>,
) -> Option<String> {
    match action {
        AiAction::BuildTent => try_build_inner(
            Building::Tent { town_idx: ti }, building_cost(BuildKind::Tent), "tent",
            tdi, center, res, grid_idx),
        AiAction::BuildFarm => {
            let score = if personality == AiPersonality::Balanced { balanced_farm_ray_score } else { farm_slot_score };
            try_build_scored(Building::Farm { town_idx: ti }, BuildKind::Farm, "farm",
                tdi, center, res, grid_idx, snapshot, score)
        }
        AiAction::BuildFarmerHome => {
            let score = if personality == AiPersonality::Balanced { balanced_house_side_score } else { farmer_home_border_score };
            try_build_scored(Building::FarmerHome { town_idx: ti }, BuildKind::FarmerHome, "farmer home",
                tdi, center, res, grid_idx, snapshot, score)
        }
        AiAction::BuildArcherHome => try_build_scored(
            Building::ArcherHome { town_idx: ti }, BuildKind::ArcherHome, "archer home",
            tdi, center, res, grid_idx, snapshot, archer_fill_score),
        AiAction::BuildMinerHome => try_build_scored(
            Building::MinerHome { town_idx: ti }, BuildKind::MinerHome, "miner home",
            tdi, center, res, grid_idx, snapshot, miner_toward_mine_score),
        AiAction::ExpandMiningRadius => {
            let Some(policy) = res.policies.policies.get_mut(tdi) else { return None; };
            let old = policy.mining_radius;
            let new = (old + 300.0).min(5000.0);
            if new <= old {
                return None;
            }
            policy.mining_radius = new;
            res.world.dirty.mining = true;
            Some(format!("expanded mining radius to {:.0}px", new))
        }
        AiAction::BuildWaypoint => {
            let cost = building_cost(BuildKind::Waypoint);
            // Use precomputed mine analysis from scoring phase when available
            let mining_radius = res.policies.policies.get(tdi).map(|p| p.mining_radius).unwrap_or(300.0);
            let fallback;
            let mines_ref = match cached_mines {
                Some(m) => m,
                None => { fallback = analyze_mines(&res.world.world_data, center, ti, mining_radius); &fallback }
            };
            if let Some(mine_pos) = mines_ref.nearest_uncovered {
                if waypoint_spacing_ok(&res.world.grid, &res.world.world_data, ti, mine_pos)
                    && world::place_waypoint_at_world_pos(
                    &mut res.world.grid, &mut res.world.world_data,
                    &mut res.world.building_hp, &mut res.food_storage,
                    &mut res.world.slot_alloc, &mut res.world.building_slots,
                    tdi, mine_pos, cost,
                ).is_ok() {
                    recalc_waypoint_patrol_order_clockwise(&mut res.world.grid, &mut res.world.world_data, ti);
                    res.world.dirty.mark_building_changed(world::BuildingKind::Waypoint);
                    return Some("built waypoint near mine".into());
                }
            }
            // Fallback: in-grid placement
            let tg = res.world.town_grids.grids.get(grid_idx)?;
            let (row, col) = find_waypoint_slot(tg, center, &res.world.grid, &res.world.world_data, ti)?;
            let ok = world::build_and_pay(&mut res.world.grid, &mut res.world.world_data, &mut res.world.farm_states,
                &mut res.food_storage, &mut res.world.spawner_state, &mut res.world.building_hp,
                &mut res.world.slot_alloc, &mut res.world.building_slots, &mut res.world.dirty,
                Building::Waypoint { town_idx: ti, patrol_order: waypoints as u32 },
                tdi, row, col, center, cost);
            if ok {
                recalc_waypoint_patrol_order_clockwise(&mut res.world.grid, &mut res.world.world_data, ti);
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


fn log_ai(log: &mut CombatLog, gt: &GameTime, faction: i32, town: &str, personality: &str, what: &str) {
    log.push(CombatEventKind::Ai, faction, gt.day(), gt.hour(), gt.minute(),
        format!("{} [{}] {}", town, personality, what));
}
