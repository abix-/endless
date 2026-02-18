//! Health systems - Damage, death detection, cleanup, healing aura

use bevy::prelude::*;
use bevy::ecs::system::SystemParam;
use crate::components::*;
use crate::constants::STARVING_HP_CAP;
use crate::messages::{GpuUpdate, GpuUpdateMsg, DamageMsg};
use crate::resources::{NpcEntityMap, HealthDebug, PopulationStats, KillStats, NpcsByTownCache, SlotAllocator, GpuReadState, FactionStats, RaidQueue, CombatLog, CombatEventKind, NpcMetaCache, GameTime, SelectedNpc, SystemTimings, HealingZoneCache, DirtyFlags, BuildingHpState, BuildingSlotMap};
use crate::systems::stats::{CombatConfig, TownUpgrades, UpgradeType, UPGRADE_PCT};
use crate::systems::economy::*;
use crate::world::{WorldData, BuildingOccupancy, BuildingKind};

/// Bundled resources for death_cleanup_system to stay under 16 params.
#[derive(SystemParam)]
pub struct CleanupResources<'w> {
    pub npc_map: ResMut<'w, NpcEntityMap>,
    pub pop_stats: ResMut<'w, PopulationStats>,
    pub faction_stats: ResMut<'w, FactionStats>,
    pub debug: ResMut<'w, HealthDebug>,
    pub kill_stats: ResMut<'w, KillStats>,
    pub npcs_by_town: ResMut<'w, NpcsByTownCache>,
    pub slots: ResMut<'w, SlotAllocator>,
    pub farm_occupancy: ResMut<'w, BuildingOccupancy>,
    pub raid_queue: ResMut<'w, RaidQueue>,
    pub dirty: ResMut<'w, DirtyFlags>,
}

/// Apply queued damage to Health component and sync to GPU.
/// Uses NpcEntityMap for O(1) entity lookup instead of O(n) iteration.
pub fn damage_system(
    mut commands: Commands,
    mut events: MessageReader<DamageMsg>,
    npc_map: Res<NpcEntityMap>,
    mut query: Query<(&mut Health, &NpcIndex)>,
    mut debug: ResMut<HealthDebug>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("damage");
    let mut damage_count = 0;
    for event in events.read() {
        damage_count += 1;
        // O(1) lookup via NpcEntityMap
        if let Some(&entity) = npc_map.0.get(&event.npc_index) {
            if let Ok((mut health, npc_idx)) = query.get_mut(entity) {
                health.0 = (health.0 - event.amount).max(0.0);
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: npc_idx.0, health: health.0 }));
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetDamageFlash { idx: npc_idx.0, intensity: 1.0 }));
                // Track last attacker for XP-on-kill
                if event.attacker >= 0 {
                    commands.entity(entity).insert(LastHitBy(event.attacker));
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

/// Mark dead entities with Dead component.
pub fn death_system(
    mut commands: Commands,
    query: Query<(Entity, &Health, &NpcIndex), Without<Dead>>,
    mut debug: ResMut<HealthDebug>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("death");
    let mut death_count = 0;
    for (entity, health, _npc_idx) in query.iter() {
        if health.0 <= 0.0 {
            commands.entity(entity).insert(Dead);
            death_count += 1;
        }
    }

    debug.deaths_this_frame = death_count;
}

/// Remove dead entities, hide on GPU by setting position to -9999, recycle slot.
pub fn death_cleanup_system(
    mut commands: Commands,
    query: Query<(Entity, &NpcIndex, &Job, &TownId, &Faction, &Activity, Option<&AssignedFarm>, Option<&WorkPosition>), With<Dead>>,
    mut res: CleanupResources,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    meta_cache: Res<NpcMetaCache>,
    mut selected: ResMut<SelectedNpc>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("death_cleanup");
    let mut despawn_count = 0;
    if !query.is_empty() {
        res.dirty.squads = true;
    }
    for (entity, npc_idx, job, town_id, faction, activity, assigned_farm, work_position) in query.iter() {
        let idx = npc_idx.0;

        // Deselect if the selected NPC died
        if selected.0 == idx as i32 {
            selected.0 = -1;
        }
        commands.entity(entity).despawn();
        despawn_count += 1;
        pop_dec_alive(&mut res.pop_stats, *job, town_id.0);
        pop_inc_dead(&mut res.pop_stats, *job, town_id.0);
        if matches!(activity, Activity::Working | Activity::MiningAtMine) {
            pop_dec_working(&mut res.pop_stats, *job, town_id.0);
        }

        // Release assigned farm if any
        if let Some(assigned) = assigned_farm {
            res.farm_occupancy.release(assigned.0);
        }

        // Release mine occupancy if miner was working at a mine
        if let Some(wp) = work_position {
            res.farm_occupancy.release(wp.0);
        }

        // Remove from raid queue if raider was waiting
        if *job == Job::Raider {
            res.raid_queue.remove(faction.0, entity);
        }
        if *job == Job::Miner {
            res.dirty.mining = true;
        }

        // Track kill statistics for UI (faction 0 = player/villager, 1+ = raiders)
        if faction.0 == 0 {
            res.kill_stats.villager_kills += 1;
        } else {
            res.kill_stats.archer_kills += 1;
        }

        // Combat log: death event
        let meta = &meta_cache.0[idx];
        let job_str = crate::job_name(meta.job);
        let msg = if meta.name.is_empty() {
            format!("{} #{} died", job_str, idx)
        } else {
            format!("{} '{}' Lv.{} died", job_str, meta.name, meta.level)
        };
        combat_log.push(CombatEventKind::Kill, faction.0, game_time.day(), game_time.hour(), game_time.minute(), msg);

        // Track per-faction stats (alive/dead)
        res.faction_stats.dec_alive(faction.0);
        res.faction_stats.inc_dead(faction.0);

        // Remove from per-town NPC list
        if town_id.0 >= 0 {
            let town_idx = town_id.0 as usize;
            if town_idx < res.npcs_by_town.0.len() {
                res.npcs_by_town.0[town_idx].retain(|&i| i != idx);
            }
        }

        // Remove from entity map
        res.npc_map.0.remove(&idx);

        // Hide NPC visually via message
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::HideNpc { idx }));

        // Return slot to free pool
        res.slots.free(idx);
    }

    res.debug.despawned_this_frame = despawn_count;
}

/// Rebuild healing zone cache when dirty (upgrade purchased, town changed, save loaded).
pub fn update_healing_zone_cache(
    mut cache: ResMut<HealingZoneCache>,
    mut dirty: ResMut<DirtyFlags>,
    world_data: Res<WorldData>,
    combat_config: Res<CombatConfig>,
    upgrades: Res<TownUpgrades>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("healing_zones");
    if !dirty.healing_zones { return; }

    let max_faction = world_data.towns.iter().map(|t| t.faction).max().unwrap_or(0);
    let faction_count = (max_faction + 1).max(0) as usize;
    cache.by_faction.clear();
    cache.by_faction.resize_with(faction_count, Vec::new);

    for (town_idx, town) in world_data.towns.iter().enumerate() {
        if town.faction < 0 { continue; }
        let heal_lvl = upgrades.levels.get(town_idx).map(|l| l[UpgradeType::HealingRate as usize]).unwrap_or(0);
        let radius_lvl = upgrades.levels.get(town_idx).map(|l| l[UpgradeType::FountainRange as usize]).unwrap_or(0);
        let radius = combat_config.heal_radius + radius_lvl as f32 * 24.0;
        let heal_rate = combat_config.heal_rate * (1.0 + heal_lvl as f32 * UPGRADE_PCT[UpgradeType::HealingRate as usize]);

        cache.by_faction[town.faction as usize].push(crate::resources::HealingZone {
            center: town.center,
            radius_sq: radius * radius,
            heal_rate,
        });
    }

    dirty.healing_zones = false;
    #[cfg(debug_assertions)]
    info!("Healing zone cache rebuilt: {} factions", cache.by_faction.len());
}

/// Heal NPCs inside their faction's town center healing aura.
/// Adds/removes Healing marker for visual feedback.
/// Starving NPCs are capped at 50% HP.
pub fn healing_system(
    mut commands: Commands,
    mut query: Query<(Entity, &NpcIndex, &mut Health, &CachedStats, &Faction, &TownId, Option<&Healing>, Option<&Starving>), Without<Dead>>,
    gpu_state: Res<GpuReadState>,
    cache: Res<HealingZoneCache>,
    time: Res<Time>,
    world_data: Res<WorldData>,
    mut building_hp: ResMut<BuildingHpState>,
    building_slots: Res<BuildingSlotMap>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut debug: ResMut<HealthDebug>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("healing");
    let positions = &gpu_state.positions;
    let dt = time.delta_secs();

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

    // Heal damaged buildings in same-faction fountain range.
    let mut heal_buildings = |kind: BuildingKind, hps: &mut Vec<f32>, entries: &[(usize, Vec2, u32)]| {
        let max_hp = BuildingHpState::max_hp(kind);
        for &(idx, pos, town_idx) in entries {
            let Some(hp) = hps.get_mut(idx) else { continue };
            if *hp <= 0.0 || *hp >= max_hp { continue; }
            let faction = world_data.towns.get(town_idx as usize).map(|t| t.faction).unwrap_or(0);
            if faction < 0 { continue; }
            let zones = cache.by_faction.get(faction as usize).map(|v| v.as_slice()).unwrap_or(&[]);
            let mut zone_heal_rate = 0.0f32;
            for zone in zones {
                let dx = pos.x - zone.center.x;
                let dy = pos.y - zone.center.y;
                if dx * dx + dy * dy <= zone.radius_sq {
                    zone_heal_rate = zone.heal_rate;
                    break;
                }
            }
            if zone_heal_rate <= 0.0 { continue; }

            *hp = (*hp + zone_heal_rate * dt).min(max_hp);
            if let Some(slot) = building_slots.get_slot(kind, idx) {
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: slot, health: *hp }));
            }
        }
    };

    let farm_entries: Vec<(usize, Vec2, u32)> = world_data.farms.iter()
        .enumerate()
        .filter(|(_, b)| crate::world::is_alive(b.position))
        .map(|(i, b)| (i, b.position, b.town_idx))
        .collect();
    heal_buildings(BuildingKind::Farm, &mut building_hp.farms, &farm_entries);

    let waypoint_entries: Vec<(usize, Vec2, u32)> = world_data.waypoints.iter()
        .enumerate()
        .filter(|(_, b)| crate::world::is_alive(b.position))
        .map(|(i, b)| (i, b.position, b.town_idx))
        .collect();
    heal_buildings(BuildingKind::Waypoint, &mut building_hp.waypoints, &waypoint_entries);

    let farmer_home_entries: Vec<(usize, Vec2, u32)> = world_data.farmer_homes.iter()
        .enumerate()
        .filter(|(_, b)| crate::world::is_alive(b.position))
        .map(|(i, b)| (i, b.position, b.town_idx))
        .collect();
    heal_buildings(BuildingKind::FarmerHome, &mut building_hp.farmer_homes, &farmer_home_entries);

    let archer_home_entries: Vec<(usize, Vec2, u32)> = world_data.archer_homes.iter()
        .enumerate()
        .filter(|(_, b)| crate::world::is_alive(b.position))
        .map(|(i, b)| (i, b.position, b.town_idx))
        .collect();
    heal_buildings(BuildingKind::ArcherHome, &mut building_hp.archer_homes, &archer_home_entries);

    let crossbow_home_entries: Vec<(usize, Vec2, u32)> = world_data.crossbow_homes.iter()
        .enumerate()
        .filter(|(_, b)| crate::world::is_alive(b.position))
        .map(|(i, b)| (i, b.position, b.town_idx))
        .collect();
    heal_buildings(BuildingKind::CrossbowHome, &mut building_hp.crossbow_homes, &crossbow_home_entries);

    let tent_entries: Vec<(usize, Vec2, u32)> = world_data.tents.iter()
        .enumerate()
        .filter(|(_, b)| crate::world::is_alive(b.position))
        .map(|(i, b)| (i, b.position, b.town_idx))
        .collect();
    heal_buildings(BuildingKind::Tent, &mut building_hp.tents, &tent_entries);

    let miner_home_entries: Vec<(usize, Vec2, u32)> = world_data.miner_homes.iter()
        .enumerate()
        .filter(|(_, b)| crate::world::is_alive(b.position))
        .map(|(i, b)| (i, b.position, b.town_idx))
        .collect();
    heal_buildings(BuildingKind::MinerHome, &mut building_hp.miner_homes, &miner_home_entries);

    let bed_entries: Vec<(usize, Vec2, u32)> = world_data.beds.iter()
        .enumerate()
        .filter(|(_, b)| crate::world::is_alive(b.position))
        .map(|(i, b)| (i, b.position, b.town_idx))
        .collect();
    heal_buildings(BuildingKind::Bed, &mut building_hp.beds, &bed_entries);

    let town_entries: Vec<(usize, Vec2, u32)> = world_data.towns.iter()
        .enumerate()
        .map(|(i, b)| (i, b.center, i as u32))
        .collect();
    heal_buildings(BuildingKind::Fountain, &mut building_hp.towns, &town_entries);

    // Update debug stats
    debug.healing_npcs_checked = npcs_checked;
    debug.healing_positions_len = positions.len();
    debug.healing_towns_count = cache.by_faction.iter().map(|v| v.len()).sum();
    debug.healing_in_zone_count = in_zone_count;
    debug.healing_healed_count = healed_count;
}
