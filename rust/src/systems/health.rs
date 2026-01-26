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
    for event in events.read() {
        for (mut health, npc_idx) in query.iter_mut() {
            if npc_idx.0 == event.npc_index {
                health.0 = (health.0 - event.amount).max(0.0);
                break;
            }
        }
    }
}

/// Mark dead entities with Dead component.
pub fn death_system(
    mut commands: Commands,
    query: Query<(Entity, &Health, &NpcIndex), Without<Dead>>,
) {
    for (entity, health, _npc_idx) in query.iter() {
        if health.0 <= 0.0 {
            commands.entity(entity).insert(Dead);
        }
    }
}

/// Remove dead entities, hide on GPU by setting position to -9999.
pub fn death_cleanup_system(
    mut commands: Commands,
    query: Query<(Entity, &NpcIndex), With<Dead>>,
) {
    for (entity, npc_idx) in query.iter() {
        commands.entity(entity).despawn();

        // Queue GPU position update to hide (-9999, -9999)
        if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
            queue.push(SetTargetMsg {
                npc_index: npc_idx.0,
                x: -9999.0,
                y: -9999.0,
            });
        }
    }
}
