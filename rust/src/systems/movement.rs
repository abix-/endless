//! Movement systems - Target tracking and arrival detection

use godot_bevy::prelude::bevy_ecs_prelude::*;

use crate::components::*;
use crate::messages::*;
use crate::resources::*;

/// Process target messages: update GPU data and add HasTarget component.
pub fn apply_targets_system(
    mut commands: Commands,
    mut events: MessageReader<SetTargetMsg>,
    mut gpu_data: ResMut<GpuData>,
    query: Query<(Entity, &NpcIndex), Without<HasTarget>>,
) {
    for event in events.read() {
        if event.npc_index < gpu_data.npc_count {
            // Update target in GPU data
            gpu_data.targets[event.npc_index * 2] = event.x;
            gpu_data.targets[event.npc_index * 2 + 1] = event.y;
            gpu_data.dirty = true;

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
