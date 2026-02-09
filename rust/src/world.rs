//! World Data - Towns, farms, beds, guard posts, sprite definitions
//! World Grid - 2D cell grid covering entire world (terrain + buildings)
//! World Generation - Procedural town placement and building layout

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

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

// ============================================================================
// WORLD RESOURCES
// ============================================================================

/// Contains all world layout data (immutable after init).
#[derive(Resource, Default)]
pub struct WorldData {
    pub towns: Vec<Town>,
    pub farms: Vec<Farm>,
    pub beds: Vec<Bed>,
    pub guard_posts: Vec<GuardPost>,
}

/// Location types for find_nearest_location.
#[derive(Clone, Copy, Debug)]
pub enum LocationKind {
    Farm,
    Bed,
    GuardPost,
    Town,
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
        LocationKind::Bed => world.beds.iter().map(|b| b.position).collect(),
        LocationKind::GuardPost => world.guard_posts.iter().map(|g| g.position).collect(),
        LocationKind::Town => world.towns.iter().map(|t| t.center).collect(),
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

/// Tracks which NPCs occupy each bed. Key = bed position, Value = NPC index (-1 = free).
#[derive(Resource, Default)]
pub struct BedOccupancy {
    pub occupants: HashMap<(i32, i32), i32>,
}

/// Tracks how many NPCs are working at each farm. Key = farm position, Value = count.
#[derive(Resource, Default)]
pub struct FarmOccupancy {
    pub occupants: HashMap<(i32, i32), i32>,
}

/// Find farm index by position (for FarmStates lookup which is still Vec-indexed).
pub fn find_farm_index_by_pos(farms: &[Farm], pos: Vec2) -> Option<usize> {
    let key = pos_to_key(pos);
    farms.iter().position(|f| pos_to_key(f.position) == key)
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

/// Atlas (col, row) positions for the 11 terrain tiles used in the TilemapChunk tileset.
pub const TERRAIN_TILES: [(u32, u32); 11] = [
    (0, 14),  // 0: Grass A
    (1, 14),  // 1: Grass B
    (13, 9),  // 2: Forest A
    (14, 9),  // 3: Forest B
    (15, 9),  // 4: Forest C
    (16, 9),  // 5: Forest D
    (17, 9),  // 6: Forest E
    (18, 9),  // 7: Forest F
    (3, 1),   // 8: Water
    (7, 13),  // 9: Rock
    (8, 10),  // 10: Dirt
];

/// Atlas (col, row) positions for the 5 building tiles used in the building TilemapChunk layer.
pub const BUILDING_TILES: [(u32, u32); 5] = [
    (50, 9),  // 0: Fountain
    (15, 2),  // 1: Bed
    (20, 20), // 2: Guard Post
    (2, 15),  // 3: Farm
    (48, 10), // 4: Camp/Tent
];

/// Extract tiles from the world atlas and build a texture_2d_array for TilemapChunk.
/// Each tile is 16x16 pixels. The atlas has 1px margins (17px cells).
/// Called with TERRAIN_TILES (11 tiles) or BUILDING_TILES (5 tiles).
pub fn build_tileset(atlas: &Image, tiles: &[(u32, u32)], images: &mut Assets<Image>) -> Handle<Image> {
    let tile_size = SPRITE_SIZE as u32; // 16
    let cell_size = CELL as u32;        // 17
    let atlas_width = atlas.width();
    let layers = tiles.len() as u32;

    // Stack tiles vertically: 16 wide × (16 * N) tall
    let mut data = vec![0u8; (tile_size * tile_size * layers * 4) as usize];
    let atlas_data = atlas.data.as_ref().expect("atlas image has no data");

    for (layer, &(col, row)) in tiles.iter().enumerate() {
        let src_x = col * cell_size;
        let src_y = row * cell_size;

        for ty in 0..tile_size {
            for tx in 0..tile_size {
                let src_idx = ((src_y + ty) * atlas_width + (src_x + tx)) as usize * 4;
                let dst_idx = (layer as u32 * tile_size * tile_size + ty * tile_size + tx) as usize * 4;
                data[dst_idx..dst_idx + 4].copy_from_slice(&atlas_data[src_idx..src_idx + 4]);
            }
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: tile_size,
            height: tile_size * layers,
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

/// Configuration for procedural world generation.
#[derive(Resource)]
pub struct WorldGenConfig {
    pub world_width: f32,
    pub world_height: f32,
    pub world_margin: f32,
    pub num_towns: usize,
    pub min_town_distance: f32,
    pub grid_spacing: f32,
    pub camp_distance: f32,
    pub farmers_per_town: usize,
    pub guards_per_town: usize,
    pub raiders_per_camp: usize,
    pub town_names: Vec<String>,
}

impl Default for WorldGenConfig {
    fn default() -> Self {
        Self {
            world_width: 8000.0,
            world_height: 8000.0,
            world_margin: 400.0,
            num_towns: 2,
            min_town_distance: 1200.0,
            grid_spacing: 34.0,
            camp_distance: 1100.0,
            farmers_per_town: 5,
            guards_per_town: 2,
            raiders_per_camp: 5,
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
) {
    use rand::Rng;
    let mut rng = rand::rng();

    // Step 1: Initialize grid
    let w = (config.world_width / grid.cell_size) as usize;
    let h = (config.world_height / grid.cell_size) as usize;
    grid.width = w;
    grid.height = h;
    grid.cells = vec![WorldCell::default(); w * h];

    // Step 2: Place town centers with min distance constraint
    let mut town_positions: Vec<Vec2> = Vec::new();
    let max_attempts = 1000;
    let mut attempts = 0;

    while town_positions.len() < config.num_towns && attempts < max_attempts {
        attempts += 1;
        let x = rng.random_range(config.world_margin..config.world_width - config.world_margin);
        let y = rng.random_range(config.world_margin..config.world_height - config.world_margin);
        let pos = Vec2::new(x, y);

        let valid = town_positions.iter().all(|existing| {
            pos.distance(*existing) >= config.min_town_distance
        });

        if valid {
            town_positions.push(pos);
        }
    }

    if town_positions.len() < config.num_towns {
        warn!("generate_world: only placed {}/{} towns after {} attempts",
            town_positions.len(), config.num_towns, max_attempts);
    }

    // Shuffle town names
    let mut names = config.town_names.clone();
    // Simple Fisher-Yates shuffle
    for i in (1..names.len()).rev() {
        let j = rng.random_range(0..=i);
        names.swap(i, j);
    }

    // Step 3: Find camp positions (furthest from all towns)
    let mut camp_positions: Vec<Vec2> = Vec::new();
    for town_center in &town_positions {
        let camp = find_camp_position(*town_center, &town_positions, config);
        camp_positions.push(camp);
    }

    // Step 4: Generate terrain via noise
    generate_terrain(grid, &town_positions, &camp_positions);

    // Step 5: Place buildings for each town
    let actual_towns = town_positions.len();
    for town_idx in 0..actual_towns {
        let center = town_positions[town_idx];
        let name = names.get(town_idx).cloned().unwrap_or_else(|| format!("Town {}", town_idx));
        let camp_pos = camp_positions[town_idx];

        // Add villager town to WorldData
        world_data.towns.push(Town {
            name: name.clone(),
            center,
            faction: 0,
            sprite_type: 0, // fountain
        });

        // Place buildings on grid around town center
        place_town_buildings(grid, world_data, farm_states, center, town_idx as u32, config);

        // Add raider camp town to WorldData
        // Raider towns use indices num_towns..2*num_towns
        world_data.towns.push(Town {
            name: "Raider Camp".into(),
            center: camp_pos,
            faction: (town_idx + 1) as i32,
            sprite_type: 1, // tent
        });

        // Place camp building on grid
        let (camp_col, camp_row) = grid.world_to_grid(camp_pos);
        if let Some(cell) = grid.cell_mut(camp_col, camp_row) {
            cell.building = Some(Building::Camp { town_idx: town_idx as u32 });
        }
    }

    info!("generate_world: {} villager towns, {} raider camps, grid {}x{}",
        actual_towns, camp_positions.len(), w, h);
}

/// Place buildings for one town on the grid: fountain, 2 farms, 4 beds, 4 guard posts.
/// Uses grid-relative offsets from center, snapped to grid cells.
fn place_town_buildings(
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    farm_states: &mut FarmStates,
    center: Vec2,
    town_idx: u32,
    config: &WorldGenConfig,
) {
    let s = config.grid_spacing;
    let cs = grid.cell_size;

    // Fountain at center
    let (fc, fr) = grid.world_to_grid(center);
    let fountain_pos = grid.grid_to_world(fc, fr);
    if let Some(cell) = grid.cell_mut(fc, fr) {
        cell.building = Some(Building::Fountain { town_idx });
    }

    // Helper: place building at offset from center, return world position
    let mut place = |dx: f32, dy: f32, building: Building| -> Vec2 {
        let world_pos = Vec2::new(center.x + dx * s, center.y + dy * s);
        let (col, row) = grid.world_to_grid(world_pos);
        let snapped_pos = grid.grid_to_world(col, row);
        if let Some(cell) = grid.cell_mut(col, row) {
            if cell.building.is_none() {
                cell.building = Some(building);
            }
        }
        snapped_pos
    };

    // 2 farms: left and right of center
    let farm0_pos = place(0.0, -1.0, Building::Farm { town_idx });
    let farm1_pos = place(0.0, 1.0, Building::Farm { town_idx });
    world_data.farms.push(Farm { position: farm0_pos, town_idx });
    world_data.farms.push(Farm { position: farm1_pos, town_idx });
    farm_states.push_farm();
    farm_states.push_farm();

    // 4 beds: inner corners
    let bed_offsets = [(-1.0, -1.0), (-1.0, 2.0), (2.0, -1.0), (2.0, 2.0)];
    for &(dx, dy) in &bed_offsets {
        let bed_pos = place(dx, dy, Building::Bed { town_idx });
        world_data.beds.push(Bed { position: bed_pos, town_idx });
    }

    // 4 guard posts: outer corners (clockwise patrol)
    let post_offsets = [(-2.0, -2.0), (-2.0, 3.0), (2.0 + 1.0, 3.0), (2.0 + 1.0, -2.0)];
    for (order, &(dx, dy)) in post_offsets.iter().enumerate() {
        let post_pos = place(dx, dy, Building::GuardPost { town_idx, patrol_order: order as u32 });
        world_data.guard_posts.push(GuardPost {
            position: post_pos,
            town_idx,
            patrol_order: order as u32,
        });
    }

    let _ = (fountain_pos, cs); // suppress unused warnings
}

/// Find camp position for a town: try 16 directions, pick furthest from all towns.
fn find_camp_position(town_center: Vec2, all_towns: &[Vec2], config: &WorldGenConfig) -> Vec2 {
    use rand::Rng;
    let mut rng = rand::rng();
    let mut best_pos = town_center;
    let mut best_score = f32::NEG_INFINITY;

    for i in 0..16 {
        let angle = i as f32 * std::f32::consts::TAU / 16.0 + rng.random_range(-0.1f32..0.1f32);
        let dir = Vec2::new(angle.cos(), angle.sin());
        let mut pos = town_center + dir * config.camp_distance;

        // Clamp to world bounds
        pos.x = pos.x.clamp(config.world_margin, config.world_width - config.world_margin);
        pos.y = pos.y.clamp(config.world_margin, config.world_height - config.world_margin);

        // Score = minimum distance to any town (higher = better)
        let min_dist = all_towns.iter()
            .map(|tc| pos.distance(*tc))
            .fold(f32::MAX, f32::min);

        if min_dist > best_score {
            best_score = min_dist;
            best_pos = pos;
        }
    }

    best_pos
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
