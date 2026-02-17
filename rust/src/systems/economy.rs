//! Economy systems - Game time, population tracking, farm growth, camp foraging, respawning

use bevy::prelude::*;
use rand::Rng;
use std::collections::HashSet;

use crate::components::*;
use crate::resources::*;
use crate::systemparams::{EconomyState, WorldState};
use crate::constants::{FARM_BASE_GROWTH_RATE, FARM_TENDED_GROWTH_RATE, CAMP_FORAGE_RATE, STARVING_SPEED_MULT, SPAWNER_RESPAWN_HOURS,
    CAMP_SPAWN_CHECK_HOURS, MAX_DYNAMIC_CAMPS, CAMP_SETTLE_RADIUS, MIGRATION_BASE_SIZE, VILLAGERS_PER_CAMP};
use crate::world::{self, WorldData, WorldGrid, BuildingOccupancy, BuildingSpatialGrid, TownGrids};
use crate::messages::{SpawnNpcMsg, GpuUpdate, GpuUpdateMsg};
use crate::systems::stats::{TownUpgrades, UpgradeType, UPGRADE_PCT};
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

    game_time.total_seconds += time.delta_secs() * game_time.time_scale;

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

    let hours_elapsed = (time.delta_secs() * game_time.time_scale) / game_time.seconds_per_hour;

    for i in 0..growth_states.states.len() {
        if growth_states.positions[i].x < -9000.0 { continue; } // tombstoned
        if growth_states.states[i] != FarmGrowthState::Growing { continue; }

        let is_tended = farm_occupancy.is_occupied(growth_states.positions[i]);

        let growth_rate = match growth_states.kinds[i] {
            GrowthKind::Farm => {
                let base_rate = if is_tended { FARM_TENDED_GROWTH_RATE } else { FARM_BASE_GROWTH_RATE };
                let town = growth_states.town_indices[i].unwrap_or(0) as usize;
                let yield_level = upgrades.levels.get(town).map(|l| l[UpgradeType::FarmYield as usize]).unwrap_or(0);
                base_rate * (1.0 + yield_level as f32 * UPGRADE_PCT[UpgradeType::FarmYield as usize])
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
// CAMP FORAGING SYSTEM
// ============================================================================

/// Camp foraging: each raider camp gains CAMP_FORAGE_RATE food per hour.
/// Only runs when game_time.hour_ticked is true.
pub fn camp_forage_system(
    game_time: Res<GameTime>,
    mut economy: EconomyState,
    world_data: Res<WorldData>,
    user_settings: Res<crate::settings::UserSettings>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("camp_forage");
    if !game_time.hour_ticked || !user_settings.raider_passive_forage {
        return;
    }

    // Add foraging food to each raider camp (faction > 0)
    for (town_idx, town) in world_data.towns.iter().enumerate() {
        if town.faction > 0 && town_idx < economy.food_storage.food.len() {
            economy.food_storage.food[town_idx] += CAMP_FORAGE_RATE;
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
    let farm_count = world_data.farms.len();
    prev_states.resize(farm_count, FarmGrowthState::Growing);
    for farm_idx in 0..farm_count.min(growth_states.states.len()) {
        let state = &growth_states.states[farm_idx];
        let prev = prev_states[farm_idx];
        if *state == FarmGrowthState::Ready && prev == FarmGrowthState::Growing {
            if world_data.farms.get(farm_idx).is_some() {
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
                if entry.building_kind == 3 {
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
                    world::resolve_spawner_npc(entry, &world_data.towns, &bgrid, &farm_occupancy, &world_data.miner_homes);

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
                if entry.building_kind == 3 {
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
    if mining.mine_enabled.len() < world_data.gold_mines.len() {
        mining.mine_enabled.resize(world_data.gold_mines.len(), true);
    }

    for town_idx in 0..world_data.towns.len() {
        let town = &world_data.towns[town_idx];
        if town.faction != 0 {
            mining.discovered_mines[town_idx].clear();
            continue;
        }
        let radius = policies.policies
            .get(town_idx)
            .map(|p| p.mining_radius)
            .unwrap_or(crate::constants::DEFAULT_MINING_RADIUS);
        let r2 = radius * radius;

        let mut discovered = Vec::new();
        for (mine_idx, mine) in world_data.gold_mines.iter().enumerate() {
            let d = mine.position - town.center;
            if d.length_squared() <= r2 {
                discovered.push(mine_idx);
            }
        }
        mining.discovered_mines[town_idx] = discovered;
    }

    for town_idx in 0..world_data.towns.len() {
        if world_data.towns[town_idx].faction != 0 { continue; }

        let enabled_mines: Vec<usize> = mining.discovered_mines[town_idx]
            .iter()
            .copied()
            .filter(|&mi| mi < mining.mine_enabled.len() && mining.mine_enabled[mi])
            .collect();

        let enabled_positions: Vec<Vec2> = enabled_mines.iter()
            .filter_map(|&mi| world_data.gold_mines.get(mi).map(|m| m.position))
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
            if world_data.miner_homes[mh_idx].manual_mine {
                continue;
            }
            auto_homes.push(mh_idx);
        }

        for &mh_idx in &auto_homes {
            let Some(mh) = world_data.miner_homes.get(mh_idx) else { continue };
            if let Some(pos) = mh.assigned_mine {
                let still_enabled = enabled_positions.iter().any(|p| (*p - pos).length() < 1.0);
                if !still_enabled {
                    // clear stale assignment if disabled or no longer discovered
                    if let Some(mh_mut) = world_data.miner_homes.get_mut(mh_idx) {
                        mh_mut.assigned_mine = None;
                    }
                }
            }
        }

        if enabled_positions.is_empty() {
            for &mh_idx in &auto_homes {
                if let Some(mh_mut) = world_data.miner_homes.get_mut(mh_idx) {
                    mh_mut.assigned_mine = None;
                }
            }
            continue;
        }

        for (i, &mh_idx) in auto_homes.iter().enumerate() {
            let mine_pos = enabled_positions[i % enabled_positions.len()];
            if let Some(mh_mut) = world_data.miner_homes.get_mut(mh_idx) {
                mh_mut.assigned_mine = Some(mine_pos);
            }
        }
    }
}

/// Remove dead NPCs from squad member lists, auto-recruit to target_size,
/// and dismiss excess if over target.
pub fn squad_cleanup_system(
    mut commands: Commands,
    mut squad_state: ResMut<SquadState>,
    npc_map: Res<NpcEntityMap>,
    available_guards: Query<(Entity, &NpcIndex, &TownId), (With<Archer>, Without<Dead>, Without<SquadId>)>,
    world_data: Res<WorldData>,
    squad_guards: Query<(Entity, &NpcIndex, &SquadId), (With<Archer>, Without<Dead>)>,
    timings: Res<SystemTimings>,
    mut dirty: ResMut<DirtyFlags>,
) {
    let _t = timings.scope("squad_cleanup");
    if !dirty.squads { return; }
    dirty.squads = false;
    let player_town = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0) as i32;

    // Phase 1: remove dead members
    for squad in squad_state.squads.iter_mut() {
        squad.members.retain(|&slot| npc_map.0.contains_key(&slot));
    }

    // Phase 2: keep Default Squad (1) as the live pool of unsquadded player archers.
    if let Some(default_squad) = squad_state.squads.get_mut(0) {
        for (entity, npc_idx, town) in available_guards.iter() {
            if town.0 != player_town { continue; }
            commands.entity(entity).insert(SquadId(0));
            if !default_squad.members.contains(&npc_idx.0) {
                default_squad.members.push(npc_idx.0);
            }
        }
    }

    // Phase 3: dismiss excess (target_size > 0 and members > target_size)
    for (si, squad) in squad_state.squads.iter_mut().enumerate() {
        if squad.target_size > 0 && squad.members.len() > squad.target_size {
            let excess = squad.members.len() - squad.target_size;
            let to_dismiss: Vec<usize> = squad.members.drain(squad.target_size..).collect();
            for slot in &to_dismiss {
                for (entity, npc_idx, sid) in squad_guards.iter() {
                    if npc_idx.0 == *slot && sid.0 == si as i32 {
                        commands.entity(entity).remove::<SquadId>();
                        break;
                    }
                }
            }
            let _ = excess; // suppress unused
        }
    }

    // Phase 4: auto-recruit to fill target_size
    let assigned_slots: HashSet<usize> = squad_state.squads.iter()
        .flat_map(|s| s.members.iter().copied())
        .collect();
    let mut pool: Vec<(Entity, usize)> = available_guards.iter()
        .filter(|(_, npc_idx, town)| town.0 == player_town && !assigned_slots.contains(&npc_idx.0))
        .map(|(e, ni, _)| (e, ni.0))
        .collect();

    for (si, squad) in squad_state.squads.iter_mut().enumerate() {
        if squad.target_size == 0 { continue; }
        while squad.members.len() < squad.target_size {
            if let Some((entity, slot)) = pool.pop() {
                commands.entity(entity).insert(SquadId(si as i32));
                squad.members.push(slot);
            } else {
                break; // no more available guards
            }
        }
    }
}

// ============================================================================
// MIGRATION SYSTEMS
// ============================================================================

/// Check trigger conditions and spawn a migrating raider group at a map edge.
/// Runs every CAMP_SPAWN_CHECK_HOURS game hours. Group walks toward nearest player town
/// via Home + Wander behavior, then settles when close enough (handled by migration_settle_system).
pub fn migration_spawn_system(
    game_time: Res<GameTime>,
    mut migration_state: ResMut<MigrationState>,
    mut world_data: ResMut<WorldData>,
    mut town_grids: ResMut<TownGrids>,
    mut food_storage: ResMut<FoodStorage>,
    mut gold_storage: ResMut<GoldStorage>,
    mut faction_stats: ResMut<FactionStats>,
    mut camp_state: ResMut<CampState>,
    mut npcs_by_town: ResMut<NpcsByTownCache>,
    mut policies: ResMut<TownPolicies>,
    mut ai_state: ResMut<AiPlayerState>,
    mut slots: ResMut<SlotAllocator>,
    mut spawn_writer: MessageWriter<SpawnNpcMsg>,
    mut combat_log: ResMut<CombatLog>,
    grid: Res<WorldGrid>,
    difficulty: Res<Difficulty>,
) {
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
        if migration_state.check_timer < CAMP_SPAWN_CHECK_HOURS { return; }
        migration_state.check_timer = 0.0;

        // Count player alive NPCs and existing camps
        let camp_count = world_data.towns.iter().filter(|t| t.sprite_type == 1).count();
        let player_town = world_data.towns.iter().position(|t| t.faction == 0);
        let Some(player_idx) = player_town else { return };
        let player_alive = faction_stats.stats.get(player_idx).map(|s| s.alive).unwrap_or(0);
        let needed_camps = (player_alive as i32 / VILLAGERS_PER_CAMP) as usize;
        if camp_count >= needed_camps { return; }
        if camp_count >= MAX_DYNAMIC_CAMPS { return; }
    }

    // Count player alive NPCs (needed for group size calc)
    let player_town = world_data.towns.iter().position(|t| t.faction == 0);
    let Some(player_idx) = player_town else { return };
    let player_alive = faction_stats.stats.get(player_idx).map(|s| s.alive).unwrap_or(0);

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

    // Create new faction (next unique ID)
    let next_faction = world_data.towns.iter().map(|t| t.faction).max().unwrap_or(0) + 1;

    // Create Town entry
    world_data.towns.push(world::Town {
        name: "Raider Camp".into(),
        center: Vec2::new(spawn_x, spawn_y), // temporary center — updated on settle
        faction: next_faction,
        sprite_type: 1,
    });
    let town_data_idx = world_data.towns.len() - 1;

    // Create TownGrid
    town_grids.grids.push(world::TownGrid::new_base(town_data_idx));
    let grid_idx = town_grids.grids.len() - 1;

    // Extend per-town resources
    let num_towns = world_data.towns.len();
    food_storage.food.resize(num_towns, 0);
    gold_storage.gold.resize(num_towns, 0);
    faction_stats.stats.resize(num_towns, FactionStat::default());
    camp_state.max_pop.resize(num_towns, 10);
    camp_state.respawn_timers.resize(num_towns, 0.0);
    camp_state.forage_timers.resize(num_towns, 0.0);
    npcs_by_town.0.resize(num_towns, Vec::new());
    policies.policies.resize(num_towns, PolicySet::default());

    // Create inactive AiPlayer (activated on settlement)
    let personalities = [AiPersonality::Aggressive, AiPersonality::Balanced, AiPersonality::Economic];
    let personality = personalities[rng.random_range(0..personalities.len())];
    ai_state.players.push(AiPlayer {
        town_data_idx,
        grid_idx,
        kind: AiKind::Raider,
        personality,
        last_actions: std::collections::VecDeque::new(),
        active: false,
    });

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
) {
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
/// When within CAMP_SETTLE_RADIUS of any town, places camp buildings and activates the AI player.
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
) {
    let Some(mg) = &migration_state.active else { return };

    // Compute average position of living migration group members from GPU readback
    let mut sum_x = 0.0f32;
    let mut sum_y = 0.0f32;
    let mut count = 0u32;
    for &slot in &mg.member_slots {
        let x = gpu_state.positions.get(slot * 2).copied().unwrap_or(-9999.0);
        let y = gpu_state.positions.get(slot * 2 + 1).copied().unwrap_or(-9999.0);
        if x > -9000.0 {
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
        i != mg.town_data_idx && avg_pos.distance(t.center) < CAMP_SETTLE_RADIUS
    });
    if !near_town { return; }

    // === SETTLE ===
    let town_data_idx = mg.town_data_idx;
    let grid_idx = mg.grid_idx;
    let member_slots = mg.member_slots.clone();

    // Update town center to average group position
    if let Some(town) = world_state.world_data.towns.get_mut(town_data_idx) {
        town.center = avg_pos;
    }

    // Place camp buildings (camp center + tents in spiral)
    if let Some(town_grid) = world_state.town_grids.grids.get_mut(grid_idx) {
        world::place_camp_buildings(&mut world_state.grid, &mut world_state.world_data, avg_pos, town_data_idx as u32, &config, town_grid);
    }

    // Register tent spawners
    for tent in world_state.world_data.tents.iter() {
        if tent.town_idx == town_data_idx as u32 {
            world::register_spawner(&mut world_state.spawner_state, world::Building::Tent { town_idx: 0 },
                town_data_idx as i32, tent.position, -1.0);
            world_state.building_hp.tents.push(crate::constants::TENT_HP);
        }
    }

    // Add town center HP
    world_state.building_hp.towns.push(crate::constants::TOWN_HP);

    // Stamp dirt around the new camp
    world::stamp_dirt(&mut world_state.grid, &[avg_pos]);

    // Activate the AiPlayer for this camp
    if let Some(player) = ai_state.players.iter_mut().find(|p| p.town_data_idx == town_data_idx) {
        player.active = true;
    }

    // Remove Migrating from all group members and update their Home to camp center
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
    world_state.dirty.waypoint_slots = true;
    tilemap_spawned.0 = false; // force tilemap rebuild with new terrain + buildings

    combat_log.push(CombatEventKind::Raid, -1, game_time.day(), game_time.hour(), game_time.minute(),
        format!("A raider band has settled nearby!"));
    info!("Migration settled at ({:.0}, {:.0}), town_data_idx={}", avg_x, avg_y, town_data_idx);

    migration_state.active = None;
}
