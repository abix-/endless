//! World Data - Towns, farms, beds, guard posts, sprite definitions
//! World Grid - 2D cell grid covering entire world (terrain + buildings)
//! World Generation - Procedural town placement and building layout

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::{HashMap, HashSet};

use crate::constants::{TOWN_GRID_SPACING, BASE_GRID_MIN, BASE_GRID_MAX, MAX_GRID_EXTENT};
use crate::resources::{FarmStates, FoodStorage, SpawnerState, SpawnerEntry, BuildingHpState, CombatLog, CombatEventKind, GameTime, DirtyFlags};

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
    /// NPC slot index for GPU spatial grid targeting (None = not yet allocated)
    pub npc_slot: Option<usize>,
}

/// A farmer home that supports 1 farmer (building spawner).
#[derive(Clone, Debug)]
pub struct FarmerHome {
    pub position: Vec2,
    pub town_idx: u32,
}

/// An archer home that supports 1 archer (building spawner).
#[derive(Clone, Debug)]
pub struct ArcherHome {
    pub position: Vec2,
    pub town_idx: u32,
}

/// A tent that supports 1 raider (building spawner).
#[derive(Clone, Debug)]
pub struct Tent {
    pub position: Vec2,
    pub town_idx: u32,
}

/// A miner home that supports 1 miner (building spawner).
#[derive(Clone, Debug)]
pub struct MinerHome {
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
    pub farmer_homes: Vec<FarmerHome>,
    pub archer_homes: Vec<ArcherHome>,
    pub tents: Vec<Tent>,
    pub miner_homes: Vec<MinerHome>,
    pub gold_mines: Vec<GoldMine>,
}

// ============================================================================
// TOWN BUILDING GRID
// ============================================================================

/// Per-town building area configuration.
/// Grid uses (row, col) relative to town center with TOWN_GRID_SPACING.
/// Base grid: (-3,-3) to (3,3) = 7x7. `area_level` expands bounds by 1 per level.
pub struct TownGrid {
    pub town_data_idx: usize,
    pub area_level: i32,
}

impl TownGrid {
    /// Create with base 7x7 build area for the given town data index.
    pub fn new_base(town_data_idx: usize) -> Self {
        Self { town_data_idx, area_level: 0 }
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

/// Buildable slot bounds for a town grid (inclusive): min_row, max_row, min_col, max_col.
pub fn build_bounds(grid: &TownGrid) -> (i32, i32, i32, i32) {
    let min = (BASE_GRID_MIN - grid.area_level).max(-MAX_GRID_EXTENT);
    let max = (BASE_GRID_MAX + grid.area_level).min(MAX_GRID_EXTENT + 1);
    (min, max, min, max)
}

/// True if (row, col) is currently inside this town's buildable area.
pub fn is_slot_buildable(grid: &TownGrid, row: i32, col: i32) -> bool {
    let (min_row, max_row, min_col, max_col) = build_bounds(grid);
    row >= min_row && row <= max_row && col >= min_col && col <= max_col
}

/// Find which town (villager or camp) has a buildable slot matching the given grid coords.
/// Returns the grid index and town data index.
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

        if is_slot_buildable(town_grid, row, col) {
            return Some(TownSlotInfo {
                grid_idx,
                town_data_idx,
                row, col,
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
                npc_slot
            });
        }
        Building::FarmerHome { town_idx } => {
            world_data.farmer_homes.push(FarmerHome { position: snapped_pos, town_idx });
        }
        Building::ArcherHome { town_idx } => {
            world_data.archer_homes.push(ArcherHome { position: snapped_pos, town_idx });
        }
        Building::Tent { town_idx } => {
            world_data.tents.push(Tent { position: snapped_pos, town_idx });
        }
        Building::MinerHome { town_idx } => {
            world_data.miner_homes.push(MinerHome { position: snapped_pos, town_idx });
        }
        _ => {} // Fountain, Camp, GoldMine not player-placeable
    }

    Ok(())
}

/// Resolve SpawnNpcMsg fields from a spawner entry's building_kind.
/// Single source of truth for the building_kind → NPC mapping used by startup and respawn.
pub fn resolve_spawner_npc(
    entry: &SpawnerEntry,
    towns: &[Town],
    bgrid: &BuildingSpatialGrid,
    occupancy: &BuildingOccupancy,
) -> (i32, i32, f32, f32, i32, i32, &'static str, &'static str) {
    let town_faction = towns.get(entry.town_idx as usize)
        .map(|t| t.faction).unwrap_or(0);

    match entry.building_kind {
        0 => {
            // FarmerHome -> Farmer: find nearest free farm in own town
            let farm = find_nearest_free(
                entry.position, bgrid, BuildingKind::Farm, occupancy, Some(entry.town_idx as u32),
            ).unwrap_or(entry.position);
            (0, town_faction, farm.x, farm.y, -1, 0, "Farmer", "Farmer Home")
        }
        1 => {
            // ArcherHome -> Archer: find nearest guard post
            let post_idx = find_location_within_radius(
                entry.position, bgrid, LocationKind::GuardPost, f32::MAX,
            ).map(|(idx, _)| idx as i32).unwrap_or(-1);
            (1, town_faction, -1.0, -1.0, post_idx, 1, "Archer", "Archer Home")
        }
        2 => {
            // Tent -> Raider
            let camp_faction = towns.get(entry.town_idx as usize)
                .map(|t| t.faction).unwrap_or(1);
            (2, camp_faction, -1.0, -1.0, -1, 0, "Raider", "Tent")
        }
        3 => {
            // MinerHome -> Miner: find nearest gold mine
            let mine = find_nearest_free(
                entry.position, bgrid, BuildingKind::GoldMine, occupancy, None,
            ).unwrap_or(entry.position);
            (4, town_faction, mine.x, mine.y, -1, 0, "Miner", "Miner Home")
        }
        _ => {
            // Unknown building kind — fallback to Raider
            let camp_faction = towns.get(entry.town_idx as usize)
                .map(|t| t.faction).unwrap_or(1);
            (2, camp_faction, -1.0, -1.0, -1, 0, "Raider", "Unknown")
        }
    }
}

/// Push a SpawnerEntry for a spawner building. No-op for non-spawner buildings.
/// Single construction site for all SpawnerEntry structs.
pub fn register_spawner(
    spawner_state: &mut SpawnerState,
    building: Building,
    town_idx: i32,
    position: Vec2,
    respawn_timer: f32,
) {
    if let Some(sk) = building.spawner_kind() {
        spawner_state.0.push(SpawnerEntry {
            building_kind: sk,
            town_idx,
            position,
            npc_slot: -1,
            respawn_timer,
        });
    }
}

/// Place a building, deduct food, and push a spawner entry if applicable.
/// Shared by player build menu and AI. Spawner kind is derived from
/// `Building::spawner_kind()` — callers never pass magic numbers.
pub fn build_and_pay(
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    farm_states: &mut FarmStates,
    food_storage: &mut FoodStorage,
    spawner_state: &mut SpawnerState,
    building_hp: &mut BuildingHpState,
    building: Building,
    town_data_idx: usize,
    row: i32, col: i32,
    town_center: Vec2,
    cost: i32,
) -> bool {
    if place_building(grid, world_data, farm_states, building, row, col, town_center).is_err() {
        return false;
    }
    if let Some(f) = food_storage.food.get_mut(town_data_idx) { *f -= cost; }
    let pos = town_grid_to_world(town_center, row, col);
    let (gc, gr) = grid.world_to_grid(pos);
    let snapped = grid.grid_to_world(gc, gr);
    register_spawner(spawner_state, building, town_data_idx as i32, snapped, 0.0);
    building_hp.push_for(&building);
    true
}

/// Expand one town's buildable area by one ring and convert new ring terrain to Dirt.
pub fn expand_town_build_area(
    grid: &mut WorldGrid,
    towns: &[Town],
    town_grids: &mut TownGrids,
    grid_idx: usize,
) -> Result<(), &'static str> {
    let Some(town_grid) = town_grids.grids.get_mut(grid_idx) else {
        return Err("invalid town grid index");
    };
    let town_data_idx = town_grid.town_data_idx;
    let Some(town) = towns.get(town_data_idx) else {
        return Err("invalid town data index");
    };

    let (old_min_row, old_max_row, old_min_col, old_max_col) = build_bounds(town_grid);
    town_grid.area_level += 1;
    let (new_min_row, new_max_row, new_min_col, new_max_col) = build_bounds(town_grid);

    for row in new_min_row..=new_max_row {
        for col in new_min_col..=new_max_col {
            let is_old = row >= old_min_row && row <= old_max_row && col >= old_min_col && col <= old_max_col;
            if is_old {
                continue;
            }
            let slot_pos = town_grid_to_world(town.center, row, col);
            let (gc, gr) = grid.world_to_grid(slot_pos);
            if let Some(cell) = grid.cell_mut(gc, gr) {
                cell.terrain = Biome::Dirt;
            }
        }
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
        Building::FarmerHome { .. } => {
            if let Some(h) = world_data.farmer_homes.iter_mut().find(|h| {
                (h.position - snapped_pos).length() < 1.0
            }) {
                h.position = tombstone;
            }
        }
        Building::ArcherHome { .. } => {
            if let Some(a) = world_data.archer_homes.iter_mut().find(|a| {
                (a.position - snapped_pos).length() < 1.0
            }) {
                a.position = tombstone;
            }
        }
        Building::Tent { .. } => {
            if let Some(t) = world_data.tents.iter_mut().find(|t| {
                (t.position - snapped_pos).length() < 1.0
            }) {
                t.position = tombstone;
            }
        }
        Building::MinerHome { .. } => {
            if let Some(m) = world_data.miner_homes.iter_mut().find(|m| {
                (m.position - snapped_pos).length() < 1.0
            }) {
                m.position = tombstone;
            }
        }
        _ => {}
    }

    Ok(())
}

/// Find the index of a building in its WorldData vec by position match.
fn find_building_data_index(world_data: &WorldData, building: Building, pos: Vec2) -> Option<usize> {
    let near = |p: Vec2| (p - pos).length() < 1.0;
    match building {
        Building::Farm { .. } => world_data.farms.iter().position(|f| near(f.position)),
        Building::GuardPost { .. } => world_data.guard_posts.iter().position(|g| near(g.position)),
        Building::FarmerHome { .. } => world_data.farmer_homes.iter().position(|h| near(h.position)),
        Building::ArcherHome { .. } => world_data.archer_homes.iter().position(|a| near(a.position)),
        Building::Tent { .. } => world_data.tents.iter().position(|t| near(t.position)),
        Building::MinerHome { .. } => world_data.miner_homes.iter().position(|m| near(m.position)),
        _ => None,
    }
}

/// Consolidated building destruction: grid clear + WorldData tombstone + spawner tombstone + HP zero + combat log.
/// Used by click-destroy, inspector-destroy, and building_damage_system (HP→0).
pub fn destroy_building(
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    farm_states: &mut FarmStates,
    spawner_state: &mut SpawnerState,
    building_hp: &mut BuildingHpState,
    combat_log: &mut CombatLog,
    game_time: &GameTime,
    row: i32, col: i32,
    town_center: Vec2,
    reason: &str,
) -> Result<(), &'static str> {
    let world_pos = town_grid_to_world(town_center, row, col);
    let (gc, gr) = grid.world_to_grid(world_pos);
    let snapped = grid.grid_to_world(gc, gr);

    // Capture building info BEFORE remove_building clears the grid cell
    let building = grid.cell(gc, gr)
        .and_then(|c| c.building)
        .ok_or("no building")?;
    let hp_index = find_building_data_index(world_data, building, snapped);

    // Grid clear + WorldData tombstone
    remove_building(grid, world_data, farm_states, row, col, town_center)?;

    // Tombstone matching spawner entry
    if let Some(se) = spawner_state.0.iter_mut().find(|s| (s.position - snapped).length() < 1.0) {
        se.position = Vec2::new(-99999.0, -99999.0);
    }

    // Zero HP entry
    if let Some(idx) = hp_index {
        if let Some(hp) = building_hp.get_mut(building.kind(), idx) {
            *hp = 0.0;
        }
    }

    // Combat log
    combat_log.push(
        CombatEventKind::Harvest,
        game_time.day(), game_time.hour(), game_time.minute(),
        reason.to_string(),
    );

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
pub fn find_nearest_location(from: Vec2, bgrid: &BuildingSpatialGrid, kind: LocationKind) -> Option<Vec2> {
    find_location_within_radius(from, bgrid, kind, f32::MAX).map(|(_, pos)| pos)
}

/// Find nearest location of a given kind within radius. Returns (index, position).
pub fn find_location_within_radius(
    from: Vec2,
    bgrid: &BuildingSpatialGrid,
    kind: LocationKind,
    radius: f32,
) -> Option<(usize, Vec2)> {
    let bkind = match kind {
        LocationKind::Farm => BuildingKind::Farm,
        LocationKind::GuardPost => BuildingKind::GuardPost,
        LocationKind::Town => BuildingKind::Town,
        LocationKind::GoldMine => BuildingKind::GoldMine,
    };
    let r2 = radius * radius;
    let mut best_d2 = f32::MAX;
    let mut result: Option<(usize, Vec2)> = None;
    bgrid.for_each_nearby(from, radius, |bref| {
        if bref.kind != bkind { return; }
        let dx = bref.position.x - from.x;
        let dy = bref.position.y - from.y;
        let d2 = dx * dx + dy * dy;
        if d2 <= r2 && d2 < best_d2 {
            best_d2 = d2;
            result = Some((bref.index, bref.position));
        }
    });
    result
}

/// Find the nearest enemy building within radius that the NPC wants to attack.
/// Raiders: only ArcherHome, GuardPost. Archers/others: any enemy building.
/// Returns (kind, index, position) of nearest enemy building.
pub fn find_nearest_enemy_building(
    from: Vec2, bgrid: &BuildingSpatialGrid, npc_faction: i32, npc_job: i32, radius: f32,
) -> Option<(BuildingKind, usize, Vec2)> {
    let r2 = radius * radius;
    let mut best_d2 = f32::MAX;
    let mut result: Option<(BuildingKind, usize, Vec2)> = None;
    let is_raider = npc_job == 2;
    bgrid.for_each_nearby(from, radius, |bref| {
        if bref.faction == npc_faction { return; } // same faction
        if bref.faction < 0 { return; } // no faction (gold mines)
        // Skip non-targetable building types
        match bref.kind {
            BuildingKind::Town | BuildingKind::GoldMine | BuildingKind::Bed => return,
            _ => {}
        }
        // Raiders only target military buildings
        if is_raider && !matches!(bref.kind, BuildingKind::ArcherHome | BuildingKind::GuardPost) {
            return;
        }
        let dx = bref.position.x - from.x;
        let dy = bref.position.y - from.y;
        let d2 = dx * dx + dy * dy;
        if d2 <= r2 && d2 < best_d2 {
            best_d2 = d2;
            result = Some((bref.kind, bref.index, bref.position));
        }
    });
    result
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

/// Find nearest unoccupied building of `kind`, optionally filtered by town.
pub fn find_nearest_free(
    from: Vec2,
    bgrid: &BuildingSpatialGrid,
    kind: BuildingKind,
    occupancy: &BuildingOccupancy,
    town_idx: Option<u32>,
) -> Option<Vec2> {
    let mut best_d2 = f32::MAX;
    let mut result: Option<Vec2> = None;
    bgrid.for_each_nearby(from, f32::MAX, |bref| {
        if bref.kind != kind { return; }
        if let Some(tid) = town_idx {
            if bref.town_idx != tid { return; }
        }
        if occupancy.is_occupied(bref.position) { return; }
        let dx = bref.position.x - from.x;
        let dy = bref.position.y - from.y;
        let d2 = dx * dx + dy * dy;
        if d2 < best_d2 {
            best_d2 = d2;
            result = Some(bref.position);
        }
    });
    result
}

/// Find nearest building of `kind` within radius, filtered by town. Returns (index, position).
pub fn find_within_radius(
    from: Vec2,
    bgrid: &BuildingSpatialGrid,
    kind: BuildingKind,
    radius: f32,
    town_idx: u32,
) -> Option<(usize, Vec2)> {
    let r2 = radius * radius;
    let mut best_d2 = f32::MAX;
    let mut result: Option<(usize, Vec2)> = None;
    bgrid.for_each_nearby(from, radius, |bref| {
        if bref.kind != kind || bref.town_idx != town_idx { return; }
        let dx = bref.position.x - from.x;
        let dy = bref.position.y - from.y;
        let d2 = dx * dx + dy * dy;
        if d2 <= r2 && d2 < best_d2 {
            best_d2 = d2;
            result = Some((bref.index, bref.position));
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuildingKind { Farm, GuardPost, Town, GoldMine, ArcherHome, FarmerHome, Tent, MinerHome, Bed }

#[derive(Clone, Copy)]
pub struct BuildingRef {
    pub kind: BuildingKind,
    pub index: usize,
    pub town_idx: u32,
    pub faction: i32,
    pub position: Vec2,
}

/// CPU-side spatial grid for O(1) building lookups.
/// Cell size 256px → 31×31 cells for an 8000px world.
#[derive(Resource, Default)]
pub struct BuildingSpatialGrid {
    cell_size: f32,
    width: usize,
    height: usize,
    cells: Vec<Vec<BuildingRef>>,
}

impl BuildingSpatialGrid {
    /// Rebuild grid from current WorldData. Called once per frame.
    pub fn rebuild(&mut self, world: &WorldData, world_size_px: f32) {
        self.cell_size = 256.0;
        self.width = (world_size_px / self.cell_size).ceil() as usize + 1;
        self.height = self.width;
        let total = self.width * self.height;
        self.cells.resize_with(total, Vec::new);
        for cell in &mut self.cells { cell.clear(); }

        // Helper: look up faction from town_idx
        let faction_of = |tidx: u32| -> i32 {
            world.towns.get(tidx as usize).map(|t| t.faction).unwrap_or(0)
        };

        for (i, farm) in world.farms.iter().enumerate() {
            if farm.position.x < -9000.0 { continue; }
            self.insert(BuildingRef {
                kind: BuildingKind::Farm, index: i,
                town_idx: farm.town_idx, faction: faction_of(farm.town_idx), position: farm.position,
            });
        }
        for (i, gp) in world.guard_posts.iter().enumerate() {
            if gp.position.x < -9000.0 { continue; }
            self.insert(BuildingRef {
                kind: BuildingKind::GuardPost, index: i,
                town_idx: gp.town_idx, faction: faction_of(gp.town_idx), position: gp.position,
            });
        }
        for (i, town) in world.towns.iter().enumerate() {
            self.insert(BuildingRef {
                kind: BuildingKind::Town, index: i,
                town_idx: i as u32, faction: town.faction, position: town.center,
            });
        }
        for (i, mine) in world.gold_mines.iter().enumerate() {
            if mine.position.x < -9000.0 { continue; }
            self.insert(BuildingRef {
                kind: BuildingKind::GoldMine, index: i,
                town_idx: u32::MAX, faction: -1, position: mine.position,
            });
        }
        for (i, h) in world.archer_homes.iter().enumerate() {
            if h.position.x < -9000.0 { continue; }
            self.insert(BuildingRef {
                kind: BuildingKind::ArcherHome, index: i,
                town_idx: h.town_idx, faction: faction_of(h.town_idx), position: h.position,
            });
        }
        for (i, h) in world.farmer_homes.iter().enumerate() {
            if h.position.x < -9000.0 { continue; }
            self.insert(BuildingRef {
                kind: BuildingKind::FarmerHome, index: i,
                town_idx: h.town_idx, faction: faction_of(h.town_idx), position: h.position,
            });
        }
        for (i, t) in world.tents.iter().enumerate() {
            if t.position.x < -9000.0 { continue; }
            self.insert(BuildingRef {
                kind: BuildingKind::Tent, index: i,
                town_idx: t.town_idx, faction: faction_of(t.town_idx), position: t.position,
            });
        }
        for (i, h) in world.miner_homes.iter().enumerate() {
            if h.position.x < -9000.0 { continue; }
            self.insert(BuildingRef {
                kind: BuildingKind::MinerHome, index: i,
                town_idx: h.town_idx, faction: faction_of(h.town_idx), position: h.position,
            });
        }
        for (i, b) in world.beds.iter().enumerate() {
            if b.position.x < -9000.0 { continue; }
            self.insert(BuildingRef {
                kind: BuildingKind::Bed, index: i,
                town_idx: b.town_idx, faction: faction_of(b.town_idx), position: b.position,
            });
        }
    }

    fn insert(&mut self, bref: BuildingRef) {
        let cx = (bref.position.x / self.cell_size) as usize;
        let cy = (bref.position.y / self.cell_size) as usize;
        if cx < self.width && cy < self.height {
            self.cells[cy * self.width + cx].push(bref);
        }
    }

    /// Iterate all buildings in cells overlapping the AABB (pos ± radius).
    /// Caller must do fine distance check in the closure if needed.
    pub fn for_each_nearby(&self, pos: Vec2, radius: f32, mut f: impl FnMut(&BuildingRef)) {
        if self.width == 0 || self.height == 0 { return; }
        let min_cx = ((pos.x - radius).max(0.0) / self.cell_size) as usize;
        let max_cx = (((pos.x + radius) / self.cell_size) as usize).min(self.width - 1);
        let min_cy = ((pos.y - radius).max(0.0) / self.cell_size) as usize;
        let max_cy = (((pos.y + radius) / self.cell_size) as usize).min(self.height - 1);
        for cy in min_cy..=max_cy {
            let row = cy * self.width;
            for cx in min_cx..=max_cx {
                for bref in &self.cells[row + cx] {
                    f(bref);
                }
            }
        }
    }
}

/// Rebuild building spatial grid from WorldData. Only runs when DirtyFlags::building_grid is set.
pub fn rebuild_building_grid_system(
    mut bgrid: ResMut<BuildingSpatialGrid>,
    mut dirty: ResMut<DirtyFlags>,
    world_data: Res<WorldData>,
    grid: Res<WorldGrid>,
) {
    if grid.width == 0 || !dirty.building_grid { return; }
    dirty.building_grid = false;
    bgrid.rebuild(&world_data, grid.width as f32 * grid.cell_size);
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
pub const BUILDING_TILES: [TileSpec; 10] = [
    TileSpec::Single(50, 9),  // 0: Fountain
    TileSpec::Single(15, 2),  // 1: Bed
    TileSpec::External(2),    // 2: Guard Post (guard_post.png)
    TileSpec::Quad([(2, 15), (4, 15), (2, 17), (4, 17)]), // 3: Farm
    TileSpec::Quad([(46, 10), (47, 10), (46, 11), (47, 11)]), // 4: Camp (center)
    TileSpec::External(0),    // 5: FarmerHome (house.png)
    TileSpec::External(1),    // 6: ArcherHome (barracks.png)
    TileSpec::Quad([(48, 10), (49, 10), (48, 11), (49, 11)]), // 7: Tent (raider spawner)
    TileSpec::Single(43, 11), // 8: Gold Mine
    TileSpec::External(3),    // 9: MinerHome (miner_house.png)
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
                let Some(ext) = extra.get(idx).copied() else { continue; };
                let ext_data = ext.data.as_ref().expect("external image has no data");
                let layer_offset = (l * out_size * out_size * 4) as usize;
                let ext_w = ext.width();
                let ext_h = ext.height();

                // External sprites may be authored at 16x16 or 32x32.
                // The tileset layer is always 32x32, so 16x16 inputs are 2x upscaled.
                if ext_w == out_size && ext_h == out_size {
                    let layer_bytes = (out_size * out_size * 4) as usize;
                    if ext_data.len() >= layer_bytes {
                        data[layer_offset..layer_offset + layer_bytes]
                            .copy_from_slice(&ext_data[..layer_bytes]);
                    }
                } else {
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
    FarmerHome { town_idx: u32 },
    ArcherHome { town_idx: u32 },
    Tent { town_idx: u32 },
    GoldMine,
    MinerHome { town_idx: u32 },
}

impl Building {
    /// Returns the spawner building_kind (0=FarmerHome, 1=ArcherHome, 2=Tent, 3=MinerHome),
    /// or None for non-spawner buildings (Farm, GuardPost, etc.).
    /// Single source of truth for the building→spawner mapping.
    pub fn spawner_kind(&self) -> Option<i32> {
        match self {
            Building::FarmerHome { .. } => Some(0),
            Building::ArcherHome { .. } => Some(1),
            Building::Tent { .. } => Some(2),
            Building::MinerHome { .. } => Some(3),
            _ => None,
        }
    }

    /// Map to BuildingKind. All variants covered — every building has HP.
    pub fn kind(&self) -> BuildingKind {
        match self {
            Building::Farm { .. } => BuildingKind::Farm,
            Building::GuardPost { .. } => BuildingKind::GuardPost,
            Building::Fountain { .. } | Building::Camp { .. } => BuildingKind::Town,
            Building::GoldMine => BuildingKind::GoldMine,
            Building::FarmerHome { .. } => BuildingKind::FarmerHome,
            Building::ArcherHome { .. } => BuildingKind::ArcherHome,
            Building::Tent { .. } => BuildingKind::Tent,
            Building::MinerHome { .. } => BuildingKind::MinerHome,
            Building::Bed { .. } => BuildingKind::Bed,
        }
    }

    /// Map building variant to tileset array index (matches BUILDING_TILES order).
    pub fn tileset_index(&self) -> u16 {
        match self {
            Building::Fountain { .. } => 0,
            Building::Bed { .. } => 1,
            Building::GuardPost { .. } => 2,
            Building::Farm { .. } => 3,
            Building::Camp { .. } => 4,
            Building::FarmerHome { .. } => 5,
            Building::ArcherHome { .. } => 6,
            Building::Tent { .. } => 7,
            Building::GoldMine => 8,
            Building::MinerHome { .. } => 9,
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
    pub archers_per_town: usize,
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
            archers_per_town: 2,
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

/// Place buildings for one town on the grid: fountain, farms, farmer homes, archer homes, guard posts.
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
    let mut place = |row: i32, col: i32, building: Building, occ: &mut HashSet<(i32, i32)>, _tg: &mut TownGrid| -> Vec2 {
        let world_pos = town_grid_to_world(center, row, col);
        let (gc, gr) = grid.world_to_grid(world_pos);
        let snapped_pos = grid.grid_to_world(gc, gr);
        if let Some(cell) = grid.cell_mut(gc, gr) {
            if cell.building.is_none() {
                cell.building = Some(building);
            }
        }
        occ.insert((row, col));
        snapped_pos
    };

    // Fountain at (0, 0) = town center
    place(0, 0, Building::Fountain { town_idx }, &mut occupied, town_grid);

    // All buildings spiral outward from center: farms first (closest), then farmer homes, then archer homes
    let needed = config.farms_per_town + config.farmers_per_town + config.archers_per_town;
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
        let pos = place(row, col, Building::FarmerHome { town_idx }, &mut occupied, town_grid);
        world_data.farmer_homes.push(FarmerHome { position: pos, town_idx });
    }

    for _ in 0..config.archers_per_town {
        let Some((row, col)) = slot_iter.next() else { break };
        let pos = place(row, col, Building::ArcherHome { town_idx }, &mut occupied, town_grid);
        world_data.archer_homes.push(ArcherHome { position: pos, town_idx });
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

    // Ensure generated buildings are always inside the buildable area.
    let required = occupied.iter().fold(0, |acc, &(row, col)| {
        let row_need = (BASE_GRID_MIN - row).max(row - BASE_GRID_MAX).max(0);
        let col_need = (BASE_GRID_MIN - col).max(col - BASE_GRID_MAX).max(0);
        acc.max(row_need).max(col_need)
    });
    town_grid.area_level = town_grid.area_level.max(required);
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
        world_data.tents.push(Tent { position: snapped, town_idx });
    }

    // Ensure generated tents are always inside the buildable area.
    let required = occupied.iter().fold(0, |acc, &(row, col)| {
        let row_need = (BASE_GRID_MIN - row).max(row - BASE_GRID_MAX).max(0);
        let col_need = (BASE_GRID_MIN - col).max(col - BASE_GRID_MAX).max(0);
        acc.max(row_need).max(col_need)
    });
    town_grid.area_level = town_grid.area_level.max(required);
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
