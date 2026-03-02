//! Economy systems - Game time, population tracking, farm growth, raider town foraging, respawning

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use rand::Rng;
use std::collections::{HashMap, HashSet};

use crate::components::*;
use crate::constants::UpgradeStatKind;
use crate::constants::{
    BOAT_SPEED, ENDLESS_RESPAWN_DELAY_HOURS, FARM_BASE_GROWTH_RATE,
    FARM_TENDED_GROWTH_RATE, MIGRATION_BASE_SIZE, RAIDER_FORAGE_RATE, RAIDER_SETTLE_RADIUS,
    SPAWNER_RESPAWN_HOURS, STARVING_HP_CAP, STARVING_SPEED_MULT, TOWN_GRID_SPACING,
};
use crate::messages::{CombatLogMsg, GpuUpdate, GpuUpdateMsg, SpawnNpcMsg};
use crate::resources::*;
use crate::systemparams::{EconomyState, WorldState};
use crate::systems::ai_player::{AiKind, AiPersonality, AiPlayer, AiPlayerState};
use crate::systems::stats::{TownUpgrades, UPGRADES};
use crate::world::{self, Biome, BuildingKind, TownGrids, WorldData};

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
    mut entity_map: ResMut<EntityMap>,
    mut health_q: Query<&mut Health, With<Building>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
) {
    if game_time.is_paused() {
        return;
    }
    let dt = game_time.delta(&time);

    // Collect slots needing update (can't mutate EntityMap while iterating)
    let constructing: Vec<usize> = entity_map
        .iter_instances()
        .filter(|i| i.under_construction > 0.0)
        .map(|i| i.slot)
        .collect();

    for slot in constructing {
        let (kind, finished, new_hp) = {
            let inst = entity_map.get_instance_mut(slot).unwrap();
            inst.under_construction -= dt;
            let total = crate::constants::BUILDING_CONSTRUCT_SECS;
            if inst.under_construction <= 0.0 {
                // Construction complete
                inst.under_construction = 0.0;
                let def = crate::constants::building_def(inst.kind);
                if def.spawner.is_some() {
                    inst.respawn_timer = 0.0; // arm spawner
                }
                (inst.kind, true, def.hp)
            } else {
                let progress = (total - inst.under_construction) / total;
                let def = crate::constants::building_def(inst.kind);
                let hp = (progress * def.hp).max(0.01);
                (inst.kind, false, hp)
            }
        };
        // Update ECS health
        if let Some(&entity) = entity_map.entities.get(&slot) {
            if let Ok(mut health) = health_q.get_mut(entity) {
                health.0 = new_hp;
            }
        }
        // Update GPU health buffer
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth {
            idx: slot,
            health: new_hp,
        }));
        let _ = (kind, finished); // suppress unused warnings
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
    mut entity_map: ResMut<EntityMap>,
    upgrades: Res<TownUpgrades>,
) {
    if game_time.is_paused() {
        return;
    }

    let hours_elapsed = game_time.delta(&time) / game_time.seconds_per_hour;

    // Precompute per-town farm yield multiplier (avoids per-farm Vec clone + string lookup)
    let max_towns = upgrades.levels.len();
    let mut farm_mults: Vec<f32> = Vec::with_capacity(max_towns);
    for t in 0..max_towns {
        let levels = upgrades.town_levels(t);
        farm_mults.push(UPGRADES.stat_mult(&levels, "Farmer", UpgradeStatKind::Yield));
    }

    for inst in entity_map.iter_instances_mut() {
        if inst.position.x < -9000.0 || inst.growth_ready || inst.under_construction > 0.0 {
            continue;
        }
        match inst.kind {
            BuildingKind::Farm => {
                let is_tended = inst.occupants >= 1;
                let base_rate = if is_tended {
                    FARM_TENDED_GROWTH_RATE
                } else {
                    FARM_BASE_GROWTH_RATE
                };
                let mult = farm_mults
                    .get(inst.town_idx as usize)
                    .copied()
                    .unwrap_or(1.0);
                let growth_rate = base_rate * mult;
                if growth_rate > 0.0 {
                    inst.growth_progress += growth_rate * hours_elapsed;
                    if inst.growth_progress >= 1.0 {
                        inst.growth_ready = true;
                        inst.growth_progress = 1.0;
                    }
                }
            }
            BuildingKind::GoldMine => {
                let worker_count = inst.occupants as i32;
                let growth_rate = if worker_count > 0 {
                    crate::constants::MINE_TENDED_GROWTH_RATE
                        * crate::constants::mine_productivity_mult(worker_count)
                } else {
                    0.0
                };
                if growth_rate > 0.0 {
                    inst.growth_progress += growth_rate * hours_elapsed;
                    if inst.growth_progress >= 1.0 {
                        inst.growth_ready = true;
                        inst.growth_progress = 1.0;
                    }
                }
            }
            _ => {}
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
        if town.faction > 0 && town_idx < economy.food_storage.food.len() {
            if town_idx < raider_state.forage_timers.len() {
                raider_state.forage_timers[town_idx] += 1.0;
                if raider_state.forage_timers[town_idx] >= interval {
                    raider_state.forage_timers[town_idx] -= interval;
                    economy.food_storage.food[town_idx] += RAIDER_FORAGE_RATE;
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
/// Growing→Ready: spawn marker. Ready→Growing (harvest): despawn marker.
pub fn farm_visual_system(
    mut commands: Commands,
    entity_map: Res<EntityMap>,
    markers: Query<(Entity, &FarmReadyMarker)>,
    mut prev_ready: Local<HashMap<usize, bool>>,
    mut frame_count: Local<u32>,
) {
    // Cadence: only check every 4th frame (crop state changes slowly)
    *frame_count = frame_count.wrapping_add(1);
    if *frame_count % 4 != 0 {
        return;
    }

    for inst in entity_map.iter_kind(BuildingKind::Farm) {
        let was_ready = prev_ready.get(&inst.slot).copied().unwrap_or(false);
        if inst.growth_ready && !was_ready {
            commands.spawn(FarmReadyMarker {
                farm_slot: inst.slot,
            });
        } else if !inst.growth_ready && was_ready {
            for (entity, marker) in markers.iter() {
                if marker.farm_slot == inst.slot {
                    commands.entity(entity).despawn();
                }
            }
        }
        prev_ready.insert(inst.slot, inst.growth_ready);
    }
}

// ============================================================================
// BUILDING SPAWNER SYSTEM
// ============================================================================

/// Detects dead NPCs linked to House/Barracks/Tent buildings, counts down respawn timers,
/// and spawns replacements via GpuSlotPool + SpawnNpcMsg.
/// Only runs when game_time.hour_ticked is true.
pub fn spawner_respawn_system(
    game_time: Res<GameTime>,
    mut entity_map: ResMut<EntityMap>,
    mut slots: ResMut<GpuSlotPool>,
    mut spawn_writer: MessageWriter<SpawnNpcMsg>,
    world_data: Res<WorldData>,
    mut combat_log: MessageWriter<CombatLogMsg>,
    mut dirty_writers: crate::messages::DirtyWriters,
    mut uid_alloc: ResMut<crate::resources::NextEntityUid>,
) {
    if !game_time.hour_ticked {
        return;
    }

    // Collect spawner slots to avoid borrow conflict (need &mut for npc_uid/respawn_timer, & for resolve)
    let spawner_slots: Vec<usize> = entity_map
        .iter_instances()
        .filter(|i| i.respawn_timer > -2.0)
        .map(|i| i.slot)
        .collect();

    for bld_slot in spawner_slots {
        let Some(inst) = entity_map.get_instance(bld_slot) else {
            continue;
        };

        // Check if linked NPC died (UID no longer maps to a live slot)
        if let Some(npc_uid) = inst.npc_uid {
            let npc_alive = entity_map
                .slot_for_uid(npc_uid)
                .map(|s| entity_map.entities.contains_key(&s))
                .unwrap_or(false);
            if !npc_alive {
                let is_miner_home = inst.kind == BuildingKind::MinerHome;
                if let Some(inst_mut) = entity_map.get_instance_mut(bld_slot) {
                    inst_mut.npc_uid = None;
                    inst_mut.respawn_timer = SPAWNER_RESPAWN_HOURS;
                }
                if is_miner_home {
                    dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
                }
            }
        }

        let Some(inst) = entity_map.get_instance(bld_slot) else {
            continue;
        };
        // Count down respawn timer (>= 0.0 catches newly-built spawners at 0.0)
        if inst.respawn_timer >= 0.0 {
            let new_timer = inst.respawn_timer - 1.0;
            if let Some(inst_mut) = entity_map.get_instance_mut(bld_slot) {
                inst_mut.respawn_timer = new_timer;
            }
            if new_timer <= 0.0 {
                // Spawn replacement NPC
                let Some(slot) = slots.alloc_reset() else { continue };
                let Some(inst) = entity_map.get_instance(bld_slot) else {
                    continue;
                };
                let town_data_idx = inst.town_idx as usize;

                let (
                    job,
                    faction,
                    work_x,
                    work_y,
                    starting_post,
                    attack_type,
                    job_name,
                    building_name,
                    _work_slot,
                ) = world::resolve_spawner_npc(inst, &world_data.towns, &entity_map);

                let pos = inst.position;
                let is_miner_home = inst.kind == BuildingKind::MinerHome;
                let npc_uid = uid_alloc.next();
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
                    attack_type,
                    uid_override: Some(npc_uid),
                });
                if let Some(inst_mut) = entity_map.get_instance_mut(bld_slot) {
                    inst_mut.npc_uid = Some(npc_uid);
                    inst_mut.respawn_timer = -1.0;
                }
                if is_miner_home {
                    dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
                }

                combat_log.write(CombatLogMsg {
                    kind: CombatEventKind::Spawn,
                    faction,
                    day: game_time.day(),
                    hour: game_time.hour(),
                    minute: game_time.minute(),
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
    mut entity_map: ResMut<EntityMap>,
    policies: Res<TownPolicies>,
    mut mining: ResMut<MiningPolicy>,
    mut mining_dirty: MessageReader<crate::messages::MiningDirtyMsg>,
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
        if town.faction < 0 {
            mining.discovered_mines[town_idx].clear();
            continue;
        }
        let radius = policies
            .policies
            .get(town_idx)
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
        if world_data.towns[town_idx].faction < 0 {
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

        // Collect auto-assign miner home slots (O(town's miner homes) instead of O(all spawners))
        let auto_home_slots: Vec<usize> = entity_map
            .iter_kind_for_town(BuildingKind::MinerHome, town_idx as u32)
            .filter(|inst| {
                !inst.manual_mine
                    && inst.npc_uid.is_some()
                    && inst
                        .npc_uid
                        .and_then(|uid| entity_map.slot_for_uid(uid))
                        .map(|s| entity_map.entities.contains_key(&s))
                        .unwrap_or(false)
            })
            .map(|inst| inst.slot)
            .collect();

        // Clear stale assignments (mine disabled or no longer discovered)
        for &slot in &auto_home_slots {
            let Some(inst) = entity_map.get_instance(slot) else {
                continue;
            };
            if let Some(pos) = inst.assigned_mine {
                let cell = (
                    (pos.x / TOWN_GRID_SPACING).floor() as i32,
                    (pos.y / TOWN_GRID_SPACING).floor() as i32,
                );
                let still_enabled = enabled_grid_cells.contains(&cell);
                if !still_enabled {
                    if let Some(inst_mut) = entity_map.get_instance_mut(slot) {
                        inst_mut.assigned_mine = None;
                    }
                }
            }
        }

        if enabled_positions.is_empty() {
            for &slot in &auto_home_slots {
                if let Some(inst_mut) = entity_map.get_instance_mut(slot) {
                    inst_mut.assigned_mine = None;
                }
            }
            continue;
        }

        // Round-robin assign mines to auto homes
        for (i, &slot) in auto_home_slots.iter().enumerate() {
            let mine_pos = enabled_positions[i % enabled_positions.len()];
            if let Some(inst_mut) = entity_map.get_instance_mut(slot) {
                inst_mut.assigned_mine = Some(mine_pos);
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
        .position(|t| t.faction == 0)
        .unwrap_or(0) as i32;

    // Track pending assignments locally to avoid deferred-Commands read-after-write issues
    let mut pending_squad: HashMap<usize, Option<i32>> = HashMap::new();

    // Phase 1: remove dead members (all squads)
    for squad in squad_state.squads.iter_mut() {
        squad.members.retain(|&uid| {
            entity_map
                .slot_for_uid(uid)
                .and_then(|slot| entity_map.get_npc(slot))
                .is_some_and(|n| !n.dead)
        });
    }

    // Phase 2: keep Default Squad (index 0) as the live pool of unsquadded player military units.
    if let Some(default_squad) = squad_state.squads.get_mut(0) {
        if default_squad.is_player() {
            let new_members: Vec<(usize, Entity, crate::components::EntityUid)> = recruit_q
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
                    let uid = entity_map.uid_for_slot(slot)?;
                    Some((slot, entity, uid))
                })
                .collect();
            for (slot, entity, uid) in new_members {
                commands.entity(entity).insert(SquadId(0));
                pending_squad.insert(slot, Some(0));
                if !default_squad.members.contains(&uid) {
                    default_squad.members.push(uid);
                }
            }
        }
    }

    // Phase 3: dismiss excess (target_size > 0 and members > target_size, all squads)
    for (si, squad) in squad_state.squads.iter_mut().enumerate() {
        if squad.target_size > 0 && squad.members.len() > squad.target_size {
            let to_dismiss: Vec<crate::components::EntityUid> =
                squad.members.drain(squad.target_size..).collect();
            for &uid in &to_dismiss {
                let Some(slot) = entity_map.slot_for_uid(uid) else {
                    continue;
                };
                if let Some(&entity) = entity_map.entities.get(&slot) {
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
                .filter_map(|uid| entity_map.slot_for_uid(*uid))
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
                if let Some(uid) = entity_map.uid_for_slot(slot) {
                    squad.members.push(uid);
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
    pub food_storage: ResMut<'w, FoodStorage>,
    pub gold_storage: ResMut<'w, GoldStorage>,
    pub faction_stats: ResMut<'w, FactionStats>,
    pub raider_state: ResMut<'w, RaiderState>,
    pub npcs_by_town: ResMut<'w, NpcsByTownCache>,
    pub policies: ResMut<'w, TownPolicies>,
    pub gpu_updates: MessageWriter<'w, GpuUpdateMsg>,
    pub npc_flags_q: Query<'w, 's, &'static mut NpcFlags>,
    pub home_q: Query<'w, 's, &'static mut Home>,
}

/// Create a new AI town: allocate faction, push Town + TownGrid, extend all per-town
/// resource vecs, create an inactive AiPlayer with random personality.
/// Returns (town_data_idx, grid_idx, faction).
fn create_ai_town(
    grid: &crate::world::WorldGrid,
    world_data: &mut WorldData,
    entity_map: &EntityMap,
    town_grids: &mut TownGrids,
    res: &mut MigrationResources,
    ai_state: &mut AiPlayerState,
    center: Vec2,
    is_raider: bool,
) -> (usize, usize, i32) {
    let next_faction = world_data
        .towns
        .iter()
        .map(|t| t.faction)
        .max()
        .unwrap_or(0)
        + 1;
    let name = if is_raider {
        "Raider Town"
    } else {
        "Rival Town"
    };
    let sprite_type = if is_raider { 1 } else { 0 };

    world_data.towns.push(world::Town {
        name: name.into(),
        center,
        faction: next_faction,
        sprite_type,
    });
    let town_data_idx = world_data.towns.len() - 1;

    let mut tg = world::TownGrid::new_base(town_data_idx);
    tg.recompute_world_caps(center, grid);
    town_grids.grids.push(tg);
    let grid_idx = town_grids.grids.len() - 1;

    // Extend per-town resources
    let num_towns = world_data.towns.len();
    res.food_storage.food.resize(num_towns, 0);
    res.gold_storage.gold.resize(num_towns, 0);
    res.faction_stats
        .stats
        .resize(num_towns, FactionStat::default());
    res.raider_state.max_pop.resize(num_towns, 10);
    res.raider_state.respawn_timers.resize(num_towns, 0.0);
    res.raider_state.forage_timers.resize(num_towns, 0.0);
    res.npcs_by_town.0.resize(num_towns, Vec::new());
    res.policies
        .policies
        .resize(num_towns, PolicySet::default());

    // Create AiPlayer with random personality and road style
    let ai_kind = if is_raider {
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
    if let Some(policy) = res.policies.policies.get_mut(town_data_idx) {
        *policy = personality.default_policies();
        policy.mining_radius = super::ai_player::initial_mining_radius(entity_map, center);
    }
    ai_state.players.push(AiPlayer {
        town_data_idx,
        grid_idx,
        kind: ai_kind,
        personality,
        road_style,
        last_actions: std::collections::VecDeque::new(),
        active: false,
        build_enabled: true,
        upgrade_enabled: true,
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
    mut endless: ResMut<EndlessMode>,
    mut migration_state: ResMut<MigrationState>,
    mut world_state: WorldState,
    mut ai_state: ResMut<AiPlayerState>,
    mut upgrades: ResMut<TownUpgrades>,
    mut combat_log: MessageWriter<CombatLogMsg>,
    game_time: Res<GameTime>,
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
            mg.boat_pos += dir * BOAT_SPEED * game_time.delta(&time);

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
                        attack_type: 0,
                        uid_override: None,
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
                combat_log.write(CombatLogMsg {
                    kind: CombatEventKind::Raid,
                    faction: -1,
                    day: game_time.day(),
                    hour: game_time.hour(),
                    minute: game_time.minute(),
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
                combat_log.write(CombatLogMsg {
                    kind: CombatEventKind::Raid,
                    faction: -1,
                    day: game_time.day(),
                    hour: game_time.hour(),
                    minute: game_time.minute(),
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

        let (town_data_idx, grid_idx, _faction) = create_ai_town(
            &world_state.grid,
            &mut world_state.world_data,
            &world_state.entity_map,
            &mut world_state.town_grids,
            &mut res,
            &mut ai_state,
            mg.settle_target,
            is_raider,
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

        // Place buildings directly into EntityMap
        if let Some(town_grid) = world_state.town_grids.grids.get_mut(grid_idx) {
            world::place_buildings(
                &mut world_state.grid,
                &world_state.world_data,
                mg.settle_target,
                town_data_idx as u32,
                &config,
                town_grid,
                is_raider,
                &mut world_state.entity_slots,
                &mut world_state.entity_map,
                &mut world_state.uid_alloc,
                &mut commands,
                &mut res.gpu_updates,
            );
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
            .write(crate::messages::TerrainDirtyMsg);

        let kind_str = if is_raider {
            "raider band"
        } else {
            "rival faction"
        };
        combat_log.write(CombatLogMsg {
            kind: CombatEventKind::Raid,
            faction: -1,
            day: game_time.day(),
            hour: game_time.hour(),
            minute: game_time.minute(),
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

    let dt_hours = game_time.delta(&time) / game_time.seconds_per_hour;
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
            attack_type: 0,
            uid_override: None,
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
        grid_idx: 0,
    });

    let kind_str = if spawn.is_raider {
        "raider band"
    } else {
        "rival faction"
    };
    combat_log.write(CombatLogMsg {
        kind: CombatEventKind::Raid,
        faction: -1,
        day: game_time.day(),
        hour: game_time.hour(),
        minute: game_time.minute(),
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
        merchant_inv.stocks.resize_with(town_count, MerchantStock::default);
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
