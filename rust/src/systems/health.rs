//! Health systems - Damage, death detection, cleanup, healing aura

use bevy::prelude::*;
use crate::components::*;
use crate::constants::STARVING_HP_CAP;
use crate::messages::{GpuUpdate, GpuUpdateMsg, DamageMsg};
use crate::resources::{NpcEntityMap, HealthDebug, PopulationStats, KillStats, NpcsByTownCache, SlotAllocator, GpuReadState, FactionStats, RaidQueue};
use crate::systems::economy::*;
use crate::world::{WorldData, FarmOccupancy, pos_to_key};

/// Heal rate in HP per second when inside healing aura.
const HEAL_RATE: f32 = 5.0;
/// Radius of healing aura around town center in pixels.
const HEAL_RADIUS: f32 = 150.0;

/// Apply queued damage to Health component and sync to GPU.
/// Uses NpcEntityMap for O(1) entity lookup instead of O(n) iteration.
pub fn damage_system(
    mut events: MessageReader<DamageMsg>,
    npc_map: Res<NpcEntityMap>,
    mut query: Query<(&mut Health, &NpcIndex)>,
    mut debug: ResMut<HealthDebug>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
) {
    let mut damage_count = 0;
    for event in events.read() {
        damage_count += 1;
        // O(1) lookup via NpcEntityMap
        if let Some(&entity) = npc_map.0.get(&event.npc_index) {
            if let Ok((mut health, npc_idx)) = query.get_mut(entity) {
                health.0 = (health.0 - event.amount).max(0.0);
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: npc_idx.0, health: health.0 }));
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetDamageFlash { idx: npc_idx.0, intensity: 1.0 }));
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
) {
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
    query: Query<(Entity, &NpcIndex, &Job, &TownId, &Faction, Option<&Working>, Option<&AssignedFarm>), With<Dead>>,
    mut npc_map: ResMut<NpcEntityMap>,
    mut pop_stats: ResMut<PopulationStats>,
    mut faction_stats: ResMut<FactionStats>,
    mut debug: ResMut<HealthDebug>,
    mut kill_stats: ResMut<KillStats>,
    mut npcs_by_town: ResMut<NpcsByTownCache>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut slots: ResMut<SlotAllocator>,
    mut farm_occupancy: ResMut<FarmOccupancy>,
    mut raid_queue: ResMut<RaidQueue>,
) {
    let mut despawn_count = 0;
    for (entity, npc_idx, job, town_id, faction, working, assigned_farm) in query.iter() {
        let idx = npc_idx.0;
        commands.entity(entity).despawn();
        despawn_count += 1;
        pop_dec_alive(&mut pop_stats, *job, town_id.0);
        pop_inc_dead(&mut pop_stats, *job, town_id.0);
        if working.is_some() {
            pop_dec_working(&mut pop_stats, *job, town_id.0);
        }

        // Release assigned farm if any
        if let Some(assigned) = assigned_farm {
            let farm_key = pos_to_key(assigned.0);
            if let Some(count) = farm_occupancy.occupants.get_mut(&farm_key) {
                *count = count.saturating_sub(1);
            }
        }

        // Remove from raid queue if raider was waiting
        if *job == Job::Raider {
            raid_queue.remove(faction.0, entity);
        }

        // Track kill statistics for UI (faction 0 = player/villager, 1+ = raiders)
        if faction.0 == 0 {
            kill_stats.villager_kills += 1;
        } else {
            kill_stats.guard_kills += 1;
        }

        // Track per-faction stats (alive/dead)
        faction_stats.dec_alive(faction.0);
        faction_stats.inc_dead(faction.0);

        // Remove from per-town NPC list
        if town_id.0 >= 0 {
            let town_idx = town_id.0 as usize;
            if town_idx < npcs_by_town.0.len() {
                npcs_by_town.0[town_idx].retain(|&i| i != idx);
            }
        }

        // Remove from entity map
        npc_map.0.remove(&idx);

        // Hide NPC visually via message
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::HideNpc { idx }));
        // Clear all equipment layers (prevents stale data on slot reuse)
        for layer in [EquipLayer::Armor, EquipLayer::Helmet, EquipLayer::Weapon, EquipLayer::Item] {
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetEquipSprite { idx, layer, col: -1.0, row: 0.0 }));
        }

        // Return slot to free pool
        slots.free(idx);
    }

    debug.despawned_this_frame = despawn_count;
}

/// Heal NPCs inside their faction's town center healing aura.
/// Adds/removes Healing marker for visual feedback.
/// Starving NPCs are capped at 50% HP.
pub fn healing_system(
    mut commands: Commands,
    mut query: Query<(Entity, &NpcIndex, &mut Health, &MaxHealth, &Faction, &TownId, Option<&Healing>, Option<&Starving>), Without<Dead>>,
    gpu_state: Res<GpuReadState>,
    world_data: Res<WorldData>,
    time: Res<Time>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut debug: ResMut<HealthDebug>,
) {
    let positions = &gpu_state.positions;
    let dt = time.delta_secs();
    let heal_amount = HEAL_RATE * dt;

    // Debug tracking
    let mut npcs_checked = 0usize;
    let mut in_zone_count = 0usize;
    let mut healed_count = 0usize;

    for (entity, npc_idx, mut health, max_health, faction, _town_id, healing_marker, starving) in query.iter_mut() {
        let idx = npc_idx.0;
        npcs_checked += 1;

        // Calculate effective HP cap (50% if starving)
        let hp_cap = if starving.is_some() {
            max_health.0 * STARVING_HP_CAP
        } else {
            max_health.0
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

        // Find if NPC is in any same-faction town's healing aura
        let mut in_healing_zone = false;
        for town in &world_data.towns {
            // Check faction match
            if town.faction != faction.to_i32() {
                continue;
            }

            let dx = x - town.center.x;
            let dy = y - town.center.y;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist <= HEAL_RADIUS {
                in_healing_zone = true;
                break;
            }
        }

        if in_healing_zone {
            in_zone_count += 1;

            // Heal up to HP cap (50% if starving, 100% otherwise)
            if health.0 < hp_cap {
                health.0 = (health.0 + heal_amount).min(hp_cap);
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health: health.0 }));
                healed_count += 1;
            }

            // Add marker if not present, send visual update
            if healing_marker.is_none() {
                commands.entity(entity).insert(Healing);
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealing { idx, healing: true }));
            }
        } else {
            // Remove marker if present, send visual update
            if healing_marker.is_some() {
                commands.entity(entity).remove::<Healing>();
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealing { idx, healing: false }));
            }
        }
    }

    // Update debug stats
    debug.healing_npcs_checked = npcs_checked;
    debug.healing_positions_len = positions.len();
    debug.healing_towns_count = world_data.towns.len();
    debug.healing_in_zone_count = in_zone_count;
    debug.healing_healed_count = healed_count;
}
