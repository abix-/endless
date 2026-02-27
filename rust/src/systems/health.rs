//! Health systems - Damage, death detection, cleanup, healing aura

use bevy::prelude::*;
use bevy::ecs::system::SystemParam;
use crate::components::*;
use crate::constants::STARVING_HP_CAP;
use crate::messages::{GpuUpdate, GpuUpdateMsg, DamageMsg, DirtyWriters};
use crate::messages::CombatLogMsg;
use crate::resources::{EntityMap, HealthDebug, PopulationStats, KillStats, NpcsByTownCache, EntitySlots, GpuReadState, FactionStats, CombatEventKind, NpcMetaCache, GameTime, SelectedNpc, SelectedBuilding, SystemTimings, HealingZoneCache, BuildingHealState, EndlessMode, SquadState, FoodStorage, GoldStorage};
use crate::systems::stats::{CombatConfig, TownUpgrades, UPGRADES, level_from_xp, resolve_combat_stats};
use crate::constants::{UpgradeStatKind, ItemKind, building_def, npc_def};
use crate::systems::economy::*;
use crate::world::{WorldData, WorldGrid, TownGrids, BuildingKind};

/// Bundled resources for death_system — merged from CleanupResources + WorldState + BuildingDeathExtra.
#[derive(SystemParam)]
pub struct DeathResources<'w, 's> {
    pub entity_map: ResMut<'w, EntityMap>,
    pub pop_stats: ResMut<'w, PopulationStats>,
    pub faction_stats: ResMut<'w, FactionStats>,
    pub debug: ResMut<'w, HealthDebug>,
    pub kill_stats: ResMut<'w, KillStats>,
    pub npcs_by_town: ResMut<'w, NpcsByTownCache>,
    pub slots: ResMut<'w, EntitySlots>,
    pub dirty_writers: DirtyWriters<'w>,
    pub grid: ResMut<'w, WorldGrid>,
    pub world_data: ResMut<'w, WorldData>,
    pub selected_building: ResMut<'w, SelectedBuilding>,
    pub town_grids: ResMut<'w, TownGrids>,
    pub ai_state: ResMut<'w, crate::systems::AiPlayerState>,
    pub endless: ResMut<'w, EndlessMode>,
    pub npc_flags_q: Query<'w, 's, &'static mut crate::components::NpcFlags>,
    pub activity_q: Query<'w, 's, &'static mut crate::components::Activity>,
    pub health_q: Query<'w, 's, &'static mut crate::components::Health, Without<Building>>,
    pub combat_state_q: Query<'w, 's, &'static mut crate::components::CombatState>,
    pub cached_stats_q: Query<'w, 's, &'static mut crate::components::CachedStats>,
    pub attack_type_q: Query<'w, 's, &'static crate::components::BaseAttackType>,
    pub speed_q: Query<'w, 's, &'static mut crate::components::Speed>,
    pub energy_q: Query<'w, 's, &'static crate::components::Energy>,
    pub last_hit_by_q: Query<'w, 's, &'static crate::components::LastHitBy>,
    pub home_q: Query<'w, 's, &'static crate::components::Home>,
    pub personality_q: Query<'w, 's, &'static crate::components::Personality>,
    pub assigned_farm_q: Query<'w, 's, &'static crate::components::AssignedFarm>,
    pub work_position_q: Query<'w, 's, &'static crate::components::WorkPosition>,
}

/// Unified damage system: applies damage to both NPCs and buildings.
/// entity_idx = unified slot (same as GPU index, no offset arithmetic).
pub fn damage_system(
    mut commands: Commands,
    mut events: MessageReader<DamageMsg>,
    entity_map: Res<EntityMap>,
    mut npc_health_q: Query<&mut Health, Without<Building>>,
    mut building_query: Query<&mut Health, With<Building>>,
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

        if let Some(npc) = entity_map.get_npc(idx) {
            // NPC damage
            if npc.dead { continue; }
            let Ok(mut health) = npc_health_q.get_mut(npc.entity) else { continue };
            health.0 = (health.0 - event.amount).max(0.0);
            if event.attacker >= 0 {
                if let Ok(mut ec) = commands.get_entity(npc.entity) {
                    ec.insert(LastHitBy(event.attacker));
                }
            }
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health: health.0 }));
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetDamageFlash { idx, intensity: 1.0 }));
        } else if let Some(inst) = entity_map.get_instance(idx) {
            // Building damage
            if matches!(inst.kind, crate::world::BuildingKind::GoldMine | crate::world::BuildingKind::Road) { continue; }
            let Some(&entity) = entity_map.entities.get(&idx) else { continue };
            let Ok(mut health) = building_query.get_mut(entity) else { continue };
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
        }
    }

    debug.damage_processed = damage_count;
    debug.bevy_entity_count = entity_map.npc_count();
    debug.health_samples.clear();
    for npc in entity_map.iter_npcs().take(10) {
        let hp = npc_health_q.get(npc.entity).map(|h| h.0).unwrap_or(0.0);
        debug.health_samples.push((npc.slot, hp));
    }
}

fn hide_npc(idx: usize, entity_map: &mut EntityMap, slots: &mut EntitySlots, gpu: &mut MessageWriter<GpuUpdateMsg>) {
    entity_map.unregister_npc(idx);
    gpu.write(GpuUpdateMsg(GpuUpdate::Hide { idx }));
    slots.free(idx);
}

fn hide_building(idx: usize, entity_map: &mut EntityMap, alloc: &mut EntitySlots, gpu: &mut MessageWriter<GpuUpdateMsg>) {
    entity_map.remove_by_slot(idx);
    gpu.write(GpuUpdateMsg(GpuUpdate::Hide { idx }));
    gpu.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health: 0.0 }));
    alloc.free(idx);
}

/// Unified death system: mark dead, XP grant, building destruction, NPC cleanup, despawn.
/// NPCs: mark dead in EntityMap (immediate), process same frame.
/// Buildings: mark Dead ECS marker (deferred), process next frame via ECS query.
pub fn death_system(
    mut commands: Commands,
    building_mark_query: Query<(Entity, &Health), (With<Building>, Without<Dead>)>,
    building_dead_query: Query<(Entity, &EntitySlot, &Faction, &TownId,
        &Building, Option<&LastHitBy>), With<Dead>>,
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

    // Phase 1a: Mark newly dead NPCs (immediate — processed same frame)
    let mut death_count = 0;
    {
        let dead_slots: Vec<usize> = res.entity_map.iter_npcs()
            .filter(|n| !n.dead)
            .filter(|n| res.health_q.get(n.entity).map(|h| h.0 <= 0.0).unwrap_or(false))
            .map(|n| n.slot)
            .collect();
        for slot in dead_slots {
            if let Some(npc) = res.entity_map.get_npc_mut(slot) {
                npc.dead = true;
                death_count += 1;
            }
        }
    }

    // Phase 1b: Mark newly dead buildings (deferred — processed next frame)
    for (entity, health) in building_mark_query.iter() {
        if health.0 <= 0.0 {
            if let Ok(mut ec) = commands.get_entity(entity) {
                ec.insert(Dead);
                death_count += 1;
            }
        }
    }
    res.debug.deaths_this_frame = death_count;

    // Phase 2a: Process dead buildings (from previous frame, via ECS Dead query)
    let mut despawn_count = 0;
    let has_dead_buildings = !building_dead_query.is_empty();
    // Collect dead NPC slots for Phase 2b (avoids borrow conflict with killer processing)
    let dead_npc_slots: Vec<usize> = res.entity_map.iter_npcs()
        .filter(|n| n.dead)
        .map(|n| n.slot)
        .collect();

    if has_dead_buildings || !dead_npc_slots.is_empty() {
        res.dirty_writers.squads.write(crate::messages::SquadsDirtyMsg);
        res.dirty_writers.ai_squads.write(crate::messages::AiSquadsDirtyMsg);
    }

    for (entity, npc_idx, _faction, _town_id, _building, last_hit_by) in building_dead_query.iter() {
        let idx = npc_idx.0;
        if selected.0 == idx as i32 { selected.0 = -1; }
        commands.entity(entity).despawn();
        despawn_count += 1;

        let attacker = last_hit_by.map(|h| h.0).unwrap_or(-1);

        // Copy fields out before mutating entity_map
        if let Some(inst) = res.entity_map.get_instance(idx) {
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
                &mut res.entity_map, &mut combat_log, &game_time,
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
                    if let Some(atk) = res.entity_map.get_npc(attacker_slot) {
                        let dc_keep_fighting = res.npc_flags_q.get(atk.entity).map(|f| f.direct_control).unwrap_or(false) && squad_state.dc_no_return;
                        if let Ok(mut act) = res.activity_q.get_mut(atk.entity) {
                            if matches!(*act, Activity::Returning { .. }) {
                                act.add_loot(drop.item, amount);
                            } else {
                                *act = Activity::Returning { loot: vec![(drop.item, amount)] };
                            }
                        }
                        let atk_entity = atk.entity;
                        let atk_home = res.home_q.get(atk_entity).map(|h| h.0).unwrap_or(Vec2::ZERO);
                        let atk_faction = atk.faction;
                        let atk_slot = atk.slot;
                        if !dc_keep_fighting {
                            intents.submit(atk_entity, Vec2::new(atk_home.x, atk_home.y), crate::resources::MovementPriority::Survival, "loot:return");
                        }
                        let item_name = match drop.item { ItemKind::Food => "food", ItemKind::Gold => "gold" };
                        let killer_name = &npc_meta.0[atk_slot].name;
                        let killer_job = crate::job_name(npc_meta.0[atk_slot].job);
                        combat_log.write(CombatLogMsg { kind: CombatEventKind::Loot, faction: atk_faction, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: format!("{} '{}' looted {} {} from {:?}", killer_job, killer_name, amount, item_name, kind), location: None });
                    }
                }
            }
        }
        hide_building(idx, &mut res.entity_map, &mut res.slots, &mut gpu_updates);
        if res.selected_building.slot == Some(idx) {
            res.selected_building.active = false;
            res.selected_building.slot = None;
            res.selected_building.kind = None;
        }
    }

    // Phase 2b: Process dead NPCs (immediate — same frame as marking)
    for &slot in &dead_npc_slots {
        // Extract dead NPC data (immutable borrow ends before killer mutation)
        let (entity, faction, town_idx, job, activity, assigned_farm, work_position, last_hit_by) = {
            let Some(npc) = res.entity_map.get_npc(slot) else { continue };
            let activity = res.activity_q.get(npc.entity).map(|a| a.clone()).unwrap_or_default();
            let lhb = res.last_hit_by_q.get(npc.entity).map(|h| h.0).unwrap_or(-1);
            let assigned_farm = res.assigned_farm_q.get(npc.entity).ok().map(|af| af.0);
            let work_position = res.work_position_q.get(npc.entity).ok().map(|wp| wp.0);
            (npc.entity, npc.faction, npc.town_idx, npc.job, activity, assigned_farm, work_position, lhb)
        };

        if selected.0 == slot as i32 { selected.0 = -1; }
        commands.entity(entity).despawn();
        despawn_count += 1;

        // XP grant: reward killer with XP, level-up, and NPC kill loot
        if last_hit_by >= 0 {
            let killer_slot = last_hit_by as usize;
            if let Some(killer) = res.entity_map.get_npc(killer_slot) {
                let k_slot = killer.slot;
                let k_entity = killer.entity;
                let k_faction = killer.faction;
                let k_home = res.home_q.get(k_entity).map(|h| h.0).unwrap_or(Vec2::ZERO);
                res.faction_stats.inc_kills(k_faction);

                let meta = &mut npc_meta.0[k_slot];
                let old_xp = meta.xp;
                meta.xp += 100;
                let old_level = level_from_xp(old_xp);
                let new_level = level_from_xp(meta.xp);
                meta.level = new_level;

                if new_level > old_level {
                    let old_max = res.cached_stats_q.get(k_entity).map(|s| s.max_health).unwrap_or(100.0);
                    let pers = res.personality_q.get(k_entity).cloned().unwrap_or_default();
                    let attack_type = res.attack_type_q.get(k_entity).copied().unwrap_or(BaseAttackType::Melee);
                    let new_cached = resolve_combat_stats(killer.job, attack_type, killer.town_idx, new_level, &pers, &config, &upgrades);
                    let new_speed = new_cached.speed;
                    let new_max = new_cached.max_health;
                    if let Ok(mut cs) = res.cached_stats_q.get_mut(k_entity) { *cs = new_cached; }
                    if let Ok(mut spd) = res.speed_q.get_mut(k_entity) { spd.0 = new_speed; }
                    if old_max > 0.0 {
                        if let Ok(mut hp) = res.health_q.get_mut(k_entity) {
                            hp.0 = hp.0 * new_max / old_max;
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: k_slot, health: hp.0 }));
                        }
                    }
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx: k_slot, speed: new_speed }));

                    let name = &meta.name;
                    let job_str = crate::job_name(meta.job);
                    combat_log.write(CombatLogMsg { kind: CombatEventKind::LevelUp, faction: k_faction, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: format!("{} '{}' reached Lv.{}", job_str, name, new_level), location: None });
                }

                // NPC kill loot
                let drops = npc_def(job).loot_drop;
                let drop = &drops[(meta.xp as usize) % drops.len()];
                let amount = if drop.min == drop.max { drop.min } else {
                    drop.min + (meta.xp as i32 % (drop.max - drop.min + 1))
                };
                if amount > 0 {
                    let dc_keep_fighting = res.npc_flags_q.get(k_entity).map(|f| f.direct_control).unwrap_or(false) && squad_state.dc_no_return;
                    if !dc_keep_fighting {
                        if let Ok(mut cs) = res.combat_state_q.get_mut(k_entity) { *cs = CombatState::None; }
                    }
                    if let Ok(mut act) = res.activity_q.get_mut(k_entity) {
                        if matches!(*act, Activity::Returning { .. }) {
                            act.add_loot(drop.item, amount);
                        } else {
                            *act = Activity::Returning { loot: vec![(drop.item, amount)] };
                        }
                    }
                    if !dc_keep_fighting {
                        intents.submit(k_entity, Vec2::new(k_home.x, k_home.y), crate::resources::MovementPriority::Survival, "loot:return");
                    }

                    let item_name = match drop.item { ItemKind::Food => "food", ItemKind::Gold => "gold" };
                    let killer_name = &npc_meta.0[k_slot].name;
                    let killer_job = crate::job_name(npc_meta.0[k_slot].job);
                    combat_log.write(CombatLogMsg { kind: CombatEventKind::Loot, faction: k_faction, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: format!("{} '{}' looted {} {}", killer_job, killer_name, amount, item_name), location: None });
                }
            }
        }

        // NPC cleanup
        pop_dec_alive(&mut res.pop_stats, job, town_idx);
        pop_inc_dead(&mut res.pop_stats, job, town_idx);
        if matches!(activity, Activity::Working | Activity::MiningAtMine) {
            pop_dec_working(&mut res.pop_stats, job, town_idx);
        }

        if let Some(assigned) = assigned_farm {
            res.entity_map.release(assigned);
        }
        if let Some(wp) = work_position {
            res.entity_map.release(wp);
        }
        if job == Job::Miner {
            res.dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
        }

        if faction == 0 {
            res.kill_stats.villager_kills += 1;
        } else {
            res.kill_stats.archer_kills += 1;
        }

        // Combat log: death event
        let meta = &npc_meta.0[slot];
        let job_str = crate::job_name(meta.job);
        let msg = if meta.name.is_empty() {
            format!("{} #{} died", job_str, slot)
        } else {
            format!("{} '{}' Lv.{} died", job_str, meta.name, meta.level)
        };
        combat_log.write(CombatLogMsg { kind: CombatEventKind::Kill, faction, day: game_time.day(), hour: game_time.hour(), minute: game_time.minute(), message: msg, location: None });

        res.faction_stats.dec_alive(faction);
        res.faction_stats.inc_dead(faction);

        if town_idx >= 0 {
            let ti = town_idx as usize;
            if ti < res.npcs_by_town.0.len() {
                res.npcs_by_town.0[ti].retain(|&i| i != slot);
            }
        }

        hide_npc(slot, &mut res.entity_map, &mut res.slots, &mut gpu_updates);
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
/// Sets NpcFlags.healing flag (for gpu.rs visual).
/// Starving NPCs are capped at 50% HP.
pub fn healing_system(
    entity_map: Res<EntityMap>,
    mut npc_q: Query<(&EntitySlot, &mut Health, &CachedStats, &mut NpcFlags), Without<Building>>,
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

    for npc in entity_map.iter_npcs() {
        if npc.dead { continue; }
        let idx = npc.slot;
        npcs_checked += 1;

        let Ok((_, mut health, cached, mut flags)) = npc_q.get_mut(npc.entity) else { continue };

        // Calculate effective HP cap (50% if starving)
        let hp_cap = if flags.starving {
            cached.max_health * STARVING_HP_CAP
        } else {
            cached.max_health
        };

        // If starving and HP > cap, drain to cap
        if flags.starving && health.0 > hp_cap {
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
        let zones = cache.by_faction.get(npc.faction as usize).map(|v| v.as_slice()).unwrap_or(&[]);
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

            let heal_amount = zone_heal_rate * dt;
            if health.0 < hp_cap {
                health.0 = (health.0 + heal_amount).min(hp_cap);
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health: health.0 }));
                healed_count += 1;

                if !flags.healing {
                    flags.healing = true;
                }
            } else if flags.healing {
                flags.healing = false;
            }
        } else if flags.healing {
            flags.healing = false;
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
