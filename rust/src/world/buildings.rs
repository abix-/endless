//! Building placement, destruction, and world setup.
//!
//! - place_building: unified placement for runtime (validated) and free (worldgen/load).
//! - destroy_building: grid cleanup, auto-tile update, combat log.
//! - resolve_spawner_npc: resolve NPC job/position from a spawner building.
//! - setup_world: full world init (calls generate_world, spawns ECS entities).
//! - expand_town_build_area: grows a town's buildable ring by one level.

use bevy::prelude::*;

use crate::messages::{CombatLogMsg, DirtyWriters, GpuUpdate, GpuUpdateMsg};
use crate::resources::{
    BuildingInstance, CombatEventKind, EntityMap, FactionList, FactionStats, GameTime, GpuSlotPool,
    RaiderState, Reputation, TownIndex,
};

use super::{
    Biome, BuildingKind, LocationKind, Town, WorldData, WorldGrid,
    autotile::update_autotile_around, find_location_within_radius, find_nearest_free,
};

// ============================================================================
// SPAWNER NPC RESOLUTION
// ============================================================================

/// Resolve SpawnNpcMsg fields from a spawner entry's building_kind.
/// Uses BUILDING_REGISTRY SpawnBehavior so new buildings with existing behaviors need no changes.
/// `assigned_mine` comes from MinerHomeConfig ECS component (None for non-miner buildings).
/// Returns (job, faction, work_x, work_y, starting_post, npc_label, bld_label, work_slot).
pub fn resolve_spawner_npc(
    inst: &BuildingInstance,
    towns: &[Town],
    entity_map: &EntityMap,
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

// ============================================================================
// BUILDING PLACEMENT
// ============================================================================

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
/// With `ctx: Some(BuildContext)` -- runtime validated placement:
///   validates cell, deducts cost, starts construction, wall auto-tile, dirty signals.
/// With `ctx: None` -- free placement (world-gen, save/load, migration, tests):
///   just creates the building at full HP (or hp_override).
pub fn place_building(
    slot_alloc: &mut GpuSlotPool,
    entity_map: &mut EntityMap,
    commands: &mut Commands,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
    kind: BuildingKind,
    pos: Vec2,
    town_idx: u32,
    faction: i32,
    overrides: &BuildingOverrides,
    mut ctx: Option<BuildContext>,
    dirty_writers: Option<&mut DirtyWriters>,
) -> Result<usize, &'static str> {
    use crate::components::*;
    use crate::constants::{
        building_def, pick_variant_atlas_layer, pick_variant_for_pos, tileset_index,
    };

    let def = building_def(kind);

    // Runtime validation + cost deduction (only when BuildContext provided)
    let (snapped, gc, gr) = if let Some(ref mut ctx) = ctx {
        let (gc, gr) = ctx.grid.world_to_grid(pos);
        let snapped = ctx.grid.grid_to_world(gc, gr);

        let cell = ctx.grid.cell(gc, gr).ok_or("cell out of bounds")?;
        if entity_map.has_building_at(gc as i32, gr as i32) {
            return Err("cell already has a building");
        }
        if cell.terrain.is_impassable() {
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
                if !super::is_road_placeable_for_town(snapped, town_idx as usize, ctx.grid) {
                    return Err("road must be adjacent to town or existing road");
                }
            } else if !ctx.grid.can_town_build(gc, gr, town_idx as u16) {
                return Err("outside buildable area");
            }
        }

        // Reject placements that would fully block access to critical buildings
        if !kind.is_road() {
            if let Some(reason) =
                ctx.grid
                    .would_block_critical_access(entity_map, gc, gr, town_idx, ctx.world_data)
            {
                return Err(reason);
            }
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

    // Create BuildingInstance (identity only -- occupancy tracked separately in EntityMap)
    entity_map.add_instance(BuildingInstance {
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
    // For Pick tile kinds, select a deterministic variant from the world position.
    let tile_layer = if matches!(def.tile, crate::constants::TileSpec::Pick(_)) {
        let variant = pick_variant_for_pos(kind, snapped.x, snapped.y);
        pick_variant_atlas_layer(kind, variant).unwrap_or(tileset_index(kind) as usize) as u16
    } else {
        tileset_index(kind)
    };
    push_building_gpu_updates(
        slot,
        kind,
        snapped,
        faction,
        hp,
        tile_layer,
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

// ============================================================================
// BUILDING DESTRUCTION
// ============================================================================

/// Consolidated building destruction: grid clear + growth tombstone + HP zero + combat log.
/// Grid cleanup for building removal: clears grid cell, updates wall auto-tile, logs combat event.
/// Does NOT mark the entity as Dead -- callers send DamageMsg for that (single Dead writer: death_system).
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

    // Combat log -- derive faction from building's town_idx
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

// ============================================================================
// WORLD SETUP
// ============================================================================

/// Create AI players for all non-player towns with random personalities.
fn create_ai_players(
    world_data: &WorldData,
    faction_list: &FactionList,
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
                decision_timer: 0.0, // staggered below
            });
        } else {
            // Player town -- inactive by default, controllable from Policies tab
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
                decision_timer: 0.0,
            });
        }
    }
    // Stagger decision timers across active players so they don't all fire simultaneously.
    // Player i fires at i * interval / n, distributing load evenly across the interval.
    let n_active = players.iter().filter(|p| p.active).count();
    if n_active > 0 {
        for (slot, p) in players.iter_mut().filter(|p| p.active).enumerate() {
            p.decision_timer =
                slot as f32 * crate::constants::DEFAULT_AI_INTERVAL / n_active as f32;
        }
    }
    players
}

/// Full world setup: generate terrain/towns, init resources, buildings, spawners.
/// Buildings get ECS entities + GPU state inline via place_building.
/// Returns ai_players for the caller to insert into AiPlayerState.
pub fn setup_world(
    config: &super::worldgen::WorldGenConfig,
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    faction_list: &mut FactionList,
    slot_alloc: &mut GpuSlotPool,
    entity_map: &mut EntityMap,
    faction_stats: &mut FactionStats,
    reputation: &mut Reputation,
    raider_state: &mut RaiderState,
    town_index: &mut TownIndex,
    commands: &mut Commands,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
) -> Vec<crate::systems::AiPlayer> {
    entity_map.clear_buildings();
    // init_spatial before generate_world so buildings are inserted into spatial
    // incrementally by add_instance instead of requiring a full rebuild after.
    entity_map.init_spatial(grid.width as f32 * grid.cell_size);
    let area_levels = super::worldgen::generate_world(
        config,
        grid,
        world_data,
        faction_list,
        slot_alloc,
        entity_map,
        commands,
        gpu_updates,
    );
    grid.init_pathfind_costs();
    grid.sync_building_costs(entity_map);
    grid.sync_town_buildability(&world_data.towns, &area_levels, entity_map);

    let n_factions = faction_list.factions.len();
    faction_stats.init(n_factions);
    reputation.init(n_factions);
    raider_state.init(world_data.towns.len(), 10);

    // Spawn ECS town entities with area_levels from world gen
    super::worldgen::spawn_town_entities(
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

    let (old_min_c, old_max_c, old_min_r, old_max_r) =
        super::build_bounds(*area_level, center, grid);
    *area_level += 1;
    let (new_min_c, new_max_c, new_min_r, new_max_r) =
        super::build_bounds(*area_level, center, grid);

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
