//! Health systems - Damage, death detection, cleanup

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::*;

use crate::components::*;
use crate::messages::*;

/// Apply queued damage to Health component.
pub fn damage_system(
    mut events: MessageReader<DamageMsg>,
    mut query: Query<(&mut Health, &NpcIndex)>,
) {
    let mut damage_count = 0;
    for event in events.read() {
        damage_count += 1;
        for (mut health, npc_idx) in query.iter_mut() {
            if npc_idx.0 == event.npc_index {
                health.0 = (health.0 - event.amount).max(0.0);
                break;
            }
        }
    }

    // Update debug info
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

/// Remove dead entities, hide on GPU by setting position to -9999.
pub fn death_cleanup_system(
    mut commands: Commands,
    query: Query<(Entity, &NpcIndex), With<Dead>>,
) {
    let mut despawn_count = 0;
    for (entity, npc_idx) in query.iter() {
        commands.entity(entity).despawn();
        despawn_count += 1;

        // Queue GPU position update to hide (-9999, -9999)
        if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
            queue.push(SetTargetMsg {
                npc_index: npc_idx.0,
                x: -9999.0,
                y: -9999.0,
            });
        }

        // Decrement authoritative NPC count
        if let Ok(mut count) = GPU_NPC_COUNT.lock() {
            *count = count.saturating_sub(1);
        }
    }

    if let Ok(mut debug) = HEALTH_DEBUG.lock() {
        debug.despawned_this_frame = despawn_count;
    }
}
