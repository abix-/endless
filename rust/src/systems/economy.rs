//! Economy systems - Game time, population tracking, farm growth, camp foraging, respawning

use bevy::prelude::*;

use crate::components::*;
use crate::resources::*;
use crate::constants::{FARM_BASE_GROWTH_RATE, FARM_TENDED_GROWTH_RATE, CAMP_FORAGE_RATE, STARVING_SPEED_MULT, SPAWNER_RESPAWN_HOURS, SPRITE_FARMER, SPRITE_MINER};
use crate::world::{self, WorldData, BuildingOccupancy, BuildingSpatialGrid, BuildingKind};
use crate::messages::{SpawnNpcMsg, GpuUpdate, GpuUpdateMsg};
use crate::systems::stats::{TownUpgrades, UpgradeType, UPGRADE_PCT};

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
// FARM GROWTH SYSTEM
// ============================================================================

/// Farm growth system: advances crop progress based on time and farmer presence.
/// - Passive growth: FARM_BASE_GROWTH_RATE per game hour (~12 hours to full)
/// - Tended growth: FARM_TENDED_GROWTH_RATE per game hour (~4 hours to full)
/// When progress >= 1.0, farm transitions to Ready state.
pub fn farm_growth_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut farm_states: ResMut<FarmStates>,
    world_data: Res<WorldData>,
    farm_occupancy: Res<BuildingOccupancy>,
    upgrades: Res<TownUpgrades>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("farm_growth");
    if game_time.paused {
        return;
    }

    // Calculate hours elapsed this frame
    let hours_elapsed = (time.delta_secs() * game_time.time_scale) / game_time.seconds_per_hour;

    for (farm_idx, farm) in world_data.farms.iter().enumerate() {
        // Skip if farm_states not initialized for this farm
        if farm_idx >= farm_states.states.len() {
            continue;
        }

        // Only grow farms that are in Growing state
        if farm_states.states[farm_idx] != FarmGrowthState::Growing {
            continue;
        }

        // Determine growth rate based on whether a farmer is working this farm
        let is_tended = farm_occupancy.is_occupied(farm.position);

        let base_rate = if is_tended {
            FARM_TENDED_GROWTH_RATE
        } else {
            FARM_BASE_GROWTH_RATE
        };

        // Apply FarmYield upgrade multiplier
        let town = farm.town_idx as usize;
        let yield_level = upgrades.levels.get(town).map(|l| l[UpgradeType::FarmYield as usize]).unwrap_or(0);
        let growth_rate = base_rate * (1.0 + yield_level as f32 * UPGRADE_PCT[UpgradeType::FarmYield as usize]);

        // Advance growth progress
        farm_states.progress[farm_idx] += growth_rate * hours_elapsed;

        // Transition to Ready when fully grown
        if farm_states.progress[farm_idx] >= 1.0 {
            farm_states.states[farm_idx] = FarmGrowthState::Ready;
            farm_states.progress[farm_idx] = 1.0; // Clamp at max
        }

        let _ = farm; // Silence unused warning
    }
}

// ============================================================================
// MINE REGEN SYSTEM
// ============================================================================

/// Gold mines slowly regenerate when below max capacity.
pub fn mine_regen_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut mine_states: ResMut<MineStates>,
    farm_occupancy: Res<BuildingOccupancy>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("mine_regen");
    if game_time.paused { return; }

    let hours_elapsed = (time.delta_secs() * game_time.time_scale) / game_time.seconds_per_hour;

    for i in 0..mine_states.gold.len() {
        // Only regen when mine is not being worked
        if farm_occupancy.is_occupied(mine_states.positions[i]) { continue; }
        if mine_states.gold[i] < mine_states.max_gold[i] {
            mine_states.gold[i] = (mine_states.gold[i] + crate::constants::MINE_REGEN_RATE * hours_elapsed)
                .min(mine_states.max_gold[i]);
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
    mut food_storage: ResMut<FoodStorage>,
    world_data: Res<WorldData>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("camp_forage");
    if !game_time.hour_ticked {
        return;
    }

    // Add foraging food to each raider camp (faction > 0)
    for (town_idx, town) in world_data.towns.iter().enumerate() {
        if town.faction > 0 && town_idx < food_storage.food.len() {
            food_storage.food[town_idx] += CAMP_FORAGE_RATE;
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
    farm_states: Res<FarmStates>,
    world_data: Res<crate::world::WorldData>,
    markers: Query<(Entity, &FarmReadyMarker)>,
    mut prev_states: Local<Vec<FarmGrowthState>>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("farm_visual");
    prev_states.resize(farm_states.states.len(), FarmGrowthState::Growing);
    for (farm_idx, state) in farm_states.states.iter().enumerate() {
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
                    world::resolve_spawner_npc(entry, &world_data.towns, &bgrid, &farm_occupancy);

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

                combat_log.push(
                    CombatEventKind::Spawn,
                    game_time.day(), game_time.hour(), game_time.minute(),
                    format!("{} respawned from {}", job_name, building_name),
                );
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
    available_guards: Query<(Entity, &NpcIndex, &TownId), (With<Guard>, Without<Dead>, Without<SquadId>)>,
    world_data: Res<WorldData>,
    squad_guards: Query<(Entity, &NpcIndex, &SquadId), (With<Guard>, Without<Dead>)>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("squad_cleanup");

    // Phase 1: remove dead members
    for squad in squad_state.squads.iter_mut() {
        squad.members.retain(|&slot| npc_map.0.contains_key(&slot));
    }

    // Phase 2: dismiss excess (target_size > 0 and members > target_size)
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

    // Phase 3: auto-recruit to fill target_size
    let player_town = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0) as i32;
    let mut pool: Vec<(Entity, usize)> = available_guards.iter()
        .filter(|(_, _, town)| town.0 == player_town)
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
// JOB REASSIGNMENT SYSTEM
// ============================================================================

/// Converts idle farmers↔miners to match MinerTarget per town.
/// Runs every frame but only converts NPCs that are Idle or Resting.
pub fn job_reassign_system(
    mut commands: Commands,
    miner_target: Res<MinerTarget>,
    npc_query: Query<(
        Entity, &NpcIndex, &Job, &TownId, &Activity, &Home,
        Option<&AssignedFarm>, Option<&WorkPosition>,
    ), Without<Dead>>,
    mut npc_meta: ResMut<NpcMetaCache>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut npc_logs: ResMut<NpcLogCache>,
    mut occupancy: ResMut<BuildingOccupancy>,
    _world_data: Res<WorldData>,
    game_time: Res<GameTime>,
    timings: Res<SystemTimings>,
    bgrid: Res<BuildingSpatialGrid>,
) {
    let _t = timings.scope("job_reassign");
    if miner_target.targets.is_empty() { return; }

    // Count current miners per town
    let num_towns = miner_target.targets.len();
    let mut miner_counts = vec![0i32; num_towns];
    for (_, _, job, town_id, _, _, _, _) in npc_query.iter() {
        if *job == Job::Miner && town_id.0 >= 0 && (town_id.0 as usize) < num_towns {
            miner_counts[town_id.0 as usize] += 1;
        }
    }

    // Collect convertible NPCs per town (idle/resting only)
    for town_idx in 0..num_towns {
        let target = miner_target.targets[town_idx];
        let current = miner_counts[town_idx];
        let diff = target - current;

        if diff == 0 { continue; }

        if diff > 0 {
            // Need more miners — convert idle farmers
            let mut converted = 0;
            let mut to_convert: Vec<(Entity, usize)> = Vec::new();
            for (entity, npc_idx, job, town_id, activity, _, _, _) in npc_query.iter() {
                if converted >= diff { break; }
                if *job != Job::Farmer || town_id.0 as usize != town_idx { continue; }
                if !matches!(*activity, Activity::Idle | Activity::Resting) { continue; }
                to_convert.push((entity, npc_idx.0));
                converted += 1;
            }
            for (entity, idx) in to_convert {
                // Release farm occupancy if assigned
                if let Ok((_, _, _, _, _, _, assigned_farm, _)) = npc_query.get(entity) {
                    if let Some(af) = assigned_farm {
                        occupancy.release(af.0);
                    }
                }
                commands.entity(entity)
                    .remove::<Farmer>()
                    .remove::<AssignedFarm>()
                    .remove::<WorkPosition>()
                    .insert(Miner)
                    .insert(Job::Miner)
                    .insert(Activity::Idle);
                // Update GPU sprite
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame {
                    idx, col: SPRITE_MINER.0, row: SPRITE_MINER.1, atlas: 0.0,
                }));
                // Update meta
                if idx < npc_meta.0.len() {
                    npc_meta.0[idx].job = 4;
                }
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                    "Reassigned → Miner");
            }
        } else {
            // Too many miners — convert idle miners back to farmers
            let needed = (-diff) as i32;
            let mut converted = 0;
            let mut to_convert: Vec<(Entity, usize, Vec2)> = Vec::new();
            for (entity, npc_idx, job, town_id, activity, home, _, work_pos) in npc_query.iter() {
                if converted >= needed { break; }
                if *job != Job::Miner || town_id.0 as usize != town_idx { continue; }
                if !matches!(*activity, Activity::Idle | Activity::Resting) { continue; }
                // Release mine occupancy if working
                if let Some(wp) = work_pos {
                    occupancy.release(wp.0);
                }
                to_convert.push((entity, npc_idx.0, home.0));
                converted += 1;
            }
            for (entity, idx, home_pos) in to_convert {
                // Find nearest free farm for this town
                let farm_pos = world::find_nearest_free(
                    home_pos, &bgrid, BuildingKind::Farm, &occupancy, Some(town_idx as u32),
                ).unwrap_or(home_pos);

                commands.entity(entity)
                    .remove::<Miner>()
                    .remove::<WorkPosition>()
                    .insert(Farmer)
                    .insert(Job::Farmer)
                    .insert(WorkPosition(farm_pos))
                    .insert(Activity::Idle);
                // Update GPU sprite
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame {
                    idx, col: SPRITE_FARMER.0, row: SPRITE_FARMER.1, atlas: 0.0,
                }));
                // Update meta
                if idx < npc_meta.0.len() {
                    npc_meta.0[idx].job = 0;
                }
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                    "Reassigned → Farmer");
            }
        }
    }
}
