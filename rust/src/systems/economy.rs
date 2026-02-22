//! Economy systems - Game time, population tracking, farm growth, raider town foraging, respawning

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use rand::Rng;
use std::collections::{HashMap, HashSet};

use crate::components::*;
use crate::resources::*;
use crate::systemparams::{EconomyState, WorldState};
use crate::constants::{FARM_BASE_GROWTH_RATE, FARM_TENDED_GROWTH_RATE, RAIDER_FORAGE_RATE, STARVING_SPEED_MULT, SPAWNER_RESPAWN_HOURS,
    RAIDER_SETTLE_RADIUS, MIGRATION_BASE_SIZE, BOAT_SPEED, ATLAS_BOAT, ENDLESS_RESPAWN_DELAY_HOURS,
};
use crate::world::{self, WorldData, BuildingKind, BuildingOccupancy, BuildingSpatialGrid, TownGrids, Biome};
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
    pub gpu_updates: MessageWriter<'w, GpuUpdateMsg>,
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

/// Pick a settlement site far from all existing towns.
/// Samples random land positions, scores by min distance to any town, picks the farthest.
fn pick_settle_site(
    grid: &crate::world::WorldGrid,
    world_data: &WorldData,
    world_w: f32, world_h: f32,
) -> Vec2 {
    let margin = 200.0;
    let mut rng = rand::rng();
    let mut best_pos = Vec2::new(world_w / 2.0, world_h / 2.0);
    let mut best_min_dist = 0.0f32;

    for _ in 0..100 {
        let x = rng.random_range(margin..world_w - margin);
        let y = rng.random_range(margin..world_h - margin);
        let pos = Vec2::new(x, y);

        // Reject water cells
        let (gc, gr) = grid.world_to_grid(pos);
        if grid.cell(gc, gr).is_some_and(|c| c.terrain == Biome::Water) { continue; }

        // Score: minimum distance to any existing town
        let min_dist = world_data.towns.iter()
            .map(|t| pos.distance(t.center))
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(f32::MAX);

        if min_dist > best_min_dist {
            best_min_dist = min_dist;
            best_pos = pos;
        }
    }

    // Snap to grid center for alignment
    let (gc, gr) = grid.world_to_grid(best_pos);
    grid.grid_to_world(gc, gr)
}

/// Endless mode lifecycle: boat → disembark → walk → settle.
/// Phase 1: Spawn boat at map edge (no town, no NPCs)
/// Phase 2: Sail toward settle site, disembark NPCs on shore
/// Phase 3: NPCs walk toward settle target, attach Migrating
/// Phase 4: Settle near target — create AI town, place buildings, activate AI
pub fn endless_system(
    mut commands: Commands,
    mut endless: ResMut<EndlessMode>,
    mut migration_state: ResMut<MigrationState>,
    mut world_state: WorldState,
    mut ai_state: ResMut<AiPlayerState>,
    mut upgrades: ResMut<TownUpgrades>,
    mut combat_log: ResMut<CombatLog>,
    mut tilemap_spawned: ResMut<crate::render::TilemapSpawned>,
    game_time: Res<GameTime>,
    time: Res<Time>,
    npc_map: Res<NpcEntityMap>,
    config: Res<world::WorldGenConfig>,
    mut res: MigrationResources,
    migrating_query: Query<(Entity, &NpcIndex, &Position), With<Migrating>>,
    mut spawn_writer: MessageWriter<SpawnNpcMsg>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("endless");

    // Debug button: queue an immediate raider spawn
    if migration_state.debug_spawn {
        migration_state.debug_spawn = false;
        endless.pending_spawns.push(PendingAiSpawn {
            delay_remaining: 0.0, is_raider: true,
            upgrade_levels: Vec::new(), starting_food: 0, starting_gold: 0,
        });
    }

    if !endless.enabled { return; }

    let world_w = world_state.grid.width as f32 * world_state.grid.cell_size;
    let world_h = world_state.grid.height as f32 * world_state.grid.cell_size;

    // === BOAT SAIL — move boat toward settle target, disembark when on shore ===
    if let Some(mg) = &mut migration_state.active {
        if let Some(boat_slot) = mg.boat_slot {
            let dir = (mg.settle_target - mg.boat_pos).normalize_or_zero();
            mg.boat_pos += dir * BOAT_SPEED * time.delta_secs();

            res.gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetPosition {
                idx: boat_slot, x: mg.boat_pos.x, y: mg.boat_pos.y,
            }));

            // Check if boat reached land
            let (gc, gr) = world_state.grid.world_to_grid(mg.boat_pos);
            let on_water = world_state.grid.cell(gc, gr)
                .map(|c| c.terrain == Biome::Water)
                .unwrap_or(true);

            if !on_water {
                // === DISEMBARK — spawn NPCs at boat position ===
                let next_faction = world_state.world_data.towns.iter()
                    .map(|t| t.faction).max().unwrap_or(0) + 1;
                let group_size = if mg.is_raider { MIGRATION_BASE_SIZE + 5 } else { 5 };
                let mut rng = rand::rng();

                for _ in 0..group_size {
                    let Some(slot) = world_state.slot_alloc.alloc() else { break };
                    let jx = mg.boat_pos.x + rng.random_range(-30.0..30.0);
                    let jy = mg.boat_pos.y + rng.random_range(-30.0..30.0);
                    let job = if mg.is_raider { 2 } else { 1 };
                    spawn_writer.write(SpawnNpcMsg {
                        slot_idx: slot, x: jx, y: jy, job,
                        faction: next_faction, town_idx: -1,
                        home_x: mg.settle_target.x, home_y: mg.settle_target.y,
                        work_x: -1.0, work_y: -1.0, starting_post: -1, attack_type: 0,
                    });
                    mg.member_slots.push(slot);
                }
                mg.faction = next_faction;

                // Free boat slot
                res.gpu_updates.write(GpuUpdateMsg(GpuUpdate::HideNpc { idx: boat_slot }));
                world_state.slot_alloc.free(boat_slot);
                mg.boat_slot = None;

                let kind_str = if mg.is_raider { "Raiders" } else { "Settlers" };
                combat_log.push_at(CombatEventKind::Raid, -1, game_time.day(), game_time.hour(), game_time.minute(),
                    format!("{} have landed!", kind_str), Some(mg.boat_pos));
                info!("Migration disembarked at ({:.0}, {:.0}), faction {}", mg.boat_pos.x, mg.boat_pos.y, next_faction);
            }

            // While boat active, skip attach/settle
            if mg.boat_slot.is_some() { return; }
        }
    }

    // === ATTACH Migrating component to newly spawned members ===
    if let Some(mg) = &migration_state.active {
        for &slot in &mg.member_slots {
            if let Some(&entity) = npc_map.0.get(&slot) {
                if migrating_query.get(entity).is_err() {
                    if let Ok(mut ec) = commands.get_entity(entity) {
                        ec.insert(Migrating);
                    }
                }
            }
        }
    }

    // === SETTLE — when NPCs are near a town, create AI town + buildings ===
    if let Some(mg) = &migration_state.active {
        if mg.town_data_idx.is_some() { return; } // already settled (shouldn't happen)

        let mut sum_x = 0.0f32;
        let mut sum_y = 0.0f32;
        let mut count = 0u32;
        for &slot in &mg.member_slots {
            if let Some(&entity) = npc_map.0.get(&slot) {
                if let Ok((_, _, pos)) = migrating_query.get(entity) {
                    sum_x += pos.x;
                    sum_y += pos.y;
                    count += 1;
                }
            }
        }
        if count == 0 {
            if !mg.member_slots.is_empty() {
                // All members dead — migration wiped out, queue replacement
                let is_raider = mg.is_raider;
                let kind_str = if is_raider { "raider band" } else { "rival faction" };
                combat_log.push(CombatEventKind::Raid, -1, game_time.day(), game_time.hour(), game_time.minute(),
                    format!("The migrating {} was wiped out!", kind_str));
                endless.pending_spawns.push(PendingAiSpawn {
                    delay_remaining: ENDLESS_RESPAWN_DELAY_HOURS,
                    is_raider,
                    upgrade_levels: mg.upgrade_levels.clone(),
                    starting_food: mg.starting_food,
                    starting_gold: mg.starting_gold,
                });
                info!("Migration wiped out (is_raider={}), queued replacement in {}h", is_raider, ENDLESS_RESPAWN_DELAY_HOURS);
            }
            migration_state.active = None;
            return;
        }
        let avg_pos = Vec2::new(sum_x / count as f32, sum_y / count as f32);

        let near_target = avg_pos.distance(mg.settle_target) < RAIDER_SETTLE_RADIUS;
        if !near_target { return; }

        // === CREATE TOWN + SETTLE ===
        let is_raider = mg.is_raider;
        let member_slots = mg.member_slots.clone();

        let (town_data_idx, grid_idx, _faction) = create_ai_town(
            &mut world_state.world_data, &mut world_state.town_grids, &mut res, &mut ai_state,
            mg.settle_target, is_raider,
        );

        // Apply stored resources and upgrades
        if let Some(food) = res.food_storage.food.get_mut(town_data_idx) {
            *food = mg.starting_food;
        }
        if let Some(gold) = res.gold_storage.gold.get_mut(town_data_idx) {
            *gold = mg.starting_gold;
        }
        let num_towns = world_state.world_data.towns.len();
        upgrades.levels.resize(num_towns, Vec::new());
        upgrades.levels[town_data_idx] = mg.upgrade_levels.clone();

        // Place buildings
        if let Some(town_grid) = world_state.town_grids.grids.get_mut(grid_idx) {
            world::place_buildings(&mut world_state.grid, &mut world_state.world_data, &mut world_state.farm_states, mg.settle_target, town_data_idx as u32, &config, town_grid, is_raider);
        }
        world::init_single_town_buildings(
            town_data_idx, &world_state.world_data,
            &mut world_state.spawner_state, &mut world_state.building_hp,
            &mut world_state.slot_alloc, &mut world_state.building_slots,
        );
        world::stamp_dirt(&mut world_state.grid, &[mg.settle_target]);

        // Activate AI
        if let Some(player) = ai_state.players.iter_mut().find(|p| p.town_data_idx == town_data_idx) {
            player.active = true;
        }

        // Settle NPCs: remove Migrating, set Home, update town_idx
        for &slot in &member_slots {
            if let Some(&entity) = npc_map.0.get(&slot) {
                if migrating_query.get(entity).is_ok() {
                    commands.entity(entity).remove::<Migrating>();
                    commands.entity(entity).insert(Home(mg.settle_target));
                }
            }
        }

        world_state.dirty.building_grid = true;
        tilemap_spawned.0 = false;

        let kind_str = if is_raider { "raider band" } else { "rival faction" };
        combat_log.push_at(CombatEventKind::Raid, -1, game_time.day(), game_time.hour(), game_time.minute(),
            format!("A {} has settled nearby!", kind_str), Some(mg.settle_target));
        info!("Migration settled at ({:.0}, {:.0}), town_data_idx={}", mg.settle_target.x, mg.settle_target.y, town_data_idx);
        migration_state.active = None;
        return;
    }

    // === SPAWN BOAT — pick edge, allocate boat GPU slot ===
    if endless.pending_spawns.is_empty() { return; }

    let dt_hours = game_time.delta(&time) / game_time.seconds_per_hour;
    for spawn in &mut endless.pending_spawns {
        spawn.delay_remaining -= dt_hours;
    }

    let Some(idx) = endless.pending_spawns.iter().position(|s| s.delay_remaining <= 0.0) else { return };
    let spawn = endless.pending_spawns.remove(idx);

    let mut rng = rand::rng();
    let edge: u8 = rng.random_range(0..4);
    let (spawn_x, spawn_y) = match edge {
        0 => (rng.random_range(0.0..world_w), 50.0),
        1 => (rng.random_range(0.0..world_w), world_h - 50.0),
        2 => (50.0, rng.random_range(0.0..world_h)),
        _ => (world_w - 50.0, rng.random_range(0.0..world_h)),
    };
    let direction = match edge { 0 => "north", 1 => "south", 2 => "west", _ => "east" };

    // Pick settlement site far from existing towns
    let settle_target = pick_settle_site(&world_state.grid, &world_state.world_data, world_w, world_h);
    info!("Endless: settle target at ({:.0}, {:.0})", settle_target.x, settle_target.y);

    // Allocate boat GPU slot
    let boat_slot = world_state.slot_alloc.alloc();
    if let Some(bs) = boat_slot {
        res.gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetPosition { idx: bs, x: spawn_x, y: spawn_y }));
        res.gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame { idx: bs, col: 0.0, row: 0.0, atlas: ATLAS_BOAT }));
        res.gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx: bs, speed: BOAT_SPEED }));
        res.gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx: bs, x: settle_target.x, y: settle_target.y }));
        res.gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: bs, health: 100.0 }));
        res.gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFaction { idx: bs, faction: 0 }));
    }

    migration_state.active = Some(MigrationGroup {
        boat_slot,
        boat_pos: Vec2::new(spawn_x, spawn_y),
        settle_target,
        is_raider: spawn.is_raider,
        upgrade_levels: spawn.upgrade_levels,
        starting_food: spawn.starting_food,
        starting_gold: spawn.starting_gold,
        member_slots: Vec::new(),
        faction: 0,
        town_data_idx: None,
        grid_idx: 0,
    });

    let kind_str = if spawn.is_raider { "raider band" } else { "rival faction" };
    combat_log.push_at(CombatEventKind::Raid, -1, game_time.day(), game_time.hour(), game_time.minute(),
        format!("A {} approaches from the {}!", kind_str, direction), Some(Vec2::new(spawn_x, spawn_y)));
    info!("Endless: boat spawned from {} edge", direction);
}
