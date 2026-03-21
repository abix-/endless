//! Procedural world generation: terrain, town placement, building layout.
//!
//! - WorldGenConfig / WorldGenStyle: generation parameters.
//! - generate_world: main entry point -- places towns, terrain, resources.
//! - spawn_town_entities: spawns ECS town entities after world gen.
//! - place_buildings: layout buildings for one town during world gen.
//! - stamp_dirt / clear_town_roads_and_dirt: terrain helpers.
//! - generate_terrain_*: terrain algorithms (classic, continents, maze, worldmap).

use std::collections::{BTreeMap, HashSet};

use bevy::prelude::*;

use crate::constants::{
    BASE_GRID_MAX, BASE_GRID_MIN, MAX_GRID_EXTENT, NPC_REGISTRY, TOWN_REGISTRY, TownKind, town_def,
};
use crate::messages::GpuUpdateMsg;
use crate::resources::{EntityMap, FactionList, GpuSlotPool, TownIndex};

use super::{
    Biome, BuildingKind, BuildingOverrides, Town, WorldCell, WorldData, WorldGrid,
    buildings::place_building,
};

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
    pub npc_counts: BTreeMap<crate::components::Job, usize>,
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
    /// Town count by kind -- bridges the 3 count fields for registry-driven loops.
    pub fn count_for(&self, kind: TownKind) -> usize {
        match kind {
            TownKind::Player => self.num_towns,
            TownKind::AiBuilder => self.ai_towns,
            TownKind::AiRaider => self.raider_towns,
        }
    }
}

// ============================================================================
// RESOURCE NODE SPAWNING
// ============================================================================

pub(crate) fn spawn_resource_nodes(
    _config: &WorldGenConfig,
    grid: &mut WorldGrid,
    slot_alloc: &mut GpuSlotPool,
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
                Biome::Forest | Biome::Jungle => BuildingKind::TreeNode,
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
/// Pure function -- takes config and writes to grid + world data.
pub fn generate_world(
    config: &WorldGenConfig,
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    faction_list: &mut FactionList,
    slot_alloc: &mut GpuSlotPool,
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
    // Initialize spatial before any buildings are placed so spatial_insert is not a no-op.
    entity_map.init_spatial(w as f32 * grid.cell_size);

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
        WorldGenStyle::WorldMap => generate_terrain_worldmap(grid, rand::random::<u64>()),
        WorldGenStyle::Classic => {}
    }

    // All settlement positions for min_distance checks
    let mut all_positions: Vec<Vec2> = Vec::new();
    // Pre-terrain styles need more attempts since many positions land on impassable cells
    let max_attempts = if needs_pre_terrain { 5000 } else { 2000 };
    // Step 2: Place towns -- single loop driven by TOWN_REGISTRY
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
                if grid.cell(gc, gr).is_some_and(|c| c.terrain.is_impassable()) {
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
            faction_list.factions.push(crate::resources::FactionData {
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
            if grid.cell(gc, gr).is_some_and(|c| c.terrain.is_impassable()) {
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
    town_index: &mut TownIndex,
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
    slot_alloc: &mut GpuSlotPool,
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

    // Center building at (0, 0) -- Fountain
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

    // Waypoints at outer corners (towns only, clockwise patrol: TL -> TR -> BR -> BL)
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

// ============================================================================
// TERRAIN HELPERS
// ============================================================================

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

            // Check proximity to towns -> Dirt
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
    slot_alloc: &mut GpuSlotPool,
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

    // Restore dirt -> original terrain within stamp_dirt radius of town center
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

// ============================================================================
// TERRAIN ALGORITHMS
// ============================================================================

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

            // 3-octave fBm elevation (large continents -> medium islands -> small coastline detail)
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

            // Biome from elevation x moisture
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
/// latitude-driven biomes (6+ types), ice caps, chokepoints, rivers, and volcanoes.
/// `seed` drives all random choices; pass `rand::random::<u64>()` at runtime,
/// or a fixed value in tests for determinism.
pub(crate) fn generate_terrain_worldmap(grid: &mut WorldGrid, seed: u64) {
    use noise::{NoiseFn, Simplex};
    use rand::Rng;
    use rand::SeedableRng;

    let elevation_noise = Simplex::new((seed & 0xffff_ffff) as u32);
    let moisture_noise = Simplex::new(((seed >> 16) & 0xffff_ffff) as u32);
    let detail_noise = Simplex::new(((seed >> 32) & 0xffff_ffff) as u32);

    let world_w = grid.width as f64 * grid.cell_size as f64;
    let world_h = grid.height as f64 * grid.cell_size as f64;
    let gw = grid.width;
    let gh = grid.height;

    let land_pct: f64 = 0.45; // 45% land
    let ice_cap_pct: f64 = 0.12; // 12% of map height at each pole

    // Randomize continent count 3-10 from seed
    let mut seed_rng = rand::rngs::SmallRng::seed_from_u64(seed);
    let continent_count: usize = seed_rng.random_range(3..=10);

    // Place continent seed points, keeping them spread apart (>25% world width)
    let min_sep_sq = (0.25_f64).powi(2); // normalized coords
    let mut continent_seeds: Vec<(f64, f64)> = Vec::with_capacity(continent_count);
    let mut attempts = 0usize;
    while continent_seeds.len() < continent_count && attempts < 2000 {
        attempts += 1;
        let cx_n = seed_rng.random_range(0.1..0.9_f64);
        let cy_n = seed_rng.random_range(0.2..0.8_f64);
        let ok = continent_seeds.iter().all(|&(ox, oy)| {
            let dx = cx_n - ox / world_w;
            let dy = cy_n - oy / world_h;
            dx * dx + dy * dy >= min_sep_sq
        });
        if ok {
            continent_seeds.push((cx_n * world_w, cy_n * world_h));
        }
    }
    // Fallback: fill remaining slots without separation constraint
    while continent_seeds.len() < continent_count {
        let cx_n = seed_rng.random_range(0.1..0.9_f64);
        let cy_n = seed_rng.random_range(0.2..0.8_f64);
        continent_seeds.push((cx_n * world_w, cy_n * world_h));
    }

    let water_threshold: f64 = 0.5 - land_pct * 0.4;

    // Pass 1: compute elevation map and assign initial biomes
    let mut elevations = vec![0.0_f64; gw * gh];

    for row in 0..gh {
        for col in 0..gw {
            let world_pos = grid.grid_to_world(col, row);
            let wx = world_pos.x as f64;
            let wy = world_pos.y as f64;
            let lat = wy / world_h;

            // Ice caps at poles: impassable
            if lat < ice_cap_pct || lat > (1.0 - ice_cap_pct) {
                let cell = &mut grid.cells[row * gw + col];
                cell.terrain = Biome::Rock;
                cell.original_terrain = Biome::Rock;
                elevations[row * gw + col] = 1.0; // treat as high land for river purposes
                continue;
            }

            // 4-octave fBm elevation
            let e_raw = (1.0 * elevation_noise.get([wx * 0.00025, wy * 0.00025])
                + 0.5 * elevation_noise.get([wx * 0.0005, wy * 0.0005])
                + 0.25 * elevation_noise.get([wx * 0.001, wy * 0.001])
                + 0.125 * elevation_noise.get([wx * 0.002, wy * 0.002]))
                / 1.875;

            // Continent seed bias: Gaussian boost near seeds
            let mut continent_boost: f64 = 0.0;
            for &(cx, cy) in &continent_seeds {
                let dx = (wx - cx) / world_w;
                let dy = (wy - cy) / world_h;
                let dist_sq = dx * dx + dy * dy;
                let radius = 0.14;
                continent_boost += (-dist_sq / (2.0 * radius * radius)).exp();
            }
            continent_boost = (continent_boost / continent_count as f64).min(1.0);

            let e_norm = (e_raw + 1.0) * 0.5;
            let elev = (e_norm * 0.6 + continent_boost * 0.4).clamp(0.0, 1.0);

            // Edge falloff: push map borders toward ocean
            let nx = (wx / world_w - 0.5) * 2.0;
            let ny = (wy / world_h - 0.5) * 2.0;
            let edge_dist = 1.0 - (1.0 - nx * nx) * (1.0 - ny * ny);
            let elev = (elev * (1.0 - edge_dist * 0.7)).max(0.0);

            // Chokepoint detail: high-frequency noise carves straits / builds land bridges
            let choke = detail_noise.get([wx * 0.003, wy * 0.003]);
            let elev =
                elev + choke * 0.04 * (1.0 - (elev - water_threshold).abs().min(0.15) / 0.15);

            elevations[row * gw + col] = elev;

            // Moisture for biome variety
            let m = (moisture_noise.get([wx * 0.0012, wy * 0.0012]) + 1.0) * 0.5;

            // Latitude-driven temperature: 0 at poles, 1 at equator
            let temp = 1.0 - (lat - 0.5).abs() * 2.0;

            // Biome from elevation x temperature x moisture (6+ types)
            let biome = if elev < water_threshold {
                Biome::Water
            } else if temp < 0.15 {
                // Near-polar: tundra (passable barren)
                Biome::Tundra
            } else if temp < 0.3 {
                // Sub-polar / boreal
                if m > 0.5 {
                    Biome::Forest // taiga
                } else {
                    Biome::Tundra
                }
            } else if temp < 0.6 {
                // Temperate
                if m > 0.6 {
                    Biome::Forest
                } else if m > 0.3 {
                    Biome::Grass
                } else {
                    Biome::Desert
                }
            } else {
                // Tropical / equatorial
                if m > 0.55 {
                    Biome::Jungle
                } else if m > 0.25 {
                    Biome::Grass
                } else {
                    Biome::Desert
                }
            };

            let cell = &mut grid.cells[row * gw + col];
            cell.terrain = biome;
            cell.original_terrain = biome;
        }
    }

    // Pass 2: carve rivers -- one per continent seed, flowing downhill to coast
    carve_rivers(grid, &elevations, &continent_seeds, water_threshold, seed);

    // Pass 3: place volcanoes -- Rock clusters at high-elevation coast-adjacent cells
    place_volcanoes(grid, &elevations, water_threshold, seed);
}

/// Carve rivers from high-elevation land toward the coast.
/// For each continent seed, find the highest non-water cell within its influence radius,
/// then do a steepest-descent walk to the ocean, marking cells as Water.
/// Only marks cells that are currently land (not already Water/Rock).
fn carve_rivers(
    grid: &mut WorldGrid,
    elevations: &[f64],
    continent_seeds: &[(f64, f64)],
    water_threshold: f64,
    seed: u64,
) {
    use rand::{Rng, SeedableRng};
    let mut rng = rand::rngs::SmallRng::seed_from_u64(seed.wrapping_add(0xdead_beef));

    let gw = grid.width;
    let gh = grid.height;

    // For each continent, find a high-elevation source point near the seed
    for (ci, &(sx, sy)) in continent_seeds.iter().enumerate() {
        // Search radius in grid cells: ~20% of grid width
        let search_radius = (gw.min(gh) as f64 * 0.20) as i32;
        let sc = (sx / grid.cell_size as f64) as i32;
        let sr = (sy / grid.cell_size as f64) as i32;

        // Find highest land cell within search radius
        let mut best_elev = water_threshold + 0.05; // must be clearly above water
        let mut best_col = -1i32;
        let mut best_row = -1i32;

        for dr in -search_radius..=search_radius {
            for dc in -search_radius..=search_radius {
                let col = sc + dc;
                let row = sr + dr;
                if col < 0 || row < 0 || col >= gw as i32 || row >= gh as i32 {
                    continue;
                }
                let idx = row as usize * gw + col as usize;
                let e = elevations[idx];
                if e > best_elev && !grid.cells[idx].terrain.is_impassable() {
                    best_elev = e;
                    best_col = col;
                    best_row = row;
                }
            }
        }

        if best_col < 0 {
            continue; // no valid source on this continent
        }

        // Steepest-descent walk to ocean -- max steps to prevent infinite loops
        let max_steps = gw + gh;
        let mut cur_col = best_col as usize;
        let mut cur_row = best_row as usize;

        for (river_len, _) in (0..max_steps).enumerate() {
            let idx = cur_row * gw + cur_col;

            // Reached water: done
            if grid.cells[idx].terrain == Biome::Water {
                break;
            }

            // Mark current cell as river (Water)
            if river_len > 2 && grid.cells[idx].terrain != Biome::Rock {
                // Keep first 2 cells dry so source isn't immediately ocean
                grid.cells[idx].terrain = Biome::Water;
                grid.cells[idx].original_terrain = Biome::Water;
            }

            // Find the lowest neighbor (steepest descent)
            let mut best_neighbor_elev = elevations[idx];
            let mut best_nc = cur_col as i32;
            let mut best_nr = cur_row as i32;

            // Check 4-directional neighbors (no diagonals for cleaner rivers)
            let dirs: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];
            for (dc, dr) in dirs {
                let nc = cur_col as i32 + dc;
                let nr = cur_row as i32 + dr;
                if nc < 0 || nr < 0 || nc >= gw as i32 || nr >= gh as i32 {
                    continue;
                }
                let ne = elevations[nr as usize * gw + nc as usize];
                if ne < best_neighbor_elev {
                    best_neighbor_elev = ne;
                    best_nc = nc;
                    best_nr = nr;
                }
            }

            if best_nc == cur_col as i32 && best_nr == cur_row as i32 {
                // Stuck in a depression: add a small random nudge to escape
                let dirs_shuffle: [(i32, i32); 4] = {
                    let mut d = [(0i32, -1i32), (0, 1), (-1, 0), (1, 0)];
                    // Simple Fisher-Yates with rng
                    for i in (1..4).rev() {
                        let j = rng.random_range(0..=i);
                        d.swap(i, j);
                    }
                    d
                };
                let mut moved = false;
                for (dc, dr) in dirs_shuffle {
                    let nc = cur_col as i32 + dc;
                    let nr = cur_row as i32 + dr;
                    if nc >= 0 && nr >= 0 && nc < gw as i32 && nr < gh as i32 {
                        let nidx = nr as usize * gw + nc as usize;
                        if grid.cells[nidx].terrain != Biome::Rock {
                            cur_col = nc as usize;
                            cur_row = nr as usize;
                            moved = true;
                            break;
                        }
                    }
                }
                if !moved {
                    break; // totally stuck, abandon river
                }
            } else {
                cur_col = best_nc as usize;
                cur_row = best_nr as usize;
            }
        }

        let _ = ci; // suppress unused warning
    }
}

/// Place volcano Rock clusters at high-elevation land cells near continent edges.
/// "Near coast" = land cells adjacent to ocean within ~3 cells of the water boundary.
/// Volcanic zones get a ring of fertile Rock around their center.
fn place_volcanoes(grid: &mut WorldGrid, elevations: &[f64], water_threshold: f64, seed: u64) {
    use rand::{Rng, SeedableRng};
    let mut rng = rand::rngs::SmallRng::seed_from_u64(seed.wrapping_add(0xcafe_babe));

    let gw = grid.width;
    let gh = grid.height;

    // Collect candidate cells: high elevation + near coast (within 4 cells of Water)
    let near_coast_dist = 5i32;
    let mut candidates: Vec<(usize, usize, f64)> = Vec::new();

    for row in 2..(gh - 2) {
        for col in 2..(gw - 2) {
            let idx = row * gw + col;
            let elev = elevations[idx];
            if elev < water_threshold + 0.12 {
                continue; // only high-elevation land
            }
            if grid.cells[idx].terrain.is_impassable() {
                continue;
            }

            // Check if any cell within near_coast_dist is Water
            let mut near_water = false;
            'outer: for dr in -near_coast_dist..=near_coast_dist {
                for dc in -near_coast_dist..=near_coast_dist {
                    let nc = col as i32 + dc;
                    let nr = row as i32 + dr;
                    if nc < 0 || nr < 0 || nc >= gw as i32 || nr >= gh as i32 {
                        continue;
                    }
                    if grid.cells[nr as usize * gw + nc as usize].terrain == Biome::Water {
                        near_water = true;
                        break 'outer;
                    }
                }
            }
            if near_water {
                candidates.push((col, row, elev));
            }
        }
    }

    if candidates.is_empty() {
        return;
    }

    // Sort by elevation descending -- volcanoes prefer the highest coastal peaks
    candidates.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    // Place volcanoes with minimum spacing (15 grid cells apart)
    let min_volcano_spacing = 15i32;
    let max_volcanoes = (gw * gh / 5000).clamp(2, 8);
    let mut placed: Vec<(i32, i32)> = Vec::new();

    for &(vc, vr, _) in &candidates {
        if placed.len() >= max_volcanoes {
            break;
        }
        let too_close = placed.iter().any(|&(pc, pr)| {
            let dc = vc as i32 - pc;
            let dr = vr as i32 - pr;
            dc * dc + dr * dr < min_volcano_spacing * min_volcano_spacing
        });
        if too_close {
            continue;
        }

        // Skip if randomly rejected (only ~60% of candidates become volcanoes)
        if rng.random_range(0..10) < 4 {
            continue;
        }

        placed.push((vc as i32, vr as i32));

        // Stamp a small Rock cluster (volcano cone): radius 1 core, radius 3 ring
        let cone_radius = 1i32;
        let ring_radius = 3i32;

        for dr in -ring_radius..=ring_radius {
            for dc in -ring_radius..=ring_radius {
                let nc = vc as i32 + dc;
                let nr = vr as i32 + dr;
                if nc < 0 || nr < 0 || nc >= gw as i32 || nr >= gh as i32 {
                    continue;
                }
                let dist_sq = dc * dc + dr * dr;
                let cell = &mut grid.cells[nr as usize * gw + nc as usize];
                if cell.terrain == Biome::Water {
                    continue; // never overwrite ocean
                }
                if dist_sq <= cone_radius * cone_radius {
                    // Rock cone at center
                    cell.terrain = Biome::Rock;
                    cell.original_terrain = Biome::Rock;
                } else {
                    // Fertile ring: Grass (rich volcanic soil)
                    if cell.terrain != Biome::Rock {
                        cell.terrain = Biome::Grass;
                        cell.original_terrain = Biome::Grass;
                    }
                }
            }
        }
    }
}
