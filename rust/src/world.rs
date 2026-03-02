//! World Data - Towns, farms, beds, waypoints, sprite definitions
//! World Grid - 2D cell grid covering entire world (terrain + buildings)
//! World Generation - Procedural town placement and building layout

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

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
    BASE_GRID_MAX, BASE_GRID_MIN, MAX_GRID_EXTENT, NPC_REGISTRY, TOWN_GRID_SPACING,
};
use crate::messages::{BuildingGridDirtyMsg, DirtyWriters};
use crate::messages::{CombatLogMsg, GpuUpdate, GpuUpdateMsg};
use crate::resources::{
    CombatEventKind, EntityMap, FactionStats, FoodStorage, GameTime, GoldStorage, GpuSlotPool,
    RaiderState,
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
    pub faction: i32,     // 0=Villager, 1+=Raider factions
    pub sprite_type: i32, // 0=fountain, 1=tent
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

// ============================================================================
// TOWN BUILDING GRID
// ============================================================================

/// Per-town building area configuration.
/// Grid uses (row, col) relative to town center with TOWN_GRID_SPACING.
/// Base grid: (-3,-3) to (3,3) = 7x7. `area_level` expands bounds by 1 per level.
pub struct TownGrid {
    pub town_data_idx: usize,
    pub area_level: i32,
    pub min_row_cap: i32,
    pub max_row_cap: i32,
    pub min_col_cap: i32,
    pub max_col_cap: i32,
}

impl TownGrid {
    /// Create with base 7x7 build area for the given town data index.
    pub fn new_base(town_data_idx: usize) -> Self {
        Self {
            town_data_idx,
            area_level: 0,
            min_row_cap: -MAX_GRID_EXTENT,
            max_row_cap: MAX_GRID_EXTENT + 1,
            min_col_cap: -MAX_GRID_EXTENT,
            max_col_cap: MAX_GRID_EXTENT + 1,
        }
    }

    /// Recompute world-edge caps for this town in town-grid coordinates.
    pub fn recompute_world_caps(&mut self, center: Vec2, grid: &WorldGrid) {
        if grid.width == 0 || grid.height == 0 {
            return;
        }
        let (center_col, center_row) = grid.world_to_grid(center);
        let center_col = center_col as i32;
        let center_row = center_row as i32;
        self.min_row_cap = -center_row;
        self.max_row_cap = grid.height as i32 - 1 - center_row;
        self.min_col_cap = -center_col;
        self.max_col_cap = grid.width as i32 - 1 - center_col;
    }
}

/// All town building grids. One per town (villager and raider towns).
#[derive(Resource, Default)]
pub struct TownGrids {
    pub grids: Vec<TownGrid>,
}

/// Recompute world-edge caps for all town grids.
pub fn sync_town_grid_world_caps(grid: &WorldGrid, towns: &[Town], town_grids: &mut TownGrids) {
    for tg in &mut town_grids.grids {
        let Some(town) = towns.get(tg.town_data_idx) else {
            continue;
        };
        tg.recompute_world_caps(town.center, grid);
    }
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
    let min_row = (BASE_GRID_MIN - grid.area_level).max(grid.min_row_cap);
    let max_row = (BASE_GRID_MAX + grid.area_level).min(grid.max_row_cap);
    let min_col = (BASE_GRID_MIN - grid.area_level).max(grid.min_col_cap);
    let max_col = (BASE_GRID_MAX + grid.area_level).min(grid.max_col_cap);
    (min_row, max_row, min_col, max_col)
}

/// True if (row, col) is currently inside this town's buildable area.
pub fn is_slot_buildable(grid: &TownGrid, row: i32, col: i32) -> bool {
    let (min_row, max_row, min_col, max_col) = build_bounds(grid);
    row >= min_row && row <= max_row && col >= min_col && col <= max_col
}

/// Returns true if world_pos falls inside any town's build area OTHER than own_town_idx.
pub fn in_foreign_build_area(
    pos: Vec2,
    own_town_idx: usize,
    towns: &[Town],
    town_grids: &TownGrids,
) -> bool {
    for tg in &town_grids.grids {
        if tg.town_data_idx == own_town_idx {
            continue;
        }
        let Some(town) = towns.get(tg.town_data_idx) else {
            continue;
        };
        let (row, col) = world_to_town_grid(town.center, pos);
        if is_slot_buildable(tg, row, col) {
            return true;
        }
    }
    false
}

/// All empty buildable slots in a town grid (excludes center 0,0).
pub fn empty_slots(
    tg: &TownGrid,
    center: Vec2,
    grid: &WorldGrid,
    entity_map: &crate::resources::EntityMap,
) -> Vec<(i32, i32)> {
    let (min_row, max_row, min_col, max_col) = build_bounds(tg);
    let mut out = Vec::new();
    for r in min_row..=max_row {
        for c in min_col..=max_col {
            if r == 0 && c == 0 {
                continue;
            }
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
pub fn find_town_slot(world_pos: Vec2, towns: &[Town], grids: &TownGrids) -> Option<TownSlotInfo> {
    for (grid_idx, town_grid) in grids.grids.iter().enumerate() {
        let town_data_idx = town_grid.town_data_idx;
        if town_data_idx >= towns.len() {
            continue;
        }
        let town = &towns[town_data_idx];

        let (row, col) = world_to_town_grid(town.center, world_pos);

        // Check click is within reasonable range of this grid's slots
        let slot_pos = town_grid_to_world(town.center, row, col);
        let click_radius = TOWN_GRID_SPACING * 0.7;
        if world_pos.distance(slot_pos) > click_radius {
            continue;
        }

        if is_slot_buildable(town_grid, row, col) {
            return Some(TownSlotInfo {
                grid_idx,
                town_data_idx,
                row,
                col,
            });
        }
    }
    None
}

/// Info about a clicked town grid slot.
pub struct TownSlotInfo {
    pub grid_idx: usize,      // Index into TownGrids.grids
    pub town_data_idx: usize, // Index into WorldData.towns
    pub row: i32,
    pub col: i32,
}

// ============================================================================
// BUILDING PLACEMENT / REMOVAL
// ============================================================================


/// Check if a grid cell contains a building of the given kind.
fn is_kind_at(entity_map: &EntityMap, col: usize, row: usize, kind: BuildingKind) -> bool {
    entity_map
        .get_at_grid(col as i32, row as i32)
        .is_some_and(|inst| inst.kind == kind)
}

/// Compute auto-tile variant (0-10) for a building at grid (col, row).
/// Uses 4-neighbor NSEW matching. Works for any autotile-enabled building kind.
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
        if !is_kind_at(entity_map, c, r, kind) {
            continue;
        }
        let variant = autotile_variant(entity_map, c, r, kind);
        let pos = grid.grid_to_world(c, r);
        if let Some(inst) = entity_map.find_by_position(pos) {
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame {
                idx: inst.slot,
                col: crate::constants::autotile_col(kind, variant),
                row: 0.0,
                atlas: crate::constants::ATLAS_BUILDING,
            }));
        }
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
/// Returns (job, faction, work_x, work_y, starting_post, attack_type, npc_label, bld_label, work_slot).
pub fn resolve_spawner_npc(
    inst: &crate::resources::BuildingInstance,
    towns: &[Town],
    entity_map: &crate::resources::EntityMap,
) -> (
    i32,
    i32,
    f32,
    f32,
    i32,
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
        return (
            2,
            raider_faction,
            -1.0,
            -1.0,
            -1,
            0,
            "Raider",
            "Unknown",
            None,
        );
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
                spawner.attack_type,
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
                spawner.attack_type,
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
                spawner.attack_type,
                npc_label,
                def.label,
                None,
            )
        }
        SpawnBehavior::Miner => {
            let (work_slot, mine) = if let Some(pos) = inst.assigned_mine {
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
                spawner.attack_type,
                npc_label,
                def.label,
                work_slot,
            )
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
    let half = if kind == BuildingKind::Road {
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
    pub food_storage: &'a mut FoodStorage,
    pub town_grids: &'a TownGrids,
    pub cost: i32,
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
    uid_alloc: &mut crate::resources::NextEntityUid,
    commands: &mut Commands,
    gpu_updates: &mut MessageWriter<crate::messages::GpuUpdateMsg>,
    kind: BuildingKind,
    pos: Vec2,
    town_idx: u32,
    faction: i32,
    patrol_order: u32,
    wall_level: u8,
    uid_override: Option<crate::components::EntityUid>,
    hp_override: Option<f32>,
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
        if cell.terrain == Biome::Water {
            return Err("cannot build on water");
        }
        if in_foreign_build_area(snapped, town_idx as usize, &ctx.world_data.towns, ctx.town_grids)
        {
            return Err("cannot build in foreign territory");
        }

        let food = ctx
            .food_storage
            .food
            .get_mut(town_idx as usize)
            .ok_or("invalid town")?;
        if *food < ctx.cost {
            return Err("not enough food");
        }
        *food -= ctx.cost;

        (snapped, gc, gr)
    } else {
        (pos, 0, 0) // gc/gr unused when no ctx
    };

    // Alloc GPU slot
    let Some(slot) = slot_alloc.alloc_reset() else {
        warn!("No building slots available for {:?}", kind);
        return Err("no GPU slots available");
    };

    // Create BuildingInstance
    let uid = uid_override.unwrap_or_else(|| uid_alloc.next());
    let has_spawner = def.spawner.is_some();
    entity_map.add_instance(crate::resources::BuildingInstance {
        kind,
        position: snapped,
        town_idx,
        slot,
        faction,
        patrol_order,
        assigned_mine: None,
        manual_mine: false,
        wall_level,
        npc_uid: None,
        respawn_timer: if has_spawner { 0.0 } else { -2.0 },
        growth_ready: false,
        growth_progress: 0.0,
        occupants: 0,
        under_construction: 0.0,
        kills: 0,
        xp: 0,
        upgrade_levels: Vec::new(),
        auto_upgrade_flags: Vec::new(),
    });
    entity_map.register_uid_slot_only(slot, uid);

    // Runtime: set construction timer + suppress spawner
    let hp = if ctx.is_some() {
        let inst = entity_map.get_instance_mut(slot).unwrap();
        inst.under_construction = crate::constants::BUILDING_CONSTRUCT_SECS;
        if inst.respawn_timer >= 0.0 {
            inst.respawn_timer = -1.0;
        }
        hp_override.unwrap_or(0.01)
    } else {
        hp_override.unwrap_or(def.hp)
    };

    // Spawn ECS entity
    let ecmds = commands.spawn((
        GpuSlot(slot),
        Position::new(snapped.x, snapped.y),
        Health(hp),
        Faction(faction),
        TownId(town_idx as i32),
        Building { kind },
        uid,
    ));
    let entity = ecmds.id();
    entity_map.entities.insert(slot, entity);
    entity_map.bind_uid_entity(uid, entity);

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
    town_grids: &TownGrids,
) -> Vec<crate::systems::AiPlayer> {
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
    for (grid_idx, tg) in town_grids.grids.iter().enumerate() {
        let tdi = tg.town_data_idx;
        if let Some(town) = world_data.towns.get(tdi) {
            if town.faction > 0 {
                let kind = if town.sprite_type == 1 {
                    AiKind::Raider
                } else {
                    AiKind::Builder
                };
                let personality = personalities[rng.random_range(0..personalities.len())];
                let road_style = RoadStyle::random(&mut rng);
                players.push(AiPlayer {
                    town_data_idx: tdi,
                    grid_idx,
                    kind,
                    personality,
                    road_style,
                    last_actions: std::collections::VecDeque::new(),
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
                    grid_idx,
                    kind: AiKind::Builder,
                    personality: AiPersonality::Balanced,
                    road_style: RoadStyle::Grid4,
                    last_actions: std::collections::VecDeque::new(),
                    active: false,
                    build_enabled: true,
                    upgrade_enabled: true,
                    squad_indices: Vec::new(),
                    squad_cmd: std::collections::HashMap::new(),
                });
            }
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
    town_grids: &mut TownGrids,
    slot_alloc: &mut GpuSlotPool,
    entity_map: &mut EntityMap,
    food_storage: &mut FoodStorage,
    gold_storage: &mut GoldStorage,
    faction_stats: &mut FactionStats,
    reputation: &mut crate::resources::Reputation,
    raider_state: &mut RaiderState,
    uid_alloc: &mut crate::resources::NextEntityUid,
    commands: &mut Commands,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) -> Vec<crate::systems::AiPlayer> {
    town_grids.grids.clear();
    entity_map.clear_buildings();
    generate_world(
        config, grid, world_data, town_grids, slot_alloc, entity_map, uid_alloc,
        commands, gpu_updates,
    );
    entity_map.init_spatial(grid.width as f32 * grid.cell_size);

    let n = world_data.towns.len();
    food_storage.init(n);
    gold_storage.init(n);
    faction_stats.init(n);
    reputation.init(n);
    raider_state.init(n, 10);

    create_ai_players(world_data, town_grids)
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
            let is_old = row >= old_min_row
                && row <= old_max_row
                && col >= old_min_col
                && col <= old_max_col;
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
    row: i32,
    col: i32,
    town_center: Vec2,
    reason: &str,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) -> Result<(), &'static str> {
    let world_pos = town_grid_to_world(town_center, row, col);
    let (gc, gr) = grid.world_to_grid(world_pos);

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
        if !matches {
            return;
        }
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
        entity_map.for_each_nearby(from, radius, |inst| {
            if inst.kind != kind {
                return;
            }
            if let Some(tid) = town_idx {
                if inst.town_idx != tid {
                    return;
                }
            }
            if inst.occupants >= 1 {
                return;
            }
            let dx = inst.position.x - from.x;
            let dy = inst.position.y - from.y;
            let d2 = dx * dx + dy * dy;
            if d2 < best_d2 {
                best_d2 = d2;
                result = Some((inst.slot, inst.position));
            }
        });
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
    entity_map.for_each_nearby(from, radius, |inst| {
        if inst.kind != kind || inst.town_idx != town_idx {
            return;
        }
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
    Wall,
    Tower,
    Merchant,
    Casino,
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
    /// Map biome + cell index to tileset array index (0-10) for TilemapChunk.
    /// Grass alternates 0/1, Forest cycles 2-7, Water=8, Rock=9, Dirt=10.
    pub fn tileset_index(self, cell_index: usize) -> u16 {
        match self {
            Biome::Grass => {
                if cell_index % 2 == 0 {
                    0
                } else {
                    1
                }
            }
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

/// Build the BUILDING_TILES array from the registry (for atlas construction).
pub fn building_tiles() -> Vec<crate::constants::TileSpec> {
    crate::constants::BUILDING_REGISTRY
        .iter()
        .map(|d| d.tile)
        .collect()
}

/// Composite tiles into a vertical strip buffer (ATLAS_CELL x ATLAS_CELL*layers).
/// Core logic shared by tilemap tileset and building atlas.
fn build_tile_strip(atlas: &Image, tiles: &[TileSpec], extra: &[&Image]) -> (Vec<u8>, u32) {
    let sprite = SPRITE_SIZE as u32; // 16 (source texel size)
    let out_size = ATLAS_CELL; // 64
    let scale = out_size / sprite; // 4x upscale from 16px source
    let half = out_size / 2; // 32 — each quadrant in a Quad tile
    let cell_size = CELL as u32; // 17 (16px + 1px margin in source sheet)
    let atlas_width = atlas.width();
    let layers = tiles.len() as u32;

    let mut data = vec![0u8; (out_size * out_size * layers * 4) as usize];
    let atlas_data = atlas.data.as_ref().expect("atlas image has no data");

    // Blit a 16x16 source sprite with 2x upscale into a 32x32 quadrant at (dx, dy)
    let blit_2x = |data: &mut [u8], layer: u32, col: u32, row: u32, dx: u32, dy: u32| {
        let src_x = col * cell_size;
        let src_y = row * cell_size;
        for ty in 0..sprite {
            for tx in 0..sprite {
                let si = ((src_y + ty) * atlas_width + (src_x + tx)) as usize * 4;
                for oy in 0..2u32 {
                    for ox in 0..2u32 {
                        let di = (layer * out_size * out_size
                            + (dy + ty * 2 + oy) * out_size
                            + (dx + tx * 2 + ox))
                            as usize
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
                // Each 16px quadrant is 2x upscaled to 32px, filling the 64px cell
                blit_2x(&mut data, l, q[0].0, q[0].1, 0, 0); // TL
                blit_2x(&mut data, l, q[1].0, q[1].1, half, 0); // TR
                blit_2x(&mut data, l, q[2].0, q[2].1, 0, half); // BL
                blit_2x(&mut data, l, q[3].0, q[3].1, half, half); // BR
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
    let (data, layers) = build_tile_strip(atlas, tiles, extra);
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
    let (mut data, base_layers) = build_tile_strip(atlas, tiles, extra);
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
        let Some(strip_img) = extra.get(ext_idx) else { continue };

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
    slot_alloc: &mut crate::resources::GpuSlotPool,
    entity_map: &mut EntityMap,
    uid_alloc: &mut crate::resources::NextEntityUid,
    commands: &mut Commands,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
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
            if grid.cell(gc, gr).is_some_and(|c| c.terrain == Biome::Water) {
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
            player_positions.push(pos);
            all_positions.push(pos);
        }
    }

    if player_positions.len() < config.num_towns {
        warn!(
            "generate_world: only placed {}/{} player towns",
            player_positions.len(),
            config.num_towns
        );
    }

    // Register player towns
    for &center in &player_positions {
        let name = names
            .get(name_idx)
            .cloned()
            .unwrap_or_else(|| format!("Town {}", name_idx));
        name_idx += 1;
        world_data.towns.push(Town {
            name,
            center,
            faction: 0,
            sprite_type: 0,
        });
        let town_data_idx = world_data.towns.len() - 1;
        let town_idx = town_data_idx as u32;
        let mut tg = TownGrid::new_base(town_data_idx);
        tg.recompute_world_caps(center, grid);
        town_grids.grids.push(tg);
        let gi = town_grids.grids.len() - 1;
        place_buildings(
            grid,
            world_data,
            center,
            town_idx,
            config,
            &mut town_grids.grids[gi],
            false,
            slot_alloc,
            entity_map,
            uid_alloc,
            commands,
            gpu_updates,
        );
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
            if grid.cell(gc, gr).is_some_and(|c| c.terrain == Biome::Water) {
                continue;
            }
        }
        if all_positions
            .iter()
            .all(|e| pos.distance(*e) >= config.min_town_distance)
        {
            let (gc, gr) = grid.world_to_grid(pos);
            let pos = grid.grid_to_world(gc, gr);
            ai_town_positions.push(pos);
            all_positions.push(pos);
        }
    }

    for &center in &ai_town_positions {
        let name = names
            .get(name_idx)
            .cloned()
            .unwrap_or_else(|| format!("AI Town {}", name_idx));
        name_idx += 1;
        let faction = next_faction;
        next_faction += 1;
        world_data.towns.push(Town {
            name,
            center,
            faction,
            sprite_type: 0,
        });
        let town_data_idx = world_data.towns.len() - 1;
        let town_idx = town_data_idx as u32;
        let mut tg = TownGrid::new_base(town_data_idx);
        tg.recompute_world_caps(center, grid);
        town_grids.grids.push(tg);
        let gi = town_grids.grids.len() - 1;
        place_buildings(
            grid,
            world_data,
            center,
            town_idx,
            config,
            &mut town_grids.grids[gi],
            false,
            slot_alloc,
            entity_map,
            uid_alloc,
            commands,
            gpu_updates,
        );
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
            if grid.cell(gc, gr).is_some_and(|c| c.terrain == Biome::Water) {
                continue;
            }
        }
        if all_positions
            .iter()
            .all(|e| pos.distance(*e) >= config.min_town_distance)
        {
            let (gc, gr) = grid.world_to_grid(pos);
            let pos = grid.grid_to_world(gc, gr);
            raider_positions.push(pos);
            all_positions.push(pos);
        }
    }

    for &center in &raider_positions {
        let faction = next_faction;
        next_faction += 1;
        world_data.towns.push(Town {
            name: "Raider Town".into(),
            center,
            faction,
            sprite_type: 1,
        });
        let town_data_idx = world_data.towns.len() - 1;
        let town_idx = town_data_idx as u32;
        let mut tg = TownGrid::new_base(town_data_idx);
        tg.recompute_world_caps(center, grid);
        town_grids.grids.push(tg);
        let gi = town_grids.grids.len() - 1;
        place_buildings(
            grid,
            world_data,
            center,
            town_idx,
            config,
            &mut town_grids.grids[gi],
            true,
            slot_alloc,
            entity_map,
            uid_alloc,
            commands,
            gpu_updates,
        );
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
            if grid.cell(gc, gr).is_some_and(|c| c.terrain == Biome::Water) {
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
            slot_alloc, entity_map, uid_alloc, commands, gpu_updates,
            BuildingKind::GoldMine, snapped, 0, crate::constants::FACTION_NEUTRAL,
            0, 0, None, None, None, None,
        );
        mine_positions.push(snapped);
    }

    info!(
        "generate_world: {} player towns, {} AI towns, {} raider towns, {} gold mines, grid {}x{} ({})",
        player_positions.len(),
        ai_town_positions.len(),
        raider_positions.len(),
        mine_positions.len(),
        w,
        h,
        if is_continents {
            "continents"
        } else {
            "classic"
        }
    );
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
    slot_alloc: &mut crate::resources::GpuSlotPool,
    entity_map: &mut EntityMap,
    uid_alloc: &mut crate::resources::NextEntityUid,
    commands: &mut Commands,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) {
    let mut occupied = HashSet::new();
    let faction = world_data
        .towns
        .get(town_idx as usize)
        .map(|t| t.faction)
        .unwrap_or(0);

    // Helper: place building at town grid (row, col), return snapped world position
    let place = |row: i32,
                 col: i32,
                 _kind: BuildingKind,
                 _ti: u32,
                 occ: &mut HashSet<(i32, i32)>|
     -> Vec2 {
        let world_pos = town_grid_to_world(center, row, col);
        let (gc, gr) = grid.world_to_grid(world_pos);
        let snapped_pos = grid.grid_to_world(gc, gr);
        occ.insert((row, col));
        snapped_pos
    };

    // Center building at (0, 0) — Fountain
    let center_kind = BuildingKind::Fountain;
    place(0, 0, center_kind, town_idx, &mut occupied);
    let _ = place_building(
        slot_alloc, entity_map, uid_alloc, commands, gpu_updates,
        center_kind, center, town_idx, faction, 0, 0, None, None, None, None,
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
            slot_alloc, entity_map, uid_alloc, commands, gpu_updates,
            BuildingKind::Farm, pos, town_idx, faction, 0, 0, None, None, None, None,
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
                slot_alloc, entity_map, uid_alloc, commands, gpu_updates,
                def.home_building, pos, town_idx, faction, 0, 0, None, None, None, None,
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
                slot_alloc, entity_map, uid_alloc, commands, gpu_updates,
                BuildingKind::Waypoint, post_pos, town_idx, faction, order as u32, 0,
                None, None, None, None,
            );
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
                    world_pos.x as f64 * frequency as f64,
                    world_pos.y as f64 * frequency as f64,
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
    // Collect road slots for this town (can't mutate while iterating)
    let road_slots: Vec<usize> = entity_map
        .iter_kind_for_town(BuildingKind::Road, town_idx)
        .map(|inst| inst.slot)
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
