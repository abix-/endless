//! World Data - Towns, farms, beds, guard posts, sprite definitions
//! World Grid - 2D cell grid covering entire world (terrain + buildings)
//! World Generation - Procedural town placement and building layout

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::{HashMap, HashSet};

use crate::constants::{TOWN_GRID_SPACING, BASE_GRID_MIN, BASE_GRID_MAX, MAX_GRID_EXTENT};
use crate::resources::FarmStates;

// ============================================================================
// SPRITE DEFINITIONS (from roguelikeSheet_transparent.png)
// ============================================================================

/// Sprite sheet constants
pub const CELL: f32 = 17.0;  // 16px sprite + 1px margin
pub const SPRITE_SIZE: f32 = 16.0;
pub const SHEET_SIZE: (f32, f32) = (968.0, 526.0);


// ============================================================================
// WORLD DATA STRUCTS
// ============================================================================

/// A town (villager or raider settlement).
#[derive(Clone, Debug)]
pub struct Town {
    pub name: String,
    pub center: Vec2,
    pub faction: i32,       // 0=Villager, 1+=Raider factions
    pub sprite_type: i32,   // 0=fountain, 1=tent
}

/// A farm building that farmers work at.
#[derive(Clone, Debug)]
pub struct Farm {
    pub position: Vec2,
    pub town_idx: u32,
}

/// A bed where NPCs sleep.
#[derive(Clone, Debug)]
pub struct Bed {
    pub position: Vec2,
    pub town_idx: u32,
}

/// A guard post where guards patrol.
#[derive(Clone, Debug)]
pub struct GuardPost {
    pub position: Vec2,
    pub town_idx: u32,
    /// Patrol order (0-3 for clockwise perimeter)
    pub patrol_order: u32,
}

/// A house that supports 1 farmer (building spawner).
#[derive(Clone, Debug)]
pub struct House {
    pub position: Vec2,
    pub town_idx: u32,
}

/// A barracks that supports 1 guard (building spawner).
#[derive(Clone, Debug)]
pub struct Barracks {
    pub position: Vec2,
    pub town_idx: u32,
}

/// A tent that supports 1 raider (building spawner).
#[derive(Clone, Debug)]
pub struct Tent {
    pub position: Vec2,
    pub town_idx: u32,
}

/// A gold mine in the wilderness (unowned, any faction can mine).
#[derive(Clone, Debug)]
pub struct GoldMine {
    pub position: Vec2,
}

// ============================================================================
// WORLD RESOURCES
// ============================================================================

/// Contains all world layout data. Mutated at runtime when buildings are placed/destroyed.
#[derive(Resource, Default)]
pub struct WorldData {
    pub towns: Vec<Town>,
    pub farms: Vec<Farm>,
    pub beds: Vec<Bed>,
    pub guard_posts: Vec<GuardPost>,
    pub houses: Vec<House>,
    pub barracks: Vec<Barracks>,
    pub tents: Vec<Tent>,
    pub gold_mines: Vec<GoldMine>,
}

// ============================================================================
// TOWN BUILDING GRID
// ============================================================================

/// Per-town building grid. Tracks which slots are unlocked.
/// Grid uses (row, col) relative to town center with TOWN_GRID_SPACING.
/// Base grid: (-2,-2) to (3,3) = 6x6, expandable by unlocking adjacent slots.
pub struct TownGrid {
    pub town_data_idx: usize,
    pub unlocked: HashSet<(i32, i32)>,
}

impl TownGrid {
    /// Create with base 6x6 grid unlocked for the given town data index.
    pub fn new_base(town_data_idx: usize) -> Self {
        let mut unlocked = HashSet::new();
        for row in BASE_GRID_MIN..=BASE_GRID_MAX {
            for col in BASE_GRID_MIN..=BASE_GRID_MAX {
                unlocked.insert((row, col));
            }
        }
        Self { town_data_idx, unlocked }
    }
}

/// All town building grids. One per town (villager and raider camps).
#[derive(Resource, Default)]
pub struct TownGrids {
    pub grids: Vec<TownGrid>,
}

/// Convert town-relative grid coords to world position.
/// Slot (0,0) = town center. Each slot = one WorldGrid cell (32px).
pub fn town_grid_to_world(center: Vec2, row: i32, col: i32) -> Vec2 {
    Vec2::new(
        center.x + col as f32 * TOWN_GRID_SPACING,
        center.y + row as f32 * TOWN_GRID_SPACING,
    )
}

/// Convert world position to nearest town grid coords (row, col).
pub fn world_to_town_grid(center: Vec2, world_pos: Vec2) -> (i32, i32) {
    let col = ((world_pos.x - center.x) / TOWN_GRID_SPACING).round() as i32;
    let row = ((world_pos.y - center.y) / TOWN_GRID_SPACING).round() as i32;
    (row, col)
}

/// Get all locked slots orthogonally adjacent to any unlocked slot.
/// These are the slots the player can unlock next.
pub fn get_adjacent_locked_slots(grid: &TownGrid) -> Vec<(i32, i32)> {
    let mut adjacent = HashSet::new();
    for &(row, col) in &grid.unlocked {
        for &(dr, dc) in &[(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let nr = row + dr;
            let nc = col + dc;
            if nr < -MAX_GRID_EXTENT || nr > MAX_GRID_EXTENT + 1 { continue; }
            if nc < -MAX_GRID_EXTENT || nc > MAX_GRID_EXTENT + 1 { continue; }
            if !grid.unlocked.contains(&(nr, nc)) {
                adjacent.insert((nr, nc));
            }
        }
    }
    adjacent.into_iter().collect()
}

/// Find which town (villager or camp) has a slot matching the given grid coords.
/// Returns the grid index, town data index, and whether the slot is unlocked/locked.
pub fn find_town_slot(
    world_pos: Vec2,
    towns: &[Town],
    grids: &TownGrids,
) -> Option<TownSlotInfo> {
    for (grid_idx, town_grid) in grids.grids.iter().enumerate() {
        let town_data_idx = town_grid.town_data_idx;
        if town_data_idx >= towns.len() { continue; }
        let town = &towns[town_data_idx];

        let (row, col) = world_to_town_grid(town.center, world_pos);

        // Check click is within reasonable range of this grid's slots
        let slot_pos = town_grid_to_world(town.center, row, col);
        let click_radius = TOWN_GRID_SPACING * 0.7;
        if world_pos.distance(slot_pos) > click_radius { continue; }

        if town_grid.unlocked.contains(&(row, col)) {
            return Some(TownSlotInfo {
                grid_idx,
                town_data_idx,
                row, col,
                slot_state: SlotState::Unlocked,
            });
        }

        // Check if it's an adjacent locked slot
        let adjacent = get_adjacent_locked_slots(town_grid);
        if adjacent.contains(&(row, col)) {
            return Some(TownSlotInfo {
                grid_idx,
                town_data_idx,
                row, col,
                slot_state: SlotState::Locked,
            });
        }
    }
    None
}

/// Info about a clicked town grid slot.
pub struct TownSlotInfo {
    pub grid_idx: usize,       // Index into TownGrids.grids
    pub town_data_idx: usize,  // Index into WorldData.towns
    pub row: i32,
    pub col: i32,
    pub slot_state: SlotState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    Unlocked,
    Locked,
}

// ============================================================================
// BUILDING PLACEMENT / REMOVAL
// ============================================================================

/// Place a building on the world grid and register it in WorldData.
/// Returns Ok(()) on success, Err with reason on failure.
pub fn place_building(
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    farm_states: &mut FarmStates,
    building: Building,
    row: i32,
    col: i32,
    town_center: Vec2,
) -> Result<(), &'static str> {
    let world_pos = town_grid_to_world(town_center, row, col);
    let (gc, gr) = grid.world_to_grid(world_pos);
    let snapped_pos = grid.grid_to_world(gc, gr);

    // Validate cell is empty
    if let Some(cell) = grid.cell(gc, gr) {
        if cell.building.is_some() {
            return Err("cell already has a building");
        }
    } else {
        return Err("cell out of bounds");
    }

    // Place on grid
    if let Some(cell) = grid.cell_mut(gc, gr) {
        cell.building = Some(building);
    }

    // Register in WorldData
    match building {
        Building::Farm { town_idx } => {
            world_data.farms.push(Farm { position: snapped_pos, town_idx });
            farm_states.push_farm(snapped_pos);
        }
        Building::Bed { town_idx } => {
            world_data.beds.push(Bed { position: snapped_pos, town_idx });
        }
        Building::GuardPost { town_idx, patrol_order } => {
            world_data.guard_posts.push(GuardPost {
                position: snapped_pos,
                town_idx,
                patrol_order,
            });
        }
        Building::House { town_idx } => {
            world_data.houses.push(House { position: snapped_pos, town_idx });
        }
        Building::Barracks { town_idx } => {
            world_data.barracks.push(Barracks { position: snapped_pos, town_idx });
        }
        Building::Tent { town_idx } => {
            world_data.tents.push(Tent { position: snapped_pos, town_idx });
        }
        _ => {} // Fountain and Camp not player-placeable
    }

    Ok(())
}

/// Remove a building from the world grid. Tombstones in WorldData (position = -99999).
pub fn remove_building(
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    _farm_states: &mut FarmStates,
    row: i32,
    col: i32,
    town_center: Vec2,
) -> Result<(), &'static str> {
    let world_pos = town_grid_to_world(town_center, row, col);
    let (gc, gr) = grid.world_to_grid(world_pos);
    let snapped_pos = grid.grid_to_world(gc, gr);

    let building = match grid.cell(gc, gr) {
        Some(cell) => match &cell.building {
            Some(b) => *b,
            None => return Err("no building to remove"),
        },
        None => return Err("cell out of bounds"),
    };

    // Don't allow removing fountains or camps
    match building {
        Building::Fountain { .. } => return Err("cannot remove fountain"),
        Building::Camp { .. } => return Err("cannot remove camp"),
        _ => {}
    }

    // Clear grid cell
    if let Some(cell) = grid.cell_mut(gc, gr) {
        cell.building = None;
    }

    // Tombstone in WorldData (set position to far offscreen so spatial queries skip it)
    let tombstone = Vec2::new(-99999.0, -99999.0);
    match building {
        Building::Farm { .. } => {
            if let Some(farm) = world_data.farms.iter_mut().find(|f| {
                (f.position - snapped_pos).length() < 1.0
            }) {
                farm.position = tombstone;
            }
            // FarmStates entry stays but is inert (farm at -99999 won't be tended)
        }
        Building::Bed { .. } => {
            if let Some(bed) = world_data.beds.iter_mut().find(|b| {
                (b.position - snapped_pos).length() < 1.0
            }) {
                bed.position = tombstone;
            }
        }
        Building::GuardPost { .. } => {
            if let Some(post) = world_data.guard_posts.iter_mut().find(|g| {
                (g.position - snapped_pos).length() < 1.0
            }) {
                post.position = tombstone;
            }
        }
        Building::House { .. } => {
            if let Some(house) = world_data.houses.iter_mut().find(|h| {
                (h.position - snapped_pos).length() < 1.0
            }) {
                house.position = tombstone;
            }
        }
        Building::Barracks { .. } => {
            if let Some(b) = world_data.barracks.iter_mut().find(|b| {
                (b.position - snapped_pos).length() < 1.0
            }) {
                b.position = tombstone;
            }
        }
        Building::Tent { .. } => {
            if let Some(t) = world_data.tents.iter_mut().find(|t| {
                (t.position - snapped_pos).length() < 1.0
            }) {
                t.position = tombstone;
            }
        }
        _ => {}
    }

    Ok(())
}

/// Location types for find_nearest_location.
#[derive(Clone, Copy, Debug)]
pub enum LocationKind {
    Farm,
    GuardPost,
    Town,
    GoldMine,
}

/// Find nearest location of a given kind (no radius limit, position only).
pub fn find_nearest_location(from: Vec2, world: &WorldData, kind: LocationKind) -> Option<Vec2> {
    find_location_within_radius(from, world, kind, f32::MAX).map(|(_, pos)| pos)
}

/// Find nearest location of a given kind within radius. Returns (index, position).
/// Core function used by both internal Rust code and FFI functions.
pub fn find_location_within_radius(
    from: Vec2,
    world: &WorldData,
    kind: LocationKind,
    radius: f32,
) -> Option<(usize, Vec2)> {
    let mut best: Option<(f32, usize, Vec2)> = None;

    let positions: Vec<Vec2> = match kind {
        LocationKind::Farm => world.farms.iter().map(|f| f.position).collect(),
        LocationKind::GuardPost => world.guard_posts.iter().map(|g| g.position).collect(),
        LocationKind::Town => world.towns.iter().map(|t| t.center).collect(),
        LocationKind::GoldMine => world.gold_mines.iter().map(|m| m.position).collect(),
    };

    for (idx, pos) in positions.iter().enumerate() {
        let dx = pos.x - from.x;
        let dy = pos.y - from.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist <= radius && (best.is_none() || dist < best.unwrap().0) {
            best = Some((dist, idx, *pos));
        }
    }

    best.map(|(_, idx, pos)| (idx, pos))
}

/// Convert Vec2 to integer key for HashMap lookup.
/// Uses rounded coordinates so slight position differences still match.
pub fn pos_to_key(pos: Vec2) -> (i32, i32) {
    (pos.x.round() as i32, pos.y.round() as i32)
}

/// Tracks how many NPCs are working at each building. Key = position, Value = count.
/// Private field — all access goes through methods to prevent double-increment bugs.
#[derive(Resource, Default)]
pub struct BuildingOccupancy {
    occupants: HashMap<(i32, i32), i32>,
}

impl BuildingOccupancy {
    pub fn claim(&mut self, pos: Vec2) {
        *self.occupants.entry(pos_to_key(pos)).or_insert(0) += 1;
    }
    pub fn release(&mut self, pos: Vec2) {
        let key = pos_to_key(pos);
        if let Some(count) = self.occupants.get_mut(&key) {
            *count = count.saturating_sub(1);
        }
    }
    pub fn is_occupied(&self, pos: Vec2) -> bool {
        self.occupants.get(&pos_to_key(pos)).copied().unwrap_or(0) >= 1
    }
    pub fn count(&self, pos: Vec2) -> i32 {
        self.occupants.get(&pos_to_key(pos)).copied().unwrap_or(0)
    }
    pub fn clear(&mut self) { self.occupants.clear(); }
}

/// Any building with a position and town affiliation. Used by generic find functions.
pub trait Worksite {
    fn position(&self) -> Vec2;
    fn town_idx(&self) -> u32;
}

impl Worksite for Farm {
    fn position(&self) -> Vec2 { self.position }
    fn town_idx(&self) -> u32 { self.town_idx }
}

/// Find nearest unoccupied worksite, optionally filtered by town.
pub fn find_nearest_free<W: Worksite>(
    from: Vec2,
    sites: &[W],
    occupancy: &BuildingOccupancy,
    town_idx: Option<u32>,
) -> Option<Vec2> {
    let mut best: Option<(f32, Vec2)> = None;
    for site in sites {
        if let Some(tid) = town_idx {
            if site.town_idx() != tid { continue; }
        }
        if occupancy.is_occupied(site.position()) { continue; }
        let dist = from.distance(site.position());
        if best.is_none() || dist < best.unwrap().0 {
            best = Some((dist, site.position()));
        }
    }
    best.map(|(_, pos)| pos)
}

/// Find nearest worksite within radius, filtered by town. Returns (index, position).
pub fn find_within_radius<W: Worksite>(
    from: Vec2,
    sites: &[W],
    radius: f32,
    town_idx: u32,
) -> Option<(usize, Vec2)> {
    let mut best: Option<(f32, usize, Vec2)> = None;
    for (idx, site) in sites.iter().enumerate() {
        if site.town_idx() != town_idx { continue; }
        let dist = from.distance(site.position());
        if dist <= radius && (best.is_none() || dist < best.unwrap().0) {
            best = Some((dist, idx, site.position()));
        }
    }
    best.map(|(_, idx, pos)| (idx, pos))
}

/// Find worksite index by position.
pub fn find_by_pos<W: Worksite>(sites: &[W], pos: Vec2) -> Option<usize> {
    let key = pos_to_key(pos);
    sites.iter().position(|s| pos_to_key(s.position()) == key)
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
    /// Map biome + cell index to tileset array index (0-10) for TilemapChunk.
    /// Grass alternates 0/1, Forest cycles 2-7, Water=8, Rock=9, Dirt=10.
    pub fn tileset_index(self, cell_index: usize) -> u16 {
        match self {
            Biome::Grass => if cell_index % 2 == 0 { 0 } else { 1 },
            Biome::Forest => 2 + (cell_index % 6) as u16,
            Biome::Water => 8,
            Biome::Rock => 9,
            Biome::Dirt => 10,
        }
    }
}

/// Tile specification: single 16x16 sprite or 2x2 composite of four 16x16 sprites.
#[derive(Clone, Copy)]
pub enum TileSpec {
    Single(u32, u32),
    Quad([(u32, u32); 4]),  // [TL, TR, BL, BR]
    External(usize),        // index into extra images slice
}

/// Atlas (col, row) positions for the 11 terrain tiles used in the TilemapChunk tileset.
pub const TERRAIN_TILES: [TileSpec; 11] = [
    TileSpec::Single(3, 16),  // 0: Grass A
    TileSpec::Single(3, 13),  // 1: Grass B
    TileSpec::Single(13, 9),  // 2: Forest A
    TileSpec::Single(14, 9),  // 3: Forest B
    TileSpec::Single(15, 9),  // 4: Forest C
    TileSpec::Single(16, 9),  // 5: Forest D
    TileSpec::Single(17, 9),  // 6: Forest E
    TileSpec::Single(18, 9),  // 7: Forest F
    TileSpec::Single(3, 1),   // 8: Water
    TileSpec::Quad([(7, 15), (9, 15), (7, 17), (9, 17)]),  // 9: Rock
    TileSpec::Single(8, 10),  // 10: Dirt
];

/// Atlas (col, row) positions for the 8 building tiles used in the building TilemapChunk layer.
pub const BUILDING_TILES: [TileSpec; 9] = [
    TileSpec::Single(50, 9),  // 0: Fountain
    TileSpec::Single(15, 2),  // 1: Bed
    TileSpec::External(2),    // 2: Guard Post (guard_post.png)
    TileSpec::Quad([(2, 15), (4, 15), (2, 17), (4, 17)]), // 3: Farm
    TileSpec::Quad([(46, 10), (47, 10), (46, 11), (47, 11)]), // 4: Camp (center)
    TileSpec::External(0),    // 5: House (house.png)
    TileSpec::External(1),    // 6: Barracks (barracks.png)
    TileSpec::Quad([(48, 10), (49, 10), (48, 11), (49, 11)]), // 7: Tent (raider spawner)
    TileSpec::Single(43, 11), // 8: Gold Mine
];

/// Extract tiles from the world atlas and build a texture_2d_array for TilemapChunk.
/// Each layer is 32x32 pixels. Single sprites are nearest-neighbor 2x upscaled.
/// Quad sprites composite four 16x16 sprites into quadrants.
/// The atlas has 1px margins (17px cells).
pub fn build_tileset(atlas: &Image, tiles: &[TileSpec], extra: &[&Image], images: &mut Assets<Image>) -> Handle<Image> {
    let sprite = SPRITE_SIZE as u32;    // 16
    let out_size = sprite * 2;          // 32
    let cell_size = CELL as u32;        // 17
    let atlas_width = atlas.width();
    let layers = tiles.len() as u32;

    let mut data = vec![0u8; (out_size * out_size * layers * 4) as usize];
    let atlas_data = atlas.data.as_ref().expect("atlas image has no data");

    // Blit a 16x16 sprite from atlas (col, row) into layer at (dx, dy) offset
    let blit = |data: &mut [u8], layer: u32, col: u32, row: u32, dx: u32, dy: u32| {
        let src_x = col * cell_size;
        let src_y = row * cell_size;
        for ty in 0..sprite {
            for tx in 0..sprite {
                let si = ((src_y + ty) * atlas_width + (src_x + tx)) as usize * 4;
                let di = (layer * out_size * out_size + (dy + ty) * out_size + (dx + tx)) as usize * 4;
                data[di..di + 4].copy_from_slice(&atlas_data[si..si + 4]);
            }
        }
    };

    for (layer, spec) in tiles.iter().enumerate() {
        let l = layer as u32;
        match *spec {
            TileSpec::Single(col, row) => {
                // Nearest-neighbor 2x upscale: each src pixel → 2x2 dst pixels
                let src_x = col * cell_size;
                let src_y = row * cell_size;
                for ty in 0..sprite {
                    for tx in 0..sprite {
                        let si = ((src_y + ty) * atlas_width + (src_x + tx)) as usize * 4;
                        for oy in 0..2u32 {
                            for ox in 0..2u32 {
                                let di = (l * out_size * out_size
                                    + (ty * 2 + oy) * out_size
                                    + (tx * 2 + ox)) as usize * 4;
                                data[di..di + 4].copy_from_slice(&atlas_data[si..si + 4]);
                            }
                        }
                    }
                }
            }
            TileSpec::Quad(q) => {
                blit(&mut data, l, q[0].0, q[0].1, 0, 0);              // TL
                blit(&mut data, l, q[1].0, q[1].1, sprite, 0);         // TR
                blit(&mut data, l, q[2].0, q[2].1, 0, sprite);         // BL
                blit(&mut data, l, q[3].0, q[3].1, sprite, sprite);    // BR
            }
            TileSpec::External(idx) => {
                let ext = extra[idx];
                let ext_data = ext.data.as_ref().expect("external image has no data");
                let layer_offset = (l * out_size * out_size * 4) as usize;
                let layer_bytes = (out_size * out_size * 4) as usize;
                data[layer_offset..layer_offset + layer_bytes]
                    .copy_from_slice(&ext_data[..layer_bytes]);
            }
        }
    }

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

    image.reinterpret_stacked_2d_as_array(layers).expect("tileset reinterpret failed");
    images.add(image)
}

/// Building occupying a grid cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Building {
    Fountain { town_idx: u32 },
    Farm { town_idx: u32 },
    Bed { town_idx: u32 },
    GuardPost { town_idx: u32, patrol_order: u32 },
    Camp { town_idx: u32 },
    House { town_idx: u32 },
    Barracks { town_idx: u32 },
    Tent { town_idx: u32 },
    GoldMine,
}

impl Building {
    /// Map building variant to tileset array index (matches BUILDING_TILES order).
    pub fn tileset_index(&self) -> u16 {
        match self {
            Building::Fountain { .. } => 0,
            Building::Bed { .. } => 1,
            Building::GuardPost { .. } => 2,
            Building::Farm { .. } => 3,
            Building::Camp { .. } => 4,
            Building::House { .. } => 5,
            Building::Barracks { .. } => 6,
            Building::Tent { .. } => 7,
            Building::GoldMine => 8,
        }
    }
}

/// A single cell in the world grid.
#[derive(Clone, Debug, Default)]
pub struct WorldCell {
    pub terrain: Biome,
    pub building: Option<Building>,
}

/// World-wide grid covering the entire map. Each cell has terrain + optional building.
#[derive(Resource)]
pub struct WorldGrid {
    pub width: usize,
    pub height: usize,
    pub cell_size: f32,
    pub cells: Vec<WorldCell>,
}

impl Default for WorldGrid {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            cell_size: 32.0,
            cells: Vec::new(),
        }
    }
}

impl WorldGrid {
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
        (col.min(self.width.saturating_sub(1)), row.min(self.height.saturating_sub(1)))
    }

    /// Convert grid coordinates to world position (center of cell).
    pub fn grid_to_world(&self, col: usize, row: usize) -> Vec2 {
        Vec2::new(
            col as f32 * self.cell_size + self.cell_size * 0.5,
            row as f32 * self.cell_size + self.cell_size * 0.5,
        )
    }
}

// ============================================================================
// WORLD GEN CONFIG
// ============================================================================

/// World generation algorithm style.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum WorldGenStyle {
    #[default]
    Classic,
    Continents,
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
    pub camp_distance: f32,
    pub farms_per_town: usize,
    pub farmers_per_town: usize,
    pub guards_per_town: usize,
    pub raiders_per_camp: usize,
    pub ai_towns: usize,
    pub raider_camps: usize,
    pub gold_mines_per_town: usize,
    pub town_names: Vec<String>,
}

impl Default for WorldGenConfig {
    fn default() -> Self {
        Self {
            gen_style: WorldGenStyle::Classic,
            world_width: 8000.0,
            world_height: 8000.0,
            world_margin: 400.0,
            num_towns: 2,
            min_town_distance: 1200.0,
            grid_spacing: 34.0,
            camp_distance: 3500.0,
            farms_per_town: 2,
            farmers_per_town: 2,
            guards_per_town: 2,
            raiders_per_camp: 1,
            ai_towns: 1,
            raider_camps: 1,
            gold_mines_per_town: 2,
            town_names: vec![
                "Miami".into(), "Orlando".into(), "Tampa".into(), "Jacksonville".into(),
                "Tallahassee".into(), "Gainesville".into(), "Pensacola".into(), "Sarasota".into(),
                "Naples".into(), "Daytona".into(), "Lakeland".into(), "Ocala".into(),
                "Boca Raton".into(), "Key West".into(), "Fort Myers".into(),
            ],
        }
    }
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
    farm_states: &mut FarmStates,
    mine_states: &mut crate::resources::MineStates,
    town_grids: &mut TownGrids,
) {
    use rand::Rng;
    let mut rng = rand::rng();

    // Step 1: Initialize grid
    let w = (config.world_width / grid.cell_size) as usize;
    let h = (config.world_height / grid.cell_size) as usize;
    grid.width = w;
    grid.height = h;
    grid.cells = vec![WorldCell::default(); w * h];

    // Shuffle town names
    let mut names = config.town_names.clone();
    for i in (1..names.len()).rev() {
        let j = rng.random_range(0..=i);
        names.swap(i, j);
    }
    let mut name_idx = 0;

    let is_continents = config.gen_style == WorldGenStyle::Continents;

    // Continents: generate terrain first so we can reject Water positions
    if is_continents {
        generate_terrain_continents(grid);
    }

    // All settlement positions for min_distance checks
    let mut all_positions: Vec<Vec2> = Vec::new();
    // Continents needs more attempts since many positions land in ocean
    let max_attempts = if is_continents { 5000 } else { 2000 };
    let mut next_faction = 1;

    // Step 2: Place player town centers (faction 0)
    let mut player_positions: Vec<Vec2> = Vec::new();
    let mut attempts = 0;
    while player_positions.len() < config.num_towns && attempts < max_attempts {
        attempts += 1;
        let x = rng.random_range(config.world_margin..config.world_width - config.world_margin);
        let y = rng.random_range(config.world_margin..config.world_height - config.world_margin);
        let pos = Vec2::new(x, y);
        // Continents: reject Water cells
        if is_continents {
            let (gc, gr) = grid.world_to_grid(pos);
            if grid.cell(gc, gr).is_some_and(|c| c.terrain == Biome::Water) { continue; }
        }
        if all_positions.iter().all(|e| pos.distance(*e) >= config.min_town_distance) {
            player_positions.push(pos);
            all_positions.push(pos);
        }
    }

    if player_positions.len() < config.num_towns {
        warn!("generate_world: only placed {}/{} player towns", player_positions.len(), config.num_towns);
    }

    // Register player towns
    for &center in &player_positions {
        let name = names.get(name_idx).cloned().unwrap_or_else(|| format!("Town {}", name_idx));
        name_idx += 1;
        world_data.towns.push(Town { name, center, faction: 0, sprite_type: 0 });
        let town_data_idx = world_data.towns.len() - 1;
        let town_idx = town_data_idx as u32;
        town_grids.grids.push(TownGrid::new_base(town_data_idx));
        let gi = town_grids.grids.len() - 1;
        place_town_buildings(grid, world_data, farm_states, center, town_idx, config, &mut town_grids.grids[gi]);
    }

    // Step 3: Place AI town centers (Builder AI, each gets unique faction)
    let mut ai_town_positions: Vec<Vec2> = Vec::new();
    attempts = 0;
    while ai_town_positions.len() < config.ai_towns && attempts < max_attempts {
        attempts += 1;
        let x = rng.random_range(config.world_margin..config.world_width - config.world_margin);
        let y = rng.random_range(config.world_margin..config.world_height - config.world_margin);
        let pos = Vec2::new(x, y);
        if is_continents {
            let (gc, gr) = grid.world_to_grid(pos);
            if grid.cell(gc, gr).is_some_and(|c| c.terrain == Biome::Water) { continue; }
        }
        if all_positions.iter().all(|e| pos.distance(*e) >= config.min_town_distance) {
            ai_town_positions.push(pos);
            all_positions.push(pos);
        }
    }

    for &center in &ai_town_positions {
        let name = names.get(name_idx).cloned().unwrap_or_else(|| format!("AI Town {}", name_idx));
        name_idx += 1;
        let faction = next_faction;
        next_faction += 1;
        world_data.towns.push(Town { name, center, faction, sprite_type: 0 });
        let town_data_idx = world_data.towns.len() - 1;
        let town_idx = town_data_idx as u32;
        town_grids.grids.push(TownGrid::new_base(town_data_idx));
        let gi = town_grids.grids.len() - 1;
        place_town_buildings(grid, world_data, farm_states, center, town_idx, config, &mut town_grids.grids[gi]);
    }

    // Step 4: Place raider camp centers (Raider AI, each gets unique faction)
    let mut camp_positions: Vec<Vec2> = Vec::new();
    attempts = 0;
    while camp_positions.len() < config.raider_camps && attempts < max_attempts {
        attempts += 1;
        let x = rng.random_range(config.world_margin..config.world_width - config.world_margin);
        let y = rng.random_range(config.world_margin..config.world_height - config.world_margin);
        let pos = Vec2::new(x, y);
        if is_continents {
            let (gc, gr) = grid.world_to_grid(pos);
            if grid.cell(gc, gr).is_some_and(|c| c.terrain == Biome::Water) { continue; }
        }
        if all_positions.iter().all(|e| pos.distance(*e) >= config.min_town_distance) {
            camp_positions.push(pos);
            all_positions.push(pos);
        }
    }

    for &center in &camp_positions {
        let faction = next_faction;
        next_faction += 1;
        world_data.towns.push(Town { name: "Raider Camp".into(), center, faction, sprite_type: 1 });
        let town_data_idx = world_data.towns.len() - 1;
        let town_idx = town_data_idx as u32;
        town_grids.grids.push(TownGrid::new_base(town_data_idx));
        let gi = town_grids.grids.len() - 1;
        place_camp_buildings(grid, world_data, center, town_idx, config, &mut town_grids.grids[gi]);
    }

    // Step 5: Generate terrain
    if is_continents {
        // Terrain already generated; stamp dirt clearings around settlements
        stamp_dirt(grid, &all_positions);
    } else {
        generate_terrain(grid, &all_positions, &[]);
    }

    // Step 6: Place gold mines in wilderness between settlements
    let total_mines = config.gold_mines_per_town * all_positions.len();
    let mut mine_positions: Vec<Vec2> = Vec::new();
    let mut mine_attempts = 0;
    while mine_positions.len() < total_mines && mine_attempts < max_attempts {
        mine_attempts += 1;
        let x = rng.random_range(config.world_margin..config.world_width - config.world_margin);
        let y = rng.random_range(config.world_margin..config.world_height - config.world_margin);
        let pos = Vec2::new(x, y);
        // Not on water
        if is_continents {
            let (gc, gr) = grid.world_to_grid(pos);
            if grid.cell(gc, gr).is_some_and(|c| c.terrain == Biome::Water) { continue; }
        }
        // Min distance from settlements
        if all_positions.iter().any(|s| pos.distance(*s) < crate::constants::MINE_MIN_SETTLEMENT_DIST) {
            continue;
        }
        // Min distance from other mines
        if mine_positions.iter().any(|m| pos.distance(*m) < crate::constants::MINE_MIN_SPACING) {
            continue;
        }
        // Snap to grid and place
        let (gc, gr) = grid.world_to_grid(pos);
        if let Some(cell) = grid.cell(gc, gr) {
            if cell.building.is_some() { continue; }
        }
        let snapped = grid.grid_to_world(gc, gr);
        if let Some(cell) = grid.cell_mut(gc, gr) {
            cell.building = Some(Building::GoldMine);
        }
        world_data.gold_mines.push(GoldMine { position: snapped });
        mine_states.push_mine(snapped, crate::constants::MINE_MAX_GOLD);
        mine_positions.push(snapped);
    }

    info!("generate_world: {} player towns, {} AI towns, {} raider camps, {} gold mines, grid {}x{} ({})",
        player_positions.len(), ai_town_positions.len(), camp_positions.len(), mine_positions.len(), w, h,
        if is_continents { "continents" } else { "classic" });
}

/// Place buildings for one town on the grid: fountain, farms, houses, barracks, guard posts.
/// Uses grid-relative offsets from center, snapped to grid cells.
fn place_town_buildings(
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    farm_states: &mut FarmStates,
    center: Vec2,
    town_idx: u32,
    config: &WorldGenConfig,
    town_grid: &mut TownGrid,
) {
    // Track which town-grid slots are occupied by buildings
    let mut occupied = HashSet::new();

    // Helper: place building at town grid (row, col), return snapped world position
    let mut place = |row: i32, col: i32, building: Building, occ: &mut HashSet<(i32, i32)>, tg: &mut TownGrid| -> Vec2 {
        let world_pos = town_grid_to_world(center, row, col);
        let (gc, gr) = grid.world_to_grid(world_pos);
        let snapped_pos = grid.grid_to_world(gc, gr);
        if let Some(cell) = grid.cell_mut(gc, gr) {
            if cell.building.is_none() {
                cell.building = Some(building);
            }
        }
        occ.insert((row, col));
        tg.unlocked.insert((row, col));
        snapped_pos
    };

    // Fountain at (0, 0) = town center
    place(0, 0, Building::Fountain { town_idx }, &mut occupied, town_grid);

    // All buildings spiral outward from center: farms first (closest), then houses, then barracks
    let needed = config.farms_per_town + config.farmers_per_town + config.guards_per_town;
    let slots = spiral_slots(&occupied, needed);
    let mut slot_iter = slots.into_iter();

    for _ in 0..config.farms_per_town {
        let Some((row, col)) = slot_iter.next() else { break };
        let pos = place(row, col, Building::Farm { town_idx }, &mut occupied, town_grid);
        world_data.farms.push(Farm { position: pos, town_idx });
        farm_states.push_farm(pos);
    }

    for _ in 0..config.farmers_per_town {
        let Some((row, col)) = slot_iter.next() else { break };
        let pos = place(row, col, Building::House { town_idx }, &mut occupied, town_grid);
        world_data.houses.push(House { position: pos, town_idx });
    }

    for _ in 0..config.guards_per_town {
        let Some((row, col)) = slot_iter.next() else { break };
        let pos = place(row, col, Building::Barracks { town_idx }, &mut occupied, town_grid);
        world_data.barracks.push(Barracks { position: pos, town_idx });
    }

    // 4 guard posts: at the outer corners of all placed buildings (clockwise patrol: TL → TR → BR → BL)
    let (min_row, max_row, min_col, max_col) = occupied.iter().fold(
        (i32::MAX, i32::MIN, i32::MAX, i32::MIN),
        |(rmin, rmax, cmin, cmax), &(r, c)| (rmin.min(r), rmax.max(r), cmin.min(c), cmax.max(c)),
    );
    // Bevy 2D: Y-up, so max_row = top on screen, min_row = bottom
    let corners = [
        (max_row + 1, min_col - 1), // TL (top-left)
        (max_row + 1, max_col + 1), // TR (top-right)
        (min_row - 1, max_col + 1), // BR (bottom-right)
        (min_row - 1, min_col - 1), // BL (bottom-left)
    ];
    for (order, (row, col)) in corners.into_iter().enumerate() {
        let post_pos = place(row, col, Building::GuardPost { town_idx, patrol_order: order as u32 }, &mut occupied, town_grid);
        world_data.guard_posts.push(GuardPost {
            position: post_pos,
            town_idx,
            patrol_order: order as u32,
        });
    }
}

/// Place buildings for a raider camp: camp center + tents in spiral.
fn place_camp_buildings(
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    center: Vec2,
    town_idx: u32,
    config: &WorldGenConfig,
    town_grid: &mut TownGrid,
) {
    let mut occupied = HashSet::new();

    // Place camp center at (0,0)
    let world_pos = town_grid_to_world(center, 0, 0);
    let (gc, gr) = grid.world_to_grid(world_pos);
    if let Some(cell) = grid.cell_mut(gc, gr) {
        cell.building = Some(Building::Camp { town_idx });
    }
    occupied.insert((0, 0));
    town_grid.unlocked.insert((0, 0));

    // Place tents in spiral around camp center
    let slots = spiral_slots(&occupied, config.raiders_per_camp);
    for (row, col) in slots {
        let world_pos = town_grid_to_world(center, row, col);
        let (gc, gr) = grid.world_to_grid(world_pos);
        let snapped = grid.grid_to_world(gc, gr);
        if let Some(cell) = grid.cell_mut(gc, gr) {
            if cell.building.is_none() {
                cell.building = Some(Building::Tent { town_idx });
            }
        }
        occupied.insert((row, col));
        town_grid.unlocked.insert((row, col));
        world_data.tents.push(Tent { position: snapped, town_idx });
    }
}

/// Generate `count` grid positions in a spiral pattern outward from (0,0), skipping occupied cells.
fn spiral_slots(occupied: &HashSet<(i32, i32)>, count: usize) -> Vec<(i32, i32)> {
    let mut result = Vec::with_capacity(count);
    // Walk rings outward: ring 1 = distance 1 from center, ring 2 = distance 2, etc.
    for ring in 1..=MAX_GRID_EXTENT {
        if result.len() >= count { break; }
        // Top edge: row = -ring, col = -ring..ring
        for col in -ring..=ring {
            if result.len() >= count { break; }
            let pos = (-ring, col);
            if !occupied.contains(&pos) { result.push(pos); }
        }
        // Right edge: row = -ring+1..ring, col = ring
        for row in (-ring + 1)..=ring {
            if result.len() >= count { break; }
            let pos = (row, ring);
            if !occupied.contains(&pos) { result.push(pos); }
        }
        // Bottom edge: row = ring, col = ring-1..-ring
        for col in (-ring..ring).rev() {
            if result.len() >= count { break; }
            let pos = (ring, col);
            if !occupied.contains(&pos) { result.push(pos); }
        }
        // Left edge: row = ring..-ring+1, col = -ring
        for row in ((-ring + 1)..ring).rev() {
            if result.len() >= count { break; }
            let pos = (row, -ring);
            if !occupied.contains(&pos) { result.push(pos); }
        }
    }
    result
}

/// Fill grid terrain using simplex noise, with Dirt override near towns and camps.
fn generate_terrain(
    grid: &mut WorldGrid,
    town_positions: &[Vec2],
    camp_positions: &[Vec2],
) {
    use noise::{NoiseFn, Simplex};

    let noise = Simplex::new(rand::random::<u32>());
    let frequency = 0.003;
    let town_clear_radius = 6.0 * grid.cell_size; // ~192px
    let camp_clear_radius = 5.0 * grid.cell_size;  // ~160px

    for row in 0..grid.height {
        for col in 0..grid.width {
            let world_pos = grid.grid_to_world(col, row);

            // Check proximity to towns → Dirt
            let near_town = town_positions.iter().any(|tc| world_pos.distance(*tc) < town_clear_radius);
            let near_camp = camp_positions.iter().any(|cp| world_pos.distance(*cp) < camp_clear_radius);

            let biome = if near_town || near_camp {
                Biome::Dirt
            } else {
                let n = noise.get([world_pos.x as f64 * frequency as f64, world_pos.y as f64 * frequency as f64]);
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

            let idx = row * grid.width + col;
            grid.cells[idx].terrain = biome;
        }
    }
}

/// Overwrite terrain near settlements with Dirt (clearing for buildings).
fn stamp_dirt(grid: &mut WorldGrid, positions: &[Vec2]) {
    let clear_radius = 6.0 * grid.cell_size;
    for row in 0..grid.height {
        for col in 0..grid.width {
            let world_pos = grid.grid_to_world(col, row);
            if positions.iter().any(|p| world_pos.distance(*p) < clear_radius) {
                grid.cells[row * grid.width + col].terrain = Biome::Dirt;
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
            let e = (1.0 * elevation_noise.get([wx * 0.0008, wy * 0.0008])
                + 0.5 * elevation_noise.get([wx * 0.0016, wy * 0.0016])
                + 0.25 * elevation_noise.get([wx * 0.0032, wy * 0.0032]))
                / 1.75;

            // Square bump edge falloff (Red Blob Games)
            let nx = (wx / world_w - 0.5) * 2.0;
            let ny = (wy / world_h - 0.5) * 2.0;
            let d = 1.0 - (1.0 - nx * nx) * (1.0 - ny * ny);

            // Push edges to ocean, redistribute elevation
            let e = ((e + 1.0) * 0.5 * (1.0 - d)).powf(1.5); // normalize to ~[0,1] then apply falloff + power

            // Independent moisture noise
            let m = (moisture_noise.get([wx * 0.003, wy * 0.003]) + 1.0) * 0.5; // [0, 1]

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

            grid.cells[row * grid.width + col].terrain = biome;
        }
    }
}
