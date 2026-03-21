//! World Data - Towns, farms, beds, waypoints, sprite definitions
//! World Grid - 2D cell grid covering entire world (terrain + buildings)
//! World Generation - Procedural town placement and building layout

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::constants::{BASE_GRID_MAX, BASE_GRID_MIN, TOWN_GRID_SPACING};
use crate::messages::BuildingGridDirtyMsg;
use crate::resources::EntityMap;

pub mod autotile;
pub mod buildings;
pub mod worldgen;

pub use self::autotile::*;
pub use self::buildings::*;
pub use self::worldgen::*;

// ============================================================================
// SERIALIZATION HELPERS
// ============================================================================

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
    /// Patrol order -- used by Waypoint only (default 0).
    #[serde(default)]
    pub patrol_order: u32,
    /// Assigned mine position -- used by MinerHome only.
    #[serde(default, with = "opt_vec2_as_array")]
    pub assigned_mine: Option<Vec2>,
    /// Whether mine was manually assigned -- used by MinerHome only.
    #[serde(default)]
    pub manual_mine: bool,
    /// Wall tier level (1-3) -- used by Wall only. 0 = not a wall (default).
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

/// Contains all world layout data. Towns only -- building instances live in EntityMap.
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

/// Find interior roads for a town -- roads whose build-area contribution is fully redundant.
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
// LOCATION HELPERS
// ============================================================================

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
/// On first init (spatial_width == 0) performs a full rebuild from all instances.
/// On subsequent calls, skips the full rebuild: add_instance/remove_instance already
/// maintain the spatial grid incrementally on every building change.
pub fn rebuild_building_grid_system(
    mut entity_map: ResMut<EntityMap>,
    mut grid_dirty: MessageReader<BuildingGridDirtyMsg>,
    grid: Res<WorldGrid>,
) {
    if grid.width == 0 || grid_dirty.read().count() == 0 {
        return;
    }
    let world_size_px = grid.width as f32 * grid.cell_size;
    let was_initialized = entity_map.is_spatial_initialized();
    entity_map.init_spatial(world_size_px);
    if !was_initialized {
        entity_map.rebuild_spatial();
    }
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

    // -- Pathfinding cost grid ------------------------------------------------

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
    /// HPA* rebuild is scoped to only cells whose cost actually changed (not all building cells).
    /// Detects both set membership changes (added/removed buildings) AND cost value changes
    /// (e.g. wall replaced by road at same cell).
    pub fn sync_building_costs(&mut self, entity_map: &crate::resources::EntityMap) {
        // Snapshot old costs at overridden cells so we can diff after rebuild
        let old_costs: Vec<(usize, u16)> = self
            .building_cost_cells
            .iter()
            .map(|&idx| (idx, self.pathfind_costs[idx]))
            .collect();

        // Revert previous overrides to terrain base
        for &(idx, _) in &old_costs {
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
        // Apply road overlays -- higher tiers override lower (iter order: dirt, stone, metal)
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
        // Rebuild HPA* cache only for cells whose cost actually changed (not all building cells).
        // symmetric_difference misses cost-value changes at the same cell (e.g. wall
        // replaced by road -- cell stays in set but cost changes 0 -> 67).
        if self.width > 0 {
            // Collect cells that changed: new overrides with different cost, or removed overrides
            let new_set: hashbrown::HashSet<usize> =
                self.building_cost_cells.iter().copied().collect();
            let mut changed: Vec<usize> = Vec::new();
            // Cells that were overridden before but now reverted (building destroyed)
            for &(idx, old_cost) in &old_costs {
                if !new_set.contains(&idx) {
                    // Reverted to terrain base -- only dirty if cost actually differs
                    if old_cost != self.pathfind_costs[idx] {
                        changed.push(idx);
                    }
                }
            }
            // Cells newly overridden or with changed cost
            let old_map: hashbrown::HashMap<usize, u16> = old_costs.into_iter().collect();
            for &idx in &self.building_cost_cells {
                let new_cost = self.pathfind_costs[idx];
                match old_map.get(&idx) {
                    Some(&prev) if prev == new_cost => {} // unchanged
                    _ => changed.push(idx),               // new or different cost
                }
            }
            if !changed.is_empty() {
                if let Some(ref mut cache) = self.hpa_cache {
                    cache.rebuild_chunks(
                        &self.pathfind_costs,
                        self.width,
                        self.height,
                        &changed,
                    );
                }
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

    /// Check if placing an impassable building at (gc, gr) would block access to any
    /// critical building (spawners, fountain, farms, mines) of the same town.
    /// Returns `Some(reason)` if blocked, `None` if safe.
    ///
    /// Only runs at placement time (player click), not per-frame.
    /// O(critical_buildings * A*) where A* uses max 5000 nodes per query.
    pub fn would_block_critical_access(
        &mut self,
        entity_map: &crate::resources::EntityMap,
        gc: usize,
        gr: usize,
        town_idx: u32,
        world_data: &WorldData,
    ) -> Option<&'static str> {
        use crate::systems::pathfinding::pathfind_with_costs;
        use bevy::math::IVec2;

        if self.width == 0 || self.height == 0 {
            return None;
        }
        let idx = gr * self.width + gc;
        if idx >= self.pathfind_costs.len() {
            return None;
        }
        let town = world_data.towns.get(town_idx as usize)?;
        let (cc, cr) = self.world_to_grid(town.center);
        let center = IVec2::new(cc as i32, cr as i32);

        // Temporarily set candidate cell as impassable
        let original_cost = self.pathfind_costs[idx];
        self.pathfind_costs[idx] = 0;

        let mut reason: Option<&'static str> = None;

        // Check spawner buildings
        'spawners: for def in crate::constants::BUILDING_REGISTRY.iter() {
            if def.spawner.is_none() {
                continue;
            }
            for inst in entity_map.iter_kind_for_town(def.kind, town_idx) {
                let (sc, sr) = self.world_to_grid(inst.position);
                let goal = IVec2::new(sc as i32, sr as i32);
                if goal == center {
                    continue;
                }
                let reachable = pathfind_with_costs(
                    &self.pathfind_costs,
                    self.width,
                    self.height,
                    center,
                    goal,
                    5000,
                );
                if reachable.is_none() {
                    reason = Some("would block access to a spawner");
                    break 'spawners;
                }
            }
        }

        // Check non-spawner critical buildings: fountain, farms, mines
        if reason.is_none() {
            let critical: &[(BuildingKind, &'static str)] = &[
                (BuildingKind::Fountain, "would block access to fountain"),
                (BuildingKind::Farm, "would block access to a farm"),
                (BuildingKind::GoldMine, "would block access to a mine"),
            ];
            'critical: for &(kind, msg) in critical {
                for inst in entity_map.iter_kind_for_town(kind, town_idx) {
                    let (sc, sr) = self.world_to_grid(inst.position);
                    let bld_cell = IVec2::new(sc as i32, sr as i32);
                    if bld_cell == center {
                        // Building IS the town center -- verify it still has a passable neighbor
                        let sealed =
                            [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y]
                                .iter()
                                .all(|&d| {
                                    let nb = bld_cell + d;
                                    if nb.x < 0
                                        || nb.y < 0
                                        || nb.x >= self.width as i32
                                        || nb.y >= self.height as i32
                                    {
                                        return true; // out of bounds counts as blocked
                                    }
                                    let ni = nb.y as usize * self.width + nb.x as usize;
                                    self.pathfind_costs[ni] == 0
                                });
                        if sealed {
                            reason = Some(msg);
                            break 'critical;
                        }
                    } else {
                        // Check that the town center can still reach this building
                        let reachable = pathfind_with_costs(
                            &self.pathfind_costs,
                            self.width,
                            self.height,
                            center,
                            bld_cell,
                            5000,
                        );
                        if reachable.is_none() {
                            reason = Some(msg);
                            break 'critical;
                        }
                    }
                }
            }
        }

        // Restore original cost
        self.pathfind_costs[idx] = original_cost;
        reason
    }

    // -- Town buildability grid -----------------------------------------------

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
            // Cell already owned by another town -- add to overlap
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

#[cfg(test)]
mod tests;
