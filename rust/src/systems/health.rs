//! Health systems - Damage, death detection, cleanup, healing aura

use crate::components::*;
use crate::constants::STARVING_HP_CAP;
use crate::messages::CombatLogMsg;
use crate::messages::{DamageMsg, DirtyWriters, GpuUpdate, GpuUpdateMsg};
use crate::resources::{
    ActiveHealingSlots, BuildingHealState, CombatEventKind, EndlessMode, EntityMap, FactionStats,
    FoodStorage, GameTime, GoldStorage, GpuReadState, GpuSlotPool, HealingZoneCache, HealthDebug,
    KillStats, NpcMetaCache, PopulationStats, SelectedBuilding, SelectedNpc,
    SquadState,
};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::constants::{ItemKind, UpgradeStatKind, building_def, npc_def};
use crate::systems::economy::*;
use crate::systems::stats::{
    CombatConfig, TownUpgrades, UPGRADES, level_from_xp, resolve_combat_stats,
};
use crate::world::{BuildingKind, WorldData, WorldGrid};

/// Bundled resources for death_system — merged from CleanupResources + WorldState + BuildingDeathExtra.
#[derive(SystemParam)]
pub struct DeathResources<'w, 's> {
    pub entity_map: ResMut<'w, EntityMap>,
    pub pop_stats: ResMut<'w, PopulationStats>,
    pub faction_stats: ResMut<'w, FactionStats>,
    pub debug: ResMut<'w, HealthDebug>,
    pub kill_stats: ResMut<'w, KillStats>,
    pub slots: ResMut<'w, GpuSlotPool>,
    pub dirty_writers: DirtyWriters<'w>,
    pub grid: ResMut<'w, WorldGrid>,
    pub world_data: ResMut<'w, WorldData>,
    pub selected_building: ResMut<'w, SelectedBuilding>,
    pub ai_state: ResMut<'w, crate::systems::AiPlayerState>,
    pub endless: ResMut<'w, EndlessMode>,
    pub npc_flags_q: Query<'w, 's, &'static mut crate::components::NpcFlags>,
    pub activity_q: Query<'w, 's, &'static mut crate::components::Activity>,
    pub health_q: Query<'w, 's, (Entity, &'static mut crate::components::Health, &'static GpuSlot), (Without<Building>, Without<Dead>)>,
    pub combat_state_q: Query<'w, 's, &'static mut crate::components::CombatState>,
    pub cached_stats_q: Query<'w, 's, &'static mut crate::components::CachedStats>,
    pub attack_type_q: Query<'w, 's, &'static crate::components::BaseAttackType>,
    pub speed_q: Query<'w, 's, &'static mut crate::components::Speed>,
    pub energy_q: Query<'w, 's, &'static crate::components::Energy>,
    pub last_hit_by_q: Query<'w, 's, &'static crate::components::LastHitBy>,
    pub home_q: Query<'w, 's, &'static mut crate::components::Home>,
    pub personality_q: Query<'w, 's, &'static crate::components::Personality>,
    pub work_state_q: Query<'w, 's, &'static crate::components::NpcWorkState>,
    pub carried_loot_q: Query<'w, 's, &'static mut crate::components::CarriedLoot>,
    pub sfx_writer: MessageWriter<'w, crate::resources::PlaySfxMsg>,
    pub work_intents: MessageWriter<'w, crate::messages::WorkIntentMsg>,
    pub gpu_state: Res<'w, crate::gpu::EntityGpuState>,
    pub next_loot_id: ResMut<'w, crate::resources::NextLootItemId>,
    pub town_inventory: ResMut<'w, crate::resources::TownInventory>,
    pub equipment_q: Query<'w, 's, &'static crate::components::NpcEquipment>,
    pub reputation: ResMut<'w, crate::resources::Reputation>,
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
) {
    let mut damage_count = 0;
    for event in events.read() {
        damage_count += 1;
        let Some(idx) = entity_map.slot_for_uid(event.target) else {
            continue;
        };

        if let Some(npc) = entity_map.get_npc(idx) {
            // NPC damage
            if npc.dead {
                continue;
            }
            let Ok(mut health) = npc_health_q.get_mut(npc.entity) else {
                continue;
            };
            health.0 = (health.0 - event.amount).max(0.0);
            if event.attacker >= 0 {
                if let Ok(mut ec) = commands.get_entity(npc.entity) {
                    ec.insert(LastHitBy(event.attacker));
                }
            }
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth {
                idx,
                health: health.0,
            }));
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetDamageFlash {
                idx,
                intensity: 1.0,
            }));
        } else if let Some(inst) = entity_map.get_instance(idx) {
            // Building damage
            if inst.kind == crate::world::BuildingKind::GoldMine || inst.kind.is_road() {
                continue;
            }
            let Some(&entity) = entity_map.entities.get(&idx) else {
                continue;
            };
            let Ok(mut health) = building_query.get_mut(entity) else {
                continue;
            };
            if health.0 <= 0.0 {
                continue;
            }

            health.0 = (health.0 - event.amount).max(0.0);
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth {
                idx,
                health: health.0,
            }));
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetDamageFlash {
                idx,
                intensity: 1.0,
            }));

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
    if damage_count > 0 {
        for npc in entity_map.iter_npcs().take(10) {
            let hp = npc_health_q.get(npc.entity).map(|h| h.0).unwrap_or(0.0);
            debug.health_samples.push((npc.slot, hp));
        }
    }
}

fn hide_npc(
    idx: usize,
    entity_map: &mut EntityMap,
    slots: &mut GpuSlotPool,
    _gpu: &mut MessageWriter<GpuUpdateMsg>,
) {
    entity_map.unregister_npc(idx);
    slots.free(idx);
}

fn hide_building(
    idx: usize,
    entity_map: &mut EntityMap,
    alloc: &mut GpuSlotPool,
    _gpu: &mut MessageWriter<GpuUpdateMsg>,
) {
    entity_map.remove_by_slot(idx);
    alloc.free(idx);
}

/// Unified death system: mark dead, XP grant, building destruction, NPC cleanup, despawn.
/// NPCs: mark dead in EntityMap (immediate), process same frame.
/// Buildings: mark Dead ECS marker (deferred), process next frame via ECS query.
pub fn death_system(
    mut commands: Commands,
    building_mark_query: Query<(Entity, &Health), (With<Building>, Without<Dead>)>,
    building_dead_query: Query<
        (
            Entity,
            &GpuSlot,
            &Faction,
            &TownId,
            &Building,
            Option<&LastHitBy>,
        ),
        With<Dead>,
    >,
    mut res: DeathResources,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut combat_log: MessageWriter<CombatLogMsg>,
    mut game_time: ResMut<GameTime>,
    mut npc_meta: ResMut<NpcMetaCache>,
    mut selected: ResMut<SelectedNpc>,
    squad_state: Res<SquadState>,
    upgrades: Res<TownUpgrades>,
    mut food_storage: ResMut<FoodStorage>,
    mut gold_storage: ResMut<GoldStorage>,
    config: Res<CombatConfig>,
    mut intents: ResMut<crate::resources::PathRequestQueue>,
    mut ui_state: ResMut<crate::resources::UiState>,
) {
    // Phase 1a: Mark newly dead NPCs via ECS query (no iter_npcs scan)
    let mut death_count = 0;
    let dead_npc_slots: Vec<usize> = {
        let mut newly_dead = Vec::new();
        for (entity, health, gpu_slot) in res.health_q.iter() {
            if health.0 <= 0.0 {
                newly_dead.push(gpu_slot.0);
                if let Ok(mut ec) = commands.get_entity(entity) {
                    ec.insert(Dead);
                }
            }
        }
        for &slot in &newly_dead {
            if let Some(npc) = res.entity_map.get_npc_mut(slot) {
                npc.dead = true;
                death_count += 1;
            }
        }
        newly_dead
    };

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
    // Reuse Phase 1a's dead_npc_slots — death_system processes all dead NPCs every frame,
    // so no dead NPCs persist across frames. Eliminates a second O(n) iter_npcs() scan.

    if has_dead_buildings || !dead_npc_slots.is_empty() {
        res.dirty_writers
            .squads
            .write(crate::messages::SquadsDirtyMsg);
    }

    for (entity, npc_idx, _faction, _town_id, _building, last_hit_by) in building_dead_query.iter()
    {
        let idx = npc_idx.0;
        if selected.0 == idx as i32 {
            selected.0 = -1;
        }
        commands.entity(entity).despawn();
        despawn_count += 1;

        let attacker = last_hit_by.map(|h| h.0).unwrap_or(-1);

        // Copy fields out before mutating entity_map
        if let Some(inst) = res.entity_map.get_instance(idx) {
            let kind = inst.kind;
            let pos = inst.position;
            let town_idx = inst.town_idx as usize;
            let npc_uid = inst.npc_uid;

            // Orphaned NPC loses its home
            if let Some(uid) = npc_uid {
                if let Some(npc_entity) = res.entity_map.entity_by_uid(uid) {
                    if let Ok(mut home) = res.home_q.get_mut(npc_entity) {
                        home.0 = Vec2::new(-1.0, -1.0);
                    }
                }
            }

            let town_name = res
                .world_data
                .towns
                .get(town_idx)
                .map(|t| t.name.clone())
                .unwrap_or_default();
            let (gc, gr) = res.grid.world_to_grid(pos);
            let defender_faction = res
                .world_data
                .towns
                .get(town_idx)
                .map(|t| t.faction)
                .unwrap_or(0);

            combat_log.write(CombatLogMsg {
                kind: CombatEventKind::BuildingDamage,
                faction: defender_faction,
                day: game_time.day(),
                hour: game_time.hour(),
                minute: game_time.minute(),
                message: format!("{:?} destroyed in {}", kind, town_name),
                location: None,
            });

            let _ = crate::world::destroy_building(
                &mut res.grid,
                &res.world_data,
                &mut res.entity_map,
                &mut combat_log,
                &game_time,
                gc,
                gr,
                &format!("{:?} destroyed in {}", kind, town_name),
                &mut gpu_updates,
            );
            res.dirty_writers.mark_building_changed(kind);

            // Fountain destroyed → deactivate AI player
            if matches!(kind, BuildingKind::Fountain) {
                if let Some(player) = res
                    .ai_state
                    .players
                    .iter_mut()
                    .find(|p| p.town_data_idx == town_idx)
                {
                    player.active = false;
                }
                combat_log.write(CombatLogMsg {
                    kind: CombatEventKind::Raid,
                    faction: defender_faction,
                    day: game_time.day(),
                    hour: game_time.hour(),
                    minute: game_time.minute(),
                    message: format!("{} has been defeated!", town_name),
                    location: None,
                });
                info!(
                    "{} (town_idx={}) defeated — AI deactivated",
                    town_name, town_idx
                );

                // Remove roads + restore dirt to natural terrain
                let town_center = res.world_data.towns.get(town_idx)
                    .map(|t| t.center).unwrap_or_default();
                crate::world::clear_town_roads_and_dirt(
                    &mut res.grid,
                    &mut res.entity_map,
                    &mut res.slots,
                    town_center,
                    town_idx as u32,
                    &mut commands,
                );
                res.dirty_writers
                    .terrain
                    .write(crate::messages::TerrainDirtyMsg);
                res.dirty_writers
                    .building_grid
                    .write(crate::messages::BuildingGridDirtyMsg);

                // Player fountain destroyed → trigger game over screen
                if defender_faction == crate::constants::FACTION_PLAYER {
                    ui_state.game_over = true;
                    game_time.paused = true;
                }

                // Endless mode: queue replacement AI scaled to player strength
                if res.endless.enabled {
                    let is_raider = res
                        .world_data
                        .towns
                        .get(town_idx)
                        .map(|t| t.sprite_type == 1)
                        .unwrap_or(true);
                    let player_town = res
                        .world_data
                        .towns
                        .iter()
                        .position(|t| t.faction == crate::constants::FACTION_PLAYER)
                        .unwrap_or(0);
                    let player_levels = upgrades.town_levels(player_town);
                    let frac = res.endless.strength_fraction;
                    let scaled_levels: Vec<u8> = player_levels
                        .iter()
                        .map(|&lv| (lv as f32 * frac).round() as u8)
                        .collect();
                    let starting_food = (food_storage.food.get(player_town).copied().unwrap_or(0)
                        as f32
                        * frac) as i32;
                    let starting_gold = (gold_storage.gold.get(player_town).copied().unwrap_or(0)
                        as f32
                        * frac) as i32;
                    res.endless
                        .pending_spawns
                        .push(crate::resources::PendingAiSpawn {
                            delay_remaining: crate::constants::ENDLESS_RESPAWN_DELAY_HOURS,
                            is_raider,
                            upgrade_levels: scaled_levels,
                            starting_food,
                            starting_gold,
                        });
                    info!(
                        "Endless mode: queued replacement AI (is_raider={}, delay={}h, strength={:.0}%)",
                        is_raider,
                        crate::constants::ENDLESS_RESPAWN_DELAY_HOURS,
                        frac * 100.0
                    );
                }
            }

            // Loot: attacker picks up building loot and returns home
            if let Some(drop) = building_def(kind).loot_drop() {
                let amount = if drop.min == drop.max {
                    drop.min
                } else {
                    drop.min + ((idx as i32) % (drop.max - drop.min + 1))
                };
                if amount > 0 && attacker >= 0 {
                    let attacker_slot = attacker as usize;
                    if let Some(atk) = res.entity_map.get_npc(attacker_slot) {
                        let dc_keep_fighting = res
                            .npc_flags_q
                            .get(atk.entity)
                            .map(|f| f.direct_control)
                            .unwrap_or(false)
                            && squad_state.dc_no_return;
                        // Add loot to attacker's CarriedLoot
                        if let Ok(mut cl) = res.carried_loot_q.get_mut(atk.entity) {
                            match drop.item {
                                ItemKind::Food => cl.food += amount,
                                ItemKind::Gold => cl.gold += amount,
                            }
                        }
                        if let Ok(mut act) = res.activity_q.get_mut(atk.entity) {
                            if !matches!(act.kind, ActivityKind::ReturnLoot) {
                                act.kind = ActivityKind::ReturnLoot;
                            }
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::MarkVisualDirty {
                                idx: attacker_slot,
                            }));
                        }
                        let atk_entity = atk.entity;
                        let atk_home = res
                            .home_q
                            .get(atk_entity)
                            .map(|h| h.0)
                            .unwrap_or(Vec2::ZERO);
                        let atk_faction = atk.faction;
                        let atk_slot = atk.slot;
                        if !dc_keep_fighting {
                            intents.submit(
                                atk_entity,
                                Vec2::new(atk_home.x, atk_home.y),
                                crate::resources::MovementPriority::Survival,
                                "loot:return",
                            );
                        }
                        let item_name = match drop.item {
                            ItemKind::Food => "food",
                            ItemKind::Gold => "gold",
                        };
                        let killer_name = &npc_meta.0[atk_slot].name;
                        let killer_job = crate::job_name(npc_meta.0[atk_slot].job);
                        combat_log.write(CombatLogMsg {
                            kind: CombatEventKind::Loot,
                            faction: atk_faction,
                            day: game_time.day(),
                            hour: game_time.hour(),
                            minute: game_time.minute(),
                            message: format!(
                                "{} '{}' looted {} {} from {:?}",
                                killer_job, killer_name, amount, item_name, kind
                            ),
                            location: None,
                        });
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
        let (entity, faction, town_idx, job, activity, worksite_uid, last_hit_by) = {
            let Some(npc) = res.entity_map.get_npc(slot) else {
                continue;
            };
            let activity = res
                .activity_q
                .get(npc.entity).cloned()
                .unwrap_or_default();
            let lhb = res.last_hit_by_q.get(npc.entity).map(|h| h.0).unwrap_or(-1);
            let ws_uid = res
                .work_state_q
                .get(npc.entity)
                .ok()
                .and_then(|ws| ws.worksite);
            (
                npc.entity,
                npc.faction,
                npc.town_idx,
                npc.job,
                activity,
                ws_uid,
                lhb,
            )
        };

        if selected.0 == slot as i32 {
            selected.0 = -1;
        }
        // Death SFX with spatial position from GPU state
        let base = slot * 2;
        if base + 1 < res.gpu_state.positions.len() {
            let pos = Vec2::new(res.gpu_state.positions[base], res.gpu_state.positions[base + 1]);
            res.sfx_writer.write(crate::resources::PlaySfxMsg {
                kind: crate::resources::SfxKind::Death,
                position: Some(pos),
            });
        }
        commands.entity(entity).despawn();
        despawn_count += 1;

        // XP grant: reward killer with XP, level-up, and NPC kill loot
        if last_hit_by >= 0 {
            // Extract equipment only when there's a killer (saves 2 Vec allocs for starvation deaths)
            let dead_carried_equip: Vec<crate::constants::LootItem> = res
                .carried_loot_q
                .get(entity)
                .map(|cl| cl.equipment.clone())
                .unwrap_or_default();
            let dead_equipped_items: Vec<crate::constants::LootItem> = res
                .equipment_q
                .get(entity)
                .map(|eq| eq.all_items().collect())
                .unwrap_or_default();
            let killer_slot = last_hit_by as usize;
            if let Some(killer) = res.entity_map.get_npc(killer_slot) {
                let k_slot = killer.slot;
                let k_entity = killer.entity;
                let k_faction = killer.faction;
                let k_home = res.home_q.get(k_entity).map(|h| h.0).unwrap_or(Vec2::ZERO);
                res.faction_stats.inc_kills(k_faction);
                res.reputation.on_kill(k_faction, faction);

                let meta = &mut npc_meta.0[k_slot];
                let old_xp = meta.xp;
                meta.xp += 100;
                let old_level = level_from_xp(old_xp);
                let new_level = level_from_xp(meta.xp);
                meta.level = new_level;

                if new_level > old_level {
                    let old_max = res
                        .cached_stats_q
                        .get(k_entity)
                        .map(|s| s.max_health)
                        .unwrap_or(100.0);
                    let pers = res.personality_q.get(k_entity).cloned().unwrap_or_default();
                    let attack_type = res
                        .attack_type_q
                        .get(k_entity)
                        .copied()
                        .unwrap_or(BaseAttackType::Melee);
                    let (wb, ab) = res.equipment_q.get(k_entity).map(|eq| {
                        (eq.total_weapon_bonus(), eq.total_armor_bonus())
                    }).unwrap_or((0.0, 0.0));
                    let new_cached = resolve_combat_stats(
                        killer.job,
                        attack_type,
                        killer.town_idx,
                        new_level,
                        &pers,
                        &config,
                        &upgrades,
                        wb,
                        ab,
                    );
                    let new_speed = new_cached.speed;
                    let new_max = new_cached.max_health;
                    if let Ok(mut cs) = res.cached_stats_q.get_mut(k_entity) {
                        *cs = new_cached;
                    }
                    if let Ok(mut spd) = res.speed_q.get_mut(k_entity) {
                        spd.0 = new_speed;
                    }
                    if old_max > 0.0 {
                        if let Ok((_entity, mut hp, _slot)) = res.health_q.get_mut(k_entity) {
                            hp.0 = hp.0 * new_max / old_max;
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetMaxHealth {
                                idx: k_slot,
                                max_health: new_max,
                            }));
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth {
                                idx: k_slot,
                                health: hp.0,
                            }));
                        }
                    }
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed {
                        idx: k_slot,
                        speed: new_speed,
                    }));

                    let name = &meta.name;
                    let job_str = crate::job_name(meta.job);
                    combat_log.write(CombatLogMsg {
                        kind: CombatEventKind::LevelUp,
                        faction: k_faction,
                        day: game_time.day(),
                        hour: game_time.hour(),
                        minute: game_time.minute(),
                        message: format!("{} '{}' reached Lv.{}", job_str, name, new_level),
                        location: None,
                    });
                }

                // NPC kill loot
                let drops = npc_def(job).loot_drop;
                let drop = &drops[(meta.xp as usize) % drops.len()];
                let amount = if drop.min == drop.max {
                    drop.min
                } else {
                    drop.min + (meta.xp % (drop.max - drop.min + 1))
                };
                if amount > 0 {
                    let dc_keep_fighting = res
                        .npc_flags_q
                        .get(k_entity)
                        .map(|f| f.direct_control)
                        .unwrap_or(false)
                        && squad_state.dc_no_return;
                    if !dc_keep_fighting {
                        if let Ok(mut cs) = res.combat_state_q.get_mut(k_entity) {
                            *cs = CombatState::None;
                        }
                    }
                    // Add loot to killer's CarriedLoot
                    if let Ok(mut cl) = res.carried_loot_q.get_mut(k_entity) {
                        match drop.item {
                            ItemKind::Food => cl.food += amount,
                            ItemKind::Gold => cl.gold += amount,
                        }
                    }
                    if let Ok(mut act) = res.activity_q.get_mut(k_entity) {
                        if !matches!(act.kind, ActivityKind::ReturnLoot) {
                            act.kind = ActivityKind::ReturnLoot;
                        }
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::MarkVisualDirty { idx: k_slot }));
                    }
                    if !dc_keep_fighting {
                        intents.submit(
                            k_entity,
                            Vec2::new(k_home.x, k_home.y),
                            crate::resources::MovementPriority::Survival,
                            "loot:return",
                        );
                    }

                    let item_name = match drop.item {
                        ItemKind::Food => "food",
                        ItemKind::Gold => "gold",
                    };
                    let killer_name = &npc_meta.0[k_slot].name;
                    let killer_job = crate::job_name(npc_meta.0[k_slot].job);
                    combat_log.write(CombatLogMsg {
                        kind: CombatEventKind::Loot,
                        faction: k_faction,
                        day: game_time.day(),
                        hour: game_time.hour(),
                        minute: game_time.minute(),
                        message: format!(
                            "{} '{}' looted {} {}",
                            killer_job, killer_name, amount, item_name
                        ),
                        location: None,
                    });
                }

                // Equipment drop from victim's NpcDef
                let equip_rate = npc_def(job).equipment_drop_rate;
                if equip_rate > 0.0 {
                    let roll = ((slot as u32).wrapping_mul(2654435761) % 1000) as f32 / 1000.0;
                    if roll < equip_rate {
                        let id = res.next_loot_id.alloc();
                        let item = crate::constants::roll_loot_item(id, slot as u32);
                        let rarity_label = item.rarity.label();
                        let item_name = item.name.clone();
                        if let Ok(mut cl) = res.carried_loot_q.get_mut(k_entity) {
                            cl.equipment.push(item);
                        }
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::MarkVisualDirty { idx: k_slot }));
                        let killer_name = &npc_meta.0[k_slot].name;
                        let killer_job = crate::job_name(npc_meta.0[k_slot].job);
                        combat_log.write(CombatLogMsg {
                            kind: CombatEventKind::Loot,
                            faction: k_faction,
                            day: game_time.day(),
                            hour: game_time.hour(),
                            minute: game_time.minute(),
                            message: format!(
                                "{} '{}' looted {} {}",
                                killer_job, killer_name, rarity_label, item_name
                            ),
                            location: None,
                        });
                    }
                }

                // Transfer victim's carried equipment (50% per item)
                for carried_item in dead_carried_equip.iter() {
                    let transfer_roll = (carried_item.id.wrapping_mul(2654435761) % 100) as f32;
                    if transfer_roll < 50.0 {
                        if let Ok(mut cl) = res.carried_loot_q.get_mut(k_entity) {
                            cl.equipment.push(carried_item.clone());
                        }
                    }
                }

                // Transfer victim's equipped items (50% per item)
                for eq_item in dead_equipped_items.iter() {
                    let transfer_roll = (eq_item.id.wrapping_mul(2654435761).wrapping_add(7) % 100) as f32;
                    if transfer_roll < 50.0 {
                        if let Ok(mut cl) = res.carried_loot_q.get_mut(k_entity) {
                            cl.equipment.push(eq_item.clone());
                        }
                    }
                }
            } else if let Some(tower_faction) = res
                .entity_map
                .get_instance(killer_slot)
                .filter(|i| i.kind == BuildingKind::Fountain || i.kind == BuildingKind::Tower)
                .map(|i| (i.faction, i.town_idx as usize))
            {
                // Tower/fountain killer — XP, kills, loot deposit
                let (tower_faction, tower_town) = tower_faction;
                res.faction_stats.inc_kills(tower_faction);
                res.reputation.on_kill(tower_faction, faction);

                let Some(inst) = res.entity_map.get_instance_mut(killer_slot) else { continue; };
                inst.kills += 1;
                let old_xp = inst.xp;
                inst.xp += 100;
                let old_level = level_from_xp(old_xp);
                let new_level = level_from_xp(inst.xp);
                let kind_name = if inst.kind == BuildingKind::Tower {
                    "Tower"
                } else {
                    "Fountain"
                };

                if new_level > old_level {
                    combat_log.write(CombatLogMsg {
                        kind: CombatEventKind::LevelUp,
                        faction: tower_faction,
                        day: game_time.day(),
                        hour: game_time.hour(),
                        minute: game_time.minute(),
                        message: format!("{} reached Lv.{}", kind_name, new_level),
                        location: None,
                    });
                }

                // Loot from victim's loot table, deposited directly to town
                let drops = npc_def(job).loot_drop;
                let tower_xp = old_xp + 100;
                let drop = &drops[(tower_xp as usize) % drops.len()];
                let amount = if drop.min == drop.max {
                    drop.min
                } else {
                    drop.min + (tower_xp % (drop.max - drop.min + 1))
                };
                if amount > 0 {
                    match drop.item {
                        ItemKind::Food => {
                            if tower_town < food_storage.food.len() {
                                food_storage.food[tower_town] += amount;
                            }
                        }
                        ItemKind::Gold => {
                            if tower_town < gold_storage.gold.len() {
                                gold_storage.gold[tower_town] += amount;
                            }
                        }
                    }
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetDamageFlash {
                        idx: killer_slot,
                        intensity: 1.0,
                    }));
                    let item_name = match drop.item {
                        ItemKind::Food => "food",
                        ItemKind::Gold => "gold",
                    };
                    combat_log.write(CombatLogMsg {
                        kind: CombatEventKind::Loot,
                        faction: tower_faction,
                        day: game_time.day(),
                        hour: game_time.hour(),
                        minute: game_time.minute(),
                        message: format!("{} looted {} {}", kind_name, amount, item_name),
                        location: None,
                    });
                }

                // Equipment drop from victim → TownInventory (towers can't carry)
                let equip_rate = npc_def(job).equipment_drop_rate;
                if equip_rate > 0.0 {
                    let roll = ((slot as u32).wrapping_mul(2654435761) % 1000) as f32 / 1000.0;
                    if roll < equip_rate {
                        let id = res.next_loot_id.alloc();
                        let item = crate::constants::roll_loot_item(id, slot as u32);
                        let rarity_label = item.rarity.label();
                        let item_name = item.name.clone();
                        res.town_inventory.add(tower_town, item);
                        combat_log.write(CombatLogMsg {
                            kind: CombatEventKind::Loot,
                            faction: tower_faction,
                            day: game_time.day(),
                            hour: game_time.hour(),
                            minute: game_time.minute(),
                            message: format!(
                                "{} deposited {} {} to inventory",
                                kind_name, rarity_label, item_name
                            ),
                            location: None,
                        });
                    }
                }

                // Victim's carried equipment → TownInventory
                for carried_item in dead_carried_equip.iter() {
                    let transfer_roll = (carried_item.id.wrapping_mul(2654435761) % 100) as f32;
                    if transfer_roll < 50.0 {
                        res.town_inventory.add(tower_town, carried_item.clone());
                    }
                }

                // Victim's equipped items → TownInventory (50% per item)
                for eq_item in dead_equipped_items.iter() {
                    let transfer_roll = (eq_item.id.wrapping_mul(2654435761).wrapping_add(7) % 100) as f32;
                    if transfer_roll < 50.0 {
                        res.town_inventory.add(tower_town, eq_item.clone());
                    }
                }
            }
        }

        // NPC cleanup
        pop_dec_alive(&mut res.pop_stats, job, town_idx);
        pop_inc_dead(&mut res.pop_stats, job, town_idx);
        if matches!(activity.kind, ActivityKind::Work { .. } | ActivityKind::Mine { .. }) {
            pop_dec_working(&mut res.pop_stats, job, town_idx);
        }

        // Defer worksite release to centralized resolver
        res.work_intents.write(crate::messages::WorkIntentMsg(
            crate::messages::WorkIntent::Release { entity, uid: worksite_uid },
        ));
        if job == Job::Miner {
            res.dirty_writers
                .mining
                .write(crate::messages::MiningDirtyMsg);
        }

        // Attribute kills to player faction only
        if last_hit_by >= 0 {
            let killer_faction = res
                .entity_map
                .get_npc(last_hit_by as usize)
                .map(|n| n.faction)
                .or_else(|| {
                    res.entity_map
                        .get_instance(last_hit_by as usize)
                        .map(|b| b.faction)
                })
                .unwrap_or(-1);
            if killer_faction == crate::constants::FACTION_PLAYER && faction != crate::constants::FACTION_PLAYER {
                res.kill_stats.archer_kills += 1;
            } else if killer_faction != crate::constants::FACTION_PLAYER && faction == crate::constants::FACTION_PLAYER {
                res.kill_stats.villager_kills += 1;
            }
        }

        // Combat log: death event
        let meta = &npc_meta.0[slot];
        let job_str = crate::job_name(meta.job);
        let msg = if meta.name.is_empty() {
            format!("{} #{} died", job_str, slot)
        } else {
            format!("{} '{}' Lv.{} died", job_str, meta.name, meta.level)
        };
        combat_log.write(CombatLogMsg {
            kind: CombatEventKind::Kill,
            faction,
            day: game_time.day(),
            hour: game_time.hour(),
            minute: game_time.minute(),
            message: msg,
            location: None,
        });

        res.faction_stats.dec_alive(faction);
        res.faction_stats.inc_dead(faction);

        // npc_by_town cleanup handled by unregister_npc inside hide_npc
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
) {
    if healing_dirty.read().count() == 0 {
        return;
    }

    let max_faction = world_data
        .towns
        .iter()
        .map(|t| t.faction)
        .max()
        .unwrap_or(0);
    let faction_count = (max_faction + 1).max(0) as usize;
    cache.by_faction.clear();
    cache.by_faction.resize_with(faction_count, Vec::new);

    for (town_idx, town) in world_data.towns.iter().enumerate() {
        if town.faction <= crate::constants::FACTION_NEUTRAL {
            continue;
        }
        let town_levels = upgrades.town_levels(town_idx);
        let heal_mult = UPGRADES.stat_mult(&town_levels, "Town", UpgradeStatKind::Healing);
        let radius_lvl = UPGRADES.stat_level(&town_levels, "Town", UpgradeStatKind::FountainRange);
        let radius = combat_config.heal_radius + radius_lvl as f32 * 24.0;
        let heal_rate = combat_config.heal_rate * heal_mult;

        let enter_radius_sq = radius * radius;
        cache.by_faction[town.faction as usize].push(crate::resources::HealingZone {
            center: town.center,
            enter_radius_sq,
            exit_radius_sq: enter_radius_sq * 1.21, // 10% larger radius for hysteresis
            heal_rate,
            town_idx,
            faction: town.faction,
        });
    }

    #[cfg(debug_assertions)]
    info!(
        "Healing zone cache rebuilt: {} factions",
        cache.by_faction.len()
    );
}

/// Candidate-driven healing: enter-check (cadenced) + sustain-check (every frame).
/// Replaces full 50k NPC iteration with O(active_healing + sampled_candidates).
/// Starving HP cap moved to starvation_system (economy.rs).
pub fn healing_system(
    mut npc_q: Query<
        (&GpuSlot, &mut Health, &CachedStats, &mut NpcFlags, &Faction),
        (Without<Building>, Without<Dead>),
    >,
    mut building_query: Query<(&GpuSlot, &mut Health, &Faction, &Building), Without<Dead>>,
    gpu_state: Res<GpuReadState>,
    entity_gpu_state: Res<crate::gpu::EntityGpuState>,
    entity_map: Res<EntityMap>,
    cache: Res<HealingZoneCache>,
    world_data: Res<WorldData>,
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut debug: ResMut<HealthDebug>,
    mut heal_state: ResMut<BuildingHealState>,
    mut active: ResMut<ActiveHealingSlots>,
    mut frame_count: Local<u32>,
) {
    let positions = &gpu_state.positions;
    let dt = game_time.delta(&time);
    *frame_count = frame_count.wrapping_add(1);
    let bucket = (*frame_count % 4) as usize;

    let mut enter_checks = 0usize;
    let mut healed_count = 0usize;
    let mut exit_count = 0usize;

    // Use cache.by_faction directly — already Vec<Vec<HealingZone>> indexed by faction

    // ========================================================================
    // 1. Sustain-check: process active healing set every frame
    // ========================================================================
    let mut i = 0;
    while i < active.slots.len() {
        let slot = active.slots[i];

        // Stale/dead slot cleanup
        let Some(npc) = entity_map.get_npc(slot) else {
            if slot < active.mark.len() {
                active.mark[slot] = 0;
            }
            active.slots.swap_remove(i);
            exit_count += 1;
            continue;
        };
        if npc.dead {
            if slot < active.mark.len() {
                active.mark[slot] = 0;
            }
            active.slots.swap_remove(i);
            exit_count += 1;
            continue;
        }

        // Position + exit check
        let base = slot * 2;
        if base + 1 >= positions.len() {
            if slot < active.mark.len() {
                active.mark[slot] = 0;
            }
            active.slots.swap_remove(i);
            exit_count += 1;
            continue;
        }
        let px = positions[base];
        let py = positions[base + 1];

        // Check against same-faction zones using exit_radius_sq (hysteresis)
        let fac = npc.faction;
        let mut in_zone = false;
        let mut zone_heal_rate = 0.0;
        if let Some(zones) = if fac >= 0 {
            cache.by_faction.get(fac as usize)
        } else {
            None
        } {
            for zone in zones.iter() {
                let dx = px - zone.center.x;
                let dy = py - zone.center.y;
                if dx * dx + dy * dy <= zone.exit_radius_sq {
                    in_zone = true;
                    zone_heal_rate = zone.heal_rate;
                    break;
                }
            }
        }

        if in_zone {
            if let Ok((_, mut health, cached, mut flags, _)) = npc_q.get_mut(npc.entity) {
                let hp_cap = if flags.starving {
                    cached.max_health * STARVING_HP_CAP
                } else {
                    cached.max_health
                };
                let was_healing = flags.healing;
                if health.0 < hp_cap {
                    health.0 = (health.0 + zone_heal_rate * dt).min(hp_cap);
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth {
                        idx: slot,
                        health: health.0,
                    }));
                    healed_count += 1;
                }
                let should_heal = health.0 < hp_cap;
                if was_healing != should_heal {
                    flags.healing = should_heal;
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::MarkVisualDirty { idx: slot }));
                }
            }
            i += 1;
        } else {
            // Exited zone
            if let Ok((_, _, _, mut flags, _)) = npc_q.get_mut(npc.entity) {
                if flags.healing {
                    flags.healing = false;
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::MarkVisualDirty { idx: slot }));
                }
            }
            if slot < active.mark.len() {
                active.mark[slot] = 0;
            }
            active.slots.swap_remove(i);
            exit_count += 1;
        }
    }

    // ========================================================================
    // 2. Enter-check: find new healing candidates (cadenced, 1/4 NPCs per frame)
    // ========================================================================
    let mut to_activate: Vec<(usize, Entity)> = Vec::new();

    for (town_idx, _town) in world_data.towns.iter().enumerate() {
        for npc in entity_map.npcs_for_town(town_idx as i32) {
            if npc.dead {
                continue;
            }
            let slot = npc.slot;
            if slot % 4 != bucket {
                continue;
            }
            if slot < active.mark.len() && active.mark[slot] == 1 {
                continue;
            }

            enter_checks += 1;

            let base = slot * 2;
            if base + 1 >= positions.len() {
                continue;
            }
            let px = positions[base];
            let py = positions[base + 1];

            // Check all same-faction zones using enter_radius_sq
            if let Some(zones) = {
                let f = npc.faction;
                if f >= 0 {
                    cache.by_faction.get(f as usize)
                } else {
                    None
                }
            } {
                for zone in zones.iter() {
                    let dx = px - zone.center.x;
                    let dy = py - zone.center.y;
                    if dx * dx + dy * dy <= zone.enter_radius_sq {
                        to_activate.push((slot, npc.entity));
                        break;
                    }
                }
            }
        }
    }

    // Dedup and activate
    to_activate.sort_unstable_by_key(|(slot, _)| *slot);
    to_activate.dedup_by_key(|(slot, _)| *slot);

    for (slot, entity) in to_activate {
        if let Ok((_, health, cached, mut flags, _)) = npc_q.get_mut(entity) {
            let hp_cap = if flags.starving {
                cached.max_health * STARVING_HP_CAP
            } else {
                cached.max_health
            };
            let should_heal = health.0 < hp_cap;
            if flags.healing != should_heal {
                flags.healing = should_heal;
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::MarkVisualDirty { idx: slot }));
            }
        }
        if slot < active.mark.len() {
            active.slots.push(slot);
            active.mark[slot] = 1;
        }
    }

    // ========================================================================
    // 3. Building healing (unchanged — already gated behind needs_healing)
    // ========================================================================
    if heal_state.needs_healing {
        let bld_positions = &entity_gpu_state.positions;
        let mut any_damaged = false;
        for (npc_idx, mut health, faction, building) in building_query.iter_mut() {
            let max_hp = crate::constants::building_def(building.kind).hp;
            if health.0 <= 0.0 || health.0 >= max_hp {
                continue;
            }
            any_damaged = true;
            let idx = npc_idx.0;
            if idx * 2 + 1 >= bld_positions.len() {
                continue;
            }
            let x = bld_positions[idx * 2];
            let y = bld_positions[idx * 2 + 1];
            if faction.0 == crate::constants::FACTION_NEUTRAL {
                continue;
            }
            if let Some(zones) = {
                let f = faction.0;
                if f >= 0 {
                    cache.by_faction.get(f as usize)
                } else {
                    None
                }
            } {
                let mut zone_heal_rate = 0.0f32;
                for zone in zones.iter() {
                    let dx = x - zone.center.x;
                    let dy = y - zone.center.y;
                    if dx * dx + dy * dy <= zone.enter_radius_sq {
                        zone_heal_rate = zone.heal_rate;
                        break;
                    }
                }
                if zone_heal_rate > 0.0 {
                    health.0 = (health.0 + zone_heal_rate * dt).min(max_hp);
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth {
                        idx,
                        health: health.0,
                    }));
                }
            }
        }
        if !any_damaged {
            heal_state.needs_healing = false;
        }
    }

    // Update debug stats
    debug.healing_npcs_checked = enter_checks;
    debug.healing_positions_len = positions.len();
    debug.healing_towns_count = cache.by_faction.iter().map(|v| v.len()).sum();
    debug.healing_in_zone_count = active.slots.len();
    debug.healing_healed_count = healed_count;
    debug.healing_active_count = active.slots.len();
    debug.healing_enter_checks = enter_checks;
    debug.healing_exits = exit_count;
}

/// Passive HP regen for NPCs with hp_regen upgrade (outside fountain healing).
pub fn npc_regen_system(
    mut npc_q: Query<(&mut Health, &CachedStats), (Without<Building>, Without<Dead>)>,
    time: Res<Time>,
    game_time: Res<GameTime>,
) {
    let dt = game_time.delta(&time);
    if dt <= 0.0 { return; }
    for (mut health, stats) in &mut npc_q {
        if stats.hp_regen > 0.0 && health.0 < stats.max_health {
            health.0 = (health.0 + stats.hp_regen * dt).min(stats.max_health);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;

    fn stats_with_regen(hp_regen: f32) -> CachedStats {
        CachedStats {
            damage: 15.0, range: 200.0, cooldown: 1.5,
            projectile_speed: 200.0, projectile_lifetime: 1.5,
            max_health: 100.0, speed: 200.0, stamina: 1.0,
            hp_regen, berserk_bonus: 0.0,
        }
    }

    fn setup_regen_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(GameTime::default());
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_systems(FixedUpdate, npc_regen_system);
        // Two priming updates: first inits Time, second accumulates FixedUpdate delta
        app.update();
        app.update();
        app
    }

    #[test]
    fn regen_heals_damaged_npc() {
        let mut app = setup_regen_app();
        let npc = app.world_mut().spawn((
            Health(50.0), stats_with_regen(5.0),
        )).id();

        app.update();
        let hp = app.world().get::<Health>(npc).unwrap().0;
        assert!(hp > 50.0, "regen should heal: {hp}");
    }

    #[test]
    fn regen_capped_at_max_health() {
        let mut app = setup_regen_app();
        let npc = app.world_mut().spawn((
            Health(99.9), stats_with_regen(100.0),
        )).id();

        app.update();
        let hp = app.world().get::<Health>(npc).unwrap().0;
        assert!(hp <= 100.0, "regen should not exceed max_health: {hp}");
    }

    #[test]
    fn zero_regen_no_heal() {
        let mut app = setup_regen_app();
        let npc = app.world_mut().spawn((
            Health(50.0), stats_with_regen(0.0),
        )).id();

        app.update();
        let hp = app.world().get::<Health>(npc).unwrap().0;
        assert!((hp - 50.0).abs() < f32::EPSILON, "zero regen should not heal: {hp}");
    }

    #[test]
    fn full_health_no_regen() {
        let mut app = setup_regen_app();
        let npc = app.world_mut().spawn((
            Health(100.0), stats_with_regen(5.0),
        )).id();

        app.update();
        let hp = app.world().get::<Health>(npc).unwrap().0;
        assert!((hp - 100.0).abs() < f32::EPSILON, "full HP should not regen: {hp}");
    }

    #[test]
    fn dead_npcs_dont_regen() {
        let mut app = setup_regen_app();
        let npc = app.world_mut().spawn((
            Health(10.0), stats_with_regen(5.0), Dead,
        )).id();

        app.update();
        let hp = app.world().get::<Health>(npc).unwrap().0;
        assert!((hp - 10.0).abs() < f32::EPSILON, "dead NPC should not regen: {hp}");
    }

    #[test]
    fn buildings_excluded_from_regen() {
        let mut app = setup_regen_app();
        let building = app.world_mut().spawn((
            Health(50.0), stats_with_regen(5.0),
            Building { kind: crate::world::BuildingKind::Tower },
        )).id();

        app.update();
        let hp = app.world().get::<Health>(building).unwrap().0;
        assert!((hp - 50.0).abs() < f32::EPSILON, "buildings should not use npc_regen: {hp}");
    }

    #[test]
    fn regen_paused_no_change() {
        let mut app = setup_regen_app();
        app.world_mut().resource_mut::<GameTime>().paused = true;
        let npc = app.world_mut().spawn((
            Health(50.0), stats_with_regen(5.0),
        )).id();

        app.update();
        let hp = app.world().get::<Health>(npc).unwrap().0;
        assert!((hp - 50.0).abs() < f32::EPSILON, "paused game should not regen: {hp}");
    }

    // ========================================================================
    // damage_system tests
    // ========================================================================

    use crate::components::EntityUid;
    use crate::messages::DamageMsg;
    use crate::resources::{BuildingHealState, EntityMap, HealthDebug};

    /// Helper system that sends a DamageMsg once, then drains the queue.
    /// Must drain to avoid re-sending on subsequent FixedUpdate sub-ticks.
    fn send_damage(
        mut writer: MessageWriter<DamageMsg>,
        mut pending: ResMut<PendingDamage>,
    ) {
        for msg in pending.0.drain(..) {
            writer.write(msg);
        }
    }

    /// Resource to hold damage events to be sent by the helper system.
    #[derive(Resource, Default)]
    struct PendingDamage(Vec<DamageMsg>);

    fn setup_damage_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(GameTime::default());
        app.insert_resource(EntityMap::default());
        app.insert_resource(HealthDebug::default());
        app.insert_resource(BuildingHealState::default());
        app.insert_resource(PendingDamage::default());
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_message::<DamageMsg>();
        app.add_message::<GpuUpdateMsg>();
        // send_damage runs first, then damage_system reads the messages
        app.add_systems(FixedUpdate, (send_damage, damage_system).chain());
        app.update();
        app.update();
        app
    }

    /// Spawn an NPC entity and register it in EntityMap with a UID.
    fn spawn_damageable_npc(app: &mut App, slot: usize, uid: u64, hp: f32) -> Entity {
        let entity = app.world_mut().spawn((
            GpuSlot(slot),
            Health(hp),
        )).id();
        let mut entity_map = app.world_mut().resource_mut::<EntityMap>();
        entity_map.register_npc(slot, entity, crate::components::Job::Archer, 0, 0);
        entity_map.register_uid(slot, EntityUid(uid), entity);
        entity
    }

    #[test]
    fn damage_reduces_npc_health() {
        let mut app = setup_damage_app();
        let npc = spawn_damageable_npc(&mut app, 0, 1, 100.0);
        app.world_mut().resource_mut::<PendingDamage>().0.push(DamageMsg {
            target: EntityUid(1),
            amount: 30.0,
            attacker: -1,
            attacker_faction: 0,
        });

        app.update();
        let hp = app.world().get::<Health>(npc).unwrap().0;
        assert!((hp - 70.0).abs() < 0.01, "damage should reduce HP from 100 to 70: {hp}");
    }

    #[test]
    fn damage_floors_at_zero() {
        let mut app = setup_damage_app();
        let npc = spawn_damageable_npc(&mut app, 0, 1, 10.0);
        app.world_mut().resource_mut::<PendingDamage>().0.push(DamageMsg {
            target: EntityUid(1),
            amount: 50.0,
            attacker: -1,
            attacker_faction: 0,
        });

        app.update();
        let hp = app.world().get::<Health>(npc).unwrap().0;
        assert!(hp >= 0.0, "HP should not go negative: {hp}");
        assert!(hp < 0.01, "HP should be at zero: {hp}");
    }

    #[test]
    fn damage_to_unknown_uid_ignored() {
        let mut app = setup_damage_app();
        let npc = spawn_damageable_npc(&mut app, 0, 1, 100.0);
        // Send damage to a UID that doesn't exist
        app.world_mut().resource_mut::<PendingDamage>().0.push(DamageMsg {
            target: EntityUid(999),
            amount: 50.0,
            attacker: -1,
            attacker_faction: 0,
        });

        app.update();
        let hp = app.world().get::<Health>(npc).unwrap().0;
        assert!((hp - 100.0).abs() < 0.01, "NPC should not take damage from mismatched UID: {hp}");
    }

    #[test]
    fn damage_updates_debug_entity_count() {
        let mut app = setup_damage_app();
        spawn_damageable_npc(&mut app, 0, 1, 100.0);

        app.update();
        let debug = app.world().resource::<HealthDebug>();
        // bevy_entity_count is updated every tick (not just when damage occurs)
        assert_eq!(debug.bevy_entity_count, 1, "debug should track entity count");
    }

    #[test]
    fn multiple_damage_events_stack() {
        let mut app = setup_damage_app();
        let npc = spawn_damageable_npc(&mut app, 0, 1, 100.0);
        let pending = &mut app.world_mut().resource_mut::<PendingDamage>().0;
        pending.push(DamageMsg { target: EntityUid(1), amount: 20.0, attacker: -1, attacker_faction: 0 });
        pending.push(DamageMsg { target: EntityUid(1), amount: 15.0, attacker: -1, attacker_faction: 0 });

        app.update();
        let hp = app.world().get::<Health>(npc).unwrap().0;
        assert!((hp - 65.0).abs() < 0.01, "two damage events (20+15) should reduce to 65: {hp}");
    }

    #[test]
    fn damage_dead_npc_ignored() {
        let mut app = setup_damage_app();
        let npc = spawn_damageable_npc(&mut app, 0, 1, 100.0);
        // Mark NPC as dead in EntityMap
        app.world_mut().resource_mut::<EntityMap>().get_npc_mut(0).unwrap().dead = true;
        app.world_mut().resource_mut::<PendingDamage>().0.push(DamageMsg {
            target: EntityUid(1),
            amount: 50.0,
            attacker: -1,
            attacker_faction: 0,
        });

        app.update();
        let hp = app.world().get::<Health>(npc).unwrap().0;
        assert!((hp - 100.0).abs() < 0.01, "dead NPC should not take damage: {hp}");
    }

    #[test]
    fn damage_building_reduces_health() {
        let mut app = setup_damage_app();
        let slot = 0usize;
        let uid = 1u64;
        let entity = app.world_mut().spawn((
            GpuSlot(slot),
            Health(200.0),
            Building { kind: BuildingKind::Tower },
        )).id();
        // Register as building instance in EntityMap
        let mut entity_map = app.world_mut().resource_mut::<EntityMap>();
        entity_map.entities.insert(slot, entity);
        entity_map.add_instance(crate::resources::BuildingInstance {
            kind: BuildingKind::Tower,
            position: Vec2::ZERO,
            town_idx: 0,
            slot,
            faction: 0,
            patrol_order: 0,
            assigned_mine: None,
            manual_mine: false,
            wall_level: 0,
            npc_uid: None,
            respawn_timer: -1.0,
            growth_ready: false,
            growth_progress: 0.0,
            occupants: 0,
            under_construction: 0.0,
            kills: 0,
            xp: 0,
            upgrade_levels: vec![],
            auto_upgrade_flags: vec![],
        });
        entity_map.register_uid(slot, EntityUid(uid), entity);

        app.world_mut().resource_mut::<PendingDamage>().0.push(DamageMsg {
            target: EntityUid(uid),
            amount: 75.0,
            attacker: -1,
            attacker_faction: 1,
        });

        app.update();
        let hp = app.world().get::<Health>(entity).unwrap().0;
        assert!((hp - 125.0).abs() < 0.01, "building should take 75 damage from 200: {hp}");
    }

    // -- update_healing_zone_cache -------------------------------------------

    #[derive(Resource, Default)]
    struct SendHealingDirty(bool);

    fn send_healing_dirty(
        mut writer: MessageWriter<crate::messages::HealingZonesDirtyMsg>,
        mut flag: ResMut<SendHealingDirty>,
    ) {
        if flag.0 {
            writer.write(crate::messages::HealingZonesDirtyMsg);
            flag.0 = false;
        }
    }

    fn setup_healing_cache_app() -> App {
        use crate::world::{Town, WorldData};
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(crate::resources::HealingZoneCache::default());
        app.insert_resource(WorldData {
            towns: vec![Town {
                name: "TestTown".to_string(),
                center: Vec2::new(500.0, 500.0),
                faction: 1,
                sprite_type: 0,
                area_level: 0,
            }],
        });
        app.insert_resource(CombatConfig::default());
        app.insert_resource(TownUpgrades::default());
        app.insert_resource(SendHealingDirty(false));
        app.add_message::<crate::messages::HealingZonesDirtyMsg>();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_systems(
            FixedUpdate,
            (send_healing_dirty, update_healing_zone_cache).chain(),
        );
        app.update();
        app.update();
        app
    }

    #[test]
    fn healing_cache_empty_without_dirty() {
        let mut app = setup_healing_cache_app();
        app.update();
        let cache = app.world().resource::<crate::resources::HealingZoneCache>();
        assert!(cache.by_faction.is_empty(), "cache should stay empty without dirty msg");
    }

    #[test]
    fn healing_cache_rebuilds_on_dirty() {
        let mut app = setup_healing_cache_app();
        app.insert_resource(SendHealingDirty(true));
        app.update();
        let cache = app.world().resource::<crate::resources::HealingZoneCache>();
        assert!(!cache.by_faction.is_empty(), "cache should have factions after dirty");
        assert!(cache.by_faction.len() > 1, "cache should have at least 2 faction slots");
        assert!(!cache.by_faction[1].is_empty(), "faction 1 should have a healing zone");
        let zone = &cache.by_faction[1][0];
        assert!((zone.center.x - 500.0).abs() < 0.1, "zone center should match town center");
        assert!(zone.heal_rate > 0.0, "heal_rate should be positive");
        assert!(zone.enter_radius_sq > 0.0, "enter_radius_sq should be positive");
    }

    #[test]
    fn healing_cache_skips_negative_faction() {
        use crate::world::{Town, WorldData};
        let mut app = setup_healing_cache_app();
        // Replace towns with one negative faction
        app.insert_resource(WorldData {
            towns: vec![Town {
                name: "Abandoned".to_string(),
                center: Vec2::ZERO,
                faction: -1,
                sprite_type: 0,
                area_level: 0,
            }],
        });
        app.insert_resource(SendHealingDirty(true));
        app.update();
        let cache = app.world().resource::<crate::resources::HealingZoneCache>();
        // With faction -1, max_faction < 0 so no factions
        assert!(
            cache.by_faction.is_empty() || cache.by_faction.iter().all(|v| v.is_empty()),
            "negative faction towns should not produce healing zones"
        );
    }

    #[test]
    fn healing_cache_multiple_towns() {
        use crate::world::{Town, WorldData};
        let mut app = setup_healing_cache_app();
        app.insert_resource(WorldData {
            towns: vec![
                Town { name: "A".to_string(), center: Vec2::new(100.0, 100.0), faction: 1, sprite_type: 0, area_level: 0 },
                Town { name: "B".to_string(), center: Vec2::new(900.0, 900.0), faction: 2, sprite_type: 1, area_level: 0 },
            ],
        });
        app.insert_resource(SendHealingDirty(true));
        app.update();
        let cache = app.world().resource::<crate::resources::HealingZoneCache>();
        assert!(cache.by_faction.len() >= 3, "should have at least 3 faction entries");
        assert!(!cache.by_faction[1].is_empty(), "faction 1 should have a zone");
        assert!(!cache.by_faction[2].is_empty(), "faction 2 should have a zone");
    }
}
