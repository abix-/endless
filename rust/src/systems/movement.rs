//! Movement systems - Target tracking, arrival detection, position sync

use godot_bevy::prelude::bevy_ecs_prelude::*;

use crate::components::*;
use crate::messages::{SetTargetMsg, GpuUpdate, GpuUpdateMsg};
use crate::resources::GpuReadState;

/// Process target messages: push to GPU update queue and add HasTarget component.
pub fn apply_targets_system(
    mut commands: Commands,
    mut events: MessageReader<SetTargetMsg>,
    query: Query<(Entity, &NpcIndex), Without<HasTarget>>,
    gpu_state: Res<GpuReadState>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
) {
    let npc_count = gpu_state.npc_count;

    for event in events.read() {
        if event.npc_index < npc_count {
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx: event.npc_index, x: event.x, y: event.y }));

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

/// Read positions from GPU and update Bevy Position components.
/// This makes Bevy the source of truth, with GPU as accelerator.
/// Only updates positions that actually changed (within epsilon).
pub fn gpu_position_readback(
    mut query: Query<(&NpcIndex, &mut Position)>,
    gpu_state: Res<GpuReadState>,
) {
    let positions = &gpu_state.positions;

    const EPSILON: f32 = 0.01;

    for (npc_idx, mut pos) in query.iter_mut() {
        let i = npc_idx.0;
        if i * 2 + 1 >= positions.len() {
            continue;
        }

        let gpu_x = positions[i * 2];
        let gpu_y = positions[i * 2 + 1];

        // Only update if position actually changed (avoids spurious Changed<Position>)
        let dx = (gpu_x - pos.x).abs();
        let dy = (gpu_y - pos.y).abs();
        if dx > EPSILON || dy > EPSILON {
            pos.x = gpu_x;
            pos.y = gpu_y;
        }
    }
}
