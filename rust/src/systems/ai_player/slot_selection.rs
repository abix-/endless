//! Slot selection and waypoint perimeter management for AI towns.

use std::collections::HashSet;

use bevy::prelude::*;

use crate::components::{Building, WaypointOrder};
use crate::resources::*;
use crate::systemparams::WorldState;
use crate::world::{self, BuildingKind, WorldGrid};

use super::{
    AiPersonality, AiPlayerState, AiTownSnapshot, PerimeterSyncDirty, RoadStyle,
    recalc_waypoint_patrol_order_clockwise,
};

// ============================================================================
// SLOT SELECTION
// ============================================================================

/// Find best empty slot closest to town center (for economy buildings).
/// Excludes road and waypoint pattern slots for the given personality.
pub(super) fn find_inner_slot(
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

pub(super) fn build_town_snapshot(
    world_data: &crate::world::WorldData,
    entity_map: &EntityMap,
    grid: &WorldGrid,
    town_data_idx: usize,
    town_area_level: i32,
    personality: AiPersonality,
    road_style: RoadStyle,
) -> Option<AiTownSnapshot> {
    let town = world_data.towns.get(town_data_idx)?;
    let center = town.center;
    let area_level = town_area_level;
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

pub(super) fn pick_best_empty_slot<F>(
    snapshot: &AiTownSnapshot,
    mut score: F,
) -> Option<(usize, usize)>
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

pub(super) fn farm_slot_score(snapshot: &AiTownSnapshot, slot: (usize, usize)) -> i32 {
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

pub(super) fn balanced_farm_ray_score(snapshot: &AiTownSnapshot, slot: (usize, usize)) -> i32 {
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

pub(super) fn farmer_home_border_score(snapshot: &AiTownSnapshot, slot: (usize, usize)) -> i32 {
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

pub(super) fn balanced_house_side_score(snapshot: &AiTownSnapshot, slot: (usize, usize)) -> i32 {
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

pub(super) fn archer_fill_score(snapshot: &AiTownSnapshot, slot: (usize, usize)) -> i32 {
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

pub(super) fn miner_toward_mine_score(
    mine_positions: &[Vec2],
    grid: &WorldGrid,
    slot: (usize, usize),
    cc: usize,
    cr: usize,
) -> i32 {
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
pub(super) fn find_waypoint_slot(
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
    town_area_level: i32,
    personality: AiPersonality,
    road_style: RoadStyle,
    waypoint_q: &mut Query<&mut WaypointOrder, With<Building>>,
) -> usize {
    // Keep exactly one perimeter ring, but only prune inner/old waypoints
    // after the new outer ring is fully established.
    let Some(town) = world.world_data.towns.get(town_data_idx) else {
        return 0;
    };
    let center = town.center;
    let area_level = town_area_level;
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
        let Some(&target_entity) = world.entity_map.entities.get(&building_gpu_slot) else {
            continue;
        };

        damage_writer.write(crate::messages::DamageMsg {
            target: target_entity,
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
        let orders = recalc_waypoint_patrol_order_clockwise(
            &mut world.world_data,
            &mut world.entity_map,
            ti,
        );
        for (slot, order) in orders {
            if let Some(&entity) = world.entity_map.entities.get(&slot) {
                if let Ok(mut w) = waypoint_q.get_mut(entity) {
                    w.0 = order;
                }
            }
        }
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
    mut waypoint_q: Query<&mut WaypointOrder, With<Building>>,
    town_access: crate::systemparams::TownAccess,
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
            town_access.area_level(town_idx as i32),
            personality,
            road_style,
            &mut waypoint_q,
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
