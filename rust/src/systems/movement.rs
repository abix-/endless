//! Movement systems - Target tracking and arrival detection

use godot_bevy::prelude::bevy_ecs_prelude::*;

use crate::components::*;
use crate::messages::*;

/// Process target messages: push to GPU update queue and add HasTarget component.
pub fn apply_targets_system(
    mut commands: Commands,
    mut events: MessageReader<SetTargetMsg>,
    query: Query<(Entity, &NpcIndex), Without<HasTarget>>,
) {
    let npc_count = GPU_READ_STATE.lock().map(|s| s.npc_count).unwrap_or(0);

    for event in events.read() {
        if event.npc_index < npc_count {
            // GPU-FIRST: Push to GPU_UPDATE_QUEUE
            if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                queue.push(GpuUpdate::SetTarget { idx: event.npc_index, x: event.x, y: event.y });
            }

            // Add HasTarget component to entity (if not already present)
            for (entity, npc_idx) in query.iter() {
                if npc_idx.0 == event.npc_index {
                    commands.entity(entity).insert(HasTarget);
                    break;
                }
            }
        }
    }
}
