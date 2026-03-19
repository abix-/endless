//! Economy systems - Game time, population tracking, farm growth, raider town foraging, respawning

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use rand::Rng;
use std::collections::{HashMap, HashSet};

use crate::components::*;
use crate::constants::UpgradeStatKind;
use crate::constants::{
    BOAT_SPEED, COW_FOOD_COST_PER_HOUR, COW_GROWTH_RATE, ENDLESS_RESPAWN_DELAY_HOURS,
    FARM_BASE_GROWTH_RATE, FARM_TENDED_GROWTH_RATE, MIGRATION_BASE_SIZE, RAIDER_FORAGE_RATE,
    RAIDER_SETTLE_RADIUS, SPAWNER_RESPAWN_HOURS, STARVING_HP_CAP, STARVING_SPEED_MULT,
    TOWN_GRID_SPACING,
};
use crate::messages::{CombatLogMsg, GpuUpdate, GpuUpdateMsg, SpawnNpcMsg};
use crate::resources::*;
use crate::systemparams::{EconomyState, GameLog, WorldState};
use crate::systems::ai_player::{AiKind, AiPersonality, AiPlayer, AiPlayerState};
use crate::systems::stats::UPGRADES;
use crate::world::{self, Biome, BuildingKind, WorldData};

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
pub fn game_time_system(time: Res<Time>, mut game_time: ResMut<GameTime>) {
    // Reset tick flag each frame
    game_time.hour_ticked = false;

    if game_time.is_paused() {
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
// CONSTRUCTION TICK SYSTEM
// ============================================================================

/// Tick building construction timers. Scales HP from 0.01→full over BUILDING_CONSTRUCT_SECS.
/// When complete, arms spawner (respawn_timer=0) and sets full HP.
pub fn construction_tick_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut buildings_q: Query<
        (
            &GpuSlot,
            &Building,
            &mut ConstructionProgress,
            &mut Health,
            Option<&mut SpawnerState>,
        ),
        Without<Sleeping>,
    >,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
) {
    if game_time.is_paused() {
        return;
    }
    let dt = game_time.delta(&time);

    for (gpu_slot, building, mut construction, mut health, spawner) in &mut buildings_q {
        if construction.0 <= 0.0 {
            continue;
        }
        let slot = gpu_slot.0;
        construction.0 -= dt;
        let total = crate::constants::BUILDING_CONSTRUCT_SECS;
        let new_hp = if construction.0 <= 0.0 {
            construction.0 = 0.0;
            // Arm spawner on completion
            if let Some(mut sp) = spawner {
                sp.respawn_timer = 0.0;
            }
            crate::constants::building_def(building.kind).hp
        } else {
            let progress = (total - construction.0) / total;
            (progress * crate::constants::building_def(building.kind).hp).max(0.01)
        };
        health.0 = new_hp;
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth {
            idx: slot,
            health: new_hp,
        }));
    }
}

// ============================================================================
// GROWTH SYSTEM (farms + mines)
// ============================================================================

/// Unified production system for farms and mines.
/// - Farms (Crops): daytime-only, passive + tended rates. Upgrade-scaled by FarmYield.
/// - Farms (Cows): day+night, autonomous growth, consumes food per hour.
/// - Mines: tended-only (MINE_TENDED_GROWTH_RATE). Zero production when unoccupied.
pub fn growth_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    entity_map: Res<EntityMap>,
    mut town_access: crate::systemparams::TownAccess,
    mut production_q: Query<
        (
            &GpuSlot,
            &Building,
            &TownId,
            &Position,
            &ConstructionProgress,
            &mut ProductionState,
            Option<&FarmModeComp>,
        ),
        Without<Sleeping>,
    >,
    world_data: Res<crate::world::WorldData>,
    skills_q: Query<&crate::components::NpcSkills>,
) {
    if game_time.is_paused() {
        return;
    }

    let hours_elapsed = game_time.delta(&time) / game_time.seconds_per_hour;
    let is_daytime = game_time.is_daytime();

    // Precompute per-town farm yield multiplier
    let max_towns = world_data.towns.len();
    let mut farm_mults: Vec<f32> = Vec::with_capacity(max_towns);
    for t in 0..max_towns {
        let levels = town_access.upgrade_levels(t as i32);
        farm_mults.push(UPGRADES.stat_mult(&levels, "Farmer", UpgradeStatKind::Yield));
    }

    // Track food costs per town from cow farms (batch deduct after loop)
    let mut cow_food_costs: Vec<(i32, i32)> = Vec::new();

    for (gpu_slot, building, town_id, pos, construction, mut production, farm_mode) in
        &mut production_q
    {
        if pos.x < -9000.0 || production.ready || construction.0 > 0.0 {
            continue;
        }
        let slot = gpu_slot.0;
        match building.kind {
            BuildingKind::Farm => {
                let mode = farm_mode.map_or(FarmMode::Crops, |m| m.0);
                match mode {
                    FarmMode::Crops => {
                        // Crops only grow during daytime
                        if !is_daytime {
                            continue;
                        }
                        let is_tended = entity_map.present_count(slot) >= 1;
                        let base_rate = if is_tended {
                            FARM_TENDED_GROWTH_RATE
                        } else {
                            FARM_BASE_GROWTH_RATE
                        };
                        let mut mult = farm_mults.get(town_id.0 as usize).copied().unwrap_or(1.0);
                        // Apply tending farmer's proficiency bonus
                        if is_tended {
                            if let Some(farmer_prof) = entity_map
                                .worksite_claimer(slot)
                                .and_then(|e| skills_q.get(e).ok())
                            {
                                mult *=
                                    crate::systems::stats::proficiency_mult(farmer_prof.farming);
                            }
                        }
                        let growth_rate = base_rate * mult;
                        if growth_rate > 0.0 {
                            production.progress += growth_rate * hours_elapsed;
                            if production.progress >= 1.0 {
                                production.ready = true;
                                production.progress = 1.0;
                            }
                        }
                    }
                    FarmMode::Cows => {
                        // Cows grow day and night, no tending needed
                        let mult = farm_mults.get(town_id.0 as usize).copied().unwrap_or(1.0);
                        let growth_rate = COW_GROWTH_RATE * mult;
                        if growth_rate > 0.0 {
                            production.progress += growth_rate * hours_elapsed;
                            // Track food cost for this cow farm
                            let food_cost =
                                (COW_FOOD_COST_PER_HOUR as f32 * hours_elapsed).ceil() as i32;
                            if food_cost > 0 {
                                cow_food_costs.push((town_id.0, food_cost));
                            }
                            if production.progress >= 1.0 {
                                production.ready = true;
                                production.progress = 1.0;
                            }
                        }
                    }
                }
            }
            BuildingKind::GoldMine => {
                let worker_count = entity_map.present_count(slot);
                let growth_rate = if worker_count > 0 {
                    crate::constants::MINE_TENDED_GROWTH_RATE
                        * crate::constants::mine_productivity_mult(worker_count)
                } else {
                    0.0
                };
                if growth_rate > 0.0 {
                    production.progress += growth_rate * hours_elapsed;
                    if production.progress >= 1.0 {
                        production.ready = true;
                        production.progress = 1.0;
                    }
                }
            }
            // Resource nodes: worker chops/quarries over time, one-shot destroy after yield
            BuildingKind::TreeNode => {
                let worker_count = entity_map.present_count(slot);
                if worker_count > 0 {
                    production.progress += crate::constants::TREE_CHOP_RATE * hours_elapsed;
                    if production.progress >= 1.0 {
                        production.ready = true;
                        production.progress = 1.0;
                    }
                }
            }
            BuildingKind::RockNode => {
                let worker_count = entity_map.present_count(slot);
                if worker_count > 0 {
                    production.progress += crate::constants::ROCK_QUARRY_RATE * hours_elapsed;
                    if production.progress >= 1.0 {
                        production.ready = true;
                        production.progress = 1.0;
                    }
                }
            }
            _ => {}
        }
    }

    // Batch-deduct cow food costs per town
    for (town_idx, cost) in cow_food_costs {
        if let Some(mut f) = town_access.food_mut(town_idx) {
            f.0 = (f.0 - cost).max(0);
        }
    }
}

// ============================================================================
// FARMING SKILL GAIN
// ============================================================================

/// Grant farming proficiency to farmers while they tend crops (Work/Active at a farm).
pub fn farming_skill_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut npc_q: Query<(
        &Job,
        &crate::components::Activity,
        &mut crate::components::NpcSkills,
    )>,
) {
    use crate::components::{ActivityKind, ActivityPhase};
    if game_time.is_paused() {
        return;
    }
    let hours_elapsed = game_time.delta(&time) / game_time.seconds_per_hour;
    if hours_elapsed <= 0.0 {
        return;
    }
    for (job, activity, mut skills) in &mut npc_q {
        if *job != Job::Farmer {
            continue;
        }
        if activity.kind != ActivityKind::Work || activity.phase != ActivityPhase::Active {
            continue;
        }
        skills.farming = (skills.farming + crate::constants::FARMING_SKILL_RATE * hours_elapsed)
            .min(crate::constants::MAX_PROFICIENCY);
    }
}

// ============================================================================
// SLEEPING SYNC SYSTEM (trees/rocks)
// ============================================================================

/// Sync `Sleeping` marker on density-spawned buildings based on occupancy.
/// Remove `Sleeping` when an NPC occupies the worksite; re-add when vacant.
/// Uses `ResourceNode` archetype filter to skip all non-resource buildings,
/// reducing iteration from O(all_buildings) to O(resource_nodes).
pub fn sync_sleeping_system(
    mut commands: Commands,
    entity_map: Res<EntityMap>,
    sleeping_q: Query<(Entity, &GpuSlot), (With<Sleeping>, With<ResourceNode>)>,
    awake_q: Query<(Entity, &GpuSlot), (Without<Sleeping>, With<ResourceNode>)>,
) {
    // Wake: remove Sleeping when occupied
    for (entity, gpu_slot) in &sleeping_q {
        if entity_map.present_count(gpu_slot.0) > 0 {
            commands.entity(entity).remove::<Sleeping>();
        }
    }
    // Re-sleep: add Sleeping when no occupants
    for (entity, gpu_slot) in &awake_q {
        if entity_map.present_count(gpu_slot.0) == 0 {
            commands.entity(entity).insert(Sleeping);
        }
    }
}

// ============================================================================
// RAIDER FORAGING SYSTEM
// ============================================================================

/// Raider foraging: each raider town accumulates hours and gains 1 food per
/// `raider_forage_hours` game hours. 0 = disabled. Only ticks when hour_ticked.
pub fn raider_forage_system(
    game_time: Res<GameTime>,
    mut economy: EconomyState,
    world_data: Res<WorldData>,
    user_settings: Res<crate::settings::UserSettings>,
    mut raider_state: ResMut<crate::resources::RaiderState>,
) {
    let interval = user_settings.raider_forage_hours;
    if !game_time.hour_ticked || interval <= 0.0 {
        return;
    }

    for (town_idx, town) in world_data.towns.iter().enumerate() {
        if town.faction != crate::constants::FACTION_PLAYER
            && town.faction != crate::constants::FACTION_NEUTRAL
        {
            if town_idx < raider_state.forage_timers.len() {
                raider_state.forage_timers[town_idx] += 1.0;
                if raider_state.forage_timers[town_idx] >= interval {
                    raider_state.forage_timers[town_idx] -= interval;
                    if let Some(mut f) = economy.towns.food_mut(town_idx as i32) {
                        f.0 += RAIDER_FORAGE_RATE;
                    }
                }
            }
        }
    }
}

// ============================================================================
// STARVATION SYSTEM
// ============================================================================

/// Starvation check: NPCs with zero energy become Starving.
/// Only runs when game_time.hour_ticked is true.
/// Starving NPCs have 50% speed and HP capped at 50%.
pub fn starvation_system(
    game_time: Res<GameTime>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut q: Query<
        (&GpuSlot, &Energy, &CachedStats, &mut NpcFlags, &mut Health),
        (Without<Building>, Without<Dead>),
    >,
) {
    if !game_time.hour_ticked {
        return;
    }

    for (slot, energy, cached, mut flags, mut health) in q.iter_mut() {
        if energy.0 <= 0.0 {
            if !flags.starving {
                flags.starving = true;
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed {
                    idx: slot.0,
                    speed: cached.speed * STARVING_SPEED_MULT,
                }));
            }
            // Always clamp HP for starving NPCs (handles transition + save/load edge cases)
            let hp_cap = cached.max_health * STARVING_HP_CAP;
            if health.0 > hp_cap {
                health.0 = hp_cap;
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth {
                    idx: slot.0,
                    health: health.0,
                }));
            }
        } else if flags.starving {
            flags.starving = false;
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed {
                idx: slot.0,
                speed: cached.speed,
            }));
        }
    }
}

// ============================================================================
// FARM VISUAL SYSTEM
// ============================================================================

/// Spawns/despawns FarmReadyMarker entities when farm state transitions.
/// Growing->Ready: spawn marker. Ready->Growing (harvest): despawn marker.
/// Uses a slot->marker-entity map for O(1) despawn instead of scanning all markers.
/// Compacts stale entries for removed farms so slot reuse stays correct.
pub fn farm_visual_system(
    mut commands: Commands,
    farms_q: Query<(&GpuSlot, &ProductionState), With<Building>>,
    markers: Query<(), With<FarmReadyMarker>>,
    mut marker_map: Local<HashMap<usize, Entity>>,
    mut frame_count: Local<u32>,
) {
    // Cadence: only check every 4th frame (crop state changes slowly)
    *frame_count = frame_count.wrapping_add(1);
    if !(*frame_count).is_multiple_of(4) {
        return;
    }

    let mut previous_markers = std::mem::take(&mut *marker_map);
    for (gpu_slot, production) in &farms_q {
        let slot = gpu_slot.0;
        let live_marker = previous_markers
            .remove(&slot)
            .filter(|entity| markers.get(*entity).is_ok());

        if production.ready {
            if let Some(marker_entity) = live_marker {
                marker_map.insert(slot, marker_entity);
            } else {
                let marker_entity = commands.spawn(FarmReadyMarker { farm_slot: slot }).id();
                marker_map.insert(slot, marker_entity);
            }
        } else if let Some(marker_entity) = live_marker {
            commands.entity(marker_entity).despawn();
        }
    }

    for marker_entity in previous_markers.into_values() {
        if markers.get(marker_entity).is_ok() {
            commands.entity(marker_entity).despawn();
        }
    }
}

// ============================================================================
// BUILDING SPAWNER SYSTEM
// ============================================================================

/// Detects dead NPCs linked to spawner buildings, counts down respawn timers,
/// and spawns replacements via GpuSlotPool + SpawnNpcMsg.
/// Only runs when game_time.hour_ticked is true.
pub fn spawner_respawn_system(
    mut game_log: GameLog,
    entity_map: ResMut<EntityMap>,
    mut slots: ResMut<GpuSlotPool>,
    mut spawn_writer: MessageWriter<SpawnNpcMsg>,
    world_data: Res<WorldData>,
    mut dirty_writers: crate::messages::DirtyWriters,
    mut spawner_q: Query<(&mut SpawnerState, Option<&MinerHomeConfig>)>,
) {
    if !game_log.game_time.hour_ticked {
        return;
    }

    // Use pre-built spawner index instead of scanning all buildings (O(spawners) not O(buildings))
    let spawner_slots = entity_map.spawner_slots().to_vec();

    for bld_slot in spawner_slots {
        let Some(inst) = entity_map.get_instance(bld_slot) else {
            continue;
        };
        let Some(&entity) = entity_map.entities.get(&bld_slot) else {
            continue;
        };
        let Ok((mut spawner, miner_cfg)) = spawner_q.get_mut(entity) else {
            continue;
        };

        // Check if linked NPC died (slot no longer has a live NPC)
        if let Some(npc_slot) = spawner.npc_slot {
            let npc_alive = entity_map.get_npc(npc_slot).is_some_and(|n| !n.dead);
            if !npc_alive {
                let is_miner_home = inst.kind == BuildingKind::MinerHome;
                spawner.npc_slot = None;
                spawner.respawn_timer = SPAWNER_RESPAWN_HOURS;
                if is_miner_home {
                    dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
                }
            }
        }

        // Count down respawn timer (>= 0.0 catches newly-built spawners at 0.0)
        if spawner.respawn_timer >= 0.0 {
            spawner.respawn_timer -= 1.0;
            if spawner.respawn_timer <= 0.0 {
                // Spawn replacement NPC
                let Some(slot) = slots.alloc_reset() else {
                    continue;
                };
                let Some(inst) = entity_map.get_instance(bld_slot) else {
                    continue;
                };
                let town_data_idx = inst.town_idx as usize;
                let assigned_mine = miner_cfg.and_then(|c| c.assigned_mine);

                let (
                    job,
                    faction,
                    work_x,
                    work_y,
                    starting_post,
                    job_name,
                    building_name,
                    _work_slot,
                ) = world::resolve_spawner_npc(inst, &world_data.towns, &entity_map, assigned_mine);

                let pos = inst.position;
                let is_miner_home = inst.kind == BuildingKind::MinerHome;
                spawn_writer.write(SpawnNpcMsg {
                    slot_idx: slot,
                    x: pos.x,
                    y: pos.y,
                    job,
                    faction,
                    town_idx: town_data_idx as i32,
                    home_x: pos.x,
                    home_y: pos.y,
                    work_x,
                    work_y,
                    starting_post,
                    entity_override: None,
                });
                spawner.npc_slot = Some(slot);
                spawner.respawn_timer = -1.0;
                if is_miner_home {
                    dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
                }

                game_log.combat_log.write(CombatLogMsg {
                    kind: CombatEventKind::Spawn,
                    faction,
                    day: game_log.game_time.day(),
                    hour: game_log.game_time.hour(),
                    minute: game_log.game_time.minute(),
                    message: format!("{} respawned from {}", job_name, building_name),
                    location: None,
                });
            }
        }
    }
}

/// Rebuild auto-mining discovery + assignments when mining topology/policy changes.
pub fn mining_policy_system(
    world_data: Res<WorldData>,
    entity_map: Res<EntityMap>,
    town_access: crate::systemparams::TownAccess,
    mut mining: ResMut<MiningPolicy>,
    mut mining_dirty: MessageReader<crate::messages::MiningDirtyMsg>,
    spawner_q: Query<&SpawnerState>,
    mut miner_cfg_q: Query<&mut MinerHomeConfig>,
) {
    if mining_dirty.read().count() == 0 {
        return;
    }

    // Mine discovery: iterate EntityMap gold mines, keyed by slot
    mining
        .discovered_mines
        .resize(world_data.towns.len(), Vec::new());

    for town_idx in 0..world_data.towns.len() {
        let town = &world_data.towns[town_idx];
        if town.faction == crate::constants::FACTION_NEUTRAL {
            mining.discovered_mines[town_idx].clear();
            continue;
        }
        let radius = town_access
            .policy(town_idx as i32)
            .map(|p| p.mining_radius)
            .unwrap_or(crate::constants::DEFAULT_MINING_RADIUS);
        let r2 = radius * radius;

        let mut discovered = Vec::new();
        for inst in entity_map.iter_kind(BuildingKind::GoldMine) {
            let d = inst.position - town.center;
            if d.length_squared() <= r2 {
                mining.mine_enabled.entry(inst.slot).or_insert(true);
                discovered.push(inst.slot);
            }
        }
        mining.discovered_mines[town_idx] = discovered;
    }

    for town_idx in 0..world_data.towns.len() {
        if world_data.towns[town_idx].faction == crate::constants::FACTION_NEUTRAL {
            continue;
        }

        let enabled_slots: Vec<usize> = mining.discovered_mines[town_idx]
            .iter()
            .copied()
            .filter(|&slot| *mining.mine_enabled.get(&slot).unwrap_or(&true))
            .collect();

        let enabled_positions: Vec<Vec2> = enabled_slots
            .iter()
            .filter_map(|&slot| entity_map.get_instance(slot).map(|i| i.position))
            .collect();
        let enabled_grid_cells: std::collections::HashSet<(i32, i32)> = enabled_positions
            .iter()
            .map(|p| {
                (
                    (p.x / TOWN_GRID_SPACING).floor() as i32,
                    (p.y / TOWN_GRID_SPACING).floor() as i32,
                )
            })
            .collect();

        // Collect auto-assign miner home slots via ECS components
        let auto_home_slots: Vec<usize> = entity_map
            .iter_kind_for_town(BuildingKind::MinerHome, town_idx as u32)
            .filter(|inst| {
                let Some(&entity) = entity_map.entities.get(&inst.slot) else {
                    return false;
                };
                let Ok(cfg) = miner_cfg_q.get(entity) else {
                    return false;
                };
                if cfg.manual_mine {
                    return false;
                }
                let Ok(sp) = spawner_q.get(entity) else {
                    return false;
                };
                sp.npc_slot
                    .map(|s| entity_map.get_npc(s).is_some_and(|n| !n.dead))
                    .unwrap_or(false)
            })
            .map(|inst| inst.slot)
            .collect();

        // Clear stale assignments (mine disabled or no longer discovered)
        for &slot in &auto_home_slots {
            let Some(&entity) = entity_map.entities.get(&slot) else {
                continue;
            };
            let Ok(cfg) = miner_cfg_q.get(entity) else {
                continue;
            };
            if let Some(pos) = cfg.assigned_mine {
                let cell = (
                    (pos.x / TOWN_GRID_SPACING).floor() as i32,
                    (pos.y / TOWN_GRID_SPACING).floor() as i32,
                );
                if !enabled_grid_cells.contains(&cell) {
                    if let Ok(mut cfg_mut) = miner_cfg_q.get_mut(entity) {
                        cfg_mut.assigned_mine = None;
                    }
                }
            }
        }

        if enabled_positions.is_empty() {
            for &slot in &auto_home_slots {
                let Some(&entity) = entity_map.entities.get(&slot) else {
                    continue;
                };
                if let Ok(mut cfg) = miner_cfg_q.get_mut(entity) {
                    cfg.assigned_mine = None;
                }
            }
            continue;
        }

        // Round-robin assign mines to auto homes
        for (i, &slot) in auto_home_slots.iter().enumerate() {
            let mine_pos = enabled_positions[i % enabled_positions.len()];
            let Some(&entity) = entity_map.entities.get(&slot) else {
                continue;
            };
            if let Ok(mut cfg) = miner_cfg_q.get_mut(entity) {
                cfg.assigned_mine = Some(mine_pos);
            }
        }
    }
}

/// Remove dead NPCs from squad member lists, auto-recruit to target_size,
/// and dismiss excess if over target. Owner-aware: recruits by TownId match.
pub fn squad_cleanup_system(
    mut squad_state: ResMut<SquadState>,
    entity_map: Res<EntityMap>,
    world_data: Res<WorldData>,
    mut squads_dirty: MessageReader<crate::messages::SquadsDirtyMsg>,
    mut commands: Commands,
    squad_id_q: Query<&SquadId>,
    mut npc_flags_q: Query<&mut NpcFlags>,
    recruit_q: Query<
        (&GpuSlot, &Job, &TownId, Option<&SquadId>),
        (Without<Building>, Without<Dead>),
    >,
) {
    if squads_dirty.read().count() == 0 {
        return;
    }
    let player_town = world_data
        .towns
        .iter()
        .position(|t| t.faction == crate::constants::FACTION_PLAYER)
        .unwrap_or(0) as i32;

    // Track pending assignments locally to avoid deferred-Commands read-after-write issues
    let mut pending_squad: HashMap<usize, Option<i32>> = HashMap::new();

    // Phase 1: remove dead members (all squads)
    for squad in squad_state.squads.iter_mut() {
        squad.members.retain(|&e| {
            entity_map
                .slot_for_entity(e)
                .and_then(|slot| entity_map.get_npc(slot))
                .is_some_and(|n| !n.dead)
        });
    }

    // Phase 2: keep Default Squad (index 0) as the live pool of unsquadded player military units.
    if let Some(default_squad) = squad_state.squads.get_mut(0) {
        if default_squad.is_player() {
            let new_members: Vec<(usize, Entity)> = recruit_q
                .iter()
                .filter(|(slot, job, town_id, sq_id)| {
                    job.is_military()
                        && town_id.0 == player_town
                        && !pending_squad
                            .get(&slot.0)
                            .map(|v| v.is_some())
                            .unwrap_or(sq_id.is_some())
                })
                .map(|(slot, _, _, _)| slot.0)
                .filter_map(|slot| {
                    let entity = *entity_map.entities.get(&slot)?;
                    Some((slot, entity))
                })
                .collect();
            for (slot, entity) in new_members {
                commands.entity(entity).insert(SquadId(0));
                pending_squad.insert(slot, Some(0));
                if !default_squad.members.contains(&entity) {
                    default_squad.members.push(entity);
                }
            }
        }
    }

    // Phase 3: dismiss excess (target_size > 0 and members > target_size, all squads)
    for (si, squad) in squad_state.squads.iter_mut().enumerate() {
        if squad.target_size > 0 && squad.members.len() > squad.target_size {
            let to_dismiss: Vec<Entity> = squad.members.drain(squad.target_size..).collect();
            for &entity in &to_dismiss {
                let Some(slot) = entity_map.slot_for_entity(entity) else {
                    continue;
                };
                {
                    let current_sq = pending_squad
                        .get(&slot)
                        .copied()
                        .flatten()
                        .or_else(|| squad_id_q.get(entity).ok().map(|s| s.0));
                    if current_sq == Some(si as i32) {
                        commands.entity(entity).remove::<SquadId>();
                        pending_squad.insert(slot, None);
                        if let Ok(mut flags) = npc_flags_q.get_mut(entity) {
                            flags.direct_control = false;
                        }
                    }
                }
            }
        }
    }

    // Phase 4: auto-recruit to fill target_size (owner-aware)
    let assigned_slots: HashSet<usize> = squad_state
        .squads
        .iter()
        .flat_map(|s| {
            s.members
                .iter()
                .filter_map(|e| entity_map.slot_for_entity(*e))
        })
        .collect();

    // Build per-owner pools: group available (unsquadded) military units by town.
    let mut pool_by_town: HashMap<i32, Vec<usize>> = HashMap::new();
    for (slot, job, town_id, sq_id) in recruit_q.iter() {
        if !job.is_military() {
            continue;
        }
        let eff_has_squad = pending_squad
            .get(&slot.0)
            .map(|v| v.is_some())
            .unwrap_or(sq_id.is_some());
        if eff_has_squad {
            continue;
        }
        if assigned_slots.contains(&slot.0) {
            continue;
        }
        pool_by_town.entry(town_id.0).or_default().push(slot.0);
    }

    for (si, squad) in squad_state.squads.iter_mut().enumerate() {
        if squad.target_size == 0 {
            continue;
        }
        let town_key = match squad.owner {
            SquadOwner::Player => player_town,
            SquadOwner::Town(tdi) => tdi as i32,
        };
        let pool = match pool_by_town.get_mut(&town_key) {
            Some(p) => p,
            None => continue,
        };
        while squad.members.len() < squad.target_size {
            if let Some(slot) = pool.pop() {
                if let Some(&entity) = entity_map.entities.get(&slot) {
                    commands.entity(entity).insert(SquadId(si as i32));
                    pending_squad.insert(slot, Some(si as i32));
                }
                if let Some(&entity) = entity_map.entities.get(&slot) {
                    squad.members.push(entity);
                }
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
pub struct MigrationResources<'w, 's> {
    pub faction_stats: ResMut<'w, FactionStats>,
    pub faction_list: ResMut<'w, crate::resources::FactionList>,
    pub raider_state: ResMut<'w, RaiderState>,
    pub town_index: ResMut<'w, crate::resources::TownIndex>,
    pub gpu_updates: MessageWriter<'w, GpuUpdateMsg>,
    pub npc_flags_q: Query<'w, 's, &'static mut NpcFlags>,
    pub home_q: Query<'w, 's, &'static mut Home>,
}

/// Create a new AI town: allocate faction, push Town, extend all per-town
/// resource vecs, create an inactive AiPlayer with random personality.
/// Returns (town_data_idx, faction).
fn create_ai_town(
    _grid: &crate::world::WorldGrid,
    world_data: &mut WorldData,
    entity_map: &EntityMap,
    res: &mut MigrationResources,
    ai_state: &mut AiPlayerState,
    commands: &mut Commands,
    center: Vec2,
    town_kind: crate::constants::TownKind,
) -> (usize, i32) {
    use crate::constants::town_def;
    let def = town_def(town_kind);
    let next_faction = world_data
        .towns
        .iter()
        .map(|t| t.faction)
        .max()
        .unwrap_or(0)
        + 1;
    world_data.towns.push(world::Town {
        name: def.label.into(),
        center,
        faction: next_faction,
        kind: town_kind,
    });
    let town_data_idx = world_data.towns.len() - 1;

    // Register in FactionList
    res.faction_list
        .factions
        .push(crate::resources::FactionData {
            kind: town_kind.faction_kind(),
            name: def.label.into(),
            towns: vec![town_data_idx],
        });

    // Extend per-town non-ECS resources
    let num_towns = world_data.towns.len();
    res.faction_stats
        .stats
        .resize(num_towns, FactionStat::default());
    res.raider_state.max_pop.resize(num_towns, 10);
    res.raider_state.respawn_timers.resize(num_towns, 0.0);
    res.raider_state.forage_timers.resize(num_towns, 0.0);

    // Create AiPlayer with random personality and road style
    let ai_kind = if def.is_raider {
        AiKind::Raider
    } else {
        AiKind::Builder
    };
    let mut rng = rand::rng();
    let personalities = [
        AiPersonality::Aggressive,
        AiPersonality::Balanced,
        AiPersonality::Economic,
    ];
    let personality = personalities[rng.random_range(0..personalities.len())];
    let road_style = super::ai_player::RoadStyle::random(&mut rng);
    let mut policy = personality.default_policies();
    policy.mining_radius = super::ai_player::initial_mining_radius(entity_map, center);
    ai_state.players.push(AiPlayer {
        town_data_idx,
        kind: ai_kind,
        personality,
        road_style,
        last_actions: std::collections::VecDeque::new(),
        policy_defaults_logged: false,
        active: false,
        build_enabled: true,
        upgrade_enabled: true,
        squad_indices: Vec::new(),
        squad_cmd: HashMap::new(),
        decision_timer: 0.0,
    });

    // Spawn ECS town entity
    let entity = commands
        .spawn((
            crate::components::TownMarker,
            crate::components::TownAreaLevel(0),
            crate::components::FoodStore(0),
            crate::components::GoldStore(0),
            crate::components::WoodStore(0),
            crate::components::StoneStore(0),
            crate::components::TownPolicy(policy),
            crate::components::TownUpgradeLevel::default(),
            crate::components::TownEquipment::default(),
        ))
        .id();
    res.town_index.0.insert(town_data_idx as i32, entity);

    (town_data_idx, next_faction)
}

/// Pick a settlement site far from all existing towns.
/// Samples random land positions, scores by min distance to any town, picks the farthest.
fn pick_settle_site(
    grid: &crate::world::WorldGrid,
    world_data: &WorldData,
    world_w: f32,
    world_h: f32,
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
        if grid.cell(gc, gr).is_some_and(|c| c.terrain == Biome::Water) {
            continue;
        }

        // Score: minimum distance to any existing town
        let min_dist = world_data
            .towns
            .iter()
            .map(|t| pos.distance(t.center))
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
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
    mut endless: ResMut<EndlessMode>,
    mut migration_state: ResMut<MigrationState>,
    mut world_state: WorldState,
    mut ai_state: ResMut<AiPlayerState>,
    mut game_log: GameLog,
    time: Res<Time>,
    config: Res<world::WorldGenConfig>,
    mut res: MigrationResources,
    mut spawn_writer: MessageWriter<SpawnNpcMsg>,
    position_q: Query<&Position>,
    mut commands: Commands,
) {
    // Debug button: queue an immediate raider spawn
    if migration_state.debug_spawn {
        migration_state.debug_spawn = false;
        endless.pending_spawns.push(PendingAiSpawn {
            delay_remaining: 0.0,
            is_raider: true,
            upgrade_levels: Vec::new(),
            starting_food: 0,
            starting_gold: 0,
        });
    }

    if !endless.enabled {
        return;
    }

    let world_w = world_state.grid.width as f32 * world_state.grid.cell_size;
    let world_h = world_state.grid.height as f32 * world_state.grid.cell_size;

    // === BOAT SAIL — move boat toward settle target, disembark when on shore ===
    if let Some(mg) = &mut migration_state.active {
        if let Some(boat_slot) = mg.boat_slot {
            let dir = (mg.settle_target - mg.boat_pos).normalize_or_zero();
            mg.boat_pos += dir * BOAT_SPEED * game_log.game_time.delta(&time);

            res.gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetPosition {
                idx: boat_slot,
                x: mg.boat_pos.x,
                y: mg.boat_pos.y,
            }));

            // Check if boat reached land
            let (gc, gr) = world_state.grid.world_to_grid(mg.boat_pos);
            let on_water = world_state
                .grid
                .cell(gc, gr)
                .map(|c| c.terrain == Biome::Water)
                .unwrap_or(true);

            if !on_water {
                // === DISEMBARK — spawn NPCs at boat position ===
                let next_faction = world_state
                    .world_data
                    .towns
                    .iter()
                    .map(|t| t.faction)
                    .max()
                    .unwrap_or(0)
                    + 1;
                let group_size = if mg.is_raider {
                    MIGRATION_BASE_SIZE + 5
                } else {
                    5
                };
                let mut rng = rand::rng();

                for _ in 0..group_size {
                    let Some(slot) = world_state.entity_slots.alloc_reset() else {
                        break;
                    };
                    let jx = mg.boat_pos.x + rng.random_range(-30.0..30.0);
                    let jy = mg.boat_pos.y + rng.random_range(-30.0..30.0);
                    let job = if mg.is_raider { 2 } else { 1 };
                    spawn_writer.write(SpawnNpcMsg {
                        slot_idx: slot,
                        x: jx,
                        y: jy,
                        job,
                        faction: next_faction,
                        town_idx: -1,
                        home_x: mg.settle_target.x,
                        home_y: mg.settle_target.y,
                        work_x: -1.0,
                        work_y: -1.0,
                        starting_post: -1,
                        entity_override: None,
                    });
                    mg.member_slots.push(slot);
                }
                mg.faction = next_faction;

                // Despawn boat entity and free GPU slot
                if let Some(npc) = world_state.entity_map.get_npc(boat_slot) {
                    commands.entity(npc.entity).despawn();
                }
                world_state.entity_map.unregister_npc(boat_slot);
                world_state.entity_slots.free(boat_slot);
                mg.boat_slot = None;

                let kind_str = if mg.is_raider { "Raiders" } else { "Settlers" };
                game_log.combat_log.write(CombatLogMsg {
                    kind: CombatEventKind::Raid,
                    faction: -1,
                    day: game_log.game_time.day(),
                    hour: game_log.game_time.hour(),
                    minute: game_log.game_time.minute(),
                    message: format!("{} have landed!", kind_str),
                    location: Some(mg.boat_pos),
                });
                info!(
                    "Migration disembarked at ({:.0}, {:.0}), faction {}",
                    mg.boat_pos.x, mg.boat_pos.y, next_faction
                );
            }

            // While boat active, skip attach/settle
            if mg.boat_slot.is_some() {
                return;
            }
        }
    }

    // === ATTACH Migrating flag to newly spawned members ===
    if let Some(mg) = &migration_state.active {
        for &slot in &mg.member_slots {
            if let Some(npc) = world_state.entity_map.get_npc(slot) {
                if let Ok(mut flags) = res.npc_flags_q.get_mut(npc.entity) {
                    if !flags.migrating {
                        flags.migrating = true;
                    }
                }
            }
        }
    }

    // === SETTLE — when NPCs are near a town, create AI town + buildings ===
    if let Some(mg) = &migration_state.active {
        if mg.town_data_idx.is_some() {
            return;
        } // already settled (shouldn't happen)

        let mut sum_x = 0.0f32;
        let mut sum_y = 0.0f32;
        let mut count = 0u32;
        let mut found = 0u32;
        for &slot in &mg.member_slots {
            if let Some(npc) = world_state.entity_map.get_npc(slot) {
                found += 1;
                let is_migrating = res
                    .npc_flags_q
                    .get(npc.entity)
                    .map(|f| f.migrating)
                    .unwrap_or(false);
                if is_migrating && !npc.dead {
                    if let Ok(pos) = position_q.get(npc.entity) {
                        sum_x += pos.x;
                        sum_y += pos.y;
                        count += 1;
                    }
                }
            }
        }
        if count == 0 {
            // found == 0 means NPCs haven't spawned yet (SpawnNpcMsg not processed) — wait
            if found == 0 && !mg.member_slots.is_empty() {
                return;
            }
            if found > 0 {
                // All spawned members are dead — migration wiped out, queue replacement
                let is_raider = mg.is_raider;
                let kind_str = if is_raider {
                    "raider band"
                } else {
                    "rival faction"
                };
                game_log.combat_log.write(CombatLogMsg {
                    kind: CombatEventKind::Raid,
                    faction: -1,
                    day: game_log.game_time.day(),
                    hour: game_log.game_time.hour(),
                    minute: game_log.game_time.minute(),
                    message: format!("The migrating {} was wiped out!", kind_str),
                    location: None,
                });
                endless.pending_spawns.push(PendingAiSpawn {
                    delay_remaining: ENDLESS_RESPAWN_DELAY_HOURS,
                    is_raider,
                    upgrade_levels: mg.upgrade_levels.clone(),
                    starting_food: mg.starting_food,
                    starting_gold: mg.starting_gold,
                });
                info!(
                    "Migration wiped out (is_raider={}), queued replacement in {}h",
                    is_raider, ENDLESS_RESPAWN_DELAY_HOURS
                );
            }
            migration_state.active = None;
            return;
        }
        let avg_pos = Vec2::new(sum_x / count as f32, sum_y / count as f32);

        let near_target = avg_pos.distance(mg.settle_target) < RAIDER_SETTLE_RADIUS;
        if !near_target {
            return;
        }

        // === CREATE TOWN + SETTLE ===
        let is_raider = mg.is_raider;
        let member_slots = mg.member_slots.clone();
        let town_kind = if is_raider {
            crate::constants::TownKind::AiRaider
        } else {
            crate::constants::TownKind::AiBuilder
        };

        let (town_data_idx, _faction) = create_ai_town(
            &world_state.grid,
            &mut world_state.world_data,
            &world_state.entity_map,
            &mut res,
            &mut ai_state,
            &mut commands,
            mg.settle_target,
            town_kind,
        );

        // Set starting resources on ECS town entity
        if let Some(&entity) = res.town_index.0.get(&(town_data_idx as i32)) {
            commands.entity(entity).insert((
                crate::components::FoodStore(mg.starting_food),
                crate::components::GoldStore(mg.starting_gold),
                crate::components::TownUpgradeLevel(mg.upgrade_levels.clone()),
            ));
        }

        // Place buildings directly into EntityMap
        let mut area_level = 0i32;
        world::place_buildings(
            &mut world_state.grid,
            &mut world_state.world_data,
            mg.settle_target,
            town_data_idx as u32,
            &config,
            town_kind,
            &mut world_state.entity_slots,
            &mut world_state.entity_map,
            &mut commands,
            &mut res.gpu_updates,
            &mut area_level,
        );
        if let Some(&entity) = res.town_index.0.get(&(town_data_idx as i32)) {
            commands
                .entity(entity)
                .insert(crate::components::TownAreaLevel(area_level));
        }
        world::stamp_dirt(&mut world_state.grid, &[mg.settle_target]);

        // Activate AI
        if let Some(player) = ai_state
            .players
            .iter_mut()
            .find(|p| p.town_data_idx == town_data_idx)
        {
            player.active = true;
        }

        // Settle NPCs: clear migrating, set home + town_idx
        for &slot in &member_slots {
            if let Some(npc) = world_state.entity_map.get_npc(slot) {
                let entity = npc.entity;
                if let Ok(mut flags) = res.npc_flags_q.get_mut(entity) {
                    flags.migrating = false;
                }
                if let Ok(mut home) = res.home_q.get_mut(entity) {
                    home.0 = mg.settle_target;
                }
            }
            // Update town_idx in the index
            if let Some(npc) = world_state.entity_map.get_npc_mut(slot) {
                npc.town_idx = town_data_idx as i32;
            }
        }

        world_state
            .dirty_writers
            .building_grid
            .write(crate::messages::BuildingGridDirtyMsg);
        world_state
            .dirty_writers
            .terrain
            .write(crate::messages::TerrainDirtyMsg { tile: None });

        let kind_str = if is_raider {
            "raider band"
        } else {
            "rival faction"
        };
        game_log.combat_log.write(CombatLogMsg {
            kind: CombatEventKind::Raid,
            faction: -1,
            day: game_log.game_time.day(),
            hour: game_log.game_time.hour(),
            minute: game_log.game_time.minute(),
            message: format!("A {} has settled nearby!", kind_str),
            location: Some(mg.settle_target),
        });
        info!(
            "Migration settled at ({:.0}, {:.0}), town_data_idx={}",
            mg.settle_target.x, mg.settle_target.y, town_data_idx
        );
        migration_state.active = None;
        return;
    }

    // === SPAWN BOAT — pick edge, allocate boat GPU slot ===
    if endless.pending_spawns.is_empty() {
        return;
    }

    let dt_hours = game_log.game_time.delta(&time) / game_log.game_time.seconds_per_hour;
    for spawn in &mut endless.pending_spawns {
        spawn.delay_remaining -= dt_hours;
    }

    let Some(idx) = endless
        .pending_spawns
        .iter()
        .position(|s| s.delay_remaining <= 0.0)
    else {
        return;
    };
    let spawn = endless.pending_spawns.remove(idx);

    // Pick settlement site first so we can approach from the nearest edge
    let settle_target =
        pick_settle_site(&world_state.grid, &world_state.world_data, world_w, world_h);
    info!(
        "Endless: settle target at ({:.0}, {:.0})",
        settle_target.x, settle_target.y
    );

    // Approach from the map edge closest to settle target
    let dist_north = settle_target.y;
    let dist_south = world_h - settle_target.y;
    let dist_west = settle_target.x;
    let dist_east = world_w - settle_target.x;
    let min_dist = dist_north.min(dist_south).min(dist_west).min(dist_east);

    let mut rng = rand::rng();
    let (spawn_x, spawn_y, direction) = if min_dist == dist_north {
        (rng.random_range(0.0..world_w), 100.0, "north")
    } else if min_dist == dist_south {
        (rng.random_range(0.0..world_w), world_h - 100.0, "south")
    } else if min_dist == dist_west {
        (100.0, rng.random_range(0.0..world_h), "west")
    } else {
        (world_w - 100.0, rng.random_range(0.0..world_h), "east")
    };

    // Spawn boat as a proper NPC entity (Job::Boat = 6)
    let boat_slot = world_state.entity_slots.alloc_reset();
    if let Some(bs) = boat_slot {
        spawn_writer.write(SpawnNpcMsg {
            slot_idx: bs,
            x: spawn_x,
            y: spawn_y,
            job: 6, // Job::Boat
            faction: 0,
            town_idx: -1,
            home_x: settle_target.x,
            home_y: settle_target.y,
            work_x: -1.0,
            work_y: -1.0,
            starting_post: -1,
            entity_override: None,
        });
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
    });

    let kind_str = if spawn.is_raider {
        "raider band"
    } else {
        "rival faction"
    };
    game_log.combat_log.write(CombatLogMsg {
        kind: CombatEventKind::Raid,
        faction: -1,
        day: game_log.game_time.day(),
        hour: game_log.game_time.hour(),
        minute: game_log.game_time.minute(),
        message: format!("A {} approaches from the {}!", kind_str, direction),
        location: Some(Vec2::new(spawn_x, spawn_y)),
    });
    info!("Endless: boat spawned from {} edge", direction);
}

// ============================================================================
// MERCHANT TICK SYSTEM
// ============================================================================

/// Countdown merchant refresh timers. When <=0 and town has a Merchant building, refresh stock.
pub fn merchant_tick_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    entity_map: Res<EntityMap>,
    mut merchant_inv: ResMut<MerchantInventory>,
    mut next_id: ResMut<NextLootItemId>,
    world_data: Res<WorldData>,
) {
    if game_time.is_paused() {
        return;
    }
    let hours_elapsed = game_time.delta(&time) / game_time.seconds_per_hour;
    if hours_elapsed <= 0.0 {
        return;
    }

    // Ensure stocks vec is sized
    let town_count = world_data.towns.len();
    if merchant_inv.stocks.len() < town_count {
        merchant_inv
            .stocks
            .resize_with(town_count, MerchantStock::default);
    }

    for town_idx in 0..town_count {
        // Only tick if this town has a merchant building
        if entity_map.count_for_town(BuildingKind::Merchant, town_idx as u32) == 0 {
            continue;
        }
        let stock = &mut merchant_inv.stocks[town_idx];
        stock.refresh_timer -= hours_elapsed;
        if stock.refresh_timer <= 0.0 {
            merchant_inv.refresh(town_idx, &mut next_id);
        }
    }
}

#[cfg(test)]
mod tests;
