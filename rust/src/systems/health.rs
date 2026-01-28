//! Health systems - Damage, death detection, cleanup

use godot_bevy::prelude::bevy_ecs_prelude::*;

use crate::components::*;
use crate::messages::*;
use crate::resources::*;
use crate::systems::economy::*;

/// Apply queued damage to Health component and sync to GPU.
/// Uses NpcEntityMap for O(1) entity lookup instead of O(n) iteration.
pub fn damage_system(
    mut events: MessageReader<DamageMsg>,
    npc_map: Res<NpcEntityMap>,
    mut query: Query<(&mut Health, &NpcIndex)>,
) {
    let mut damage_count = 0;
    for event in events.read() {
        damage_count += 1;
        // O(1) lookup via NpcEntityMap
        if let Some(&entity) = npc_map.0.get(&event.npc_index) {
            if let Ok((mut health, npc_idx)) = query.get_mut(entity) {
                health.0 = (health.0 - event.amount).max(0.0);
                // GPU-FIRST: Push to GPU_UPDATE_QUEUE
                if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                    queue.push(GpuUpdate::SetHealth { idx: npc_idx.0, health: health.0 });
                }
            }
        }
    }

    if let Ok(mut debug) = HEALTH_DEBUG.lock() {
        debug.damage_processed = damage_count;
        debug.bevy_entity_count = query.iter().count();
        debug.health_samples.clear();
        for (health, npc_idx) in query.iter().take(10) {
            debug.health_samples.push((npc_idx.0, health.0));
        }
    }
}

/// Mark dead entities with Dead component.
pub fn death_system(
    mut commands: Commands,
    query: Query<(Entity, &Health, &NpcIndex), Without<Dead>>,
) {
    let mut death_count = 0;
    for (entity, health, _npc_idx) in query.iter() {
        if health.0 <= 0.0 {
            commands.entity(entity).insert(Dead);
            death_count += 1;
        }
    }

    if let Ok(mut debug) = HEALTH_DEBUG.lock() {
        debug.deaths_this_frame = death_count;
    }
}

/// Remove dead entities, hide on GPU by setting position to -9999, recycle slot.
pub fn death_cleanup_system(
    mut commands: Commands,
    query: Query<(Entity, &NpcIndex, &Job, &TownId, Option<&Working>), With<Dead>>,
    mut npc_map: ResMut<NpcEntityMap>,
    mut pop_stats: ResMut<PopulationStats>,
) {
    let mut despawn_count = 0;
    for (entity, npc_idx, job, clan, working) in query.iter() {
        let idx = npc_idx.0;
        commands.entity(entity).despawn();
        despawn_count += 1;
        pop_dec_alive(&mut pop_stats, *job, clan.0);
        if working.is_some() {
            pop_dec_working(&mut pop_stats, *job, clan.0);
        }

        // Remove from entity map
        npc_map.0.remove(&idx);

        // GPU-FIRST: Hide NPC visually
        if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
            queue.push(GpuUpdate::HideNpc { idx });
        }

        // SLOT REUSE: Return slot to free pool
        if let Ok(mut free) = FREE_SLOTS.lock() {
            free.push(idx);
        }
    }

    if let Ok(mut debug) = HEALTH_DEBUG.lock() {
        debug.despawned_this_frame = despawn_count;
    }
}
