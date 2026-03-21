//! AI building action helpers -- slot selection wrappers, road building, action executor, and logging.
//! Extracted from decision.rs to keep that file under 1000 lines.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use crate::constants::*;
use crate::resources::*;
use crate::world::{self, BuildingKind, WorldGrid};

use super::mine_analysis::{MineAnalysis, TownContext};
use super::slot_selection::*;
use super::*;

pub(super) fn try_build_at_slot(
    kind: BuildingKind,
    cost: i32,
    label: &str,
    tdi: usize,
    res: &mut AiBuildRes,
    food: &mut i32,
    col: usize,
    row: usize,
) -> Option<String> {
    let pos = res.world.grid.grid_to_world(col, row);
    res.world
        .place_building(
            food,
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

pub(super) fn pick_slot_from_snapshot_or_inner(
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
    find_inner_slot(
        town_idx,
        center,
        area_level,
        grid,
        entity_map,
        personality,
        road_style,
    )
}

pub(super) fn try_build_inner(
    kind: BuildingKind,
    cost: i32,
    label: &str,
    tdi: usize,
    center: Vec2,
    res: &mut AiBuildRes,
    food: &mut i32,
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
    try_build_at_slot(kind, cost, label, tdi, res, food, col, row)
}

pub(super) fn try_build_scored(
    kind: BuildingKind,
    label: &str,
    tdi: usize,
    center: Vec2,
    res: &mut AiBuildRes,
    food: &mut i32,
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
    try_build_at_slot(kind, building_cost(kind), label, tdi, res, food, col, row)
}

pub(super) fn try_build_miner_home(
    ctx: &TownContext,
    mines: &MineAnalysis,
    res: &mut AiBuildRes,
    food: &mut i32,
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
        food,
        slot.0,
        slot.1,
    )
}

/// Count available road candidate slots (road-pattern slots near economy buildings, minus existing roads).
/// Used to gate road scoring so roads aren't scored when no candidates exist.
pub(super) fn count_road_candidates(
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
    let road_slots: HashSet<(usize, usize)> = [
        BuildingKind::Road,
        BuildingKind::StoneRoad,
        BuildingKind::MetalRoad,
    ]
    .iter()
    .flat_map(|&kind| entity_map.iter_kind_for_town(kind, ti))
    .map(|b| grid.world_to_grid(b.position))
    .collect();
    // Precompute expanded adjacency set: each econ slot expanded by radius 2.
    // Converts O(grid_cells * econ_buildings) adjacency check to O(1) per cell.
    let econ_adj: HashSet<(i32, i32)> = econ_slots
        .iter()
        .flat_map(|&(ec, er)| {
            (-2i32..=2)
                .flat_map(move |dc| (-2i32..=2).map(move |dr| (ec as i32 + dc, er as i32 + dr)))
        })
        .collect();
    let (min_c, max_c, min_r, max_r) = world::build_bounds(area_level, center, grid);
    // Cardinal: extend axes to 2x build radius for attack corridors
    let (ext_min_c, ext_max_c, ext_min_r, ext_max_r) = if road_style == RoadStyle::Cardinal {
        let half_c = cc - min_c;
        let half_r = cr - min_r;
        (
            cc.saturating_sub(half_c * 2),
            max_c + half_c,
            cr.saturating_sub(half_r * 2),
            max_r + half_r,
        )
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
            let adj = econ_adj.contains(&(col as i32, row as i32));
            if adj || !in_bounds {
                count += 1;
            }
        }
    }
    count
}

/// Build roads around economy buildings using the town's road style.
pub(super) fn try_build_road_grid(
    ctx: &TownContext,
    res: &mut AiBuildRes,
    food: &mut i32,
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

    let road_slots: HashSet<(usize, usize)> = [
        BuildingKind::Road,
        BuildingKind::StoneRoad,
        BuildingKind::MetalRoad,
    ]
    .iter()
    .flat_map(|&kind| entity_map.iter_kind_for_town(kind, ti))
    .map(|b| grid.world_to_grid(b.position))
    .collect();

    // Precompute adjacency counts: each econ slot votes for its radius-2 neighbors.
    // Converts O(grid_cells * econ_buildings) to O(econ_buildings * 25 + grid_cells).
    let mut econ_adj_counts: HashMap<(i32, i32), i32> = HashMap::new();
    for &(ec, er) in &econ_slots {
        for dc in -2i32..=2 {
            for dr in -2i32..=2 {
                *econ_adj_counts
                    .entry((ec as i32 + dc, er as i32 + dr))
                    .or_default() += 1;
            }
        }
    }
    let mut candidates: HashMap<(usize, usize), i32> = HashMap::new();
    let (min_c, max_c, min_r, max_r) =
        world::build_bounds(ctx.area_level, ctx.center, &res.world.grid);
    // Cardinal: extend axes to 2x build radius for attack corridors
    let (ext_min_c, ext_max_c, ext_min_r, ext_max_r) = if road_style == RoadStyle::Cardinal {
        let half_c = cc - min_c;
        let half_r = cr - min_r;
        (
            cc.saturating_sub(half_c * 2),
            max_c + half_c,
            cr.saturating_sub(half_r * 2),
            max_r + half_r,
        )
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
            let adj = econ_adj_counts
                .get(&(col as i32, row as i32))
                .copied()
                .unwrap_or(0);
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
            let da = (a.0.0 as i32 - cc as i32).pow(2) + (a.0.1 as i32 - cr as i32).pow(2);
            let db = (b.0.0 as i32 - cc as i32).pow(2) + (b.0.1 as i32 - cr as i32).pow(2);
            da.cmp(&db)
        })
    });

    let mut placed = 0usize;
    for &((col, row), _score) in ranked.iter().take(batch_size * 2) {
        if placed >= batch_size {
            break;
        }
        if *food < cost {
            break;
        }

        let pos = res.world.grid.grid_to_world(col, row);
        if res
            .world
            .place_building(
                food,
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
pub(super) fn execute_action(
    action: AiAction,
    ctx: &TownContext,
    res: &mut AiBuildRes,
    food: &mut i32,
    current_mining_radius: f32,
    new_mining_radius: &mut Option<f32>,
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
            food,
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
                food,
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
                food,
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
            food,
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
            food,
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
            try_build_miner_home(ctx, mines, res, food, snapshot, personality, road_style)
        }
        AiAction::ExpandMiningRadius => {
            let old = current_mining_radius;
            let new = (old + MINING_RADIUS_STEP).min(MAX_MINING_RADIUS);
            if new <= old {
                return None;
            }
            *new_mining_radius = Some(new);
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
                    food,
                    world::BuildingKind::Waypoint,
                    ctx.tdi,
                    pos,
                    cost,
                    &mut res.gpu_updates,
                    &mut res.commands,
                )
                .is_ok()
            {
                let orders = recalc_waypoint_patrol_order_clockwise(
                    &mut res.world.world_data,
                    &mut res.world.entity_map,
                    ctx.ti,
                );
                for (slot, order) in orders {
                    if let Some(&entity) = res.world.entity_map.entities.get(&slot) {
                        if let Ok(mut w) = res.waypoint_q.get_mut(entity) {
                            w.0 = order;
                        }
                    }
                }
                Some("built waypoint".into())
            } else {
                None
            }
        }
        AiAction::BuildRoads => {
            try_build_road_grid(ctx, res, food, personality.road_batch_size(), road_style)
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

pub(super) fn log_ai(
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::Vec2;

    fn make_grid(size: usize) -> world::WorldGrid {
        let mut grid = world::WorldGrid::default();
        grid.width = size;
        grid.height = size;
        grid.cell_size = crate::constants::TOWN_GRID_SPACING;
        grid.cells = vec![world::WorldCell::default(); size * size];
        grid.init_town_buildable();
        grid
    }

    /// count_road_candidates returns 0 when there are no econ buildings.
    /// Verifies early-return path is not broken by the adjacency precompute.
    #[test]
    fn count_road_candidates_zero_with_no_econ_buildings() {
        let em = EntityMap::default();
        let grid = make_grid(20);
        let center = Vec2::new(640.0, 640.0);
        let count = count_road_candidates(&em, 1, center, &grid, 0, RoadStyle::Grid4);
        assert_eq!(count, 0);
    }

    /// count_road_candidates finds road slots adjacent to econ buildings.
    /// This test would FAIL if the HashSet precompute returned wrong results
    /// compared to the original iter().any() brute-force check.
    #[test]
    fn count_road_candidates_finds_slots_near_farm() {
        let mut em = EntityMap::default();
        em.init_spatial(2000.0);
        let mut grid = make_grid(20);
        let center = Vec2::new(640.0, 640.0); // cell (10, 10)
        let ti = 0u32;

        // Mark build-area cells as buildable for town 0
        for dr in -4i32..=3i32 {
            for dc in -4i32..=3i32 {
                let col = 10i32 + dc;
                let row = 10i32 + dr;
                if col >= 0 && row >= 0 {
                    grid.add_town_buildable(col as usize, row as usize, 0u16);
                }
            }
        }

        // Add a farm at cell (8, 10): world pos = (8*64, 10*64) = (512, 640)
        let mut pool = crate::resources::GpuSlotPool::default();
        let slot = pool.alloc_reset().unwrap();
        em.add_instance(crate::entity_map::BuildingInstance {
            kind: world::BuildingKind::Farm,
            position: Vec2::new(512.0, 640.0),
            town_idx: ti,
            slot,
            faction: 1,
        });

        let count = count_road_candidates(&em, 1, center, &grid, ti, RoadStyle::Grid4);
        assert!(count > 0, "expected road candidates near farm; got {count}");
    }

    /// count_road_candidates result matches a brute-force reference impl.
    /// If the HashSet precompute changes which cells count, this catches it.
    #[test]
    fn count_road_candidates_matches_brute_force() {
        let mut em = EntityMap::default();
        em.init_spatial(2000.0);
        let mut grid = make_grid(20);
        let center = Vec2::new(640.0, 640.0);
        let ti = 0u32;

        for dr in -4i32..=3i32 {
            for dc in -4i32..=3i32 {
                let col = 10i32 + dc;
                let row = 10i32 + dr;
                if col >= 0 && row >= 0 {
                    grid.add_town_buildable(col as usize, row as usize, 0u16);
                }
            }
        }

        let mut pool = crate::resources::GpuSlotPool::default();
        for k in 0..5usize {
            let slot = pool.alloc_reset().unwrap();
            let x = (8 + k) as f32 * crate::constants::TOWN_GRID_SPACING;
            let y = 10.0 * crate::constants::TOWN_GRID_SPACING;
            em.add_instance(crate::entity_map::BuildingInstance {
                kind: world::BuildingKind::Farm,
                position: Vec2::new(x, y),
                town_idx: ti,
                slot,
                faction: 1,
            });
        }

        let count_new = count_road_candidates(&em, 1, center, &grid, ti, RoadStyle::Grid4);
        // Brute-force reference: same logic without the precompute optimization
        let count_ref =
            count_road_candidates_brute_force(&em, 1, center, &grid, ti, RoadStyle::Grid4);
        assert_eq!(
            count_new, count_ref,
            "optimized count {count_new} != brute-force {count_ref}"
        );
    }

    /// Reference implementation of count_road_candidates using the original
    /// O(cells * econ_buildings) adjacency check. Used to validate the O(1) precompute.
    fn count_road_candidates_brute_force(
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
        let road_slots: HashSet<(usize, usize)> = [
            BuildingKind::Road,
            BuildingKind::StoneRoad,
            BuildingKind::MetalRoad,
        ]
        .iter()
        .flat_map(|&kind| entity_map.iter_kind_for_town(kind, ti))
        .map(|b| grid.world_to_grid(b.position))
        .collect();
        let (min_c, max_c, min_r, max_r) = world::build_bounds(area_level, center, grid);
        let (ext_min_c, ext_max_c, ext_min_r, ext_max_r) = if road_style == RoadStyle::Cardinal {
            let half_c = cc - min_c;
            let half_r = cr - min_r;
            (
                cc.saturating_sub(half_c * 2),
                max_c + half_c,
                cr.saturating_sub(half_r * 2),
                max_r + half_r,
            )
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
                let adj = econ_slots.iter().any(|&(ec, er)| {
                    (ec as i32 - col as i32).abs() <= 2 && (er as i32 - row as i32).abs() <= 2
                });
                if adj || !in_bounds {
                    count += 1;
                }
            }
        }
        count
    }
}
