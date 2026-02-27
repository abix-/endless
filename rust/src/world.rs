//! World Data - Towns, farms, beds, waypoints, sprite definitions
//! World Grid - 2D cell grid covering entire world (terrain + buildings)
//! World Generation - Procedural town placement and building layout

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use serde::{Serialize, Deserialize};
use std::collections::{BTreeMap, HashSet};

/// Serialize Vec2 as [f32; 2] for save file backwards compat.
pub mod vec2_as_array {
    use bevy::prelude::Vec2;
    use serde::{Serialize, Deserialize, Serializer, Deserializer};
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
    use serde::{Serializer, Deserializer, Serialize, Deserialize};
    pub fn serialize<S: Serializer>(v: &Option<Vec2>, s: S) -> Result<S::Ok, S::Error> {
        v.map(|v| [v.x, v.y]).serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec2>, D::Error> {
        Ok(<Option<[f32; 2]>>::deserialize(d)?.map(|[x, y]| Vec2::new(x, y)))
    }
}

use crate::components::Job;
use crate::constants::{TOWN_GRID_SPACING, BASE_GRID_MIN, BASE_GRID_MAX, MAX_GRID_EXTENT, NPC_REGISTRY};
use crate::resources::{FoodStorage, GoldStorage, FactionStats, RaiderState, EntityMap, CombatEventKind, GameTime, SystemTimings, EntitySlots};
use crate::messages::{DirtyWriters, BuildingGridDirtyMsg};
use crate::messages::{GpuUpdate, GpuUpdateMsg, CombatLogMsg};

/// True if a position has not been tombstoned (i.e. the entity still exists).
/// Tombstoned entities have position.x = -99999.0; this checks > -9000.0.
#[inline]
pub fn is_alive(pos: Vec2) -> bool { pos.x > -9000.0 }


// ============================================================================
// SPRITE DEFINITIONS (from roguelikeSheet_transparent.png)
// ============================================================================

/// Sprite sheet constants
pub const CELL: f32 = crate::render::WORLD_CELL;  // 16px sprite + 1px margin
pub const SPRITE_SIZE: f32 = crate::render::WORLD_SPRITE_SIZE;
pub const SHEET_SIZE: (f32, f32) = crate::render::WORLD_SHEET_SIZE;


// ============================================================================
// WORLD DATA STRUCTS
// ============================================================================

/// A town (villager or raider settlement).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Town {
    pub name: String,
    #[serde(with = "vec2_as_array")]
    pub center: Vec2,
    pub faction: i32,       // 0=Villager, 1+=Raider factions
    pub sprite_type: i32,   // 0=fountain, 1=tent
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
}

impl PlacedBuilding {
    pub fn new(position: Vec2, town_idx: u32) -> Self {
        Self { position, town_idx, patrol_order: 0, assigned_mine: None, manual_mine: false, wall_level: 0 }
    }
    pub fn new_wall(position: Vec2, town_idx: u32) -> Self {
        Self { position, town_idx, patrol_order: 0, assigned_mine: None, manual_mine: false, wall_level: 1 }
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

/// All town building grids. One per town (villager and raider towns).
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

/// Returns true if world_pos falls inside any town's build area OTHER than own_town_idx.
pub fn in_foreign_build_area(pos: Vec2, own_town_idx: usize, towns: &[Town], town_grids: &TownGrids) -> bool {
    for tg in &town_grids.grids {
        if tg.town_data_idx == own_town_idx { continue; }
        let Some(town) = towns.get(tg.town_data_idx) else { continue };
        let (row, col) = world_to_town_grid(town.center, pos);
        if is_slot_buildable(tg, row, col) {
            return true;
        }
    }
    false
}

/// All empty buildable slots in a town grid (excludes center 0,0).
pub fn empty_slots(tg: &TownGrid, center: Vec2, grid: &WorldGrid, entity_map: &crate::resources::EntityMap) -> Vec<(i32, i32)> {
    let (min_row, max_row, min_col, max_col) = build_bounds(tg);
    let mut out = Vec::new();
    for r in min_row..=max_row {
        for c in min_col..=max_col {
            if r == 0 && c == 0 { continue; }
            let pos = town_grid_to_world(center, r, c);
            let (gc, gr) = grid.world_to_grid(pos);
            if grid.cell(gc, gr).is_some() && !entity_map.has_building_at(gc as _, gr as _) {
                out.push((r, c));
            }
        }
    }
    out
}

/// Find which town has a buildable slot matching the given grid coords.
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

/// Unified building placement: validate, pay, place on grid, register in WorldData,
/// push spawner/HP/GPU slot, and mark dirty flags. Single entry point for all
/// runtime building placement (player and AI, town-grid and wilderness).
pub(crate) fn place_building(
    grid: &mut WorldGrid,
    world_data: &WorldData,
    food_storage: &mut FoodStorage,
    slot_alloc: &mut crate::resources::EntitySlots,
    entity_map: &mut EntityMap,
    dirty_writers: &mut DirtyWriters,
    kind: BuildingKind,
    town_data_idx: usize,
    world_pos: Vec2,
    cost: i32,
    town_grids: &TownGrids,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
    commands: &mut Commands,
) -> Result<(), &'static str> {
    let (gc, gr) = grid.world_to_grid(world_pos);
    let snapped = grid.grid_to_world(gc, gr);
    let town_idx = town_data_idx as u32;

    // Validate: cell exists, empty, not water
    let cell = grid.cell(gc, gr).ok_or("cell out of bounds")?;
    if entity_map.has_building_at(gc as i32, gr as i32) { return Err("cell already has a building"); }
    if cell.terrain == Biome::Water { return Err("cannot build on water"); }

    // Reject placement inside another faction's build area
    if in_foreign_build_area(snapped, town_data_idx, &world_data.towns, town_grids) {
        return Err("cannot build in foreign territory");
    }

    // Deduct food
    let food = food_storage.food.get_mut(town_data_idx).ok_or("invalid town")?;
    if *food < cost { return Err("not enough food"); }
    *food -= cost;

    let def = crate::constants::building_def(kind);
    let faction = world_data.towns.get(town_data_idx).map(|t| t.faction).unwrap_or(0);

    // Allocate GPU slot + create instance + register spawner
    let patrol_order = if kind == BuildingKind::Waypoint {
        entity_map.count_for_town(BuildingKind::Waypoint, town_idx) as u32
    } else { 0 };
    let wall_level = if kind == BuildingKind::Wall { 1 } else { 0 };
    let Some(slot) = place_building_instance(slot_alloc, entity_map, kind, snapped, town_idx, faction, patrol_order, wall_level) else {
        return Err("no GPU slots available");
    };

    // Spawn building entity
    {
        use crate::components::*;
        let entity = commands.spawn((
            EntitySlot(slot),
            Position::new(snapped.x, snapped.y),
            Health(def.hp),
            Faction(faction),
            TownId(town_data_idx as i32),
            Building { kind },
        )).id();
        entity_map.entities.insert(slot, entity);
    }
    push_building_gpu_updates(
        slot, kind, snapped, faction, def.hp,
        crate::constants::tileset_index(kind), def.is_tower, gpu_updates,
    );

    // Wall auto-tile: update sprites for new wall + neighbors
    if kind == BuildingKind::Wall {
        update_wall_sprites_around(grid, entity_map, gc, gr, gpu_updates);
    }

    // Signal rebuild systems via messages
    dirty_writers.mark_building_changed(kind);

    Ok(())
}

/// Check if a grid cell contains a wall building.
fn is_wall_at(entity_map: &EntityMap, col: usize, row: usize) -> bool {
    entity_map.get_at_grid(col as i32, row as i32)
        .is_some_and(|inst| inst.kind == BuildingKind::Wall)
}

/// Compute auto-tile variant offset (0-5) for a wall at grid (col, row).
/// Atlas layers: 0=E-W, 1=N-S, 2=TL, 3=BL, 4=BR, 5=TR (screen-space corners).
/// Note: row-1 = south (lower Y), row+1 = north (higher Y) in Bevy's Y-up coords.
pub fn wall_autotile_variant(entity_map: &EntityMap, col: usize, row: usize) -> u16 {
    let n = row > 0 && is_wall_at(entity_map, col, row - 1);
    let s = is_wall_at(entity_map, col, row + 1);
    let e = is_wall_at(entity_map, col + 1, row);
    let w = col > 0 && is_wall_at(entity_map, col - 1, row);
    use crate::constants::*;
    match (n, s, e, w) {
        (false, false, true, true)  => WALL_EW,
        (false, false, true, false) => WALL_EW,
        (false, false, false, true) => WALL_EW,
        (true, true, false, false)  => WALL_NS,
        (true, false, false, false) => WALL_NS,
        (false, true, false, false) => WALL_NS,
        (true, false, true, false)  => WALL_TR,
        (true, false, false, true)  => WALL_TL,
        (false, true, false, true)  => WALL_BL,
        (false, true, true, false)  => WALL_BR,
        (true, false, true, true)   => WALL_T_OPEN_N,  // 3 neighbors, open north
        (true, true, true, false)   => WALL_T_OPEN_W,  // 3 neighbors, open west
        (false, true, true, true)   => WALL_T_OPEN_S,  // 3 neighbors, open south
        (true, true, false, true)   => WALL_T_OPEN_E,  // 3 neighbors, open east
        (true, true, true, true)    => WALL_CROSS,      // 4-way
        _ => WALL_EW,
    }
}

/// Recompute wall auto-tile sprites for the wall at (col, row) and its 4 neighbors.
/// Pushes GPU SetSpriteFrame updates for each wall found.
pub fn update_wall_sprites_around(
    grid: &WorldGrid,
    entity_map: &EntityMap,
    col: usize, row: usize,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) {
    let wall_base = crate::constants::tileset_index(BuildingKind::Wall) as f32;
    let offsets: [(i32, i32); 5] = [(0, 0), (0, -1), (0, 1), (1, 0), (-1, 0)];
    for (dc, dr) in offsets {
        let c = col as i32 + dc;
        let r = row as i32 + dr;
        if c < 0 || r < 0 { continue; }
        let (c, r) = (c as usize, r as usize);
        if !is_wall_at(entity_map, c, r) { continue; }
        let variant = wall_autotile_variant(entity_map, c, r);
        let pos = grid.grid_to_world(c, r);
        if let Some(inst) = entity_map.find_by_position(pos) {
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame {
                idx: inst.slot, col: wall_base + variant as f32, row: 0.0,
                atlas: crate::constants::ATLAS_BUILDING,
            }));
        }
    }
}

/// Set auto-tile sprites for all walls in the world. Call after building instances are created.
pub fn update_all_wall_sprites(
    grid: &WorldGrid,
    entity_map: &EntityMap,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) {
    let wall_base = crate::constants::tileset_index(BuildingKind::Wall) as f32;
    for inst in entity_map.iter_kind(BuildingKind::Wall) {
        let (gc, gr) = grid.world_to_grid(inst.position);
        let variant = wall_autotile_variant(entity_map, gc, gr);
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame {
            idx: inst.slot, col: wall_base + variant as f32, row: 0.0,
            atlas: crate::constants::ATLAS_BUILDING,
        }));
    }
}

/// Resolve SpawnNpcMsg fields from a spawner entry's building_kind.
/// Uses BUILDING_REGISTRY SpawnBehavior so new buildings with existing behaviors need no changes.
/// Returns (job, faction, work_x, work_y, starting_post, attack_type, npc_label, bld_label, work_slot).
pub fn resolve_spawner_npc(
    inst: &crate::resources::BuildingInstance,
    towns: &[Town],
    entity_map: &crate::resources::EntityMap,
) -> (i32, i32, f32, f32, i32, i32, &'static str, &'static str, Option<usize>) {
    use crate::constants::{SpawnBehavior, building_def, npc_def};
    use crate::components::Job;

    let town_faction = towns.get(inst.town_idx as usize)
        .map(|t| t.faction).unwrap_or(0);

    let def = building_def(inst.kind);
    let Some(ref spawner) = def.spawner else {
        let raider_faction = towns.get(inst.town_idx as usize).map(|t| t.faction).unwrap_or(1);
        return (2, raider_faction, -1.0, -1.0, -1, 0, "Raider", "Unknown", None);
    };

    let npc_label = npc_def(Job::from_i32(spawner.job)).label;

    match spawner.behavior {
        SpawnBehavior::FindNearestFarm => {
            let found = find_nearest_free(
                inst.position, entity_map, BuildingKind::Farm, Some(inst.town_idx),
            );
            let (work_slot, farm) = found.map(|(s, p)| (Some(s), p)).unwrap_or((None, inst.position));
            (spawner.job, town_faction, farm.x, farm.y, -1, spawner.attack_type, npc_label, def.label, work_slot)
        }
        SpawnBehavior::FindNearestWaypoint => {
            let post_idx = find_location_within_radius(
                inst.position, entity_map, LocationKind::Waypoint, f32::MAX,
            ).map(|(idx, _)| idx as i32).unwrap_or(-1);
            (spawner.job, town_faction, -1.0, -1.0, post_idx, spawner.attack_type, npc_label, def.label, None)
        }
        SpawnBehavior::Raider => {
            let raider_faction = towns.get(inst.town_idx as usize)
                .map(|t| t.faction).unwrap_or(1);
            (spawner.job, raider_faction, -1.0, -1.0, -1, spawner.attack_type, npc_label, def.label, None)
        }
        SpawnBehavior::Miner => {
            let (work_slot, mine) = if let Some(pos) = inst.assigned_mine {
                (entity_map.slot_at_position(pos), pos)
            } else {
                find_nearest_free(
                    inst.position, entity_map, BuildingKind::GoldMine, None,
                ).map(|(s, p)| (Some(s), p)).unwrap_or((None, inst.position))
            };
            (spawner.job, town_faction, mine.x, mine.y, -1, spawner.attack_type, npc_label, def.label, work_slot)
        }
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
    } else if kind == BuildingKind::Road {
        crate::constants::ENTITY_FLAG_BUILDING | crate::constants::ENTITY_FLAG_UNTARGETABLE
    } else {
        crate::constants::ENTITY_FLAG_BUILDING
    };
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetPosition { idx: slot, x: pos.x, y: pos.y }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFaction { idx: slot, faction }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: slot, health: max_hp }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFlags { idx: slot, flags }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHalfSize { idx: slot, half_w: crate::constants::BUILDING_HITBOX_HALF[0], half_h: crate::constants::BUILDING_HITBOX_HALF[1] }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame {
        idx: slot, col: tileset_idx as f32, row: 0.0,
        atlas: crate::constants::ATLAS_BUILDING,
    }));
}

/// Allocate GPU slot + create BuildingInstance + register spawner in one call.
/// Returns the allocated slot, or None if no slots available.
pub fn place_building_instance(
    slot_alloc: &mut crate::resources::EntitySlots,
    entity_map: &mut EntityMap,
    kind: BuildingKind,
    pos: Vec2,
    town_idx: u32,
    faction: i32,
    patrol_order: u32,
    wall_level: u8,
) -> Option<usize> {
    use crate::constants::building_def;
    let def = building_def(kind);
    let Some(slot) = slot_alloc.alloc() else {
        warn!("No building slots available for {:?}", kind);
        return None;
    };
    let has_spawner = def.spawner.is_some();
    entity_map.add_instance(crate::resources::BuildingInstance {
        kind, position: pos, town_idx, slot, faction,
        patrol_order, assigned_mine: None, manual_mine: false, wall_level,
        npc_slot: -1,
        respawn_timer: if has_spawner { 0.0 } else { -2.0 },
        growth_ready: false,
        growth_progress: 0.0,
        occupants: 0,
    });
    Some(slot)
}



/// Spawn ECS entities for all building instances in EntityMap.
/// Reads instances (which have Entity::PLACEHOLDER), spawns real entities, updates the map.
/// `loaded_hp`: optional HP overrides keyed by slot.
pub fn spawn_building_entities(
    commands: &mut Commands,
    entity_map: &mut EntityMap,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
    loaded_hp: Option<&std::collections::HashMap<usize, f32>>,
) {
    use crate::components::*;
    use crate::constants::{building_def, tileset_index};

    // Collect slots to iterate (can't mutate map while iterating)
    let slots: Vec<usize> = entity_map.all_entity_slots().collect();
    let mut count = 0usize;
    for slot in slots {
        let Some(inst) = entity_map.get_instance(slot) else { continue };
        let def = building_def(inst.kind);
        let hp = loaded_hp.and_then(|m| m.get(&slot).copied()).unwrap_or(def.hp);
        if hp <= 0.0 { continue; }
        let pos = inst.position;
        let faction = inst.faction;
        let town_idx = inst.town_idx;
        let kind = inst.kind;
        let entity = commands.spawn((
            EntitySlot(slot),
            Position::new(pos.x, pos.y),
            Health(hp),
            Faction(faction),
            TownId(town_idx as i32),
            Building { kind },
        )).id();
        entity_map.entities.insert(slot, entity);
        push_building_gpu_updates(slot, kind, pos, faction, hp, tileset_index(kind), def.is_tower, gpu_updates);
        count += 1;
    }
    info!("Spawned {} building entities", count);
}

/// Spawn one NPC per building spawner. Returns messages for the caller to write.
fn spawn_npcs_from_spawners(
    slot_alloc: &mut EntitySlots,
    towns: &[Town],
    entity_map: &mut EntityMap,
) -> Vec<crate::messages::SpawnNpcMsg> {
    let mut msgs = Vec::new();
    // Collect spawner slots first (need immutable entity_map for resolve_spawner_npc, then mutate)
    let spawner_slots: Vec<usize> = entity_map.iter_instances()
        .filter(|i| crate::constants::building_def(i.kind).spawner.is_some())
        .map(|i| i.slot)
        .collect();
    for bld_slot in spawner_slots {
        let Some(slot) = slot_alloc.alloc() else { break };
        let Some(inst) = entity_map.get_instance(bld_slot) else { continue };
        let (job, faction, work_x, work_y, starting_post, attack_type, _, _, work_slot) =
            resolve_spawner_npc(inst, towns, entity_map);
        let pos = inst.position;
        let town_idx = inst.town_idx as i32;
        msgs.push(crate::messages::SpawnNpcMsg {
            slot_idx: slot,
            x: pos.x, y: pos.y,
            job, faction, town_idx,
            home_x: pos.x, home_y: pos.y,
            work_x, work_y, starting_post, attack_type,
        });
        if let Some(inst_mut) = entity_map.get_instance_mut(bld_slot) {
            inst_mut.npc_slot = slot as i32;
        }
    }
    msgs
}

/// Create AI players for all non-player towns with random personalities.
fn create_ai_players(
    world_data: &WorldData,
    town_grids: &TownGrids,
) -> Vec<crate::systems::AiPlayer> {
    use crate::systems::{AiKind, AiPlayer, AiPersonality};
    use rand::Rng;
    let personalities = [AiPersonality::Aggressive, AiPersonality::Balanced, AiPersonality::Economic];
    let mut rng = rand::rng();
    let mut players = Vec::new();
    for (grid_idx, tg) in town_grids.grids.iter().enumerate() {
        let tdi = tg.town_data_idx;
        if let Some(town) = world_data.towns.get(tdi) {
            if town.faction > 0 {
                let kind = if town.sprite_type == 1 { AiKind::Raider } else { AiKind::Builder };
                let personality = personalities[rng.random_range(0..personalities.len())];
                players.push(AiPlayer {
                    town_data_idx: tdi, grid_idx, kind, personality,
                    last_actions: std::collections::VecDeque::new(),
                    active: true, squad_indices: Vec::new(),
                    squad_cmd: std::collections::HashMap::new(),
                });
            }
        }
    }
    players
}

/// Full world setup: generate terrain/towns, init resources, buildings, spawners, NPCs.
/// Returns (spawn_messages, ai_players) for the caller to write into Bevy.
pub fn setup_world(
    config: &WorldGenConfig,
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    town_grids: &mut TownGrids,
    slot_alloc: &mut EntitySlots,
    entity_map: &mut EntityMap,
    food_storage: &mut FoodStorage,
    gold_storage: &mut GoldStorage,
    faction_stats: &mut FactionStats,
    raider_state: &mut RaiderState,
) -> (Vec<crate::messages::SpawnNpcMsg>, Vec<crate::systems::AiPlayer>) {
    town_grids.grids.clear();
    entity_map.clear_buildings();
    generate_world(config, grid, world_data, town_grids, slot_alloc, entity_map);
    entity_map.init_spatial(grid.width as f32 * grid.cell_size);

    let n = world_data.towns.len();
    food_storage.init(n);
    gold_storage.init(n);
    faction_stats.init(n);
    raider_state.init(n, 10);

    let npc_msgs = spawn_npcs_from_spawners(
        slot_alloc,
        &world_data.towns, entity_map,
    );
    let ai_players = create_ai_players(world_data, town_grids);

    (npc_msgs, ai_players)
}

/// Shared startup materialization for generated worlds.
/// Writes NPC spawn messages and spawns ECS building entities.
pub fn materialize_generated_world(
    commands: &mut Commands,
    entity_map: &mut EntityMap,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
    spawn_writer: &mut MessageWriter<crate::messages::SpawnNpcMsg>,
    npc_msgs: Vec<crate::messages::SpawnNpcMsg>,
) -> usize {
    let total = npc_msgs.len();
    for msg in npc_msgs {
        spawn_writer.write(msg);
    }
    spawn_building_entities(commands, entity_map, gpu_updates, None);
    total
}

/// Place a waypoint at an arbitrary world position (not tied to town grid).
/// Place a wilderness building (world-grid snapping, not town-grid).
/// Used for Waypoint, Road, and AI territorial expansion.

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


/// Consolidated building destruction: grid clear + growth tombstone + HP zero + combat log.
/// Grid cleanup for building removal: clears grid cell, updates wall auto-tile, logs combat event.
/// Does NOT mark the entity as Dead — callers send DamageMsg for that (single Dead writer: death_system).
pub(crate) fn destroy_building(
    grid: &mut WorldGrid,
    world_data: &WorldData,
    entity_map: &mut EntityMap,
    combat_log: &mut MessageWriter<CombatLogMsg>,
    game_time: &GameTime,
    row: i32, col: i32,
    town_center: Vec2,
    reason: &str,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) -> Result<(), &'static str> {
    let world_pos = town_grid_to_world(town_center, row, col);
    let (gc, gr) = grid.world_to_grid(world_pos);

    let inst = entity_map.get_at_grid(gc as i32, gr as i32)
        .ok_or("no building")?;
    let kind = inst.kind;
    let bld_town_idx = inst.town_idx;

    // Wall auto-tile: update neighbor sprites after wall removed
    if kind == BuildingKind::Wall {
        update_wall_sprites_around(grid, entity_map, gc, gr, gpu_updates);
    }

    // Combat log — derive faction from building's town_idx
    let bld_town = bld_town_idx as usize;
    let faction = world_data.towns.get(bld_town).map(|t| t.faction).unwrap_or(0);
    combat_log.write(CombatLogMsg {
        kind: CombatEventKind::Harvest, faction,
        day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(),
        message: reason.to_string(), location: None,
    });

    Ok(())
}

/// Location types for find_nearest_location.
#[derive(Clone, Copy, Debug)]
#[derive(PartialEq, Eq)]
pub enum LocationKind {
    Farm,
    Waypoint,
    Town,
    GoldMine,
}

/// Find nearest location of a given kind (no radius limit, position only).
pub fn find_nearest_location(from: Vec2, entity_map: &crate::resources::EntityMap, kind: LocationKind) -> Option<Vec2> {
    find_location_within_radius(from, entity_map, kind, f32::MAX).map(|(_, pos)| pos)
}

/// Find nearest location of a given kind within radius. Returns (index, position).
pub fn find_location_within_radius(
    from: Vec2,
    entity_map: &crate::resources::EntityMap,
    kind: LocationKind,
    radius: f32,
) -> Option<(usize, Vec2)> {
    let is_town = kind == LocationKind::Town;
    let bkind = match kind {
        LocationKind::Farm => BuildingKind::Farm,
        LocationKind::Waypoint => BuildingKind::Waypoint,
        LocationKind::Town => BuildingKind::Fountain,
        LocationKind::GoldMine => BuildingKind::GoldMine,
    };
    let r2 = radius * radius;
    let mut best_d2 = f32::MAX;
    let mut result: Option<(usize, Vec2)> = None;
    entity_map.for_each_nearby(from, radius, |inst| {
        let matches = if is_town {
            inst.kind == BuildingKind::Fountain
        } else {
            inst.kind == bkind
        };
        if !matches { return; }
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
    fn position(&self) -> Vec2 { self.position }
    fn town_idx(&self) -> u32 { self.town_idx }
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
        entity_map.for_each_nearby(from, radius, |inst| {
            if inst.kind != kind { return; }
            if let Some(tid) = town_idx {
                if inst.town_idx != tid { return; }
            }
            if inst.occupants >= 1 { return; }
            let dx = inst.position.x - from.x;
            let dy = inst.position.y - from.y;
            let d2 = dx * dx + dy * dy;
            if d2 < best_d2 {
                best_d2 = d2;
                result = Some((inst.slot, inst.position));
            }
        });
        // Found one within this ring, or searched the whole world
        if result.is_some() || radius >= max_radius { break; }
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
    entity_map.for_each_nearby(from, radius, |inst| {
        if inst.kind != kind || inst.town_idx != town_idx { return; }
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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BuildingKind { Fountain, Bed, Waypoint, Farm, FarmerHome, ArcherHome, Tent, GoldMine, MinerHome, CrossbowHome, FighterHome, Road, Wall }

/// Rebuild building spatial grid. Only runs when BuildingGridDirtyMsg is received.
pub fn rebuild_building_grid_system(
    mut entity_map: ResMut<EntityMap>,
    mut grid_dirty: MessageReader<BuildingGridDirtyMsg>,
    grid: Res<WorldGrid>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("rebuild_grid");
    if grid.width == 0 || grid_dirty.read().count() == 0 { return; }
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

// TileSpec is now in constants.rs (part of BUILDING_REGISTRY)
pub use crate::constants::TileSpec;

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

/// Build the BUILDING_TILES array from the registry (for atlas construction).
pub fn building_tiles() -> Vec<crate::constants::TileSpec> {
    crate::constants::BUILDING_REGISTRY.iter().map(|d| d.tile).collect()
}

/// Composite tiles into a vertical strip buffer (32 x 32*layers).
/// Core logic shared by tilemap tileset and building atlas.
fn build_tile_strip(atlas: &Image, tiles: &[TileSpec], extra: &[&Image]) -> (Vec<u8>, u32) {
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

    let mut ext_counter = 0usize;
    for (layer, spec) in tiles.iter().enumerate() {
        let l = layer as u32;
        match *spec {
            TileSpec::Single(col, row) => {
                // Nearest-neighbor 2x upscale: each src pixel -> 2x2 dst pixels
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
            TileSpec::External(_path) => {
                let Some(ext) = extra.get(ext_counter).copied() else { continue; };
                ext_counter += 1;
                let ext_data = ext.data.as_ref().expect("external image has no data");
                let layer_offset = (l * out_size * out_size * 4) as usize;
                let ext_w = ext.width();
                let ext_h = ext.height();

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

    (data, layers)
}

/// Tilemap: strip -> texture_2d_array (for TilemapChunk).
pub fn build_tileset(atlas: &Image, tiles: &[TileSpec], extra: &[&Image], images: &mut Assets<Image>) -> Handle<Image> {
    let (data, layers) = build_tile_strip(atlas, tiles, extra);
    let out_size = SPRITE_SIZE as u32 * 2;
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

/// Rotate 32x32 RGBA pixel data 90° clockwise. Load-time only.
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

/// Extract a 32x32 sprite from a wider strip image at pixel offset `src_x`.
pub fn extract_sprite_32(img: &Image, src_x: u32) -> Vec<u8> {
    let iw = img.width();
    let data = img.data.as_ref().expect("image has no data");
    let mut out = vec![0u8; (32 * 32 * 4) as usize];
    for y in 0..32u32.min(img.height()) {
        for x in 0..32u32 {
            let si = ((y * iw + src_x + x) * 4) as usize;
            let di = ((y * 32 + x) * 4) as usize;
            if si + 4 <= data.len() {
                out[di..di + 4].copy_from_slice(&data[si..si + 4]);
            }
        }
    }
    out
}

/// Building atlas: strip as texture_2d (for NPC instanced shader).
/// Appends wall auto-tile variant layers (rotated E-W and corner sprites).
pub fn build_building_atlas(atlas: &Image, tiles: &[TileSpec], extra: &[&Image], images: &mut Assets<Image>) -> Handle<Image> {
    let (mut data, base_layers) = build_tile_strip(atlas, tiles, extra);
    let out_size = SPRITE_SIZE as u32 * 2; // 32
    let layer_bytes = (out_size * out_size * 4) as usize;

    // Find wall strip image: it's the External image for the Wall registry entry.
    // Count External entries before Wall to find its index in the extra slice.
    let wall_ext_idx = {
        let mut idx = 0usize;
        let mut found = None;
        for def in crate::constants::BUILDING_REGISTRY {
            if def.kind == BuildingKind::Wall {
                if matches!(def.tile, crate::constants::TileSpec::External(_)) { found = Some(idx); }
                break;
            }
            if matches!(def.tile, crate::constants::TileSpec::External(_)) { idx += 1; }
        }
        found
    };

    let total_layers = if let Some(ext_idx) = wall_ext_idx {
        if let Some(wall_img) = extra.get(ext_idx) {
            // Extract source sprites: sprite 0 (E-W) at x=0, sprite 2 (BR corner) at x=66
            let ew_sprite = extract_sprite_32(wall_img, 0);
            let br_sprite = extract_sprite_32(wall_img, 66);

            // Overwrite wall's base layer (External path stretched the full 98x32 strip)
            let wall_base = crate::constants::tileset_index(BuildingKind::Wall) as usize;
            let base_offset = wall_base * layer_bytes;
            if base_offset + layer_bytes <= data.len() {
                data[base_offset..base_offset + layer_bytes].copy_from_slice(&ew_sprite);
            }

            // Generate rotated variants
            let ns_sprite = rotate_90_cw(&ew_sprite, out_size);  // N-S straight
            let bl_sprite = rotate_90_cw(&br_sprite, out_size);  // BL corner
            let tl_sprite = rotate_90_cw(&bl_sprite, out_size);  // TL corner (180°)
            let tr_sprite = rotate_90_cw(&tl_sprite, out_size);  // TR corner (270°)

            // Extract junction/cross sprite at x=33, T-junction at x=99
            let cross_sprite = extract_sprite_32(wall_img, 33);
            let t_sprite = extract_sprite_32(wall_img, 99);
            let t_90 = rotate_90_cw(&t_sprite, out_size);
            let t_180 = rotate_90_cw(&t_90, out_size);
            let t_270 = rotate_90_cw(&t_180, out_size);

            // Append 10 extra layers: N-S, BR, BL, TL, TR, Cross, T×4
            data.extend_from_slice(&ns_sprite);
            data.extend_from_slice(&br_sprite);
            data.extend_from_slice(&bl_sprite);
            data.extend_from_slice(&tl_sprite);
            data.extend_from_slice(&tr_sprite);
            data.extend_from_slice(&cross_sprite);
            data.extend_from_slice(&t_sprite);  // open N on screen
            data.extend_from_slice(&t_90);      // open W on screen
            data.extend_from_slice(&t_180);     // open S on screen
            data.extend_from_slice(&t_270);     // open E on screen

            base_layers + crate::constants::WALL_EXTRA_LAYERS as u32
        } else {
            base_layers
        }
    } else {
        base_layers
    };

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

/// Extras atlas: composites individual 16x16 sprites into a horizontal grid (32x32 cells, 2x upscale).
/// Used for heal, sleep, arrow, boat — any single-sprite overlay. Order matches atlas_id mapping in shader.
pub fn build_extras_atlas(sprites: &[Image], images: &mut Assets<Image>) -> Handle<Image> {
    let cell = 32u32; // 2x upscale of 16px sprites
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
        Extent3d { width: cell * count, height: cell, depth_or_array_layers: 1 },
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
            cell_size: TOWN_GRID_SPACING,
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
    Classic,
    #[default]
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
            world_width: 8000.0,
            world_height: 8000.0,
            world_margin: 400.0,
            num_towns: 2,
            min_town_distance: 1200.0,
            grid_spacing: 34.0,
            raider_distance: 3500.0,
            farms_per_town: 2,
            npc_counts: NPC_REGISTRY.iter().map(|d| (d.job, d.default_count)).collect(),
            ai_towns: 1,
            raider_towns: 1,
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
    town_grids: &mut TownGrids,
    slot_alloc: &mut crate::resources::EntitySlots,
    entity_map: &mut EntityMap,
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
            // Snap to grid cell center so fountain sprite aligns with its grid cell
            let (gc, gr) = grid.world_to_grid(pos);
            let pos = grid.grid_to_world(gc, gr);
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
        place_buildings(grid, world_data, center, town_idx, config,&mut town_grids.grids[gi], false, slot_alloc, entity_map);
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
            let (gc, gr) = grid.world_to_grid(pos);
            let pos = grid.grid_to_world(gc, gr);
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
        place_buildings(grid, world_data, center, town_idx, config,&mut town_grids.grids[gi], false, slot_alloc, entity_map);
    }

    // Step 4: Place raider town centers (Raider AI, each gets unique faction)
    let mut raider_positions: Vec<Vec2> = Vec::new();
    attempts = 0;
    while raider_positions.len() < config.raider_towns && attempts < max_attempts {
        attempts += 1;
        let x = rng.random_range(config.world_margin..config.world_width - config.world_margin);
        let y = rng.random_range(config.world_margin..config.world_height - config.world_margin);
        let pos = Vec2::new(x, y);
        if is_continents {
            let (gc, gr) = grid.world_to_grid(pos);
            if grid.cell(gc, gr).is_some_and(|c| c.terrain == Biome::Water) { continue; }
        }
        if all_positions.iter().all(|e| pos.distance(*e) >= config.min_town_distance) {
            let (gc, gr) = grid.world_to_grid(pos);
            let pos = grid.grid_to_world(gc, gr);
            raider_positions.push(pos);
            all_positions.push(pos);
        }
    }

    for &center in &raider_positions {
        let faction = next_faction;
        next_faction += 1;
        world_data.towns.push(Town { name: "Raider Town".into(), center, faction, sprite_type: 1 });
        let town_data_idx = world_data.towns.len() - 1;
        let town_idx = town_data_idx as u32;
        town_grids.grids.push(TownGrid::new_base(town_data_idx));
        let gi = town_grids.grids.len() - 1;
        place_buildings(grid, world_data, center, town_idx, config,&mut town_grids.grids[gi], true, slot_alloc, entity_map);
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
        if entity_map.has_building_at(gc as i32, gr as i32) { continue; }
        let snapped = grid.grid_to_world(gc, gr);
        place_building_instance(slot_alloc, entity_map, BuildingKind::GoldMine, snapped, 0, crate::constants::FACTION_NEUTRAL, 0, 0);
        mine_positions.push(snapped);
    }

    info!("generate_world: {} player towns, {} AI towns, {} raider towns, {} gold mines, grid {}x{} ({})",
        player_positions.len(), ai_town_positions.len(), raider_positions.len(), mine_positions.len(), w, h,
        if is_continents { "continents" } else { "classic" });
}

/// Place buildings for a town. Unified builder for both AI kinds:
/// - Builder (`is_raider: false`): fountain + farms + village NPC homes + corner waypoints
/// - Raider (`is_raider: true`): fountain + raider NPC homes (tents)
pub fn place_buildings(
    grid: &mut WorldGrid,
    world_data: &WorldData,
    center: Vec2,
    town_idx: u32,
    config: &WorldGenConfig,
    town_grid: &mut TownGrid,
    is_raider: bool,
    slot_alloc: &mut crate::resources::EntitySlots,
    entity_map: &mut EntityMap,
) {
    let mut occupied = HashSet::new();
    let faction = world_data.towns.get(town_idx as usize).map(|t| t.faction).unwrap_or(0);

    // Helper: place building at town grid (row, col), return snapped world position
    let place = |row: i32, col: i32, _kind: BuildingKind, _ti: u32, occ: &mut HashSet<(i32, i32)>| -> Vec2 {
        let world_pos = town_grid_to_world(center, row, col);
        let (gc, gr) = grid.world_to_grid(world_pos);
        let snapped_pos = grid.grid_to_world(gc, gr);
        occ.insert((row, col));
        snapped_pos
    };

    // Center building at (0, 0) — Fountain
    let center_kind = BuildingKind::Fountain;
    place(0, 0, center_kind, town_idx, &mut occupied);
    place_building_instance(slot_alloc, entity_map, center_kind, center, town_idx, faction, 0, 0);

    // Count NPC homes needed (raider units for raider towns, village units for builder towns)
    let homes: usize = NPC_REGISTRY.iter()
        .filter(|d| d.is_raider_unit == is_raider)
        .map(|d| config.npc_counts.get(&d.job).copied().unwrap_or(0))
        .sum();
    let farms_count = if is_raider { 0 } else { config.farms_per_town };
    let slots = spiral_slots(&occupied, farms_count + homes);
    let mut slot_iter = slots.into_iter();

    // Farms (towns only)
    for _ in 0..farms_count {
        let Some((row, col)) = slot_iter.next() else { break };
        let pos = place(row, col, BuildingKind::Farm, town_idx, &mut occupied);
        place_building_instance(slot_alloc, entity_map, BuildingKind::Farm, pos, town_idx, faction, 0, 0);
    }

    // NPC homes from registry (filtered by is_raider_unit matching is_raider)
    for def in NPC_REGISTRY.iter().filter(|d| d.is_raider_unit == is_raider) {
        let count = config.npc_counts.get(&def.job).copied().unwrap_or(0);
        for _ in 0..count {
            let Some((row, col)) = slot_iter.next() else { break };
            let pos = place(row, col, def.home_building, town_idx, &mut occupied);
            place_building_instance(slot_alloc, entity_map, def.home_building, pos, town_idx, faction, 0, 0);
        }
    }

    // Waypoints at outer corners (towns only, clockwise patrol: TL → TR → BR → BL)
    if !is_raider {
        let (min_row, max_row, min_col, max_col) = occupied.iter().fold(
            (i32::MAX, i32::MIN, i32::MAX, i32::MIN),
            |(rmin, rmax, cmin, cmax), &(r, c)| (rmin.min(r), rmax.max(r), cmin.min(c), cmax.max(c)),
        );
        let corners = [
            (max_row + 1, min_col - 1), // TL (top-left)
            (max_row + 1, max_col + 1), // TR (top-right)
            (min_row - 1, max_col + 1), // BR (bottom-right)
            (min_row - 1, min_col - 1), // BL (bottom-left)
        ];
        for (order, (row, col)) in corners.into_iter().enumerate() {
            let post_pos = place(row, col, BuildingKind::Waypoint, town_idx, &mut occupied);
            place_building_instance(slot_alloc, entity_map, BuildingKind::Waypoint, post_pos, town_idx, faction, order as u32, 0);
        }
    }

    // Ensure generated buildings are always inside the buildable area
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

/// Fill grid terrain using simplex noise, with Dirt override near towns.
fn generate_terrain(
    grid: &mut WorldGrid,
    town_positions: &[Vec2],
    raider_positions: &[Vec2],
) {
    use noise::{NoiseFn, Simplex};

    let noise = Simplex::new(rand::random::<u32>());
    let frequency = 0.003;
    let town_clear_radius = 6.0 * grid.cell_size; // ~192px
    let raider_clear_radius = 5.0 * grid.cell_size;  // ~160px

    for row in 0..grid.height {
        for col in 0..grid.width {
            let world_pos = grid.grid_to_world(col, row);

            // Check proximity to towns → Dirt
            let near_town = town_positions.iter().any(|tc| world_pos.distance(*tc) < town_clear_radius);
            let near_raider = raider_positions.iter().any(|cp| world_pos.distance(*cp) < raider_clear_radius);

            let biome = if near_town || near_raider {
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
pub fn stamp_dirt(grid: &mut WorldGrid, positions: &[Vec2]) {
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
