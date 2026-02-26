//! Health systems - Damage, death detection, cleanup, healing aura

use bevy::prelude::*;
use bevy::ecs::system::SystemParam;
use crate::components::*;
use crate::constants::STARVING_HP_CAP;
use crate::messages::{GpuUpdate, GpuUpdateMsg, DamageMsg, DirtyWriters};
use crate::messages::CombatLogMsg;
use crate::resources::{EntityMap, HealthDebug, PopulationStats, KillStats, NpcsByTownCache, EntitySlots, GpuReadState, FactionStats, CombatEventKind, NpcMetaCache, GameTime, SelectedNpc, SystemTimings, HealingZoneCache, BuildingEntityMap, BuildingHealState, EndlessMode, SquadState, FoodStorage, GoldStorage};
use crate::systems::stats::{CombatConfig, TownUpgrades, UPGRADES, level_from_xp, resolve_combat_stats};
use crate::constants::{UpgradeStatKind, ItemKind, building_def, npc_def};
use crate::systems::economy::*;
use crate::world::{WorldData, WorldGrid, TownGrids, BuildingOccupancy, BuildingKind};

/// Bundled resources for death_system — merged from CleanupResources + WorldState + BuildingDeathExtra.
#[derive(SystemParam)]
pub struct DeathResources<'w> {
    pub npc_map: ResMut<'w, EntityMap>,
    pub pop_stats: ResMut<'w, PopulationStats>,
    pub faction_stats: ResMut<'w, FactionStats>,
    pub debug: ResMut<'w, HealthDebug>,
    pub kill_stats: ResMut<'w, KillStats>,
    pub npcs_by_town: ResMut<'w, NpcsByTownCache>,
    pub slots: ResMut<'w, EntitySlots>,
    pub farm_occupancy: ResMut<'w, BuildingOccupancy>,
    pub dirty_writers: DirtyWriters<'w>,
    pub building_slots: ResMut<'w, BuildingEntityMap>,
    pub grid: ResMut<'w, WorldGrid>,
    pub world_data: ResMut<'w, WorldData>,
    pub town_grids: ResMut<'w, TownGrids>,
    pub ai_state: ResMut<'w, crate::systems::AiPlayerState>,
    pub endless: ResMut<'w, EndlessMode>,
}

/// Unified damage system: applies damage to both NPCs and buildings.
/// entity_idx = unified slot (same as GPU index, no offset arithmetic).
pub fn damage_system(
    mut commands: Commands,
    mut events: MessageReader<DamageMsg>,
    npc_map: Res<EntityMap>,
    bmap: Res<BuildingEntityMap>,
    mut query: Query<(&mut Health, &EntitySlot)>,
    mut debug: ResMut<HealthDebug>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut heal_state: ResMut<BuildingHealState>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("damage");
    let mut damage_count = 0;
    for event in events.read() {
        damage_count += 1;
        let idx = event.entity_idx;

        // Try building first (BuildingEntityMap), then NPC (EntityMap)
        if let Some(entity) = bmap.get_entity(idx) {
            // Building damage
            let Some(inst) = bmap.get_instance(idx) else { continue };
            if matches!(inst.kind, crate::world::BuildingKind::GoldMine | crate::world::BuildingKind::Road) { continue; }
            let Ok((mut health, _npc_idx)) = query.get_mut(entity) else { continue };
            if health.0 <= 0.0 { continue; }

            health.0 = (health.0 - event.amount).max(0.0);
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health: health.0 }));
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetDamageFlash { idx, intensity: 1.0 }));

            if health.0 > 0.0 {
                heal_state.needs_healing = true;
            }
            if event.attacker >= 0 {
                if let Ok(mut ec) = commands.get_entity(entity) {
                    ec.insert(LastHitBy(event.attacker));
                }
            }
        } else if let Some(&entity) = npc_map.0.get(&idx) {
            // NPC damage
            if let Ok((mut health, npc_idx)) = query.get_mut(entity) {
                health.0 = (health.0 - event.amount).max(0.0);
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: npc_idx.0, health: health.0 }));
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetDamageFlash { idx: npc_idx.0, intensity: 1.0 }));
                if event.attacker >= 0 {
                    if let Ok(mut ec) = commands.get_entity(entity) {
                        ec.insert(LastHitBy(event.attacker));
                    }
                }
            }
        }
    }

    debug.damage_processed = damage_count;
    debug.bevy_entity_count = query.iter().count();
    debug.health_samples.clear();
    for (health, npc_idx) in query.iter().take(10) {
        debug.health_samples.push((npc_idx.0, health.0));
    }
}

fn hide_npc(idx: usize, npc_map: &mut EntityMap, slots: &mut EntitySlots, gpu: &mut MessageWriter<GpuUpdateMsg>) {
    npc_map.0.remove(&idx);
    gpu.write(GpuUpdateMsg(GpuUpdate::Hide { idx }));
    slots.free(idx);
}

fn hide_building(idx: usize, alloc: &mut EntitySlots, gpu: &mut MessageWriter<GpuUpdateMsg>) {
    gpu.write(GpuUpdateMsg(GpuUpdate::Hide { idx }));
    gpu.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health: 0.0 }));
    alloc.free(idx);
}

/// Unified death system: mark dead, XP grant, building destruction, NPC cleanup, despawn.
/// Phase 1: mark newly dead entities (deferred — takes effect next frame).
/// Phase 2: process dead entities from previous frame (XP, loot, building effects, despawn).
pub fn death_system(
    mut commands: Commands,
    mut params: ParamSet<(
        // p0: mark-dead check (reads Health)
        Query<(Entity, &Health, &EntitySlot), Without<Dead>>,
        // p1: killer/loot entity access (writes Health for level-up HP rescaling)
        Query<(&EntitySlot, &Job, &TownId, &BaseAttackType, &Personality,
            &mut Health, &mut CachedStats, &mut Speed, &Faction, &mut Activity,
            &Home, &mut CombatState, Option<&DirectControl>), Without<Dead>>,
    )>,
    dead_query: Query<(Entity, &EntitySlot, &Faction, &TownId,
        Option<&Job>, Option<&Activity>, Option<&AssignedFarm>,
        Option<&WorkPosition>, Option<&Building>, Option<&LastHitBy>), With<Dead>>,
    mut res: DeathResources,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut combat_log: MessageWriter<CombatLogMsg>,
    game_time: Res<GameTime>,
    mut npc_meta: ResMut<NpcMetaCache>,
    mut selected: ResMut<SelectedNpc>,
    timings: Res<SystemTimings>,
    squad_state: Res<SquadState>,
    upgrades: Res<TownUpgrades>,
    food_storage: Res<FoodStorage>,
    gold_storage: Res<GoldStorage>,
    config: Res<CombatConfig>,
    mut intents: ResMut<crate::resources::MovementIntents>,
) {
    let _t = timings.scope("death");

    // Phase 1: mark newly dead entities (deferred commands — takes effect next frame)
    let mut death_count = 0;
    for (entity, health, _npc_idx) in params.p0().iter() {
        if health.0 <= 0.0 {
            if let Ok(mut ec) = commands.get_entity(entity) {
                ec.insert(Dead);
                death_count += 1;
            }
        }
    }
    res.debug.deaths_this_frame = death_count;

    // Phase 2: process dead entities from previous frame
    let mut despawn_count = 0;
    if !dead_query.is_empty() {
        res.dirty_writers.squads.write(crate::messages::SquadsDirtyMsg);
        res.dirty_writers.ai_squads.write(crate::messages::AiSquadsDirtyMsg);
    }
    for (entity, npc_idx, faction, town_id, job, activity, assigned_farm, work_position, building, last_hit_by) in dead_query.iter() {
        let idx = npc_idx.0;

        if selected.0 == idx as i32 {
            selected.0 = -1;
        }
        commands.entity(entity).despawn();
        despawn_count += 1;

        // ── Building death ──────────────────────────────────────────────
        if building.is_some() {
            let attacker = last_hit_by.map(|h| h.0).unwrap_or(-1);

            // Copy fields out before mutating building_slots
            if let Some(inst) = res.building_slots.get_instance(idx) {
                let kind = inst.kind;
                let pos = inst.position;
                let town_idx = inst.town_idx as usize;

                let town_name = res.world_data.towns.get(town_idx)
                    .map(|t| t.name.clone()).unwrap_or_default();
                let center = res.world_data.towns.get(town_idx)
                    .map(|t| t.center).unwrap_or_default();
                let (trow, tcol) = crate::world::world_to_town_grid(center, pos);
                let defender_faction = res.world_data.towns.get(town_idx).map(|t| t.faction).unwrap_or(0);

                combat_log.write(CombatLogMsg { kind: CombatEventKind::BuildingDamage, faction: defender_faction, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: format!("{:?} destroyed in {}", kind, town_name), location: None });

                let _ = crate::world::destroy_building(
                    &mut res.grid, &res.world_data,
                    &mut res.building_slots, &mut combat_log, &game_time,
                    trow, tcol, center,
                    &format!("{:?} destroyed in {}", kind, town_name),
                    &mut gpu_updates,
                );
                res.dirty_writers.mark_building_changed(kind);

                // Fountain destroyed → deactivate AI player
                if matches!(kind, BuildingKind::Fountain) {
                    if let Some(player) = res.ai_state.players.iter_mut().find(|p| p.town_data_idx == town_idx) {
                        player.active = false;
                    }
                    combat_log.write(CombatLogMsg { kind: CombatEventKind::Raid, faction: defender_faction, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: format!("{} has been defeated!", town_name), location: None });
                    info!("{} (town_idx={}) defeated — AI deactivated", town_name, town_idx);

                    // Endless mode: queue replacement AI scaled to player strength
                    if res.endless.enabled {
                        let is_raider = res.world_data.towns.get(town_idx)
                            .map(|t| t.sprite_type == 1).unwrap_or(true);
                        let player_town = res.world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);
                        let player_levels = upgrades.town_levels(player_town);
                        let frac = res.endless.strength_fraction;
                        let scaled_levels: Vec<u8> = player_levels.iter()
                            .map(|&lv| (lv as f32 * frac).round() as u8)
                            .collect();
                        let starting_food = (food_storage.food.get(player_town).copied().unwrap_or(0) as f32 * frac) as i32;
                        let starting_gold = (gold_storage.gold.get(player_town).copied().unwrap_or(0) as f32 * frac) as i32;
                        res.endless.pending_spawns.push(crate::resources::PendingAiSpawn {
                            delay_remaining: crate::constants::ENDLESS_RESPAWN_DELAY_HOURS,
                            is_raider,
                            upgrade_levels: scaled_levels,
                            starting_food,
                            starting_gold,
                        });
                        info!("Endless mode: queued replacement AI (is_raider={}, delay={}h, strength={:.0}%)",
                            is_raider, crate::constants::ENDLESS_RESPAWN_DELAY_HOURS, frac * 100.0);
                    }
                }

                // Loot: attacker picks up building loot and returns home
                if let Some(drop) = building_def(kind).loot_drop() {
                    let amount = if drop.min == drop.max { drop.min } else {
                        drop.min + ((idx as i32) % (drop.max - drop.min + 1))
                    };
                    if amount > 0 && attacker >= 0 {
                        let attacker_slot = attacker as usize;
                        if let Some(&attacker_entity) = res.npc_map.0.get(&attacker_slot) {
                            if let Ok((loot_npc_idx, _job, _town, _atk, _pers, _health, _cached, _speed, loot_faction, mut loot_activity, loot_home, _combat, dc)) = params.p1().get_mut(attacker_entity) {
                                let dc_keep_fighting = dc.is_some() && squad_state.dc_no_return;

                                if matches!(&*loot_activity, Activity::Returning { .. }) {
                                    loot_activity.add_loot(drop.item, amount);
                                } else {
                                    *loot_activity = Activity::Returning { loot: vec![(drop.item, amount)] };
                                }
                                if !dc_keep_fighting {
                                    intents.submit(attacker_entity, Vec2::new(loot_home.0.x, loot_home.0.y), crate::resources::MovementPriority::Survival, "loot:return");
                                }

                                let item_name = match drop.item { ItemKind::Food => "food", ItemKind::Gold => "gold" };
                                let killer_name = &npc_meta.0[loot_npc_idx.0].name;
                                let killer_job = crate::job_name(npc_meta.0[loot_npc_idx.0].job);
                                combat_log.write(CombatLogMsg { kind: CombatEventKind::Loot, faction: loot_faction.0, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: format!("{} '{}' looted {} {} from {:?}", killer_job, killer_name, amount, item_name, kind), location: None });
                            }
                        }
                    }
                }
            }
            res.building_slots.remove_by_slot(idx);
            hide_building(idx, &mut res.slots, &mut gpu_updates);
            continue;
        }

        // ── NPC death ───────────────────────────────────────────────────

        // XP grant: reward killer with XP, level-up, and NPC kill loot
        if let Some(last_hit) = last_hit_by {
            if last_hit.0 >= 0 {
                let killer_slot = last_hit.0 as usize;
                if let Some(&killer_entity) = res.npc_map.0.get(&killer_slot) {
                    if let Ok((k_npc_idx, k_job, k_town, k_atk, k_pers, mut k_health, mut k_cached, mut k_speed, k_faction, mut k_activity, k_home, mut k_combat, k_dc)) = params.p1().get_mut(killer_entity) {
                        let k_idx = k_npc_idx.0;
                        res.faction_stats.inc_kills(k_faction.0);
                        let meta = &mut npc_meta.0[k_idx];
                        let old_xp = meta.xp;
                        meta.xp += 100;
                        let old_level = level_from_xp(old_xp);
                        let new_level = level_from_xp(meta.xp);
                        meta.level = new_level;

                        if new_level > old_level {
                            let old_max = k_cached.max_health;
                            *k_cached = resolve_combat_stats(*k_job, *k_atk, k_town.0, new_level, k_pers, &config, &upgrades);
                            k_speed.0 = k_cached.speed;
                            if old_max > 0.0 {
                                k_health.0 = k_health.0 * k_cached.max_health / old_max;
                            }
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx: k_idx, speed: k_cached.speed }));
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: k_idx, health: k_health.0 }));

                            let name = &meta.name;
                            let job_str = crate::job_name(meta.job);
                            combat_log.write(CombatLogMsg { kind: CombatEventKind::LevelUp, faction: k_faction.0, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: format!("{} '{}' reached Lv.{}", job_str, name, new_level), location: None });
                        }

                        // NPC kill loot
                        let dead_job = job.copied().unwrap_or(Job::Farmer);
                        let drops = npc_def(dead_job).loot_drop;
                        let drop = &drops[(meta.xp as usize) % drops.len()];
                        let amount = if drop.min == drop.max { drop.min } else {
                            drop.min + (meta.xp as i32 % (drop.max - drop.min + 1))
                        };
                        if amount > 0 {
                            let dc_keep_fighting = k_dc.is_some() && squad_state.dc_no_return;
                            if !dc_keep_fighting {
                                *k_combat = CombatState::None;
                            }
                            if matches!(&*k_activity, Activity::Returning { .. }) {
                                k_activity.add_loot(drop.item, amount);
                            } else {
                                *k_activity = Activity::Returning { loot: vec![(drop.item, amount)] };
                            }
                            if !dc_keep_fighting {
                                intents.submit(killer_entity, Vec2::new(k_home.0.x, k_home.0.y), crate::resources::MovementPriority::Survival, "loot:return");
                            }

                            let item_name = match drop.item { ItemKind::Food => "food", ItemKind::Gold => "gold" };
                            let killer_name = &npc_meta.0[k_idx].name;
                            let killer_job = crate::job_name(npc_meta.0[k_idx].job);
                            combat_log.write(CombatLogMsg { kind: CombatEventKind::Loot, faction: k_faction.0, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: format!("{} '{}' looted {} {}", killer_job, killer_name, amount, item_name), location: None });
                        }
                    }
                }
            }
        }

        // NPC cleanup
        let job = job.copied().unwrap_or(Job::Farmer);
        pop_dec_alive(&mut res.pop_stats, job, town_id.0);
        pop_inc_dead(&mut res.pop_stats, job, town_id.0);
        if matches!(activity, Some(Activity::Working) | Some(Activity::MiningAtMine)) {
            pop_dec_working(&mut res.pop_stats, job, town_id.0);
        }

        if let Some(assigned) = assigned_farm {
            res.farm_occupancy.release(assigned.0);
        }
        if let Some(wp) = work_position {
            res.farm_occupancy.release(wp.0);
        }
        if job == Job::Miner {
            res.dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
        }

        if faction.0 == 0 {
            res.kill_stats.villager_kills += 1;
        } else {
            res.kill_stats.archer_kills += 1;
        }

        // Combat log: death event
        let meta = &npc_meta.0[idx];
        let job_str = crate::job_name(meta.job);
        let msg = if meta.name.is_empty() {
            format!("{} #{} died", job_str, idx)
        } else {
            format!("{} '{}' Lv.{} died", job_str, meta.name, meta.level)
        };
        combat_log.write(CombatLogMsg { kind: CombatEventKind::Kill, faction: faction.0, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: msg, location: None });

        res.faction_stats.dec_alive(faction.0);
        res.faction_stats.inc_dead(faction.0);

        if town_id.0 >= 0 {
            let town_idx = town_id.0 as usize;
            if town_idx < res.npcs_by_town.0.len() {
                res.npcs_by_town.0[town_idx].retain(|&i| i != idx);
            }
        }

        hide_npc(idx, &mut res.npc_map, &mut res.slots, &mut gpu_updates);
    }

    res.debug.despawned_this_frame = despawn_count;
}

/// Rebuild healing zone cache when dirty (upgrade purchased, town changed, save loaded).
pub fn update_healing_zone_cache(
    mut cache: ResMut<HealingZoneCache>,
    mut healing_dirty: MessageReader<crate::messages::HealingZonesDirtyMsg>,
    world_data: Res<WorldData>,
    combat_config: Res<CombatConfig>,
    upgrades: Res<TownUpgrades>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("healing_zones");
    if healing_dirty.read().count() == 0 { return; }

    let max_faction = world_data.towns.iter().map(|t| t.faction).max().unwrap_or(0);
    let faction_count = (max_faction + 1).max(0) as usize;
    cache.by_faction.clear();
    cache.by_faction.resize_with(faction_count, Vec::new);

    for (town_idx, town) in world_data.towns.iter().enumerate() {
        if town.faction < 0 { continue; }
        let town_levels = upgrades.town_levels(town_idx);
        let heal_mult = UPGRADES.stat_mult(&town_levels, "Town", UpgradeStatKind::Healing);
        let radius_lvl = UPGRADES.stat_level(&town_levels, "Town", UpgradeStatKind::FountainRange);
        let radius = combat_config.heal_radius + radius_lvl as f32 * 24.0;
        let heal_rate = combat_config.heal_rate * heal_mult;

        cache.by_faction[town.faction as usize].push(crate::resources::HealingZone {
            center: town.center,
            radius_sq: radius * radius,
            heal_rate,
        });
    }

    #[cfg(debug_assertions)]
    info!("Healing zone cache rebuilt: {} factions", cache.by_faction.len());
}

/// Heal NPCs inside their faction's town center healing aura.
/// Adds/removes Healing marker for visual feedback.
/// Starving NPCs are capped at 50% HP.
pub fn healing_system(
    mut commands: Commands,
    mut query: Query<(Entity, &EntitySlot, &mut Health, &CachedStats, &Faction, &TownId, Option<&Healing>, Option<&Starving>), (Without<Dead>, Without<Building>)>,
    mut building_query: Query<(&EntitySlot, &mut Health, &Faction, &Building), Without<Dead>>,
    gpu_state: Res<GpuReadState>,
    entity_gpu_state: Res<crate::gpu::EntityGpuState>,
    cache: Res<HealingZoneCache>,
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut debug: ResMut<HealthDebug>,
    mut heal_state: ResMut<BuildingHealState>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("healing");
    let positions = &gpu_state.positions;
    let dt = game_time.delta(&time);

    // Debug tracking
    let mut npcs_checked = 0usize;
    let mut in_zone_count = 0usize;
    let mut healed_count = 0usize;

    for (entity, npc_idx, mut health, cached, faction, _town_id, healing_marker, starving) in query.iter_mut() {
        let idx = npc_idx.0;
        npcs_checked += 1;

        // Calculate effective HP cap (50% if starving)
        let hp_cap = if starving.is_some() {
            cached.max_health * STARVING_HP_CAP
        } else {
            cached.max_health
        };

        // If starving and HP > cap, drain to cap
        if starving.is_some() && health.0 > hp_cap {
            health.0 = hp_cap;
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health: health.0 }));
        }

        if idx * 2 + 1 >= positions.len() {
            continue;
        }

        let x = positions[idx * 2];
        let y = positions[idx * 2 + 1];

        // Faction-indexed zone lookup: only check same-faction zones, dist² (no sqrt)
        let mut in_healing_zone = false;
        let mut zone_heal_rate = 0.0;
        let zones = cache.by_faction.get(faction.0 as usize).map(|v| v.as_slice()).unwrap_or(&[]);
        for zone in zones {
            let dx = x - zone.center.x;
            let dy = y - zone.center.y;
            if dx * dx + dy * dy <= zone.radius_sq {
                in_healing_zone = true;
                zone_heal_rate = zone.heal_rate;
                break;
            }
        }

        if in_healing_zone {
            in_zone_count += 1;

            // Heal up to HP cap (50% if starving, 100% otherwise)
            let heal_amount = zone_heal_rate * dt;
            if health.0 < hp_cap {
                health.0 = (health.0 + heal_amount).min(hp_cap);
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health: health.0 }));
                healed_count += 1;

                // Add marker if not present (visual derived by build_visual_upload)
                if healing_marker.is_none() {
                    commands.entity(entity).insert(Healing);
                }
            } else if healing_marker.is_some() {
                // Fully healed — remove marker
                commands.entity(entity).remove::<Healing>();
            }
        } else {
            // Remove marker if present
            if healing_marker.is_some() {
                commands.entity(entity).remove::<Healing>();
            }
        }
    }

    // Heal damaged buildings in same-faction fountain range (entity-driven).
    if heal_state.needs_healing {
        let bld_positions = &entity_gpu_state.positions;
        let mut any_damaged = false;
        for (npc_idx, mut health, faction, building) in building_query.iter_mut() {
            let max_hp = crate::constants::building_def(building.kind).hp;
            if health.0 <= 0.0 || health.0 >= max_hp { continue; }
            any_damaged = true;
            let idx = npc_idx.0;
            if idx * 2 + 1 >= bld_positions.len() { continue; }
            let x = bld_positions[idx * 2];
            let y = bld_positions[idx * 2 + 1];
            if faction.0 < 0 { continue; }
            let zones = cache.by_faction.get(faction.0 as usize).map(|v| v.as_slice()).unwrap_or(&[]);
            let mut zone_heal_rate = 0.0f32;
            for zone in zones {
                let dx = x - zone.center.x;
                let dy = y - zone.center.y;
                if dx * dx + dy * dy <= zone.radius_sq {
                    zone_heal_rate = zone.heal_rate;
                    break;
                }
            }
            if zone_heal_rate <= 0.0 { continue; }
            health.0 = (health.0 + zone_heal_rate * dt).min(max_hp);
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health: health.0 }));
        }
        if !any_damaged { heal_state.needs_healing = false; }
    }

    // Update debug stats
    debug.healing_npcs_checked = npcs_checked;
    debug.healing_positions_len = positions.len();
    debug.healing_towns_count = cache.by_faction.iter().map(|v| v.len()).sum();
    debug.healing_in_zone_count = in_zone_count;
    debug.healing_healed_count = healed_count;
}
