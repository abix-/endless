//! Economy systems - Game time, population tracking, farm growth, raider town foraging, respawning

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use rand::Rng;
use std::collections::{HashMap, HashSet};

use crate::components::*;
use crate::resources::*;
use crate::systemparams::{EconomyState, WorldState};
use crate::constants::{FARM_BASE_GROWTH_RATE, FARM_TENDED_GROWTH_RATE, RAIDER_FORAGE_RATE, STARVING_SPEED_MULT, SPAWNER_RESPAWN_HOURS,
    RAIDER_SPAWN_CHECK_HOURS, MAX_RAIDER_TOWNS, RAIDER_SETTLE_RADIUS, MIGRATION_BASE_SIZE, VILLAGERS_PER_RAIDER,
};
use crate::world::{self, WorldData, WorldGrid, BuildingKind, BuildingOccupancy, BuildingSpatialGrid, TownGrids};
use crate::messages::{SpawnNpcMsg, GpuUpdate, GpuUpdateMsg};
use crate::systems::stats::{TownUpgrades, UPGRADES};
use crate::constants::UpgradeStatKind;
use crate::systems::ai_player::{AiPlayer, AiPlayerState, AiKind, AiPersonality};

// ============================================================================
// POPULATION TRACKING HELPERS
// ============================================================================

/// Increment alive count for a (job, clan) pair.
pub fn pop_inc_alive(stats: &mut PopulationStats, job: Job, clan: i32) {
    let key = (job as i32, clan);
    stats.0.entry(key).or_default().alive += 1;
}

/// Decrement alive count for a (job, clan) pair.
pub fn pop_dec_alive(stats: &mut PopulationStats, job: Job, clan: i32) {
    let key = (job as i32, clan);
    if let Some(entry) = stats.0.get_mut(&key) {
        entry.alive = (entry.alive - 1).max(0);
    }
}

/// Increment working count for a (job, clan) pair.
pub fn pop_inc_working(stats: &mut PopulationStats, job: Job, clan: i32) {
    let key = (job as i32, clan);
    stats.0.entry(key).or_default().working += 1;
}

/// Decrement working count for a (job, clan) pair.
pub fn pop_dec_working(stats: &mut PopulationStats, job: Job, clan: i32) {
    let key = (job as i32, clan);
    if let Some(entry) = stats.0.get_mut(&key) {
        entry.working = (entry.working - 1).max(0);
    }
}

/// Increment dead count for a (job, clan) pair.
pub fn pop_inc_dead(stats: &mut PopulationStats, job: Job, clan: i32) {
    let key = (job as i32, clan);
    stats.0.entry(key).or_default().dead += 1;
}

// ============================================================================
// GAME TIME SYSTEM
// ============================================================================

/// Advances game time based on delta and time_scale.
/// Sets hour_ticked = true when the hour changes (for hourly systems).
pub fn game_time_system(
    time: Res<Time>,
    mut game_time: ResMut<GameTime>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("game_time");
    // Reset tick flag each frame
    game_time.hour_ticked = false;

    if game_time.paused {
        return;
    }

    let dt = game_time.delta(&time);
    game_time.total_seconds += dt;

    // Check if hour changed
    let current_hour = game_time.total_hours();
    if current_hour > game_time.last_hour {
        game_time.last_hour = current_hour;
        game_time.hour_ticked = true;
    }
}

// ============================================================================
// GROWTH SYSTEM (farms + mines)
// ============================================================================

/// Unified growth system for farms and mines.
/// - Farms: passive + tended rates (unchanged). Upgrade-scaled by FarmYield.
/// - Mines: tended-only (MINE_TENDED_GROWTH_RATE). Zero growth when unoccupied.
pub fn growth_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut growth_states: ResMut<GrowthStates>,
    farm_occupancy: Res<BuildingOccupancy>,
    upgrades: Res<TownUpgrades>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("growth");
    if game_time.paused { return; }

    let hours_elapsed = game_time.delta(&time) / game_time.seconds_per_hour;

    for i in 0..growth_states.states.len() {
        if growth_states.positions[i].x < -9000.0 { continue; } // tombstoned
        if growth_states.states[i] != FarmGrowthState::Growing { continue; }

        let is_tended = farm_occupancy.is_occupied(growth_states.positions[i]);

        let growth_rate = match growth_states.kinds[i] {
            GrowthKind::Farm => {
                let base_rate = if is_tended { FARM_TENDED_GROWTH_RATE } else { FARM_BASE_GROWTH_RATE };
                let town = growth_states.town_indices[i].unwrap_or(0) as usize;
                let town_levels = upgrades.town_levels(town);
                base_rate * UPGRADES.stat_mult(&town_levels, "Farmer", UpgradeStatKind::Yield)
            }
            GrowthKind::Mine => {
                let worker_count = farm_occupancy.count(growth_states.positions[i]);
                if worker_count > 0 {
                    crate::constants::MINE_TENDED_GROWTH_RATE * crate::constants::mine_productivity_mult(worker_count)
                } else {
                    0.0
                }
            }
        };

        if growth_rate > 0.0 {
            growth_states.progress[i] += growth_rate * hours_elapsed;
            if growth_states.progress[i] >= 1.0 {
                growth_states.states[i] = FarmGrowthState::Ready;
                growth_states.progress[i] = 1.0;
            }
        }
    }
}

// ============================================================================
// RAIDER FORAGING SYSTEM
// ============================================================================

/// Raider foraging: each raider town gains RAIDER_FORAGE_RATE food per hour.
/// Only runs when game_time.hour_ticked is true.
pub fn raider_forage_system(
    game_time: Res<GameTime>,
    mut economy: EconomyState,
    world_data: Res<WorldData>,
    user_settings: Res<crate::settings::UserSettings>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("raider_forage");
    if !game_time.hour_ticked || !user_settings.raider_passive_forage {
        return;
    }

    // Add foraging food to each raider town (faction > 0)
    for (town_idx, town) in world_data.towns.iter().enumerate() {
        if town.faction > 0 && town_idx < economy.food_storage.food.len() {
            economy.food_storage.food[town_idx] += RAIDER_FORAGE_RATE;
        }
    }
}

// ============================================================================
// STARVATION SYSTEM
// ============================================================================

/// Starvation check: NPCs with zero energy become Starving.
/// Only runs when game_time.hour_ticked is true.
/// Starving NPCs have 50% speed.
pub fn starvation_system(
    mut commands: Commands,
    game_time: Res<GameTime>,
    query: Query<(Entity, &NpcIndex, &Energy, &CachedStats, Option<&Starving>), Without<Dead>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("starvation");
    if !game_time.hour_ticked {
        return;
    }

    for (entity, npc_idx, energy, cached, starving) in query.iter() {
        let idx = npc_idx.0;

        if energy.0 <= 0.0 {
            if starving.is_none() {
                commands.entity(entity).insert(Starving);
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx, speed: cached.speed * STARVING_SPEED_MULT }));
            }
        } else if starving.is_some() {
            commands.entity(entity).remove::<Starving>();
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx, speed: cached.speed }));
        }
    }
}

// ============================================================================
// FARM VISUAL SYSTEM
// ============================================================================

/// Spawns/despawns FarmReadyMarker entities when farm state transitions.
/// Growing→Ready: spawn marker. Ready→Growing (harvest): despawn marker.
pub fn farm_visual_system(
    mut commands: Commands,
    growth_states: Res<GrowthStates>,
    world_data: Res<crate::world::WorldData>,
    markers: Query<(Entity, &FarmReadyMarker)>,
    mut prev_states: Local<Vec<FarmGrowthState>>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("farm_visual");
    // Only process farm entries (first N entries matching WorldData.farms)
    let farm_count = world_data.farms().len();
    prev_states.resize(farm_count, FarmGrowthState::Growing);
    for farm_idx in 0..farm_count.min(growth_states.states.len()) {
        let state = &growth_states.states[farm_idx];
        let prev = prev_states[farm_idx];
        if *state == FarmGrowthState::Ready && prev == FarmGrowthState::Growing {
            if world_data.farms().get(farm_idx).is_some() {
                commands.spawn(FarmReadyMarker { farm_idx });
            }
        } else if *state == FarmGrowthState::Growing && prev == FarmGrowthState::Ready {
            for (entity, marker) in markers.iter() {
                if marker.farm_idx == farm_idx {
                    commands.entity(entity).despawn();
                }
            }
        }
        prev_states[farm_idx] = *state;
    }
}

// ============================================================================
// BUILDING SPAWNER SYSTEM
// ============================================================================

/// Detects dead NPCs linked to House/Barracks/Tent buildings, counts down respawn timers,
/// and spawns replacements via SlotAllocator + SpawnNpcMsg.
/// Only runs when game_time.hour_ticked is true.
pub fn spawner_respawn_system(
    game_time: Res<GameTime>,
    mut spawner_state: ResMut<SpawnerState>,
    npc_map: Res<NpcEntityMap>,
    mut slots: ResMut<SlotAllocator>,
    mut spawn_writer: MessageWriter<SpawnNpcMsg>,
    world_data: Res<WorldData>,
    mut combat_log: ResMut<CombatLog>,
    farm_occupancy: Res<BuildingOccupancy>,
    timings: Res<SystemTimings>,
    bgrid: Res<BuildingSpatialGrid>,
    mut dirty: ResMut<DirtyFlags>,
) {
    let _t = timings.scope("spawner_respawn");
    if !game_time.hour_ticked {
        return;
    }

    for entry in spawner_state.0.iter_mut() {
        // Skip tombstoned entries (building was destroyed)
        if entry.position.x < -9000.0 {
            continue;
        }

        // Check if linked NPC died
        if entry.npc_slot >= 0 {
            if !npc_map.0.contains_key(&(entry.npc_slot as usize)) {
                entry.npc_slot = -1;
                entry.respawn_timer = SPAWNER_RESPAWN_HOURS;
                if entry.building_kind == crate::constants::tileset_index(BuildingKind::MinerHome) as i32 {
                    dirty.mining = true;
                }
            }
        }

        // Count down respawn timer (>= 0.0 catches newly-built spawners at 0.0)
        if entry.respawn_timer >= 0.0 {
            entry.respawn_timer -= 1.0;
            if entry.respawn_timer <= 0.0 {
                // Spawn replacement NPC
                let Some(slot) = slots.alloc() else { continue };
                let town_data_idx = entry.town_idx as usize;

                let (job, faction, work_x, work_y, starting_post, attack_type, job_name, building_name) =
                    world::resolve_spawner_npc(entry, &world_data.towns, &bgrid, &farm_occupancy, world_data.miner_homes());

                // Home = spawner building position (house/barracks/tent)
                let (home_x, home_y) = (entry.position.x, entry.position.y);

                spawn_writer.write(SpawnNpcMsg {
                    slot_idx: slot,
                    x: entry.position.x,
                    y: entry.position.y,
                    job,
                    faction,
                    town_idx: town_data_idx as i32,
                    home_x,
                    home_y,
                    work_x,
                    work_y,
                    starting_post,
                    attack_type,
                });
                entry.npc_slot = slot as i32;
                entry.respawn_timer = -1.0;
                if entry.building_kind == crate::constants::tileset_index(BuildingKind::MinerHome) as i32 {
                    dirty.mining = true;
                }

                combat_log.push(
                    CombatEventKind::Spawn, faction,
                    game_time.day(), game_time.hour(), game_time.minute(),
                    format!("{} respawned from {}", job_name, building_name),
                );
            }
        }
    }
}

/// Rebuild auto-mining discovery + assignments when mining topology/policy changes.
pub fn mining_policy_system(
    mut world_data: ResMut<WorldData>,
    policies: Res<TownPolicies>,
    spawner_state: Res<SpawnerState>,
    npc_map: Res<NpcEntityMap>,
    mut mining: ResMut<MiningPolicy>,
    mut dirty: ResMut<DirtyFlags>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("mining_policy");
    if !dirty.mining { return; }
    dirty.mining = false;

    mining.discovered_mines.resize(world_data.towns.len(), Vec::new());
    if mining.mine_enabled.len() < world_data.gold_mines().len() {
        mining.mine_enabled.resize(world_data.gold_mines().len(), true);
    }

    for town_idx in 0..world_data.towns.len() {
        let town = &world_data.towns[town_idx];
        if town.faction < 0 {
            mining.discovered_mines[town_idx].clear();
            continue;
        }
        let radius = policies.policies
            .get(town_idx)
            .map(|p| p.mining_radius)
            .unwrap_or(crate::constants::DEFAULT_MINING_RADIUS);
        let r2 = radius * radius;

        let mut discovered = Vec::new();
        for (mine_idx, mine) in world_data.gold_mines().iter().enumerate() {
            let d = mine.position - town.center;
            if d.length_squared() <= r2 {
                discovered.push(mine_idx);
            }
        }
        mining.discovered_mines[town_idx] = discovered;
    }

    for town_idx in 0..world_data.towns.len() {
        if world_data.towns[town_idx].faction < 0 { continue; }

        let enabled_mines: Vec<usize> = mining.discovered_mines[town_idx]
            .iter()
            .copied()
            .filter(|&mi| mi < mining.mine_enabled.len() && mining.mine_enabled[mi])
            .collect();

        let enabled_positions: Vec<Vec2> = enabled_mines.iter()
            .filter_map(|&mi| world_data.gold_mines().get(mi).map(|m| m.position))
            .collect();

        let mut auto_homes: Vec<usize> = Vec::new();
        for entry in spawner_state.0.iter() {
            if entry.building_kind != 3 || entry.town_idx != town_idx as i32 || entry.npc_slot < 0 {
                continue;
            }
            if !npc_map.0.contains_key(&(entry.npc_slot as usize)) {
                continue;
            }
            let Some(mh_idx) = world_data.miner_home_at(entry.position) else {
                continue;
            };
            if world_data.miner_homes()[mh_idx].manual_mine {
                continue;
            }
            auto_homes.push(mh_idx);
        }

        for &mh_idx in &auto_homes {
            let Some(mh) = world_data.miner_homes().get(mh_idx) else { continue };
            if let Some(pos) = mh.assigned_mine {
                let still_enabled = enabled_positions.iter().any(|p| (*p - pos).length() < 1.0);
                if !still_enabled {
                    // clear stale assignment if disabled or no longer discovered
                    if let Some(mh_mut) = world_data.miner_homes_mut().get_mut(mh_idx) {
                        mh_mut.assigned_mine = None;
                    }
                }
            }
        }

        if enabled_positions.is_empty() {
            for &mh_idx in &auto_homes {
                if let Some(mh_mut) = world_data.miner_homes_mut().get_mut(mh_idx) {
                    mh_mut.assigned_mine = None;
                }
            }
            continue;
        }

        for (i, &mh_idx) in auto_homes.iter().enumerate() {
            let mine_pos = enabled_positions[i % enabled_positions.len()];
            if let Some(mh_mut) = world_data.miner_homes_mut().get_mut(mh_idx) {
                mh_mut.assigned_mine = Some(mine_pos);
            }
        }
    }
}

/// Remove dead NPCs from squad member lists, auto-recruit to target_size,
/// and dismiss excess if over target. Owner-aware: recruits by TownId match.
pub fn squad_cleanup_system(
    mut commands: Commands,
    mut squad_state: ResMut<SquadState>,
    npc_map: Res<NpcEntityMap>,
    available_units: Query<(Entity, &NpcIndex, &TownId), (With<SquadUnit>, Without<Dead>, Without<SquadId>)>,
    world_data: Res<WorldData>,
    squad_units: Query<(Entity, &NpcIndex, &SquadId), (With<SquadUnit>, Without<Dead>)>,
    timings: Res<SystemTimings>,
    mut dirty: ResMut<DirtyFlags>,
) {
    let _t = timings.scope("squad_cleanup");
    if !dirty.squads { return; }
    dirty.squads = false;
    let player_town = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0) as i32;

    // Phase 1: remove dead members (all squads)
    for squad in squad_state.squads.iter_mut() {
        squad.members.retain(|&slot| npc_map.0.contains_key(&slot));
    }

    // Phase 2: keep Default Squad (index 0) as the live pool of unsquadded player military units.
    // Player-only — AI squads handle recruitment via target_size in Phase 4.
    if let Some(default_squad) = squad_state.squads.get_mut(0) {
        if default_squad.is_player() {
            for (entity, npc_idx, town) in available_units.iter() {
                if town.0 != player_town { continue; }
                commands.entity(entity).insert(SquadId(0));
                if !default_squad.members.contains(&npc_idx.0) {
                    default_squad.members.push(npc_idx.0);
                }
            }
        }
    }

    // Phase 3: dismiss excess (target_size > 0 and members > target_size, all squads)
    for (si, squad) in squad_state.squads.iter_mut().enumerate() {
        if squad.target_size > 0 && squad.members.len() > squad.target_size {
            let to_dismiss: Vec<usize> = squad.members.drain(squad.target_size..).collect();
            for slot in &to_dismiss {
                for (entity, npc_idx, sid) in squad_units.iter() {
                    if npc_idx.0 == *slot && sid.0 == si as i32 {
                        commands.entity(entity).remove::<SquadId>();
                        commands.entity(entity).remove::<crate::components::DirectControl>();
                        break;
                    }
                }
            }
        }
    }

    // Phase 4: auto-recruit to fill target_size (owner-aware)
    let assigned_slots: HashSet<usize> = squad_state.squads.iter()
        .flat_map(|s| s.members.iter().copied())
        .collect();

    // Build per-owner pools: group available (unsquadded) military units by town.
    // Each squad draws from its owner's pool only.
    let mut pool_by_town: HashMap<i32, Vec<(Entity, usize)>> = HashMap::new();
    for (entity, npc_idx, town) in available_units.iter() {
        if assigned_slots.contains(&npc_idx.0) { continue; }
        pool_by_town.entry(town.0).or_default().push((entity, npc_idx.0));
    }

    for (si, squad) in squad_state.squads.iter_mut().enumerate() {
        if squad.target_size == 0 { continue; }
        let town_key = match squad.owner {
            SquadOwner::Player => player_town,
            SquadOwner::Town(tdi) => tdi as i32,
        };
        let pool = match pool_by_town.get_mut(&town_key) {
            Some(p) => p,
            None => continue,
        };
        while squad.members.len() < squad.target_size {
            if let Some((entity, slot)) = pool.pop() {
                commands.entity(entity).insert(SquadId(si as i32));
                squad.members.push(slot);
            } else {
                break;
            }
        }
    }
}

// ============================================================================
// MIGRATION SYSTEMS
// ============================================================================

/// Check trigger conditions and spawn a migrating raider group at a map edge.
/// Per-town resources that need extending when a new faction spawns.
#[derive(SystemParam)]
pub struct MigrationResources<'w> {
    pub food_storage: ResMut<'w, FoodStorage>,
    pub gold_storage: ResMut<'w, GoldStorage>,
    pub faction_stats: ResMut<'w, FactionStats>,
    pub raider_state: ResMut<'w, RaiderState>,
    pub npcs_by_town: ResMut<'w, NpcsByTownCache>,
    pub policies: ResMut<'w, TownPolicies>,
}

/// Create a new AI town: allocate faction, push Town + TownGrid, extend all per-town
/// resource vecs, create an inactive AiPlayer with random personality.
/// Returns (town_data_idx, grid_idx, faction).
fn create_ai_town(
    world_data: &mut WorldData,
    town_grids: &mut TownGrids,
    res: &mut MigrationResources,
    ai_state: &mut AiPlayerState,
    center: Vec2,
    is_raider: bool,
) -> (usize, usize, i32) {
    let next_faction = world_data.towns.iter().map(|t| t.faction).max().unwrap_or(0) + 1;
    let name = if is_raider { "Raider Town" } else { "Rival Town" };
    let sprite_type = if is_raider { 1 } else { 0 };

    world_data.towns.push(world::Town {
        name: name.into(),
        center,
        faction: next_faction,
        sprite_type,
    });
    let town_data_idx = world_data.towns.len() - 1;

    town_grids.grids.push(world::TownGrid::new_base(town_data_idx));
    let grid_idx = town_grids.grids.len() - 1;

    // Extend per-town resources
    let num_towns = world_data.towns.len();
    res.food_storage.food.resize(num_towns, 0);
    res.gold_storage.gold.resize(num_towns, 0);
    res.faction_stats.stats.resize(num_towns, FactionStat::default());
    res.raider_state.max_pop.resize(num_towns, 10);
    res.raider_state.respawn_timers.resize(num_towns, 0.0);
    res.raider_state.forage_timers.resize(num_towns, 0.0);
    res.npcs_by_town.0.resize(num_towns, Vec::new());
    res.policies.policies.resize(num_towns, PolicySet::default());

    // Create AiPlayer with random personality
    let ai_kind = if is_raider { AiKind::Raider } else { AiKind::Builder };
    let mut rng = rand::rng();
    let personalities = [AiPersonality::Aggressive, AiPersonality::Balanced, AiPersonality::Economic];
    let personality = personalities[rng.random_range(0..personalities.len())];
    if let Some(policy) = res.policies.policies.get_mut(town_data_idx) {
        *policy = personality.default_policies();
    }
    ai_state.players.push(AiPlayer {
        town_data_idx,
        grid_idx,
        kind: ai_kind,
        personality,
        last_actions: std::collections::VecDeque::new(),
        active: false,
        squad_indices: Vec::new(),
        squad_cmd: HashMap::new(),
    });

    (town_data_idx, grid_idx, next_faction)
}

/// Runs every RAIDER_SPAWN_CHECK_HOURS game hours. Group walks toward nearest player town
/// via Home + Wander behavior, then settles when close enough (handled by migration_settle_system).
pub fn migration_spawn_system(
    game_time: Res<GameTime>,
    mut migration_state: ResMut<MigrationState>,
    mut world_data: ResMut<WorldData>,
    mut town_grids: ResMut<TownGrids>,
    mut res: MigrationResources,
    mut ai_state: ResMut<AiPlayerState>,
    mut slots: ResMut<SlotAllocator>,
    mut spawn_writer: MessageWriter<SpawnNpcMsg>,
    mut combat_log: ResMut<CombatLog>,
    grid: Res<WorldGrid>,
    difficulty: Res<Difficulty>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("migration_spawn");
    let debug_force = migration_state.debug_spawn;
    if debug_force {
        migration_state.debug_spawn = false;
    }

    if !debug_force {
        if !game_time.hour_ticked { return; }
    }
    if migration_state.active.is_some() { return; }

    if !debug_force {
        migration_state.check_timer += 1.0;
        if migration_state.check_timer < RAIDER_SPAWN_CHECK_HOURS { return; }
        migration_state.check_timer = 0.0;

        // Count player alive NPCs and existing raider towns
        let raider_count = world_data.towns.iter().filter(|t| t.sprite_type == 1).count();
        let player_town = world_data.towns.iter().position(|t| t.faction == 0);
        let Some(player_idx) = player_town else { return };
        let player_alive = res.faction_stats.stats.get(player_idx).map(|s| s.alive).unwrap_or(0);
        let needed_raiders = (player_alive as i32 / VILLAGERS_PER_RAIDER) as usize;
        if raider_count >= needed_raiders { return; }
        if raider_count >= MAX_RAIDER_TOWNS { return; }
    }

    // Count player alive NPCs (needed for group size calc)
    let player_town = world_data.towns.iter().position(|t| t.faction == 0);
    let Some(player_idx) = player_town else { return };
    let player_alive = res.faction_stats.stats.get(player_idx).map(|s| s.alive).unwrap_or(0);

    // Determine group size
    let scaling = difficulty.migration_scaling().max(1);
    let group_size = MIGRATION_BASE_SIZE + (player_alive as usize / scaling as usize);
    let group_size = group_size.min(20); // cap at 20 raiders per group

    // Pick random edge position
    let world_w = grid.width as f32 * grid.cell_size;
    let world_h = grid.height as f32 * grid.cell_size;
    let mut rng = rand::rng();
    let edge: u8 = rng.random_range(0..4);
    let (spawn_x, spawn_y) = match edge {
        0 => (rng.random_range(0.0..world_w), 50.0),                // top
        1 => (rng.random_range(0.0..world_w), world_h - 50.0),      // bottom
        2 => (50.0, rng.random_range(0.0..world_h)),                 // left
        _ => (world_w - 50.0, rng.random_range(0.0..world_h)),      // right
    };
    let direction = match edge { 0 => "north", 1 => "south", 2 => "west", _ => "east" };

    let (town_data_idx, grid_idx, next_faction) = create_ai_town(
        &mut world_data, &mut town_grids, &mut res, &mut ai_state,
        Vec2::new(spawn_x, spawn_y), true,
    );

    // Find nearest player town center as wander target
    let player_center = world_data.towns[player_idx].center;

    // Spawn raiders via SpawnNpcMsg with Home = player town center
    let mut member_slots = Vec::with_capacity(group_size);
    for _ in 0..group_size {
        let Some(slot) = slots.alloc() else { break };
        // Slight jitter around spawn point
        let jx = spawn_x + rng.random_range(-30.0..30.0);
        let jy = spawn_y + rng.random_range(-30.0..30.0);
        spawn_writer.write(SpawnNpcMsg {
            slot_idx: slot,
            x: jx,
            y: jy,
            job: 2,              // Raider
            faction: next_faction,
            town_idx: town_data_idx as i32,
            home_x: player_center.x,
            home_y: player_center.y,
            work_x: -1.0,
            work_y: -1.0,
            starting_post: -1,
            attack_type: 0,      // melee
        });
        member_slots.push(slot);
    }

    migration_state.active = Some(MigrationGroup {
        town_data_idx,
        grid_idx,
        member_slots,
        is_raider: true,
    });

    combat_log.push(CombatEventKind::Raid, -1, game_time.day(), game_time.hour(), game_time.minute(),
        format!("A raider band approaches from the {}!", direction));
    info!("Migration spawned: {} raiders from {} edge, faction {}", group_size, direction, next_faction);
}

/// Attach Migrating component to newly spawned migration group members.
/// Runs after spawn_npc_system so entities exist before we try to tag them.
pub fn migration_attach_system(
    mut commands: Commands,
    migration_state: Res<MigrationState>,
    npc_map: Res<NpcEntityMap>,
    existing: Query<&Migrating>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("migration_attach");
    let Some(mg) = &migration_state.active else { return };
    for &slot in &mg.member_slots {
        if let Some(&entity) = npc_map.0.get(&slot) {
            if existing.get(entity).is_err() {
                commands.entity(entity).insert(Migrating);
            }
        }
    }
}

/// Check if the migrating raider group has reached near a town and should settle.
/// When within RAIDER_SETTLE_RADIUS of any town, places raider town buildings and activates the AI player.
pub fn migration_settle_system(
    mut commands: Commands,
    mut migration_state: ResMut<MigrationState>,
    mut world_state: WorldState,
    mut ai_state: ResMut<AiPlayerState>,
    mut combat_log: ResMut<CombatLog>,
    mut tilemap_spawned: ResMut<crate::render::TilemapSpawned>,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    npc_map: Res<NpcEntityMap>,
    config: Res<world::WorldGenConfig>,
    migrating_query: Query<(Entity, &NpcIndex), With<Migrating>>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("migration_settle");
    let Some(mg) = &migration_state.active else { return };

    // Compute average position of living migration group members from GPU readback
    let mut sum_x = 0.0f32;
    let mut sum_y = 0.0f32;
    let mut count = 0u32;
    for &slot in &mg.member_slots {
        let x = gpu_state.positions.get(slot * 2).copied().unwrap_or(-9999.0);
        let y = gpu_state.positions.get(slot * 2 + 1).copied().unwrap_or(-9999.0);
        if world::is_alive(Vec2::new(x, y)) {
            sum_x += x;
            sum_y += y;
            count += 1;
        }
    }
    if count == 0 {
        // All members dead — cancel migration
        migration_state.active = None;
        return;
    }
    let avg_x = sum_x / count as f32;
    let avg_y = sum_y / count as f32;
    let avg_pos = Vec2::new(avg_x, avg_y);

    // Check distance to any town (player or AI)
    let near_town = world_state.world_data.towns.iter().enumerate().any(|(i, t)| {
        // Skip our own temporary town entry
        i != mg.town_data_idx && avg_pos.distance(t.center) < RAIDER_SETTLE_RADIUS
    });
    if !near_town { return; }

    // === SETTLE ===
    let town_data_idx = mg.town_data_idx;
    let grid_idx = mg.grid_idx;
    let member_slots = mg.member_slots.clone();
    let is_raider = mg.is_raider;

    // Update town center to average group position
    if let Some(town) = world_state.world_data.towns.get_mut(town_data_idx) {
        town.center = avg_pos;
    }

    // Place buildings (raider town: tents, town: farms + homes + waypoints)
    if let Some(town_grid) = world_state.town_grids.grids.get_mut(grid_idx) {
        world::place_buildings(&mut world_state.grid, &mut world_state.world_data, &mut world_state.farm_states, avg_pos, town_data_idx as u32, &config, town_grid, is_raider);
    }

    // Register tent spawners
    let tent_def = crate::constants::building_def(BuildingKind::Tent);
    for i in 0..(tent_def.len)(&world_state.world_data) {
        if let Some((pos, ti)) = (tent_def.pos_town)(&world_state.world_data, i) {
            if ti == town_data_idx as u32 {
                world::register_spawner(&mut world_state.spawner_state, BuildingKind::Tent,
                    town_data_idx as i32, pos, -1.0);
                (tent_def.hps_mut)(&mut world_state.building_hp).push(tent_def.hp);
            }
        }
    }

    // Add town center HP
    world_state.building_hp.towns.push(crate::constants::building_def(BuildingKind::Fountain).hp);

    // Stamp dirt around the new raider town
    world::stamp_dirt(&mut world_state.grid, &[avg_pos]);

    // Activate the AiPlayer for this raider town
    if let Some(player) = ai_state.players.iter_mut().find(|p| p.town_data_idx == town_data_idx) {
        player.active = true;
    }

    // Remove Migrating from all group members and update their Home to raider town center
    for &slot in &member_slots {
        if let Some(&entity) = npc_map.0.get(&slot) {
            // Check if entity still has Migrating (may have died)
            if migrating_query.get(entity).is_ok() {
                commands.entity(entity).remove::<Migrating>();
                commands.entity(entity).insert(Home(avg_pos));
            }
        }
    }

    // Mark dirty for building grid + tilemap rebuild
    world_state.dirty.building_grid = true;
    tilemap_spawned.0 = false; // force tilemap rebuild with new terrain + buildings

    combat_log.push(CombatEventKind::Raid, -1, game_time.day(), game_time.hour(), game_time.minute(),
        format!("A raider band has settled nearby!"));
    info!("Migration settled at ({:.0}, {:.0}), town_data_idx={}", avg_x, avg_y, town_data_idx);

    migration_state.active = None;
}

/// Tick pending endless respawns and trigger migrations when ready.
pub fn endless_respawn_system(
    mut endless: ResMut<EndlessMode>,
    mut migration_state: ResMut<MigrationState>,
    mut world_data: ResMut<WorldData>,
    mut town_grids: ResMut<TownGrids>,
    mut ai_state: ResMut<AiPlayerState>,
    mut upgrades: ResMut<TownUpgrades>,
    grid: Res<WorldGrid>,
    game_time: Res<GameTime>,
    time: Res<Time>,
    mut res: MigrationResources,
    mut slots: ResMut<SlotAllocator>,
    mut spawn_writer: MessageWriter<SpawnNpcMsg>,
    mut combat_log: ResMut<CombatLog>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("endless_respawn");
    if !endless.enabled || endless.pending_spawns.is_empty() { return; }
    if migration_state.active.is_some() { return; } // wait for current migration to finish

    let dt_hours = time.delta_secs() * game_time.time_scale / 3600.0;

    // Tick all pending spawns
    for spawn in &mut endless.pending_spawns {
        spawn.delay_remaining -= dt_hours;
    }

    // Find first ready spawn
    let Some(idx) = endless.pending_spawns.iter().position(|s| s.delay_remaining <= 0.0) else { return };
    let spawn = endless.pending_spawns.remove(idx);

    // --- Trigger migration (reuses migration_spawn_system pattern) ---
    let world_w = grid.width as f32 * grid.cell_size;
    let world_h = grid.height as f32 * grid.cell_size;
    let mut rng = rand::rng();
    let edge: u8 = rng.random_range(0..4);
    let (spawn_x, spawn_y) = match edge {
        0 => (rng.random_range(0.0..world_w), 50.0),
        1 => (rng.random_range(0.0..world_w), world_h - 50.0),
        2 => (50.0, rng.random_range(0.0..world_h)),
        _ => (world_w - 50.0, rng.random_range(0.0..world_h)),
    };
    let direction = match edge { 0 => "north", 1 => "south", 2 => "west", _ => "east" };

    let (town_data_idx, grid_idx, next_faction) = create_ai_town(
        &mut world_data, &mut town_grids, &mut res, &mut ai_state,
        Vec2::new(spawn_x, spawn_y), spawn.is_raider,
    );

    // Pre-set starting resources
    if let Some(food) = res.food_storage.food.get_mut(town_data_idx) {
        *food = spawn.starting_food;
    }
    if let Some(gold) = res.gold_storage.gold.get_mut(town_data_idx) {
        *gold = spawn.starting_gold;
    }

    // Pre-set upgrade levels
    let num_towns = world_data.towns.len();
    upgrades.levels.resize(num_towns, Vec::new());
    upgrades.levels[town_data_idx] = spawn.upgrade_levels;

    // Spawn NPCs at edge (raider units for raiders, village units for builders)
    let player_center = world_data.towns.iter()
        .find(|t| t.faction == 0)
        .map(|t| t.center)
        .unwrap_or(Vec2::new(world_w / 2.0, world_h / 2.0));

    let group_size = if spawn.is_raider {
        // Raider: scale group like migration
        MIGRATION_BASE_SIZE + 5
    } else {
        // Builder: spawn a few archers to walk in (town buildings placed on settle)
        5
    };

    let mut member_slots = Vec::with_capacity(group_size);
    for _ in 0..group_size {
        let Some(slot) = slots.alloc() else { break };
        let jx = spawn_x + rng.random_range(-30.0..30.0);
        let jy = spawn_y + rng.random_range(-30.0..30.0);
        let job = if spawn.is_raider { 2 } else { 1 }; // Raider or Archer
        spawn_writer.write(SpawnNpcMsg {
            slot_idx: slot,
            x: jx,
            y: jy,
            job,
            faction: next_faction,
            town_idx: town_data_idx as i32,
            home_x: player_center.x,
            home_y: player_center.y,
            work_x: -1.0,
            work_y: -1.0,
            starting_post: -1,
            attack_type: 0,
        });
        member_slots.push(slot);
    }

    migration_state.active = Some(MigrationGroup {
        town_data_idx,
        grid_idx,
        member_slots,
        is_raider: spawn.is_raider,
    });

    combat_log.push(CombatEventKind::Raid, -1, game_time.day(), game_time.hour(), game_time.minute(),
        format!("A new {} approaches from the {}!", if spawn.is_raider { "raider band" } else { "rival faction" }, direction));
    let kind_str = if spawn.is_raider { "raider" } else { "builder" };
    info!("Endless respawn: {} town from {} edge, faction {}", kind_str, direction, next_faction);
}
