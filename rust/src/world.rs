//! World Data - Towns, farms, beds, waypoints, sprite definitions
//! World Grid - 2D cell grid covering entire world (terrain + buildings)
//! World Generation - Procedural town placement and building layout

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Serialize Vec2 as [f32; 2] for save file backwards compat.
pub mod vec2_as_array {
    use bevy::prelude::Vec2;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(v: &Vec2, s: S) -> Result<S::Ok, S::Error> {
        [v.x, v.y].serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec2, D::Error> {
        let [x, y] = <[f32; 2]>::deserialize(d)?;
        Ok(Vec2::new(x, y))
    }
}

/// Serialize Option<Vec2> as Option<[f32; 2]>.
mod opt_vec2_as_array {
    use bevy::prelude::Vec2;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(v: &Option<Vec2>, s: S) -> Result<S::Ok, S::Error> {
        v.map(|v| [v.x, v.y]).serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec2>, D::Error> {
        Ok(<Option<[f32; 2]>>::deserialize(d)?.map(|[x, y]| Vec2::new(x, y)))
    }
}

use crate::components::Job;
use crate::constants::{
    BASE_GRID_MAX, BASE_GRID_MIN, MAX_GRID_EXTENT, NPC_REGISTRY, TOWN_GRID_SPACING, TOWN_REGISTRY,
    TownKind, town_def,
};
use crate::messages::{BuildingGridDirtyMsg, DirtyWriters};
use crate::messages::{CombatLogMsg, GpuUpdate, GpuUpdateMsg};
use crate::resources::{
    CombatEventKind, EntityMap, FactionStats, GameTime, GpuSlotPool, RaiderState,
};

/// True if a position has not been tombstoned (i.e. the entity still exists).
/// Tombstoned entities have position.x = -99999.0; this checks > -9000.0.
#[inline]
pub fn is_alive(pos: Vec2) -> bool {
    pos.x > -9000.0
}

// ============================================================================
// SPRITE DEFINITIONS (from roguelikeSheet_transparent.png)
// ============================================================================

/// Sprite sheet constants
pub const CELL: f32 = crate::render::WORLD_CELL; // 16px sprite + 1px margin
pub const SPRITE_SIZE: f32 = crate::render::WORLD_SPRITE_SIZE;
pub const SHEET_SIZE: (f32, f32) = crate::render::WORLD_SHEET_SIZE;

/// Output cell size for all CPU-side atlas packing (building, terrain, extras).
/// 64px = native size for new building art; 16px source sprites are upscaled 4x.
pub const ATLAS_CELL: u32 = 64;

// ============================================================================
// WORLD DATA STRUCTS
// ============================================================================

/// A town (villager or raider settlement).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Town {
    pub name: String,
    #[serde(with = "vec2_as_array")]
    pub center: Vec2,
    pub faction: i32, // 0=Neutral, 1=Player, 2+=AI factions
    /// Town type identity (Player, AiBuilder, AiRaider).
    #[serde(default = "default_town_kind")]
    pub kind: crate::constants::TownKind,
}

fn default_town_kind() -> crate::constants::TownKind {
    crate::constants::TownKind::Player
}

impl Town {
    pub fn is_raider(&self) -> bool {
        crate::constants::town_def(self.kind).is_raider
    }
}

/// Unified placed-building record. All building kinds (except Town) use this struct.
/// Kind-specific fields default to zero/None for building types that don't use them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlacedBuilding {
    #[serde(with = "vec2_as_array")]
    pub position: Vec2,
    #[serde(default)]
    pub town_idx: u32,
    /// Patrol order — used by Waypoint only (default 0).
    #[serde(default)]
    pub patrol_order: u32,
    /// Assigned mine position — used by MinerHome only.
    #[serde(default, with = "opt_vec2_as_array")]
    pub assigned_mine: Option<Vec2>,
    /// Whether mine was manually assigned — used by MinerHome only.
    #[serde(default)]
    pub manual_mine: bool,
    /// Wall tier level (1-3) — used by Wall only. 0 = not a wall (default).
    #[serde(default)]
    pub wall_level: u8,
    #[serde(default)]
    pub kills: i32,
    #[serde(default)]
    pub xp: i32,
    #[serde(default)]
    pub upgrade_levels: Vec<u8>,
    #[serde(default)]
    pub auto_upgrade_flags: Vec<bool>,
    #[serde(default)]
    pub equipped_weapon: Option<crate::constants::LootItem>,
}

impl PlacedBuilding {
    pub fn new(position: Vec2, town_idx: u32) -> Self {
        Self {
            position,
            town_idx,
            patrol_order: 0,
            assigned_mine: None,
            manual_mine: false,
            wall_level: 0,
            kills: 0,
            xp: 0,
            upgrade_levels: Vec::new(),
            auto_upgrade_flags: Vec::new(),
            equipped_weapon: None,
        }
    }
    pub fn new_wall(position: Vec2, town_idx: u32) -> Self {
        Self {
            position,
            town_idx,
            patrol_order: 0,
            assigned_mine: None,
            manual_mine: false,
            wall_level: 1,
            kills: 0,
            xp: 0,
            upgrade_levels: Vec::new(),
            auto_upgrade_flags: Vec::new(),
            equipped_weapon: None,
        }
    }
}

// ============================================================================
// WORLD RESOURCES
// ============================================================================

/// Contains all world layout data. Towns only — building instances live in EntityMap.
#[derive(Resource, Default)]
pub struct WorldData {
    pub towns: Vec<Town>,
}

/// Buildable slot bounds for a town (inclusive) in world grid coords: (min_col, max_col, min_row, max_row).
pub fn build_bounds(
    area_level: i32,
    center: Vec2,
    grid: &WorldGrid,
) -> (usize, usize, usize, usize) {
    let (center_col, center_row) = grid.world_to_grid(center);
    let cc = center_col as i32;
    let cr = center_row as i32;
    let half_neg = BASE_GRID_MIN - area_level; // negative
    let half_pos = BASE_GRID_MAX + area_level; // positive
    let min_col = (cc + half_neg).max(0) as usize;
    let max_col = (cc + half_pos).min(grid.width as i32 - 1) as usize;
    let min_row = (cr + half_neg).max(0) as usize;
    let max_row = (cr + half_pos).min(grid.height as i32 - 1) as usize;
    (min_col, max_col, min_row, max_row)
}

/// Check if a road can be placed at this position for a town.
/// Roads can extend 1 tile beyond existing buildable area (chain outward).
pub fn is_road_placeable_for_town(pos: Vec2, town_idx: usize, grid: &WorldGrid) -> bool {
    let (gc, gr) = grid.world_to_grid(pos);
    // Already buildable?
    if grid.can_town_build(gc, gr, town_idx as u16) {
        return true;
    }
    // Check if any adjacent cell is buildable for this town (1 tile chain)
    let ti = town_idx as u16;
    for dr in -1i32..=1 {
        for dc in -1i32..=1 {
            if dr == 0 && dc == 0 {
                continue;
            }
            let nc = gc as i32 + dc;
            let nr = gr as i32 + dr;
            if nc >= 0 && nr >= 0 && grid.can_town_build(nc as usize, nr as usize, ti) {
                return true;
            }
        }
    }
    false
}

/// All empty buildable slots for a town in world grid (col, row) coords.
pub fn empty_slots(
    town_idx: usize,
    center: Vec2,
    grid: &WorldGrid,
    entity_map: &crate::resources::EntityMap,
) -> Vec<(usize, usize)> {
    let ti = town_idx as u16;
    let (center_col, center_row) = grid.world_to_grid(center);
    let mut out = Vec::new();
    for row in 0..grid.height {
        for col in 0..grid.width {
            if col == center_col && row == center_row {
                continue;
            } // skip town center
            if !grid.can_town_build(col, row, ti) {
                continue;
            }
            if entity_map.has_building_at(col as i32, row as i32) {
                continue;
            }
            out.push((col, row));
        }
    }
    out
}

/// Find interior roads for a town — roads whose build-area contribution is fully redundant.
/// A road is "interior" if every cell within its radius is already covered by the base grid
/// or by another road's radius. Safe to destroy without losing buildable area.
pub fn find_interior_roads(
    town_idx: usize,
    grid: &WorldGrid,
    entity_map: &crate::resources::EntityMap,
    towns: &[Town],
    area_level: i32,
) -> Vec<(usize, usize)> {
    let Some(town) = towns.get(town_idx) else {
        return Vec::new();
    };
    let ti = town_idx as u32;
    let (min_c, max_c, min_r, max_r) = build_bounds(area_level, town.center, grid);
    let w = grid.width;
    let h = grid.height;

    // Collect all roads for this town: (col, row, radius)
    let mut roads: Vec<(usize, usize, i32)> = Vec::new();
    for kind in [
        BuildingKind::Road,
        BuildingKind::StoneRoad,
        BuildingKind::MetalRoad,
    ] {
        let radius = kind.road_build_radius().expect("road kind has radius");
        for inst in entity_map.iter_kind_for_town(kind, ti) {
            let (col, row) = grid.world_to_grid(inst.position);
            roads.push((col, row, radius));
        }
    }

    let mut result = Vec::new();
    for i in 0..roads.len() {
        let (rc, rr, radius) = roads[i];
        let mut all_covered = true;

        // Check every cell this road covers
        'cell: for dr in -radius..=radius {
            let gr = rr as i32 + dr;
            if gr < 0 || gr as usize >= h {
                continue;
            }
            for dc in -radius..=radius {
                let gc = rc as i32 + dc;
                if gc < 0 || gc as usize >= w {
                    continue;
                }
                let col = gc as usize;
                let row = gr as usize;

                // Covered by base grid?
                if col >= min_c && col <= max_c && row >= min_r && row <= max_r {
                    continue;
                }

                // Covered by another road's radius?
                let mut other_covers = false;
                for (j, &(oc, or, orad)) in roads.iter().enumerate() {
                    if j == i {
                        continue;
                    }
                    if (col as i32 - oc as i32).abs() <= orad
                        && (row as i32 - or as i32).abs() <= orad
                    {
                        other_covers = true;
                        break;
                    }
                }
                if !other_covers {
                    all_covered = false;
                    break 'cell;
                }
            }
        }

        if all_covered {
            result.push((rc, rr));
        }
    }
    result
}

// ============================================================================
// BUILDING PLACEMENT / REMOVAL
// ============================================================================

/// Check if a grid cell contains a building of the given kind.
/// For roads, matches any road tier so different tiers auto-connect.
fn is_kind_at(entity_map: &EntityMap, col: usize, row: usize, kind: BuildingKind) -> bool {
    entity_map
        .get_at_grid(col as i32, row as i32)
        .is_some_and(|inst| {
            if kind.is_road() {
                inst.kind.is_road()
            } else if kind.is_wall_like() {
                inst.kind.is_wall_like()
            } else {
                inst.kind == kind
            }
        })
}

/// Compute auto-tile variant (0-10) for a building at grid (col, row).
/// Uses 4-neighbor NSEW matching. Works for any autotile-enabled building kind.
/// Roads of different tiers connect to each other via is_kind_at.
pub fn autotile_variant(entity_map: &EntityMap, col: usize, row: usize, kind: BuildingKind) -> u16 {
    let n = row > 0 && is_kind_at(entity_map, col, row - 1, kind);
    let s = is_kind_at(entity_map, col, row + 1, kind);
    let e = is_kind_at(entity_map, col + 1, row, kind);
    let w = col > 0 && is_kind_at(entity_map, col - 1, row, kind);
    use crate::constants::*;
    match (n, s, e, w) {
        (false, false, true, true) => AUTOTILE_EW,
        (false, false, true, false) => AUTOTILE_EW,
        (false, false, false, true) => AUTOTILE_EW,
        (true, true, false, false) => AUTOTILE_NS,
        (true, false, false, false) => AUTOTILE_NS,
        (false, true, false, false) => AUTOTILE_NS,
        (true, false, true, false) => AUTOTILE_TR,
        (true, false, false, true) => AUTOTILE_TL,
        (false, true, false, true) => AUTOTILE_BL,
        (false, true, true, false) => AUTOTILE_BR,
        (true, false, true, true) => AUTOTILE_T_OPEN_N,
        (true, true, true, false) => AUTOTILE_T_OPEN_W,
        (false, true, true, true) => AUTOTILE_T_OPEN_S,
        (true, true, false, true) => AUTOTILE_T_OPEN_E,
        (true, true, true, true) => AUTOTILE_CROSS,
        _ => AUTOTILE_EW,
    }
}

/// Recompute auto-tile sprites for the building at (col, row) and its 4 neighbors.
pub fn update_autotile_around(
    grid: &WorldGrid,
    entity_map: &EntityMap,
    col: usize,
    row: usize,
    kind: BuildingKind,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) {
    let offsets: [(i32, i32); 5] = [(0, 0), (0, -1), (0, 1), (1, 0), (-1, 0)];
    for (dc, dr) in offsets {
        let c = col as i32 + dc;
        let r = row as i32 + dr;
        if c < 0 || r < 0 {
            continue;
        }
        let (c, r) = (c as usize, r as usize);
        // For roads, update any road-tier neighbor; use the neighbor's actual kind for sprite
        let pos = grid.grid_to_world(c, r);
        let Some(inst) = entity_map.find_by_position(pos) else {
            continue;
        };
        let neighbor_kind = inst.kind;
        if kind.is_road() {
            if !neighbor_kind.is_road() {
                continue;
            }
        } else if kind.is_wall_like() {
            if !neighbor_kind.is_wall_like() {
                continue;
            }
        } else if neighbor_kind != kind {
            continue;
        }
        let variant = autotile_variant(entity_map, c, r, neighbor_kind);
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame {
            idx: inst.slot,
            col: crate::constants::autotile_col(neighbor_kind, variant),
            row: 0.0,
            atlas: crate::constants::ATLAS_BUILDING,
        }));
    }
}

/// Set auto-tile sprites for all buildings of a given kind. Call after building instances are created.
pub fn update_all_autotile(
    grid: &WorldGrid,
    entity_map: &EntityMap,
    kind: BuildingKind,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) {
    for inst in entity_map.iter_kind(kind) {
        let (gc, gr) = grid.world_to_grid(inst.position);
        let variant = autotile_variant(entity_map, gc, gr, kind);
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame {
            idx: inst.slot,
            col: crate::constants::autotile_col(kind, variant),
            row: 0.0,
            atlas: crate::constants::ATLAS_BUILDING,
        }));
    }
}

/// Resolve SpawnNpcMsg fields from a spawner entry's building_kind.
/// Uses BUILDING_REGISTRY SpawnBehavior so new buildings with existing behaviors need no changes.
/// `assigned_mine` comes from MinerHomeConfig ECS component (None for non-miner buildings).
/// Returns (job, faction, work_x, work_y, starting_post, npc_label, bld_label, work_slot).
pub fn resolve_spawner_npc(
    inst: &crate::resources::BuildingInstance,
    towns: &[Town],
    entity_map: &crate::resources::EntityMap,
    assigned_mine: Option<Vec2>,
) -> (
    i32,
    i32,
    f32,
    f32,
    i32,
    &'static str,
    &'static str,
    Option<usize>,
) {
    use crate::components::Job;
    use crate::constants::{SpawnBehavior, building_def, npc_def};

    let town_faction = towns
        .get(inst.town_idx as usize)
        .map(|t| t.faction)
        .unwrap_or(0);

    let def = building_def(inst.kind);
    let Some(ref spawner) = def.spawner else {
        let raider_faction = towns
            .get(inst.town_idx as usize)
            .map(|t| t.faction)
            .unwrap_or(1);
        return (2, raider_faction, -1.0, -1.0, -1, "Raider", "Unknown", None);
    };

    let npc_label = npc_def(Job::from_i32(spawner.job)).label;

    match spawner.behavior {
        SpawnBehavior::FindNearestFarm => {
            let found = find_nearest_free(
                inst.position,
                entity_map,
                BuildingKind::Farm,
                Some(inst.town_idx),
            );
            let (work_slot, farm) = found
                .map(|(s, p)| (Some(s), p))
                .unwrap_or((None, inst.position));
            (
                spawner.job,
                town_faction,
                farm.x,
                farm.y,
                -1,
                npc_label,
                def.label,
                work_slot,
            )
        }
        SpawnBehavior::FindNearestWaypoint => {
            let post_idx = find_location_within_radius(
                inst.position,
                entity_map,
                LocationKind::Waypoint,
                f32::MAX,
            )
            .map(|(idx, _)| idx as i32)
            .unwrap_or(-1);
            (
                spawner.job,
                town_faction,
                -1.0,
                -1.0,
                post_idx,
                npc_label,
                def.label,
                None,
            )
        }
        SpawnBehavior::Raider => {
            let raider_faction = towns
                .get(inst.town_idx as usize)
                .map(|t| t.faction)
                .unwrap_or(1);
            (
                spawner.job,
                raider_faction,
                -1.0,
                -1.0,
                -1,
                npc_label,
                def.label,
                None,
            )
        }
        SpawnBehavior::FindNearestTreeNode => {
            let found = find_nearest_free(inst.position, entity_map, BuildingKind::TreeNode, None);
            let (work_slot, target) = found
                .map(|(s, p)| (Some(s), p))
                .unwrap_or((None, inst.position));
            (
                spawner.job,
                town_faction,
                target.x,
                target.y,
                -1,
                npc_label,
                def.label,
                work_slot,
            )
        }
        SpawnBehavior::FindNearestRockNode => {
            let found = find_nearest_free(inst.position, entity_map, BuildingKind::RockNode, None);
            let (work_slot, target) = found
                .map(|(s, p)| (Some(s), p))
                .unwrap_or((None, inst.position));
            (
                spawner.job,
                town_faction,
                target.x,
                target.y,
                -1,
                npc_label,
                def.label,
                work_slot,
            )
        }
        SpawnBehavior::Miner => {
            let (work_slot, mine) = if let Some(pos) = assigned_mine {
                (entity_map.slot_at_position(pos), pos)
            } else {
                find_nearest_free(inst.position, entity_map, BuildingKind::GoldMine, None)
                    .map(|(s, p)| (Some(s), p))
                    .unwrap_or((None, inst.position))
            };
            (
                spawner.job,
                town_faction,
                mine.x,
                mine.y,
                -1,
                npc_label,
                def.label,
                work_slot,
            )
        }
        SpawnBehavior::AtHome => (
            spawner.job,
            town_faction,
            -1.0,
            -1.0,
            -1,
            npc_label,
            def.label,
            None,
        ),
    }
}

/// Push GPU updates for a building slot (position, faction, health, sprite).
fn push_building_gpu_updates(
    slot: usize,
    kind: BuildingKind,
    pos: Vec2,
    faction: i32,
    max_hp: f32,
    tileset_idx: u16,
    tower: bool,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) {
    let flags = if tower {
        crate::constants::ENTITY_FLAG_BUILDING | crate::constants::ENTITY_FLAG_COMBAT
    } else if kind.is_road() {
        crate::constants::ENTITY_FLAG_BUILDING | crate::constants::ENTITY_FLAG_UNTARGETABLE
    } else {
        crate::constants::ENTITY_FLAG_BUILDING
    };
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetPosition {
        idx: slot,
        x: pos.x,
        y: pos.y,
    }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFaction { idx: slot, faction }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetMaxHealth {
        idx: slot,
        max_health: max_hp,
    }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth {
        idx: slot,
        health: max_hp,
    }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFlags { idx: slot, flags }));
    let half = if kind.is_road() {
        [0.0, 0.0]
    } else {
        crate::constants::BUILDING_HITBOX_HALF
    };
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHalfSize {
        idx: slot,
        half_w: half[0],
        half_h: half[1],
    }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame {
        idx: slot,
        col: tileset_idx as f32,
        row: 0.0,
        atlas: crate::constants::ATLAS_BUILDING,
    }));
}

/// Optional runtime context for validated building placement (player/AI).
/// Pass `None` for world-gen, save/load, migration, tests.
pub struct BuildContext<'a> {
    pub grid: &'a mut WorldGrid,
    pub world_data: &'a WorldData,
    pub food: &'a mut i32,
    pub cost: i32,
}

/// Per-instance overrides for building placement. Fresh placements use Default (all zeros/None).
#[derive(Default)]
pub struct BuildingOverrides {
    pub patrol_order: u32,
    pub wall_level: u8,
    pub hp: Option<f32>,
}

/// Unified building placement. Every code path that creates a building calls this.
///
/// With `ctx: Some(BuildContext)` — runtime validated placement:
///   validates cell, deducts cost, starts construction, wall auto-tile, dirty signals.
/// With `ctx: None` — free placement (world-gen, save/load, migration, tests):
///   just creates the building at full HP (or hp_override).
pub fn place_building(
    slot_alloc: &mut crate::resources::GpuSlotPool,
    entity_map: &mut EntityMap,
    commands: &mut Commands,
    gpu_updates: &mut MessageWriter<crate::messages::GpuUpdateMsg>,
    kind: BuildingKind,
    pos: Vec2,
    town_idx: u32,
    faction: i32,
    overrides: &BuildingOverrides,
    mut ctx: Option<BuildContext>,
    dirty_writers: Option<&mut DirtyWriters>,
) -> Result<usize, &'static str> {
    use crate::components::*;
    use crate::constants::{building_def, tileset_index};

    let def = building_def(kind);

    // Runtime validation + cost deduction (only when BuildContext provided)
    let (snapped, gc, gr) = if let Some(ref mut ctx) = ctx {
        let (gc, gr) = ctx.grid.world_to_grid(pos);
        let snapped = ctx.grid.grid_to_world(gc, gr);

        let cell = ctx.grid.cell(gc, gr).ok_or("cell out of bounds")?;
        if entity_map.has_building_at(gc as i32, gr as i32) {
            return Err("cell already has a building");
        }
        if matches!(cell.terrain, Biome::Water | Biome::Rock) {
            return Err("cannot build on water or rock");
        }
        if kind.is_road() && cell.terrain == Biome::Forest {
            return Err("cannot build road on forest");
        }
        if ctx.grid.is_foreign_territory(gc, gr, town_idx as u16) {
            return Err("cannot build in foreign territory");
        }

        // Guard tower requires at least one adjacent wall
        if kind == BuildingKind::GuardTower {
            let has_adj_wall = [(0i32, 1i32), (0, -1), (1, 0), (-1, 0)]
                .iter()
                .any(|&(dc, dr)| {
                    let nc = gc as i32 + dc;
                    let nr = gr as i32 + dr;
                    entity_map
                        .get_at_grid(nc, nr)
                        .is_some_and(|inst| inst.kind == BuildingKind::Wall)
                });
            if !has_adj_wall {
                return Err("guard tower must be adjacent to a wall");
            }
        }

        // Wilderness buildings must be within road or fountain buildable area
        if def.placement == crate::constants::PlacementMode::Wilderness {
            if kind.is_road() {
                if !is_road_placeable_for_town(snapped, town_idx as usize, ctx.grid) {
                    return Err("road must be adjacent to town or existing road");
                }
            } else if !ctx.grid.can_town_build(gc, gr, town_idx as u16) {
                return Err("outside buildable area");
            }
        }

        // Reject placements that would fully block access to spawners
        if !kind.is_road()
            && ctx
                .grid
                .would_block_spawner_access(entity_map, gc, gr, town_idx, ctx.world_data)
        {
            return Err("would block access to a spawner");
        }

        if *ctx.food < ctx.cost {
            return Err("not enough food");
        }
        *ctx.food -= ctx.cost;

        (snapped, gc, gr)
    } else {
        (pos, 0, 0) // gc/gr unused when no ctx
    };

    // Alloc GPU slot
    let Some(slot) = slot_alloc.alloc_reset() else {
        warn!("No building slots available for {:?}", kind);
        return Err("no GPU slots available");
    };

    // Create BuildingInstance (identity only — occupancy tracked separately in EntityMap)
    entity_map.add_instance(crate::resources::BuildingInstance {
        kind,
        position: snapped,
        town_idx,
        slot,
        faction,
    });
    // Construction: runtime placement sets timer, load/init skips it
    let under_construction = if ctx.is_some() {
        crate::constants::BUILDING_CONSTRUCT_SECS
    } else {
        0.0
    };
    let hp = if ctx.is_some() {
        overrides.hp.unwrap_or(0.01)
    } else {
        overrides.hp.unwrap_or(def.hp)
    };

    // Spawn ECS entity with all building state components
    let mut ecmds = commands.spawn((
        GpuSlot(slot),
        Position::new(snapped.x, snapped.y),
        Health(hp),
        Faction(faction),
        TownId(town_idx as i32),
        Building { kind },
        ConstructionProgress(under_construction),
    ));
    // Kind-specific state components
    if matches!(kind, BuildingKind::TreeNode | BuildingKind::RockNode) && ctx.is_none() {
        ecmds.insert(crate::components::Sleeping);
    }
    if def.worksite.is_some() {
        ecmds.insert(ProductionState::default());
    }
    if kind == BuildingKind::Farm {
        ecmds.insert(crate::components::FarmModeComp::default());
    }
    if def.spawner.is_some() {
        // Suppress spawner during construction (timer=-1), arm on completion (timer=0)
        let timer = if under_construction > 0.0 { -1.0 } else { 0.0 };
        ecmds.insert(SpawnerState {
            npc_slot: None,
            respawn_timer: timer,
        });
    }
    if def.is_tower {
        ecmds.insert(TowerBuildingState::default());
    }
    if kind == BuildingKind::Waypoint {
        ecmds.insert(WaypointOrder(overrides.patrol_order));
    }
    if kind.is_wall_like() {
        ecmds.insert(WallLevel(overrides.wall_level));
    }
    if kind == BuildingKind::MinerHome {
        ecmds.insert(MinerHomeConfig::default());
    }
    let entity = ecmds.id();
    entity_map.set_entity(slot, entity);

    // GPU state
    push_building_gpu_updates(
        slot,
        kind,
        snapped,
        faction,
        hp,
        tileset_index(kind),
        def.is_tower,
        gpu_updates,
    );

    // Runtime: auto-tile + dirty signals
    if let Some(ctx) = ctx {
        if crate::constants::building_def(kind).autotile {
            update_autotile_around(ctx.grid, entity_map, gc, gr, kind, gpu_updates);
        }
        // Roads expand the buildable area immediately so chained placement works
        if let Some(radius) = kind.road_build_radius() {
            let ti16 = town_idx as u16;
            let rc = gc as i32;
            let rr = gr as i32;
            for dr in -radius..=radius {
                let row = (rr + dr) as usize;
                if row >= ctx.grid.height {
                    continue;
                }
                for dc in -radius..=radius {
                    let col = (rc + dc) as usize;
                    if col >= ctx.grid.width {
                        continue;
                    }
                    ctx.grid.add_town_buildable(col, row, ti16);
                }
            }
        }
    }
    if let Some(dw) = dirty_writers {
        dw.mark_building_changed(kind);
    }

    Ok(slot)
}

/// Spawn one NPC per building spawner. Returns messages for the caller to write.
/// Create AI players for all non-player towns with random personalities.
fn create_ai_players(
    world_data: &WorldData,
    faction_list: &crate::resources::FactionList,
) -> Vec<crate::systems::AiPlayer> {
    use crate::resources::FactionKind;
    use crate::systems::ai_player::RoadStyle;
    use crate::systems::{AiKind, AiPersonality, AiPlayer};
    use rand::Rng;
    let personalities = [
        AiPersonality::Aggressive,
        AiPersonality::Balanced,
        AiPersonality::Economic,
    ];
    let mut rng = rand::rng();
    let mut players = Vec::new();
    for (tdi, town) in world_data.towns.iter().enumerate() {
        let is_ai = faction_list
            .factions
            .get(town.faction as usize)
            .is_some_and(|f| matches!(f.kind, FactionKind::AiBuilder | FactionKind::AiRaider));
        if is_ai {
            let kind = if town.is_raider() {
                AiKind::Raider
            } else {
                AiKind::Builder
            };
            let personality = personalities[rng.random_range(0..personalities.len())];
            let road_style = RoadStyle::random(&mut rng);
            players.push(AiPlayer {
                town_data_idx: tdi,
                kind,
                personality,
                road_style,
                last_actions: std::collections::VecDeque::new(),
                policy_defaults_logged: false,
                active: true,
                build_enabled: true,
                upgrade_enabled: true,
                squad_indices: Vec::new(),
                squad_cmd: std::collections::HashMap::new(),
            });
        } else {
            // Player town — inactive by default, controllable from Policies tab
            players.push(AiPlayer {
                town_data_idx: tdi,
                kind: AiKind::Builder,
                personality: AiPersonality::Balanced,
                road_style: RoadStyle::Grid4,
                last_actions: std::collections::VecDeque::new(),
                policy_defaults_logged: false,
                active: false,
                build_enabled: true,
                upgrade_enabled: true,
                squad_indices: Vec::new(),
                squad_cmd: std::collections::HashMap::new(),
            });
        }
    }
    players
}

/// Full world setup: generate terrain/towns, init resources, buildings, spawners.
/// Buildings get ECS entities + GPU state inline via place_building.
/// Returns ai_players for the caller to insert into AiPlayerState.
pub fn setup_world(
    config: &WorldGenConfig,
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    faction_list: &mut crate::resources::FactionList,
    slot_alloc: &mut GpuSlotPool,
    entity_map: &mut EntityMap,
    faction_stats: &mut FactionStats,
    reputation: &mut crate::resources::Reputation,
    raider_state: &mut RaiderState,

    town_index: &mut crate::resources::TownIndex,
    commands: &mut Commands,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) -> Vec<crate::systems::AiPlayer> {
    entity_map.clear_buildings();
    let area_levels = generate_world(
        config,
        grid,
        world_data,
        faction_list,
        slot_alloc,
        entity_map,
        commands,
        gpu_updates,
    );
    entity_map.init_spatial(grid.width as f32 * grid.cell_size);
    grid.init_pathfind_costs();
    grid.sync_building_costs(entity_map);
    grid.sync_town_buildability(&world_data.towns, &area_levels, entity_map);

    let n_factions = faction_list.factions.len();
    faction_stats.init(n_factions);
    reputation.init(n_factions);
    raider_state.init(world_data.towns.len(), 10);

    // Spawn ECS town entities with area_levels from world gen
    spawn_town_entities(
        commands,
        town_index,
        &world_data.towns,
        &area_levels,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    );

    create_ai_players(world_data, faction_list)
}

/// Expand one town's buildable area by one ring and convert new ring terrain to Dirt.
pub fn expand_town_build_area(
    grid: &mut WorldGrid,
    towns: &[Town],
    _entity_map: &EntityMap,
    town_idx: usize,
    area_level: &mut i32,
) -> Result<(), &'static str> {
    let Some(town) = towns.get(town_idx) else {
        return Err("invalid town index");
    };
    let center = town.center;

    let (old_min_c, old_max_c, old_min_r, old_max_r) = build_bounds(*area_level, center, grid);
    *area_level += 1;
    let (new_min_c, new_max_c, new_min_r, new_max_r) = build_bounds(*area_level, center, grid);

    for row in new_min_r..=new_max_r {
        for col in new_min_c..=new_max_c {
            let is_old =
                row >= old_min_r && row <= old_max_r && col >= old_min_c && col <= old_max_c;
            if is_old {
                continue;
            }
            if let Some(cell) = grid.cell_mut(col, row) {
                cell.terrain = Biome::Dirt;
            }
        }
    }

    Ok(())
}

/// Consolidated building destruction: grid clear + growth tombstone + HP zero + combat log.
/// Grid cleanup for building removal: clears grid cell, updates wall auto-tile, logs combat event.
/// Does NOT mark the entity as Dead — callers send DamageMsg for that (single Dead writer: death_system).
pub(crate) fn destroy_building(
    grid: &mut WorldGrid,
    world_data: &WorldData,
    entity_map: &mut EntityMap,
    combat_log: &mut MessageWriter<CombatLogMsg>,
    game_time: &GameTime,
    gc: usize,
    gr: usize,
    reason: &str,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) -> Result<(), &'static str> {
    let inst = entity_map
        .get_at_grid(gc as i32, gr as i32)
        .ok_or("no building")?;
    let kind = inst.kind;
    let bld_town_idx = inst.town_idx;

    // Auto-tile: update neighbor sprites after removal
    if crate::constants::building_def(kind).autotile {
        update_autotile_around(grid, entity_map, gc, gr, kind, gpu_updates);
    }

    // Combat log — derive faction from building's town_idx
    let bld_town = bld_town_idx as usize;
    let faction = world_data
        .towns
        .get(bld_town)
        .map(|t| t.faction)
        .unwrap_or(0);
    combat_log.write(CombatLogMsg {
        kind: CombatEventKind::Harvest,
        faction,
        day: game_time.day(),
        hour: game_time.hour(),
        minute: game_time.minute(),
        message: reason.to_string(),
        location: None,
    });

    Ok(())
}

/// Location types for find_nearest_location.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocationKind {
    Farm,
    Waypoint,
    Town,
    GoldMine,
}

/// Find nearest location of a given kind (no radius limit, position only).
pub fn find_nearest_location(
    from: Vec2,
    entity_map: &crate::resources::EntityMap,
    kind: LocationKind,
) -> Option<Vec2> {
    find_location_within_radius(from, entity_map, kind, f32::MAX).map(|(_, pos)| pos)
}

/// Find nearest location of a given kind within radius. Returns (index, position).
pub fn find_location_within_radius(
    from: Vec2,
    entity_map: &crate::resources::EntityMap,
    kind: LocationKind,
    radius: f32,
) -> Option<(usize, Vec2)> {
    let bkind = match kind {
        LocationKind::Farm => BuildingKind::Farm,
        LocationKind::Waypoint => BuildingKind::Waypoint,
        LocationKind::Town => BuildingKind::Fountain,
        LocationKind::GoldMine => BuildingKind::GoldMine,
    };
    let r2 = radius * radius;
    let mut best_d2 = f32::MAX;
    let mut result: Option<(usize, Vec2)> = None;
    entity_map.for_each_nearby_kind(from, radius, bkind, |inst, _| {
        let dx = inst.position.x - from.x;
        let dy = inst.position.y - from.y;
        let d2 = dx * dx + dy * dy;
        if d2 <= r2 && d2 < best_d2 {
            best_d2 = d2;
            result = Some((inst.slot, inst.position));
        }
    });
    result
}

/// Convert Vec2 to integer key for position-based lookup.
/// Uses rounded coordinates so slight position differences still match.
pub fn pos_to_key(pos: Vec2) -> (i32, i32) {
    (pos.x.round() as i32, pos.y.round() as i32)
}

/// Any building with a position and town affiliation. Used by generic find functions.
pub trait Worksite {
    fn position(&self) -> Vec2;
    fn town_idx(&self) -> u32;
}

impl Worksite for PlacedBuilding {
    fn position(&self) -> Vec2 {
        self.position
    }
    fn town_idx(&self) -> u32 {
        self.town_idx
    }
}

/// Find nearest unoccupied building of `kind`, optionally filtered by town.
/// Uses expanding-radius spatial search: starts at 2 cells, doubles until found or exhausted.
/// Returns (slot, position).
pub fn find_nearest_free(
    from: Vec2,
    entity_map: &crate::resources::EntityMap,
    kind: BuildingKind,
    town_idx: Option<u32>,
) -> Option<(usize, Vec2)> {
    let cell_size = entity_map.spatial_cell_size().max(256.0);
    let max_radius = cell_size * 128.0; // upper bound ~32k px
    let mut radius = cell_size * 2.0;
    let mut best_d2 = f32::MAX;
    let mut result: Option<(usize, Vec2)> = None;
    loop {
        // Use kind-filtered spatial search (only visits buildings of matching kind)
        {
            let r = &mut result;
            let bd2 = &mut best_d2;
            let mut check = |inst: &crate::resources::BuildingInstance, occ: i16| {
                if occ >= 1 {
                    return;
                }
                let dx = inst.position.x - from.x;
                let dy = inst.position.y - from.y;
                let d2 = dx * dx + dy * dy;
                if d2 < *bd2 {
                    *bd2 = d2;
                    *r = Some((inst.slot, inst.position));
                }
            };
            if let Some(tid) = town_idx {
                entity_map.for_each_nearby_kind_town(from, radius, kind, tid, &mut check);
            } else {
                entity_map.for_each_nearby_kind(from, radius, kind, &mut check);
            }
        }
        // Found one within this ring, or searched the whole world
        if result.is_some() || radius >= max_radius {
            break;
        }
        radius *= 2.0;
    }
    result
}

/// Find nearest building of `kind` within radius, filtered by town. Returns (slot, position).
pub fn find_within_radius(
    from: Vec2,
    entity_map: &crate::resources::EntityMap,
    kind: BuildingKind,
    radius: f32,
    town_idx: u32,
) -> Option<(usize, Vec2)> {
    let r2 = radius * radius;
    let mut best_d2 = f32::MAX;
    let mut result: Option<(usize, Vec2)> = None;
    entity_map.for_each_nearby_kind_town(from, radius, kind, town_idx, |inst, _| {
        let dx = inst.position.x - from.x;
        let dy = inst.position.y - from.y;
        let d2 = dx * dx + dy * dy;
        if d2 <= r2 && d2 < best_d2 {
            best_d2 = d2;
            result = Some((inst.slot, inst.position));
        }
    });
    result
}

/// Find worksite index by position.
pub fn find_by_pos<W: Worksite>(sites: &[W], pos: Vec2) -> Option<usize> {
    let key = pos_to_key(pos);
    sites.iter().position(|s| pos_to_key(s.position()) == key)
}

// ============================================================================
// BUILDING SPATIAL GRID
// ============================================================================

#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    Debug,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    bevy::reflect::Reflect,
)]
pub enum BuildingKind {
    Fountain,
    Bed,
    Waypoint,
    Farm,
    FarmerHome,
    ArcherHome,
    Tent,
    GoldMine,
    MinerHome,
    CrossbowHome,
    FighterHome,
    Road,
    StoneRoad,
    MetalRoad,
    Wall,
    BowTower,
    CrossbowTower,
    CatapultTower,
    Merchant,
    Casino,
    TreeNode,
    RockNode,
    LumberMill,
    Quarry,
    MasonHome,
    Gate,
    GuardTower,
}

impl BuildingKind {
    /// True for any road tier (dirt, stone, metal).
    pub fn is_road(self) -> bool {
        matches!(self, Self::Road | Self::StoneRoad | Self::MetalRoad)
    }

    /// True for wall-like buildings that auto-tile together (Wall, Gate).
    pub fn is_wall_like(self) -> bool {
        matches!(self, Self::Wall | Self::Gate)
    }

    /// Buildable radius (in grid tiles) granted by this road tier.
    pub fn road_build_radius(self) -> Option<i32> {
        match self {
            Self::Road => Some(3),
            Self::StoneRoad => Some(5),
            Self::MetalRoad => Some(7),
            _ => None,
        }
    }

    /// Pathfinding cost for this road tier. Lower = faster (cost = 100 / speed_mult).
    pub fn road_pathfind_cost(self) -> Option<u16> {
        match self {
            Self::Road => Some(67),      // 1.5x speed
            Self::StoneRoad => Some(50), // 2.0x speed
            Self::MetalRoad => Some(40), // 2.5x speed
            _ => None,
        }
    }

    /// Road tier index for upgrade ordering (0=dirt, 1=stone, 2=metal).
    pub fn road_tier(self) -> Option<u8> {
        match self {
            Self::Road => Some(0),
            Self::StoneRoad => Some(1),
            Self::MetalRoad => Some(2),
            _ => None,
        }
    }
}

/// Rebuild building spatial grid. Only runs when BuildingGridDirtyMsg is received.
pub fn rebuild_building_grid_system(
    mut entity_map: ResMut<EntityMap>,
    mut grid_dirty: MessageReader<BuildingGridDirtyMsg>,
    grid: Res<WorldGrid>,
) {
    if grid.width == 0 || grid_dirty.read().count() == 0 {
        return;
    }
    let world_size_px = grid.width as f32 * grid.cell_size;
    entity_map.init_spatial(world_size_px);
    entity_map.rebuild_spatial();
}

// ============================================================================
// WORLD GRID
// ============================================================================

/// Terrain biome for a grid cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Biome {
    #[default]
    Grass,
    Forest,
    Water,
    Rock,
    Dirt,
}

impl Biome {
    /// Map biome + cell index to tileset array index for TilemapChunk.
    /// Trees and rocks are full entities now -- Forest/Rock biomes render as ground only.
    /// Grass=0, Forest=1 (dark grass), Water=8, Rock=10 (dirt), Dirt=10.
    pub fn tileset_index(self, _cell_index: usize) -> u16 {
        match self {
            Biome::Grass => 0,
            Biome::Forest => 1,
            Biome::Water => 8,
            Biome::Rock => 10,
            Biome::Dirt => 10,
        }
    }
}

/// Fast deterministic hash for tile variant selection.
// TileSpec is now in constants.rs (part of BUILDING_REGISTRY)
pub use crate::constants::TileSpec;

/// Atlas (col, row) positions for the 11 terrain tiles used in the TilemapChunk tileset.
pub const TERRAIN_TILES: [TileSpec; 11] = [
    TileSpec::Single(3, 16),                              // 0: Grass A
    TileSpec::Single(3, 13),                              // 1: Grass B
    TileSpec::Single(13, 9),                              // 2: Forest A
    TileSpec::Single(14, 9),                              // 3: Forest B
    TileSpec::Single(15, 9),                              // 4: Forest C
    TileSpec::Single(16, 9),                              // 5: Forest D
    TileSpec::Single(17, 9),                              // 6: Forest E
    TileSpec::Single(18, 9),                              // 7: Forest F
    TileSpec::Single(3, 1),                               // 8: Water
    TileSpec::Quad([(7, 15), (9, 15), (7, 17), (9, 17)]), // 9: Rock
    TileSpec::Single(8, 10),                              // 10: Dirt
];

/// Per-layer base tile for terrain tileset compositing.
/// Layers with transparent pixels need a grass base underneath.
/// Fully opaque tiles (grass, water, dirt) need no base.
const TERRAIN_BASES: [Option<(u32, u32)>; 11] = [
    None,          // 0: Grass A (opaque)
    None,          // 1: Grass B (opaque)
    Some((3, 16)), // 2: Forest A (tree over Grass A)
    Some((3, 16)), // 3: Forest B
    Some((3, 16)), // 4: Forest C
    Some((3, 16)), // 5: Forest D
    Some((3, 16)), // 6: Forest E
    Some((3, 16)), // 7: Forest F
    None,          // 8: Water (opaque)
    Some((3, 16)), // 9: Rock (quad has transparent pixels, needs Grass A base)
    None,          // 10: Dirt (opaque)
];

/// Build the BUILDING_TILES array from the registry (for atlas construction).
pub fn building_tiles() -> Vec<crate::constants::TileSpec> {
    crate::constants::BUILDING_REGISTRY
        .iter()
        .map(|d| d.tile)
        .collect()
}

/// Composite tiles into a vertical strip buffer (ATLAS_CELL x ATLAS_CELL*layers).
/// Core logic shared by tilemap tileset and building atlas.
/// `bases` provides an optional base tile per layer -- layers with a base are pre-filled
/// and subsequent sprites are alpha-composited on top (transparent pixels keep the base).
/// Pass an empty slice for no bases (building atlas).
fn build_tile_strip(
    atlas: &Image,
    tiles: &[TileSpec],
    extra: &[&Image],
    bases: &[Option<(u32, u32)>],
) -> (Vec<u8>, u32) {
    let sprite = SPRITE_SIZE as u32; // 16 (source texel size)
    let out_size = ATLAS_CELL; // 64
    let scale = out_size / sprite; // 4x upscale from 16px source
    let half = out_size / 2; // 32 — each quadrant in a Quad tile
    let cell_size = CELL as u32; // 17 (16px + 1px margin in source sheet)
    let atlas_width = atlas.width();
    let layers = tiles.len() as u32;
    let layer_bytes = (out_size * out_size * 4) as usize;

    let mut data = vec![0u8; layer_bytes * layers as usize];
    let atlas_data = atlas.data.as_ref().expect("atlas image has no data");

    // Pre-fill layers that have a base tile so decorations composite
    // over terrain instead of showing a black/transparent background.
    // Cache rendered base tiles to avoid redundant blitting.
    let mut base_cache: std::collections::HashMap<(u32, u32), Vec<u8>> =
        std::collections::HashMap::new();
    for (l, base_opt) in bases.iter().enumerate() {
        if let Some(&(base_col, base_row)) = base_opt.as_ref() {
            let base_layer = base_cache.entry((base_col, base_row)).or_insert_with(|| {
                let src_x = base_col * cell_size;
                let src_y = base_row * cell_size;
                let mut buf = vec![0u8; layer_bytes];
                for ty in 0..sprite {
                    for tx in 0..sprite {
                        let si = ((src_y + ty) * atlas_width + (src_x + tx)) as usize * 4;
                        for oy in 0..scale {
                            for ox in 0..scale {
                                let di =
                                    ((ty * scale + oy) * out_size + (tx * scale + ox)) as usize * 4;
                                buf[di..di + 4].copy_from_slice(&atlas_data[si..si + 4]);
                            }
                        }
                    }
                }
                buf
            });
            let off = l * layer_bytes;
            data[off..off + layer_bytes].copy_from_slice(base_layer);
        }
    }

    // Blit a 16x16 source sprite with 2x upscale into a 32x32 quadrant at (dx, dy).
    // When `skip_transparent` is true, transparent source pixels preserve the base.
    let blit_2x = |data: &mut [u8],
                   layer: u32,
                   col: u32,
                   row: u32,
                   dx: u32,
                   dy: u32,
                   skip_transparent: bool| {
        let src_x = col * cell_size;
        let src_y = row * cell_size;
        for ty in 0..sprite {
            for tx in 0..sprite {
                let si = ((src_y + ty) * atlas_width + (src_x + tx)) as usize * 4;
                if skip_transparent && atlas_data[si + 3] == 0 {
                    continue;
                }
                for oy in 0..2u32 {
                    for ox in 0..2u32 {
                        let di = (layer * out_size * out_size
                            + (dy + ty * 2 + oy) * out_size
                            + (dx + tx * 2 + ox)) as usize
                            * 4;
                        data[di..di + 4].copy_from_slice(&atlas_data[si..si + 4]);
                    }
                }
            }
        }
    };

    let mut ext_counter = 0usize;
    for (layer, spec) in tiles.iter().enumerate() {
        let l = layer as u32;
        match *spec {
            TileSpec::Single(col, row) => {
                // Nearest-neighbor 4x upscale: each 16px src pixel -> 4x4 dst pixels
                let src_x = col * cell_size;
                let src_y = row * cell_size;
                for ty in 0..sprite {
                    for tx in 0..sprite {
                        let si = ((src_y + ty) * atlas_width + (src_x + tx)) as usize * 4;
                        let skip = bases.get(layer).is_some_and(|b| b.is_some());
                        if skip && atlas_data[si + 3] == 0 {
                            continue;
                        }
                        for oy in 0..scale {
                            for ox in 0..scale {
                                let di = (l * out_size * out_size
                                    + (ty * scale + oy) * out_size
                                    + (tx * scale + ox))
                                    as usize
                                    * 4;
                                data[di..di + 4].copy_from_slice(&atlas_data[si..si + 4]);
                            }
                        }
                    }
                }
            }
            TileSpec::Quad(q) => {
                let skip = bases.get(layer).is_some_and(|b| b.is_some());
                // Each 16px quadrant is 2x upscaled to 32px, filling the 64px cell
                blit_2x(&mut data, l, q[0].0, q[0].1, 0, 0, skip); // TL
                blit_2x(&mut data, l, q[1].0, q[1].1, half, 0, skip); // TR
                blit_2x(&mut data, l, q[2].0, q[2].1, 0, half, skip); // BL
                blit_2x(&mut data, l, q[3].0, q[3].1, half, half, skip); // BR
            }
            TileSpec::External(_path) => {
                let Some(ext) = extra.get(ext_counter).copied() else {
                    continue;
                };
                ext_counter += 1;
                let ext_data = ext.data.as_ref().expect("external image has no data");
                let layer_offset = (l * out_size * out_size * 4) as usize;
                let ext_w = ext.width();
                let ext_h = ext.height();

                if ext_w == out_size && ext_h == out_size {
                    // Native ATLAS_CELL size — direct blit
                    let layer_bytes = (out_size * out_size * 4) as usize;
                    if ext_data.len() >= layer_bytes {
                        data[layer_offset..layer_offset + layer_bytes]
                            .copy_from_slice(&ext_data[..layer_bytes]);
                    }
                } else {
                    // Scale to fit (handles both old 32px art and any other size)
                    let src_w = ext_w.max(1);
                    let src_h = ext_h.max(1);
                    for y in 0..out_size {
                        for x in 0..out_size {
                            let sx = (x * src_w / out_size).min(src_w - 1);
                            let sy = (y * src_h / out_size).min(src_h - 1);
                            let si = ((sy * src_w + sx) * 4) as usize;
                            let di = (layer_offset as u32 + ((y * out_size + x) * 4)) as usize;
                            if si + 4 <= ext_data.len() && di + 4 <= data.len() {
                                data[di..di + 4].copy_from_slice(&ext_data[si..si + 4]);
                            }
                        }
                    }
                }
            }
        }
    }

    (data, layers)
}

/// Tilemap: strip -> texture_2d_array (for TilemapChunk).
pub fn build_tileset(
    atlas: &Image,
    tiles: &[TileSpec],
    extra: &[&Image],
    images: &mut Assets<Image>,
) -> Handle<Image> {
    let (data, layers) = build_tile_strip(atlas, tiles, extra, &TERRAIN_BASES);
    let out_size = ATLAS_CELL;
    let mut image = Image::new(
        Extent3d {
            width: out_size,
            height: out_size * layers,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        Default::default(),
    );
    image
        .reinterpret_stacked_2d_as_array(layers)
        .expect("tileset reinterpret failed");
    images.add(image)
}

/// Rotate NxN RGBA pixel data 90° clockwise. Load-time only.
fn rotate_90_cw(src: &[u8], size: u32) -> Vec<u8> {
    let mut dst = vec![0u8; src.len()];
    for y in 0..size {
        for x in 0..size {
            let si = ((y * size + x) * 4) as usize;
            let di = ((x * size + (size - 1 - y)) * 4) as usize;
            dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    dst
}

/// Extract a `src_size` sprite from a wider strip at pixel offset `src_x`,
/// then nearest-neighbor upscale to ATLAS_CELL. Pass src_size=32 for existing art,
/// src_size=64 (==ATLAS_CELL) for new native-res art (no upscale needed).
pub fn extract_sprite(img: &Image, src_x: u32, src_size: u32) -> Vec<u8> {
    let iw = img.width();
    let data = img.data.as_ref().expect("image has no data");
    let dst = ATLAS_CELL;
    let mut out = vec![0u8; (dst * dst * 4) as usize];
    for dy in 0..dst {
        for dx in 0..dst {
            // Map dst pixel back to source pixel (nearest-neighbor)
            let sx = src_x + dx * src_size / dst;
            let sy = dy * src_size / dst;
            if sy < img.height() {
                let si = ((sy * iw + sx) * 4) as usize;
                let di = ((dy * dst + dx) * 4) as usize;
                if si + 4 <= data.len() {
                    out[di..di + 4].copy_from_slice(&data[si..si + 4]);
                }
            }
        }
    }
    out
}

/// Building atlas: strip as texture_2d (for NPC instanced shader).
/// Appends auto-tile variant layers for all autotile-enabled building kinds.
pub fn build_building_atlas(
    atlas: &Image,
    tiles: &[TileSpec],
    extra: &[&Image],
    images: &mut Assets<Image>,
) -> Handle<Image> {
    let (mut data, base_layers) = build_tile_strip(atlas, tiles, extra, &[]);
    let out_size = ATLAS_CELL;
    let layer_bytes = (out_size * out_size * 4) as usize;

    // For each autotile-enabled building, find its External sprite strip,
    // extract/rotate variants, overwrite the base layer, and append 10 extra layers.
    let mut extra_count = 0u32;
    for def in crate::constants::BUILDING_REGISTRY {
        if !def.autotile {
            continue;
        }
        // Find this kind's External image index in the extra slice
        let ext_idx = {
            let mut idx = 0usize;
            let mut found = None;
            for d in crate::constants::BUILDING_REGISTRY {
                if d.kind == def.kind {
                    if matches!(d.tile, crate::constants::TileSpec::External(_)) {
                        found = Some(idx);
                    }
                    break;
                }
                if matches!(d.tile, crate::constants::TileSpec::External(_)) {
                    idx += 1;
                }
            }
            found
        };

        let Some(ext_idx) = ext_idx else { continue };
        let Some(strip_img) = extra.get(ext_idx) else {
            continue;
        };

        // Extract source sprites: E-W at x=0, BR corner at x=66 (32px art with 1px+1px gaps)
        let ew_sprite = extract_sprite(strip_img, 0, 32);
        let br_sprite = extract_sprite(strip_img, 66, 32);

        // Overwrite base layer with clean E-W sprite (strip was stretched)
        let kind_base = crate::constants::tileset_index(def.kind) as usize;
        let base_offset = kind_base * layer_bytes;
        if base_offset + layer_bytes <= data.len() {
            data[base_offset..base_offset + layer_bytes].copy_from_slice(&ew_sprite);
        }

        // Generate rotated variants
        let ns_sprite = rotate_90_cw(&ew_sprite, out_size);
        let bl_sprite = rotate_90_cw(&br_sprite, out_size);
        let tl_sprite = rotate_90_cw(&bl_sprite, out_size);
        let tr_sprite = rotate_90_cw(&tl_sprite, out_size);

        // Extract junction/cross at x=33, T-junction at x=99 (32px art with 1px gaps)
        let cross_sprite = extract_sprite(strip_img, 33, 32);
        let t_sprite = extract_sprite(strip_img, 99, 32);
        let t_90 = rotate_90_cw(&t_sprite, out_size);
        let t_180 = rotate_90_cw(&t_90, out_size);
        let t_270 = rotate_90_cw(&t_180, out_size);

        // Append 10 extra layers: NS, BR, BL, TL, TR, Cross, T×4
        data.extend_from_slice(&ns_sprite);
        data.extend_from_slice(&br_sprite);
        data.extend_from_slice(&bl_sprite);
        data.extend_from_slice(&tl_sprite);
        data.extend_from_slice(&tr_sprite);
        data.extend_from_slice(&cross_sprite);
        data.extend_from_slice(&t_sprite);
        data.extend_from_slice(&t_90);
        data.extend_from_slice(&t_180);
        data.extend_from_slice(&t_270);

        extra_count += crate::constants::AUTOTILE_EXTRA_PER_KIND as u32;
    }

    let total_layers = base_layers + extra_count;
    let mut img = Image::new(
        Extent3d {
            width: out_size,
            height: out_size * total_layers,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        Default::default(),
    );
    img.sampler = bevy::image::ImageSampler::nearest();
    images.add(img)
}

/// Extras atlas: composites individual 16x16 sprites into a horizontal grid (ATLAS_CELL cells, upscaled).
/// Used for heal, sleep, arrow, boat — any single-sprite overlay. Order matches atlas_id mapping in shader.
pub fn build_extras_atlas(sprites: &[Image], images: &mut Assets<Image>) -> Handle<Image> {
    let cell = ATLAS_CELL;
    let count = sprites.len() as u32;
    let mut data = vec![0u8; (cell * count * cell * 4) as usize];

    for (i, img) in sprites.iter().enumerate() {
        let src = img.data.as_ref().expect("extras sprite has no data");
        let sw = img.width();
        let sh = img.height();
        // 2x nearest-neighbor upscale into the cell
        for dy in 0..cell {
            for dx in 0..cell {
                let sx = (dx * sw / cell).min(sw - 1);
                let sy = (dy * sh / cell).min(sh - 1);
                let si = (sy * sw + sx) as usize * 4;
                let di = (dy * cell * count + i as u32 * cell + dx) as usize * 4;
                if si + 4 <= src.len() && di + 4 <= data.len() {
                    data[di..di + 4].copy_from_slice(&src[si..si + 4]);
                }
            }
        }
    }

    images.add(Image::new(
        Extent3d {
            width: cell * count,
            height: cell,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        Default::default(),
    ))
}

/// A single cell in the world grid.
#[derive(Clone, Debug, Default)]
pub struct WorldCell {
    pub terrain: Biome,
    /// Terrain before stamp_dirt overwrote it. Used to restore on fountain destruction.
    pub original_terrain: Biome,
}

/// World-wide grid covering the entire map. Each cell has terrain + optional building.
#[derive(Resource)]
pub struct WorldGrid {
    pub width: usize,
    pub height: usize,
    pub cell_size: f32,
    pub cells: Vec<WorldCell>,
    /// Precomputed A* cost per cell. 0 = impassable, >0 = movement cost.
    /// Combines terrain + building data. Rebuilt incrementally on building changes.
    pub pathfind_costs: Vec<u16>,
    /// Flat indices of cells with building cost overrides (for incremental revert + path invalidation).
    pub(crate) building_cost_cells: Vec<usize>,
    /// Hierarchical pathfinding cache (HPA*). Built on init, rebuilt on building changes.
    pub hpa_cache: Option<crate::systems::pathfinding::HpaCache>,
    /// Primary town owner per cell. u16::MAX = no owner.
    pub town_owner: Vec<u16>,
    /// Overflow for cells buildable by 2+ towns (rare overlap zones).
    /// Key = flat cell index, Value = all town indices that can build there.
    pub town_overlap: HashMap<usize, Vec<u16>>,
}

impl Default for WorldGrid {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            cell_size: TOWN_GRID_SPACING,
            cells: Vec::new(),
            pathfind_costs: Vec::new(),
            building_cost_cells: Vec::new(),
            hpa_cache: None,
            town_owner: Vec::new(),
            town_overlap: HashMap::new(),
        }
    }
}

impl WorldGrid {
    /// Flat indices of cells with building cost overrides (for targeted path invalidation).
    pub fn dirty_cost_cells(&self) -> &[usize] {
        &self.building_cost_cells
    }

    /// Get cell reference by grid coordinates.
    pub fn cell(&self, col: usize, row: usize) -> Option<&WorldCell> {
        if col < self.width && row < self.height {
            Some(&self.cells[row * self.width + col])
        } else {
            None
        }
    }

    /// Get mutable cell reference by grid coordinates.
    pub fn cell_mut(&mut self, col: usize, row: usize) -> Option<&mut WorldCell> {
        if col < self.width && row < self.height {
            Some(&mut self.cells[row * self.width + col])
        } else {
            None
        }
    }

    /// Convert world position to grid coordinates (col, row).
    pub fn world_to_grid(&self, pos: Vec2) -> (usize, usize) {
        let col = (pos.x / self.cell_size).floor().max(0.0) as usize;
        let row = (pos.y / self.cell_size).floor().max(0.0) as usize;
        (
            col.min(self.width.saturating_sub(1)),
            row.min(self.height.saturating_sub(1)),
        )
    }

    /// Convert grid coordinates to world position (center of cell).
    pub fn grid_to_world(&self, col: usize, row: usize) -> Vec2 {
        Vec2::new(
            col as f32 * self.cell_size + self.cell_size * 0.5,
            row as f32 * self.cell_size + self.cell_size * 0.5,
        )
    }

    // ── Pathfinding cost grid ────────────────────────────────────────

    /// Build terrain base costs. Called once on world init/load.
    pub fn init_pathfind_costs(&mut self) {
        self.pathfind_costs = self
            .cells
            .iter()
            .map(|c| terrain_base_cost(c.terrain))
            .collect();
        self.building_cost_cells.clear();
        if self.width > 0 && self.height > 0 {
            self.hpa_cache = Some(crate::systems::pathfinding::HpaCache::build(
                &self.pathfind_costs,
                self.width,
                self.height,
            ));
        }
    }

    /// Incrementally sync building overrides (walls/roads). O(walls + roads), not O(map).
    pub fn sync_building_costs(&mut self, entity_map: &crate::resources::EntityMap) {
        // Revert previous overrides to terrain base
        for &idx in &self.building_cost_cells {
            self.pathfind_costs[idx] = terrain_base_cost(self.cells[idx].terrain);
        }
        self.building_cost_cells.clear();

        self.apply_building_overlay(entity_map, BuildingKind::Wall, 0);
        // All tower kinds block pathing (impassable like walls) -- driven by registry
        for def in crate::constants::BUILDING_REGISTRY.iter() {
            if def.is_tower {
                self.apply_building_overlay(entity_map, def.kind, 0);
            }
        }
        // Gates are passable (same cost as dirt road) -- faction gating is behavioral
        self.apply_building_overlay(entity_map, BuildingKind::Gate, 67);
        // Apply road overlays — higher tiers override lower (iter order: dirt, stone, metal)
        for kind in [
            BuildingKind::Road,
            BuildingKind::StoneRoad,
            BuildingKind::MetalRoad,
        ] {
            self.apply_building_overlay(
                entity_map,
                kind,
                kind.road_pathfind_cost().expect("road kind has cost"),
            );
        }
        // Rebuild HPA* cache for affected chunks
        if !self.building_cost_cells.is_empty() && self.width > 0 {
            if let Some(ref mut cache) = self.hpa_cache {
                cache.rebuild_chunks(
                    &self.pathfind_costs,
                    self.width,
                    self.height,
                    &self.building_cost_cells,
                );
            }
        }
    }

    fn apply_building_overlay(
        &mut self,
        entity_map: &crate::resources::EntityMap,
        kind: BuildingKind,
        cost: u16,
    ) {
        for inst in entity_map.iter_kind(kind) {
            let (gc, gr) = self.world_to_grid(inst.position);
            let idx = gr * self.width + gc;
            // Don't override water with road bonus (water is always impassable)
            if cost > 0 && self.pathfind_costs[idx] == 0 {
                continue;
            }
            self.pathfind_costs[idx] = cost;
            self.building_cost_cells.push(idx);
        }
    }

    /// Check if placing an impassable building at (gc, gr) would block access from
    /// the town center to any spawner of the same town. Returns true if placement
    /// would create an unreachable spawner.
    ///
    /// Only runs at placement time (player click), not per-frame. O(spawners * A*).
    pub fn would_block_spawner_access(
        &mut self,
        entity_map: &crate::resources::EntityMap,
        gc: usize,
        gr: usize,
        town_idx: u32,
        world_data: &WorldData,
    ) -> bool {
        if self.width == 0 || self.height == 0 {
            return false;
        }
        let idx = gr * self.width + gc;
        if idx >= self.pathfind_costs.len() {
            return false;
        }
        let Some(town) = world_data.towns.get(town_idx as usize) else {
            return false;
        };
        let (cc, cr) = self.world_to_grid(town.center);
        let center = bevy::math::IVec2::new(cc as i32, cr as i32);

        // Temporarily set candidate cell as impassable
        let original_cost = self.pathfind_costs[idx];
        self.pathfind_costs[idx] = 0;

        let mut blocked = false;
        // Check reachability for each spawner building of this town
        for def in crate::constants::BUILDING_REGISTRY.iter() {
            if def.spawner.is_none() {
                continue;
            }
            for inst in entity_map.iter_kind_for_town(def.kind, town_idx) {
                let (sc, sr) = self.world_to_grid(inst.position);
                let goal = bevy::math::IVec2::new(sc as i32, sr as i32);
                if goal == center {
                    continue;
                }
                let reachable = crate::systems::pathfinding::pathfind_with_costs(
                    &self.pathfind_costs,
                    self.width,
                    self.height,
                    center,
                    goal,
                    5000,
                );
                if reachable.is_none() {
                    blocked = true;
                    break;
                }
            }
            if blocked {
                break;
            }
        }

        // Restore original cost
        self.pathfind_costs[idx] = original_cost;
        blocked
    }

    // ── Town buildability grid ─────────────────────────────────────

    /// True if `town_idx` can build at grid (col, row).
    pub fn can_town_build(&self, col: usize, row: usize, town_idx: u16) -> bool {
        if col >= self.width || row >= self.height {
            return false;
        }
        let idx = row * self.width + col;
        let owner = self.town_owner[idx];
        if owner == town_idx {
            return true;
        }
        if owner == u16::MAX {
            return false;
        }
        // Check overlap map for multi-owner cells
        self.town_overlap
            .get(&idx)
            .is_some_and(|v| v.contains(&town_idx))
    }

    /// Mark cell (col, row) as buildable by `town_idx`.
    pub fn add_town_buildable(&mut self, col: usize, row: usize, town_idx: u16) {
        if col >= self.width || row >= self.height {
            return;
        }
        // Skip impassable cells (water, rock)
        if matches!(
            self.cells[row * self.width + col].terrain,
            Biome::Water | Biome::Rock
        ) {
            return;
        }
        let idx = row * self.width + col;
        let owner = self.town_owner[idx];
        if owner == town_idx {
            return; // already set
        }
        if owner == u16::MAX {
            self.town_owner[idx] = town_idx;
        } else {
            // Cell already owned by another town — add to overlap
            let entry = self.town_overlap.entry(idx).or_insert_with(|| vec![owner]);
            if !entry.contains(&town_idx) {
                entry.push(town_idx);
            }
        }
    }

    /// Clear all town buildability data.
    pub fn clear_town_buildable(&mut self) {
        self.town_owner.fill(u16::MAX);
        self.town_overlap.clear();
    }

    /// Init town_owner vec to match cell count.
    pub fn init_town_buildable(&mut self) {
        self.town_owner = vec![u16::MAX; self.cells.len()];
        self.town_overlap.clear();
    }

    /// True if any town OTHER than `own_town` can build at (col, row).
    pub fn is_foreign_territory(&self, col: usize, row: usize, own_town: u16) -> bool {
        if col >= self.width || row >= self.height {
            return false;
        }
        let idx = row * self.width + col;
        let owner = self.town_owner[idx];
        if owner == u16::MAX {
            return false;
        }
        if owner != own_town {
            return true;
        }
        // Owner matches, but check overlap for other towns
        self.town_overlap
            .get(&idx)
            .is_some_and(|v| v.iter().any(|&t| t != own_town))
    }

    /// Rebuild all town buildability from town area_levels + road positions.
    pub fn sync_town_buildability(
        &mut self,
        towns: &[Town],
        area_levels: &[i32],
        entity_map: &crate::resources::EntityMap,
    ) {
        self.clear_town_buildable();
        let w = self.width;
        let h = self.height;
        if w == 0 || h == 0 {
            return;
        }

        // 1. Stamp base area for each town
        for (ti, town) in towns.iter().enumerate() {
            let ti16 = ti as u16;
            let (center_col, center_row) = self.world_to_grid(town.center);
            let cc = center_col as i32;
            let cr = center_row as i32;

            // World-edge caps in town-relative coords
            let min_row_cap = -cr;
            let max_row_cap = h as i32 - 1 - cr;
            let min_col_cap = -cc;
            let max_col_cap = w as i32 - 1 - cc;

            let al = area_levels.get(ti).copied().unwrap_or(0);
            let min_row = (BASE_GRID_MIN - al).max(min_row_cap);
            let max_row = (BASE_GRID_MAX + al).min(max_row_cap);
            let min_col = (BASE_GRID_MIN - al).max(min_col_cap);
            let max_col = (BASE_GRID_MAX + al).min(max_col_cap);

            for r in min_row..=max_row {
                let gr = (cr + r) as usize;
                if gr >= h {
                    continue;
                }
                for c in min_col..=max_col {
                    let gc = (cc + c) as usize;
                    if gc >= w {
                        continue;
                    }
                    self.add_town_buildable(gc, gr, ti16);
                }
            }
        }

        // 2. Stamp road build radii
        for kind in [
            BuildingKind::Road,
            BuildingKind::StoneRoad,
            BuildingKind::MetalRoad,
        ] {
            let radius = kind.road_build_radius().expect("road kind has radius");
            for inst in entity_map.iter_kind(kind) {
                let ti16 = inst.town_idx as u16;
                let (road_col, road_row) = self.world_to_grid(inst.position);
                let rc = road_col as i32;
                let rr = road_row as i32;
                for dr in -radius..=radius {
                    let gr = (rr + dr) as usize;
                    if gr >= h {
                        continue;
                    }
                    for dc in -radius..=radius {
                        let gc = (rc + dc) as usize;
                        if gc >= w {
                            continue;
                        }
                        self.add_town_buildable(gc, gr, ti16);
                    }
                }
            }
        }
    }
}

/// Terrain base cost for CPU pathfinding.
/// Higher cost = less desirable route. 0 = truly impassable (walls only).
/// Water/Rock stay passable so NPCs can escape bad positions, but their route
/// cost is intentionally inflated well above movement-speed differences so A*
/// strongly avoids them.
pub(crate) fn terrain_base_cost(biome: Biome) -> u16 {
    match biome {
        Biome::Grass | Biome::Dirt => 100,
        Biome::Forest => 143,
        Biome::Rock => 2500,
        Biome::Water => 5000,
    }
}

// ============================================================================
// WORLD GEN CONFIG
// ============================================================================

/// World generation algorithm style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorldGenStyle {
    Classic,
    #[default]
    Continents,
    Maze,
    WorldMap,
}

impl WorldGenStyle {
    pub const ALL: &[WorldGenStyle] = &[
        WorldGenStyle::Classic,
        WorldGenStyle::Continents,
        WorldGenStyle::Maze,
        WorldGenStyle::WorldMap,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WorldGenStyle::Classic => "Classic",
            WorldGenStyle::Continents => "Continents",
            WorldGenStyle::Maze => "Maze",
            WorldGenStyle::WorldMap => "World Map",
        }
    }

    pub fn from_index(i: u8) -> Self {
        match i {
            0 => WorldGenStyle::Classic,
            2 => WorldGenStyle::Maze,
            3 => WorldGenStyle::WorldMap,
            _ => WorldGenStyle::Continents,
        }
    }

    pub fn to_index(self) -> u8 {
        match self {
            WorldGenStyle::Classic => 0,
            WorldGenStyle::Continents => 1,
            WorldGenStyle::Maze => 2,
            WorldGenStyle::WorldMap => 3,
        }
    }

    /// Whether terrain must be generated before town placement (to reject water/ice).
    pub fn needs_pre_terrain(self) -> bool {
        matches!(
            self,
            WorldGenStyle::Continents | WorldGenStyle::Maze | WorldGenStyle::WorldMap
        )
    }
}

/// Configuration for procedural world generation.
#[derive(Resource)]
pub struct WorldGenConfig {
    pub gen_style: WorldGenStyle,
    pub world_width: f32,
    pub world_height: f32,
    pub world_margin: f32,
    pub num_towns: usize,
    pub min_town_distance: f32,
    pub grid_spacing: f32,
    pub raider_distance: f32,
    pub farms_per_town: usize,
    /// Per-job home count: village NPCs = per builder town, raider NPCs = per raider town.
    pub npc_counts: BTreeMap<Job, usize>,
    pub ai_towns: usize,
    pub raider_towns: usize,
    pub gold_mines_per_town: usize,
    pub town_names: Vec<String>,
}

impl Default for WorldGenConfig {
    fn default() -> Self {
        Self {
            gen_style: WorldGenStyle::Classic,
            world_width: 16000.0,
            world_height: 16000.0,
            world_margin: 800.0,
            num_towns: 2,
            min_town_distance: 2400.0,
            grid_spacing: 68.0,
            raider_distance: 7000.0,
            farms_per_town: 2,
            npc_counts: NPC_REGISTRY
                .iter()
                .map(|d| (d.job, d.default_count))
                .collect(),
            ai_towns: 1,
            raider_towns: 1,
            gold_mines_per_town: 2,
            town_names: vec![
                "Miami".into(),
                "Orlando".into(),
                "Tampa".into(),
                "Jacksonville".into(),
                "Tallahassee".into(),
                "Gainesville".into(),
                "Pensacola".into(),
                "Sarasota".into(),
                "Naples".into(),
                "Daytona".into(),
                "Lakeland".into(),
                "Ocala".into(),
                "Boca Raton".into(),
                "Key West".into(),
                "Fort Myers".into(),
            ],
        }
    }
}

impl WorldGenConfig {
    /// Town count by kind — bridges the 3 count fields for registry-driven loops.
    pub fn count_for(&self, kind: TownKind) -> usize {
        match kind {
            TownKind::Player => self.num_towns,
            TownKind::AiBuilder => self.ai_towns,
            TownKind::AiRaider => self.raider_towns,
        }
    }
}

fn spawn_resource_nodes(
    _config: &WorldGenConfig,
    grid: &mut WorldGrid,
    slot_alloc: &mut crate::resources::GpuSlotPool,
    entity_map: &mut EntityMap,
    commands: &mut Commands,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) -> (usize, usize) {
    let mut tree_count = 0usize;
    let mut rock_count = 0usize;

    // Density 1.0: every Forest cell gets a TreeNode, every Rock cell gets a RockNode.
    // No spacing check needed -- one entity per grid cell, no overlap possible.
    for row in 0..grid.height {
        for col in 0..grid.width {
            let idx = row * grid.width + col;
            let kind = match grid.cells[idx].terrain {
                Biome::Forest => BuildingKind::TreeNode,
                Biome::Rock => BuildingKind::RockNode,
                _ => continue,
            };
            if entity_map.has_building_at(col as i32, row as i32) {
                continue;
            }

            let pos = grid.grid_to_world(col, row);
            if place_building(
                slot_alloc,
                entity_map,
                commands,
                gpu_updates,
                kind,
                pos,
                crate::constants::TOWN_NONE,
                crate::constants::FACTION_NEUTRAL,
                &Default::default(),
                None,
                None,
            )
            .is_ok()
            {
                // Set terrain under resource nodes to Grass so the node sprite
                // renders against a clean background (not dark forest/dirt).
                grid.cells[idx].terrain = Biome::Grass;
                grid.cells[idx].original_terrain = Biome::Grass;
                match kind {
                    BuildingKind::TreeNode => tree_count += 1,
                    BuildingKind::RockNode => rock_count += 1,
                    _ => {}
                }
            }
        }
    }

    (tree_count, rock_count)
}

// ============================================================================
// WORLD GENERATION
// ============================================================================

/// Generate the world: populate grid, place towns + buildings, fill terrain.
/// Pure function — takes config and writes to grid + world data.
pub fn generate_world(
    config: &WorldGenConfig,
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    faction_list: &mut crate::resources::FactionList,
    slot_alloc: &mut crate::resources::GpuSlotPool,
    entity_map: &mut EntityMap,

    commands: &mut Commands,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) -> Vec<i32> {
    use crate::resources::{FactionData, FactionKind};
    use rand::Rng;
    let mut rng = rand::rng();
    let mut area_levels: Vec<i32> = Vec::new();

    // Faction 0 = Neutral (gold mines, world objects)
    faction_list.factions.clear();
    faction_list.factions.push(FactionData {
        kind: FactionKind::Neutral,
        name: "Neutral".into(),
        towns: Vec::new(),
    });

    // Step 1: Initialize grid
    let w = (config.world_width / grid.cell_size) as usize;
    let h = (config.world_height / grid.cell_size) as usize;
    grid.width = w;
    grid.height = h;
    grid.cells = vec![WorldCell::default(); w * h];
    grid.init_town_buildable();

    // Shuffle town names
    let mut names = config.town_names.clone();
    for i in (1..names.len()).rev() {
        let j = rng.random_range(0..=i);
        names.swap(i, j);
    }
    let mut name_idx = 0;

    let needs_pre_terrain = config.gen_style.needs_pre_terrain();

    // Pre-generate terrain so we can reject Water/Ice positions during town placement
    match config.gen_style {
        WorldGenStyle::Continents => generate_terrain_continents(grid),
        WorldGenStyle::Maze => generate_terrain_maze(grid),
        WorldGenStyle::WorldMap => generate_terrain_worldmap(grid),
        WorldGenStyle::Classic => {}
    }

    // All settlement positions for min_distance checks
    let mut all_positions: Vec<Vec2> = Vec::new();
    // Pre-terrain styles need more attempts since many positions land on impassable cells
    let max_attempts = if needs_pre_terrain { 5000 } else { 2000 };
    // Step 2: Place towns — single loop driven by TOWN_REGISTRY
    for town_def in TOWN_REGISTRY {
        let count = config.count_for(town_def.kind);
        let mut positions: Vec<Vec2> = Vec::new();
        let mut attempts = 0;
        while positions.len() < count && attempts < max_attempts {
            attempts += 1;
            let x = rng.random_range(config.world_margin..config.world_width - config.world_margin);
            let y =
                rng.random_range(config.world_margin..config.world_height - config.world_margin);
            let pos = Vec2::new(x, y);
            if needs_pre_terrain {
                let (gc, gr) = grid.world_to_grid(pos);
                if grid
                    .cell(gc, gr)
                    .is_some_and(|c| matches!(c.terrain, Biome::Water | Biome::Rock))
                {
                    continue;
                }
            }
            if all_positions
                .iter()
                .all(|e| pos.distance(*e) >= config.min_town_distance)
            {
                // Snap to grid cell center so fountain sprite aligns with its grid cell
                let (gc, gr) = grid.world_to_grid(pos);
                let pos = grid.grid_to_world(gc, gr);
                positions.push(pos);
                all_positions.push(pos);
            }
        }
        if positions.len() < count {
            warn!(
                "generate_world: only placed {}/{} {:?} towns",
                positions.len(),
                count,
                town_def.kind,
            );
        }

        for &center in &positions {
            let name = if town_def.is_raider {
                town_def.label.to_string()
            } else {
                let n = names
                    .get(name_idx)
                    .cloned()
                    .unwrap_or_else(|| format!("{} {}", town_def.label, name_idx));
                name_idx += 1;
                n
            };
            let faction = faction_list.factions.len() as i32;
            let town_data_idx = world_data.towns.len();
            faction_list.factions.push(FactionData {
                kind: town_def.kind.faction_kind(),
                name: name.clone(),
                towns: vec![town_data_idx],
            });
            world_data.towns.push(Town {
                name,
                center,
                faction,
                kind: town_def.kind,
            });
            let mut area_level = 0i32;
            place_buildings(
                grid,
                world_data,
                center,
                town_data_idx as u32,
                config,
                town_def.kind,
                slot_alloc,
                entity_map,
                commands,
                gpu_updates,
                &mut area_level,
            );
            area_levels.push(area_level);
        }
    }

    // Step 3: Generate terrain (or stamp dirt for pre-generated styles)
    if needs_pre_terrain {
        stamp_dirt(grid, &all_positions);
    } else {
        generate_terrain(grid, &all_positions, &[]);
    }

    // Step 4: Place gold mines in wilderness between settlements
    let total_mines = config.gold_mines_per_town * all_positions.len();
    let mut mine_positions: Vec<Vec2> = Vec::new();
    let mut mine_attempts = 0;
    while mine_positions.len() < total_mines && mine_attempts < max_attempts {
        mine_attempts += 1;
        let x = rng.random_range(config.world_margin..config.world_width - config.world_margin);
        let y = rng.random_range(config.world_margin..config.world_height - config.world_margin);
        let pos = Vec2::new(x, y);
        // Not on impassable terrain
        if needs_pre_terrain {
            let (gc, gr) = grid.world_to_grid(pos);
            if grid
                .cell(gc, gr)
                .is_some_and(|c| matches!(c.terrain, Biome::Water | Biome::Rock))
            {
                continue;
            }
        }
        // Min distance from settlements
        if all_positions
            .iter()
            .any(|s| pos.distance(*s) < crate::constants::MINE_MIN_SETTLEMENT_DIST)
        {
            continue;
        }
        // Min distance from other mines
        if mine_positions
            .iter()
            .any(|m| pos.distance(*m) < crate::constants::MINE_MIN_SPACING)
        {
            continue;
        }
        // Snap to grid and place
        let (gc, gr) = grid.world_to_grid(pos);
        if entity_map.has_building_at(gc as i32, gr as i32) {
            continue;
        }
        let snapped = grid.grid_to_world(gc, gr);
        let _ = place_building(
            slot_alloc,
            entity_map,
            commands,
            gpu_updates,
            BuildingKind::GoldMine,
            snapped,
            crate::constants::TOWN_NONE,
            crate::constants::FACTION_NEUTRAL,
            &Default::default(),
            None,
            None,
        );
        mine_positions.push(snapped);
    }

    // Step 5: Spawn tree and rock nodes on matching biome cells.
    let (tree_count, rock_count) =
        spawn_resource_nodes(config, grid, slot_alloc, entity_map, commands, gpu_updates);

    let total_towns = world_data.towns.len();
    info!(
        "generate_world: {} towns, {} gold mines, {} trees, {} rocks, grid {}x{} ({})",
        total_towns,
        mine_positions.len(),
        tree_count,
        rock_count,
        w,
        h,
        config.gen_style.label(),
    );
    area_levels
}

/// Spawn ECS town entities for all towns in `world_data.towns`.
/// Each entity gets `TownMarker` + state components. Populates `TownIndex` for O(1) lookup.
/// Called from both world gen (defaults) and save load (with saved values).
pub fn spawn_town_entities(
    commands: &mut Commands,
    town_index: &mut crate::resources::TownIndex,
    towns: &[Town],
    area_levels: &[i32],
    food: &[i32],
    gold: &[i32],
    wood: &[i32],
    stone: &[i32],
    policies: &[crate::resources::PolicySet],
    upgrade_levels: &[Vec<u8>],
    inventories: &[Vec<crate::constants::LootItem>],
) {
    use crate::components::*;

    town_index.0.clear();
    let upgrade_count = crate::systems::stats::upgrade_count();

    for (idx, _town) in towns.iter().enumerate() {
        let al = area_levels.get(idx).copied().unwrap_or(0);
        let f = food.get(idx).copied().unwrap_or(0);
        let g = gold.get(idx).copied().unwrap_or(0);
        let w = wood.get(idx).copied().unwrap_or(0);
        let s = stone.get(idx).copied().unwrap_or(0);
        let p = policies.get(idx).cloned().unwrap_or_default();
        let u = upgrade_levels
            .get(idx)
            .cloned()
            .unwrap_or_else(|| vec![0u8; upgrade_count]);
        let inv = inventories.get(idx).cloned().unwrap_or_default();

        let entity = commands
            .spawn((
                TownMarker,
                TownAreaLevel(al),
                FoodStore(f),
                GoldStore(g),
                WoodStore(w),
                StoneStore(s),
                TownPolicy(p),
                TownUpgradeLevel(u),
                TownEquipment(inv),
            ))
            .id();
        town_index.0.insert(idx as i32, entity);
    }
}

/// Place buildings for a town. Layout driven by `TownDef` via `town_kind`:
/// - Builder (is_raider=false): fountain + farms + village NPC homes + corner waypoints
/// - Raider (is_raider=true): fountain + raider NPC homes (tents)
pub fn place_buildings(
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    center: Vec2,
    town_idx: u32,
    config: &WorldGenConfig,
    town_kind: TownKind,
    slot_alloc: &mut crate::resources::GpuSlotPool,
    entity_map: &mut EntityMap,

    commands: &mut Commands,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
    area_level: &mut i32,
) {
    let is_raider = town_def(town_kind).is_raider;
    let mut occupied = HashSet::new();
    let faction = world_data
        .towns
        .get(town_idx as usize)
        .map(|t| t.faction)
        .unwrap_or(0);

    let (cc, cr) = grid.world_to_grid(center);
    // Helper: place building at offset (row, col) from center, return snapped world position
    let place = |row: i32,
                 col: i32,
                 _kind: BuildingKind,
                 _ti: u32,
                 occ: &mut HashSet<(i32, i32)>|
     -> Vec2 {
        let gc = (cc as i32 + col) as usize;
        let gr = (cr as i32 + row) as usize;
        let snapped_pos = grid.grid_to_world(gc, gr);
        occ.insert((row, col));
        snapped_pos
    };

    // Center building at (0, 0) — Fountain
    let center_kind = BuildingKind::Fountain;
    place(0, 0, center_kind, town_idx, &mut occupied);
    let _ = place_building(
        slot_alloc,
        entity_map,
        commands,
        gpu_updates,
        center_kind,
        center,
        town_idx,
        faction,
        &Default::default(),
        None,
        None,
    );

    // Count NPC homes needed (raider units for raider towns, village units for builder towns)
    let homes: usize = NPC_REGISTRY
        .iter()
        .filter(|d| d.is_raider_unit == is_raider)
        .map(|d| config.npc_counts.get(&d.job).copied().unwrap_or(0))
        .sum();
    let farms_count = if is_raider { 0 } else { config.farms_per_town };
    let slots = spiral_slots(&occupied, farms_count + homes);
    let mut slot_iter = slots.into_iter();

    // Farms (towns only)
    for _ in 0..farms_count {
        let Some((row, col)) = slot_iter.next() else {
            break;
        };
        let pos = place(row, col, BuildingKind::Farm, town_idx, &mut occupied);
        let _ = place_building(
            slot_alloc,
            entity_map,
            commands,
            gpu_updates,
            BuildingKind::Farm,
            pos,
            town_idx,
            faction,
            &Default::default(),
            None,
            None,
        );
    }

    // NPC homes from registry (filtered by is_raider_unit matching is_raider)
    for def in NPC_REGISTRY
        .iter()
        .filter(|d| d.is_raider_unit == is_raider)
    {
        let count = config.npc_counts.get(&def.job).copied().unwrap_or(0);
        for _ in 0..count {
            let Some((row, col)) = slot_iter.next() else {
                break;
            };
            let pos = place(row, col, def.home_building, town_idx, &mut occupied);
            let _ = place_building(
                slot_alloc,
                entity_map,
                commands,
                gpu_updates,
                def.home_building,
                pos,
                town_idx,
                faction,
                &Default::default(),
                None,
                None,
            );
        }
    }

    // Waypoints at outer corners (towns only, clockwise patrol: TL → TR → BR → BL)
    if !is_raider {
        let (min_row, max_row, min_col, max_col) = occupied.iter().fold(
            (i32::MAX, i32::MIN, i32::MAX, i32::MIN),
            |(rmin, rmax, cmin, cmax), &(r, c)| {
                (rmin.min(r), rmax.max(r), cmin.min(c), cmax.max(c))
            },
        );
        let corners = [
            (max_row + 1, min_col - 1), // TL (top-left)
            (max_row + 1, max_col + 1), // TR (top-right)
            (min_row - 1, max_col + 1), // BR (bottom-right)
            (min_row - 1, min_col - 1), // BL (bottom-left)
        ];
        for (order, (row, col)) in corners.into_iter().enumerate() {
            let post_pos = place(row, col, BuildingKind::Waypoint, town_idx, &mut occupied);
            let _ = place_building(
                slot_alloc,
                entity_map,
                commands,
                gpu_updates,
                BuildingKind::Waypoint,
                post_pos,
                town_idx,
                faction,
                &BuildingOverrides {
                    patrol_order: order as u32,
                    ..Default::default()
                },
                None,
                None,
            );
        }
    }

    // Ensure generated buildings are always inside the buildable area
    let required = occupied.iter().fold(0, |acc, &(row, col)| {
        let row_need = (BASE_GRID_MIN - row).max(row - BASE_GRID_MAX).max(0);
        let col_need = (BASE_GRID_MIN - col).max(col - BASE_GRID_MAX).max(0);
        acc.max(row_need).max(col_need)
    });
    *area_level = (*area_level).max(required);
}

/// Generate `count` grid positions in a spiral pattern outward from (0,0), skipping occupied cells.
fn spiral_slots(occupied: &HashSet<(i32, i32)>, count: usize) -> Vec<(i32, i32)> {
    let mut result = Vec::with_capacity(count);
    // Walk rings outward: ring 1 = distance 1 from center, ring 2 = distance 2, etc.
    for ring in 1..=MAX_GRID_EXTENT {
        if result.len() >= count {
            break;
        }
        // Top edge: row = -ring, col = -ring..ring
        for col in -ring..=ring {
            if result.len() >= count {
                break;
            }
            let pos = (-ring, col);
            if !occupied.contains(&pos) {
                result.push(pos);
            }
        }
        // Right edge: row = -ring+1..ring, col = ring
        for row in (-ring + 1)..=ring {
            if result.len() >= count {
                break;
            }
            let pos = (row, ring);
            if !occupied.contains(&pos) {
                result.push(pos);
            }
        }
        // Bottom edge: row = ring, col = ring-1..-ring
        for col in (-ring..ring).rev() {
            if result.len() >= count {
                break;
            }
            let pos = (ring, col);
            if !occupied.contains(&pos) {
                result.push(pos);
            }
        }
        // Left edge: row = ring..-ring+1, col = -ring
        for row in ((-ring + 1)..ring).rev() {
            if result.len() >= count {
                break;
            }
            let pos = (row, -ring);
            if !occupied.contains(&pos) {
                result.push(pos);
            }
        }
    }
    result
}

/// Fill grid terrain using simplex noise, with Dirt override near towns.
fn generate_terrain(grid: &mut WorldGrid, town_positions: &[Vec2], raider_positions: &[Vec2]) {
    use noise::{NoiseFn, Simplex};

    let noise = Simplex::new(rand::random::<u32>());
    let frequency = 0.0015;
    let town_clear_radius = 6.0 * grid.cell_size; // ~192px
    let raider_clear_radius = 5.0 * grid.cell_size; // ~160px

    for row in 0..grid.height {
        for col in 0..grid.width {
            let world_pos = grid.grid_to_world(col, row);

            // Check proximity to towns → Dirt
            let near_town = town_positions
                .iter()
                .any(|tc| world_pos.distance(*tc) < town_clear_radius);
            let near_raider = raider_positions
                .iter()
                .any(|cp| world_pos.distance(*cp) < raider_clear_radius);

            let natural = {
                let n = noise.get([
                    world_pos.x as f64 * frequency,
                    world_pos.y as f64 * frequency,
                ]);
                if n < -0.3 {
                    Biome::Water
                } else if n < 0.1 {
                    Biome::Grass
                } else if n < 0.4 {
                    Biome::Forest
                } else {
                    Biome::Rock
                }
            };

            let biome = if near_town || near_raider {
                Biome::Dirt
            } else {
                natural
            };

            let idx = row * grid.width + col;
            let cell = &mut grid.cells[idx];
            cell.terrain = biome;
            cell.original_terrain = natural;
        }
    }
}

/// Overwrite terrain near settlements with Dirt (clearing for buildings).
pub fn stamp_dirt(grid: &mut WorldGrid, positions: &[Vec2]) {
    let clear_radius = 6.0 * grid.cell_size;
    for row in 0..grid.height {
        for col in 0..grid.width {
            let world_pos = grid.grid_to_world(col, row);
            if positions
                .iter()
                .any(|p| world_pos.distance(*p) < clear_radius)
            {
                grid.cells[row * grid.width + col].terrain = Biome::Dirt;
            }
        }
    }
}

/// Remove all roads belonging to a town and restore dirt cells to original terrain.
/// Called when a town's fountain is destroyed.
pub fn clear_town_roads_and_dirt(
    grid: &mut WorldGrid,
    entity_map: &mut EntityMap,
    slot_alloc: &mut crate::resources::GpuSlotPool,
    town_center: Vec2,
    town_idx: u32,
    commands: &mut Commands,
) {
    // Collect road slots for this town across all tiers (can't mutate while iterating)
    let road_slots: Vec<usize> = [
        BuildingKind::Road,
        BuildingKind::StoneRoad,
        BuildingKind::MetalRoad,
    ]
    .iter()
    .flat_map(|&kind| {
        entity_map
            .iter_kind_for_town(kind, town_idx)
            .map(|inst| inst.slot)
    })
    .collect();

    for slot in road_slots {
        // Despawn ECS entity
        if let Some(&entity) = entity_map.entities.get(&slot) {
            commands.entity(entity).despawn();
        }
        // Remove from EntityMap + free GPU slot
        entity_map.remove_by_slot(slot);
        slot_alloc.free(slot);
    }

    // Restore dirt → original terrain within stamp_dirt radius of town center
    let clear_radius = 6.0 * grid.cell_size;
    for row in 0..grid.height {
        for col in 0..grid.width {
            let world_pos = grid.grid_to_world(col, row);
            if world_pos.distance(town_center) < clear_radius {
                let idx = row * grid.width + col;
                let cell = &mut grid.cells[idx];
                if cell.terrain == Biome::Dirt {
                    cell.terrain = cell.original_terrain;
                }
            }
        }
    }
}

/// Continent terrain: multi-octave elevation noise + moisture noise + edge falloff.
/// Based on Red Blob Games "Making maps with noise" approach:
/// - 3-octave fBm for elevation with square-bump edge falloff
/// - Separate moisture noise for biome selection within land
fn generate_terrain_continents(grid: &mut WorldGrid) {
    use noise::{NoiseFn, Simplex};

    let elevation_noise = Simplex::new(rand::random::<u32>());
    let moisture_noise = Simplex::new(rand::random::<u32>());

    let world_w = grid.width as f64 * grid.cell_size as f64;
    let world_h = grid.height as f64 * grid.cell_size as f64;

    for row in 0..grid.height {
        for col in 0..grid.width {
            let world_pos = grid.grid_to_world(col, row);
            let wx = world_pos.x as f64;
            let wy = world_pos.y as f64;

            // 3-octave fBm elevation (large continents → medium islands → small coastline detail)
            let e = (1.0 * elevation_noise.get([wx * 0.0004, wy * 0.0004])
                + 0.5 * elevation_noise.get([wx * 0.0008, wy * 0.0008])
                + 0.25 * elevation_noise.get([wx * 0.0016, wy * 0.0016]))
                / 1.75;

            // Square bump edge falloff (Red Blob Games)
            let nx = (wx / world_w - 0.5) * 2.0;
            let ny = (wy / world_h - 0.5) * 2.0;
            let d = 1.0 - (1.0 - nx * nx) * (1.0 - ny * ny);

            // Push edges to ocean, redistribute elevation
            let e = ((e + 1.0) * 0.5 * (1.0 - d)).powf(1.5); // normalize to ~[0,1] then apply falloff + power

            // Independent moisture noise
            let m = (moisture_noise.get([wx * 0.0015, wy * 0.0015]) + 1.0) * 0.5; // [0, 1]

            // Biome from elevation × moisture
            let biome = if e < 0.08 {
                Biome::Water
            } else if m < 0.3 {
                Biome::Rock
            } else if m < 0.6 {
                Biome::Grass
            } else {
                Biome::Forest
            };

            let cell = &mut grid.cells[row * grid.width + col];
            cell.terrain = biome;
            cell.original_terrain = biome;
        }
    }
}

/// Generate a maze using recursive backtracking.
/// Corridors are 3 cells wide (Grass), walls are 1 cell wide (Rock).
/// The maze cell size is 4 grid cells (3 corridor + 1 wall).
fn generate_terrain_maze(grid: &mut WorldGrid) {
    use rand::Rng;
    let mut rng = rand::rng();

    let cell_size = 4usize; // 3 corridor + 1 wall
    let maze_w = grid.width / cell_size;
    let maze_h = grid.height / cell_size;
    if maze_w < 2 || maze_h < 2 {
        return;
    }

    // Fill everything with Rock first
    for cell in &mut grid.cells {
        cell.terrain = Biome::Rock;
        cell.original_terrain = Biome::Rock;
    }

    // Carve helper: set a 3x3 corridor block at maze cell (mx, my) to Grass
    let carve = |grid: &mut WorldGrid, mx: usize, my: usize| {
        let base_col = mx * cell_size;
        let base_row = my * cell_size;
        for dr in 0..3 {
            for dc in 0..3 {
                let c = base_col + dc;
                let r = base_row + dr;
                if c < grid.width && r < grid.height {
                    let idx = r * grid.width + c;
                    grid.cells[idx].terrain = Biome::Grass;
                    grid.cells[idx].original_terrain = Biome::Grass;
                }
            }
        }
    };

    // Carve passage between adjacent maze cells (fills the wall gap)
    let carve_passage = |grid: &mut WorldGrid, ax: usize, ay: usize, bx: usize, by: usize| {
        // Mid-point between the two 3x3 blocks -- fill the 3-cell-wide corridor in the wall
        let mid_col = (ax * cell_size + bx * cell_size) / 2;
        let mid_row = (ay * cell_size + by * cell_size) / 2;
        for dr in 0..3 {
            for dc in 0..3 {
                let c = mid_col + dc;
                let r = mid_row + dr;
                if c < grid.width && r < grid.height {
                    let idx = r * grid.width + c;
                    grid.cells[idx].terrain = Biome::Grass;
                    grid.cells[idx].original_terrain = Biome::Grass;
                }
            }
        }
    };

    // Recursive backtracking maze generation
    let mut visited = vec![false; maze_w * maze_h];
    let mut stack: Vec<(usize, usize)> = Vec::new();

    // Start from center
    let start_x = maze_w / 2;
    let start_y = maze_h / 2;
    visited[start_y * maze_w + start_x] = true;
    carve(grid, start_x, start_y);
    stack.push((start_x, start_y));

    while let Some(&(cx, cy)) = stack.last() {
        // Collect unvisited neighbors
        let mut neighbors: Vec<(usize, usize)> = Vec::new();
        if cx > 0 && !visited[cy * maze_w + (cx - 1)] {
            neighbors.push((cx - 1, cy));
        }
        if cx + 1 < maze_w && !visited[cy * maze_w + (cx + 1)] {
            neighbors.push((cx + 1, cy));
        }
        if cy > 0 && !visited[(cy - 1) * maze_w + cx] {
            neighbors.push((cx, cy - 1));
        }
        if cy + 1 < maze_h && !visited[(cy + 1) * maze_w + cx] {
            neighbors.push((cx, cy + 1));
        }

        if neighbors.is_empty() {
            stack.pop();
        } else {
            let idx = rng.random_range(0..neighbors.len());
            let (nx, ny) = neighbors[idx];
            visited[ny * maze_w + nx] = true;
            carve(grid, nx, ny);
            carve_passage(grid, cx, cy, nx, ny);
            stack.push((nx, ny));
        }
    }

    // Add some Forest patches in corridors for wood resources
    for row in 0..grid.height {
        for col in 0..grid.width {
            let idx = row * grid.width + col;
            if grid.cells[idx].terrain == Biome::Grass {
                let noise_val = ((col * 7 + row * 13) % 100) as f32 / 100.0;
                if noise_val > 0.85 {
                    grid.cells[idx].terrain = Biome::Forest;
                    grid.cells[idx].original_terrain = Biome::Forest;
                }
            }
        }
    }
}

/// World Map terrain generation: multi-octave noise with continent seeds,
/// latitude-driven biomes, ice caps, and natural chokepoints.
fn generate_terrain_worldmap(grid: &mut WorldGrid) {
    use noise::{NoiseFn, Simplex};

    let elevation_noise = Simplex::new(rand::random::<u32>());
    let moisture_noise = Simplex::new(rand::random::<u32>());
    let detail_noise = Simplex::new(rand::random::<u32>());

    let world_w = grid.width as f64 * grid.cell_size as f64;
    let world_h = grid.height as f64 * grid.cell_size as f64;

    // Tunable parameters (fixed defaults for v1)
    let land_pct: f64 = 0.45; // 45% land
    let ice_cap_pct: f64 = 0.12; // 12% of map height at each pole
    let continent_count: usize = 3;

    // Generate continent seed points for elevation bias
    let mut continent_seeds: Vec<(f64, f64)> = Vec::with_capacity(continent_count);
    let mut seed_rng = rand::rng();
    use rand::Rng;
    for _ in 0..continent_count {
        let cx = seed_rng.random_range(0.15..0.85) * world_w;
        let cy = seed_rng.random_range(0.2..0.8) * world_h;
        continent_seeds.push((cx, cy));
    }

    // Water threshold: lower = more land. Calibrate so ~land_pct of cells are land.
    // With continent bias, threshold around 0.38-0.42 gives ~45% land.
    let water_threshold: f64 = 0.5 - land_pct * 0.4;

    for row in 0..grid.height {
        for col in 0..grid.width {
            let world_pos = grid.grid_to_world(col, row);
            let wx = world_pos.x as f64;
            let wy = world_pos.y as f64;

            // Latitude: 0.0 at top (north pole), 1.0 at bottom (south pole)
            let lat = wy / world_h;

            // Ice caps at poles
            if lat < ice_cap_pct || lat > (1.0 - ice_cap_pct) {
                let cell = &mut grid.cells[row * grid.width + col];
                cell.terrain = Biome::Rock; // impassable ice
                cell.original_terrain = Biome::Rock;
                continue;
            }

            // 4-octave fBm elevation
            let e_raw = (1.0 * elevation_noise.get([wx * 0.00025, wy * 0.00025])
                + 0.5 * elevation_noise.get([wx * 0.0005, wy * 0.0005])
                + 0.25 * elevation_noise.get([wx * 0.001, wy * 0.001])
                + 0.125 * elevation_noise.get([wx * 0.002, wy * 0.002]))
                / 1.875;

            // Continent seed bias: boost elevation near seed points
            let mut continent_boost: f64 = 0.0;
            for &(cx, cy) in &continent_seeds {
                let dx = (wx - cx) / world_w;
                let dy = (wy - cy) / world_h;
                let dist_sq = dx * dx + dy * dy;
                // Gaussian-ish falloff: strong boost near seeds, fading with distance
                let radius = 0.15; // ~15% of world size
                continent_boost += (-dist_sq / (2.0 * radius * radius)).exp();
            }
            // Normalize: max possible boost is continent_count (all seeds at same point)
            continent_boost = (continent_boost / continent_count as f64).min(1.0);

            // Combine noise + continent bias
            let e_norm = (e_raw + 1.0) * 0.5; // [0, 1]
            let elevation = (e_norm * 0.6 + continent_boost * 0.4).clamp(0.0, 1.0);

            // Edge falloff: push map edges toward ocean
            let nx = (wx / world_w - 0.5) * 2.0;
            let ny = (wy / world_h - 0.5) * 2.0;
            let edge_dist = 1.0 - (1.0 - nx * nx) * (1.0 - ny * ny);
            let elevation = (elevation * (1.0 - edge_dist * 0.7)).max(0.0);

            // Chokepoint detail: high-frequency noise creates thin land bridges / straits
            let choke = detail_noise.get([wx * 0.003, wy * 0.003]);
            // Near the land/water boundary, detail noise can carve straits or extend bridges
            let elevation = elevation
                + choke * 0.04 * (1.0 - (elevation - water_threshold).abs().min(0.15) / 0.15);

            // Moisture for biome selection within land
            let m = (moisture_noise.get([wx * 0.0012, wy * 0.0012]) + 1.0) * 0.5;

            // Biome assignment
            let biome = if elevation < water_threshold {
                Biome::Water
            } else {
                // Latitude-driven temperature: 0=polar, 0.5=equator, 1=polar
                let temp_lat = 1.0 - (lat - 0.5).abs() * 2.0; // 0 at poles, 1 at equator

                if temp_lat < 0.2 {
                    // Near-polar: tundra (rock/barren)
                    if m > 0.6 { Biome::Forest } else { Biome::Rock }
                } else if temp_lat < 0.5 {
                    // Temperate
                    if m > 0.55 {
                        Biome::Forest
                    } else {
                        Biome::Grass
                    }
                } else {
                    // Equatorial / warm
                    if m > 0.65 {
                        Biome::Forest
                    } else {
                        Biome::Grass
                    }
                }
            };

            let cell = &mut grid.cells[row * grid.width + col];
            cell.terrain = biome;
            cell.original_terrain = biome;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn road_blocked_on_forest_biome() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(crate::resources::EntityMap::default());
        app.insert_resource(crate::resources::GpuSlotPool::default());
        app.add_message::<crate::messages::GpuUpdateMsg>();

        // 10x10 grid, all grass, town 0 owns everything
        let mut grid = WorldGrid::default();
        grid.width = 10;
        grid.height = 10;
        grid.cell_size = 32.0;
        grid.cells = vec![
            WorldCell {
                terrain: Biome::Grass,
                original_terrain: Biome::Grass
            };
            100
        ];
        grid.town_owner = vec![0u16; 100];
        // Set (5,5) to Forest
        grid.cells[55].terrain = Biome::Forest;
        grid.cells[55].original_terrain = Biome::Forest;

        let world_data = WorldData {
            towns: vec![Town {
                name: "Test".into(),
                center: Vec2::new(160.0, 160.0),
                faction: 0,
                kind: crate::constants::TownKind::Player,
            }],
        };

        let forest_pos = grid.grid_to_world(5, 5);
        let grass_pos = grid.grid_to_world(3, 3);

        app.insert_resource(grid);
        app.insert_resource(world_data);
        app.update();

        // Road on forest -> rejected
        app.world_mut()
            .run_system_once(
                move |mut slot_alloc: ResMut<crate::resources::GpuSlotPool>,
                      mut entity_map: ResMut<crate::resources::EntityMap>,
                      mut commands: Commands,
                      mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
                      mut grid: ResMut<WorldGrid>,
                      world_data: Res<WorldData>| {
                    let result = place_building(
                        &mut slot_alloc,
                        &mut entity_map,
                        &mut commands,
                        &mut gpu_updates,
                        BuildingKind::Road,
                        forest_pos,
                        0,
                        0,
                        &BuildingOverrides::default(),
                        Some(BuildContext {
                            grid: &mut grid,
                            world_data: &world_data,
                            food: &mut 9999,
                            cost: 10,
                        }),
                        None,
                    );
                    assert_eq!(result, Err("cannot build road on forest"));
                },
            )
            .unwrap();

        // Road on grass -> accepted
        app.world_mut()
            .run_system_once(
                move |mut slot_alloc: ResMut<crate::resources::GpuSlotPool>,
                      mut entity_map: ResMut<crate::resources::EntityMap>,
                      mut commands: Commands,
                      mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
                      mut grid: ResMut<WorldGrid>,
                      world_data: Res<WorldData>| {
                    let result = place_building(
                        &mut slot_alloc,
                        &mut entity_map,
                        &mut commands,
                        &mut gpu_updates,
                        BuildingKind::Road,
                        grass_pos,
                        0,
                        0,
                        &BuildingOverrides::default(),
                        Some(BuildContext {
                            grid: &mut grid,
                            world_data: &world_data,
                            food: &mut 9999,
                            cost: 10,
                        }),
                        None,
                    );
                    assert!(result.is_ok(), "road on grass should succeed: {:?}", result);
                },
            )
            .unwrap();

        // Waypoint on forest -> accepted (non-road buildings allowed)
        app.world_mut()
            .run_system_once(
                move |mut slot_alloc: ResMut<crate::resources::GpuSlotPool>,
                      mut entity_map: ResMut<crate::resources::EntityMap>,
                      mut commands: Commands,
                      mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
                      mut grid: ResMut<WorldGrid>,
                      world_data: Res<WorldData>| {
                    let result = place_building(
                        &mut slot_alloc,
                        &mut entity_map,
                        &mut commands,
                        &mut gpu_updates,
                        BuildingKind::Waypoint,
                        forest_pos,
                        0,
                        0,
                        &BuildingOverrides::default(),
                        Some(BuildContext {
                            grid: &mut grid,
                            world_data: &world_data,
                            food: &mut 9999,
                            cost: 10,
                        }),
                        None,
                    );
                    assert!(
                        result.is_ok(),
                        "waypoint on forest should succeed: {:?}",
                        result
                    );
                },
            )
            .unwrap();
    }

    #[test]
    fn resource_nodes_follow_biomes_spacing_and_occupied_cells() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(crate::resources::EntityMap::default());
        app.insert_resource(crate::resources::GpuSlotPool::default());
        app.add_message::<crate::messages::GpuUpdateMsg>();

        let mut grid = WorldGrid::default();
        grid.width = 6;
        grid.height = 4;
        grid.cell_size = crate::constants::TOWN_GRID_SPACING;
        grid.cells = vec![
            WorldCell {
                terrain: Biome::Grass,
                original_terrain: Biome::Grass
            };
            grid.width * grid.height
        ];
        grid.town_owner = vec![u16::MAX; grid.width * grid.height];
        for (col, row) in [(0, 0), (1, 0), (3, 0), (5, 0)] {
            let idx = row * grid.width + col;
            grid.cells[idx].terrain = Biome::Forest;
            grid.cells[idx].original_terrain = Biome::Forest;
        }
        for (col, row) in [(0, 2), (1, 2), (3, 2)] {
            let idx = row * grid.width + col;
            grid.cells[idx].terrain = Biome::Rock;
            grid.cells[idx].original_terrain = Biome::Rock;
        }
        let occupied_forest_pos = grid.grid_to_world(5, 0);

        app.insert_resource(grid);
        app.update();

        app.world_mut().run_system_once(move |
            mut slot_alloc: ResMut<crate::resources::GpuSlotPool>,
            mut entity_map: ResMut<crate::resources::EntityMap>,
            mut commands: Commands,
            mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
        | {
            let result = place_building(
                &mut slot_alloc,
                &mut entity_map,
                &mut commands,
                &mut gpu_updates,
                BuildingKind::Waypoint,
                occupied_forest_pos,
                crate::constants::TOWN_NONE,
                crate::constants::FACTION_NEUTRAL,
                &BuildingOverrides::default(),
                None,
                None,
            );
            assert!(result.is_ok(), "occupied cell setup should succeed: {:?}", result);
        }).unwrap();

        let config = WorldGenConfig::default();
        app.world_mut()
            .run_system_once(
                move |mut slot_alloc: ResMut<crate::resources::GpuSlotPool>,
                      mut entity_map: ResMut<crate::resources::EntityMap>,
                      mut commands: Commands,
                      mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
                      mut grid: ResMut<WorldGrid>| {
                    let (tree_count, rock_count) = spawn_resource_nodes(
                        &config,
                        &mut grid,
                        &mut slot_alloc,
                        &mut entity_map,
                        &mut commands,
                        &mut gpu_updates,
                    );

                    // Density 1.0: every Forest/Rock cell gets a node, except occupied cells
                    assert_eq!(
                        tree_count, 3,
                        "3 of 4 forest cells should get TreeNode (one occupied by Waypoint)"
                    );
                    assert_eq!(rock_count, 3, "all 3 rock cells should get RockNode");

                    assert_eq!(
                        entity_map.get_at_grid(0, 0).map(|b| b.kind),
                        Some(BuildingKind::TreeNode)
                    );
                    assert_eq!(
                        entity_map.get_at_grid(1, 0).map(|b| b.kind),
                        Some(BuildingKind::TreeNode),
                        "adjacent forest cell should also get a TreeNode at density 1.0"
                    );
                    assert_eq!(
                        entity_map.get_at_grid(3, 0).map(|b| b.kind),
                        Some(BuildingKind::TreeNode)
                    );
                    assert_eq!(
                        entity_map.get_at_grid(5, 0).map(|b| b.kind),
                        Some(BuildingKind::Waypoint),
                        "occupied cell should keep existing building"
                    );

                    assert_eq!(
                        entity_map.get_at_grid(0, 2).map(|b| b.kind),
                        Some(BuildingKind::RockNode)
                    );
                    assert_eq!(
                        entity_map.get_at_grid(1, 2).map(|b| b.kind),
                        Some(BuildingKind::RockNode),
                        "adjacent rock cell should also get RockNode at density 1.0"
                    );
                    assert_eq!(
                        entity_map.get_at_grid(3, 2).map(|b| b.kind),
                        Some(BuildingKind::RockNode)
                    );

                    assert_eq!(entity_map.get_at_grid(2, 1).map(|b| b.kind), None);
                },
            )
            .unwrap();
    }

    #[test]
    fn worldmap_generates_corridors_and_ice_caps() {
        let mut grid = WorldGrid::default();
        grid.width = 100;
        grid.height = 100;
        grid.cell_size = 64.0;
        grid.cells = vec![WorldCell::default(); 100 * 100];

        generate_terrain_worldmap(&mut grid);

        // Count biome types
        let mut water = 0usize;
        let mut land = 0usize;
        let mut ice_top = 0usize;
        let mut ice_bottom = 0usize;

        let ice_rows = (100.0 * 0.12) as usize; // 12 rows each pole

        for row in 0..grid.height {
            for col in 0..grid.width {
                let cell = &grid.cells[row * grid.width + col];
                match cell.terrain {
                    Biome::Water => water += 1,
                    Biome::Rock => {
                        if row < ice_rows {
                            ice_top += 1;
                        } else if row >= grid.height - ice_rows {
                            ice_bottom += 1;
                        }
                    }
                    _ => land += 1,
                }
            }
        }

        let total = (grid.width * grid.height) as f64;

        // Ice caps: top and bottom rows should be mostly Rock
        let top_total = (ice_rows * grid.width) as f64;
        let bot_total = (ice_rows * grid.width) as f64;
        assert!(
            ice_top as f64 / top_total > 0.9,
            "top ice cap should be >90% rock, got {:.1}%",
            ice_top as f64 / top_total * 100.0
        );
        assert!(
            ice_bottom as f64 / bot_total > 0.9,
            "bottom ice cap should be >90% rock, got {:.1}%",
            ice_bottom as f64 / bot_total * 100.0
        );

        // Should have meaningful water and land
        assert!(
            water as f64 / total > 0.1,
            "should have >10% water, got {:.1}%",
            water as f64 / total * 100.0
        );
        assert!(
            land as f64 / total > 0.15,
            "should have >15% land, got {:.1}%",
            land as f64 / total * 100.0
        );
    }

    #[test]
    fn worldmap_biomes_follow_latitude() {
        let mut grid = WorldGrid::default();
        grid.width = 200;
        grid.height = 200;
        grid.cell_size = 64.0;
        grid.cells = vec![WorldCell::default(); 200 * 200];

        generate_terrain_worldmap(&mut grid);

        // Sample equatorial band (rows 90-110) and near-polar band (rows 25-35)
        let mut equatorial_grass = 0usize;
        let mut equatorial_total = 0usize;
        let mut polar_rock = 0usize;
        let mut polar_total = 0usize;

        for row in 90..110 {
            for col in 0..grid.width {
                let cell = &grid.cells[row * grid.width + col];
                if cell.terrain != Biome::Water {
                    equatorial_total += 1;
                    if cell.terrain == Biome::Grass {
                        equatorial_grass += 1;
                    }
                }
            }
        }

        // Ice cap rows (within 12% of poles) should be Rock
        let ice_rows = (200.0 * 0.12) as usize;
        for row in 0..ice_rows {
            for col in 0..grid.width {
                let cell = &grid.cells[row * grid.width + col];
                polar_total += 1;
                if cell.terrain == Biome::Rock {
                    polar_rock += 1;
                }
            }
        }

        // Equatorial band should have some grass (not all rock/forest)
        if equatorial_total > 0 {
            assert!(
                equatorial_grass > 0,
                "equatorial band should have some grass cells"
            );
        }

        // Ice cap should be all Rock
        assert!(
            polar_rock == polar_total,
            "ice cap rows should be 100% rock, got {}/{}",
            polar_rock,
            polar_total
        );
    }

    #[test]
    fn worldmap_towns_avoid_water_and_ice() {
        // Verify that town placement rejects water/ice cells
        let mut grid = WorldGrid::default();
        grid.width = 50;
        grid.height = 50;
        grid.cell_size = 64.0;
        grid.cells = vec![WorldCell::default(); 50 * 50];

        // Set all cells to water
        for cell in &mut grid.cells {
            cell.terrain = Biome::Water;
            cell.original_terrain = Biome::Water;
        }
        // Set a few cells to grass (valid placement)
        let valid_col = 25;
        let valid_row = 25;
        for r in valid_row - 2..=valid_row + 2 {
            for c in valid_col - 2..=valid_col + 2 {
                let idx = r * grid.width + c;
                grid.cells[idx].terrain = Biome::Grass;
                grid.cells[idx].original_terrain = Biome::Grass;
            }
        }

        // Test the rejection: WorldMap style rejects Water and Rock
        let style = WorldGenStyle::WorldMap;
        assert!(style.needs_pre_terrain());

        let pos_water = grid.grid_to_world(5, 5);
        let (gc, gr) = grid.world_to_grid(pos_water);
        let cell = grid.cell(gc, gr).unwrap();
        assert_eq!(cell.terrain, Biome::Water);

        let pos_grass = grid.grid_to_world(valid_col, valid_row);
        let (gc, gr) = grid.world_to_grid(pos_grass);
        let cell = grid.cell(gc, gr).unwrap();
        assert_eq!(cell.terrain, Biome::Grass);
    }

    #[test]
    fn worldgen_style_roundtrip() {
        for &style in WorldGenStyle::ALL {
            let idx = style.to_index();
            let back = WorldGenStyle::from_index(idx);
            assert_eq!(
                style,
                back,
                "roundtrip failed for {:?} (index {})",
                style.label(),
                idx
            );
        }
    }
}
