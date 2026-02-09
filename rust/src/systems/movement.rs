//! Movement systems - Target tracking, arrival detection, position sync

use bevy::prelude::*;

use crate::components::*;
use crate::constants::ARRIVAL_THRESHOLD;
use crate::gpu::NpcBufferWrites;
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
/// Also detects arrivals: if NPC has HasTarget and is within ARRIVAL_THRESHOLD
/// of their goal, add AtDestination marker for decision_system to handle.
pub fn gpu_position_readback(
    mut commands: Commands,
    mut query: Query<(Entity, &NpcIndex, &mut Position, Option<&HasTarget>, Option<&AtDestination>)>,
    gpu_state: Res<GpuReadState>,
    buffer_writes: Res<NpcBufferWrites>,
) {
    let positions = &gpu_state.positions;
    let targets = &buffer_writes.targets;
    let threshold_sq = ARRIVAL_THRESHOLD * ARRIVAL_THRESHOLD;

    const EPSILON: f32 = 0.01;

    for (entity, npc_idx, mut pos, has_target, at_dest) in query.iter_mut() {
        let i = npc_idx.0;
        if i * 2 + 1 >= positions.len() {
            continue;
        }

        let gpu_x = positions[i * 2];
        let gpu_y = positions[i * 2 + 1];

        // Skip hidden NPCs
        if gpu_x < -9000.0 {
            continue;
        }

        // Update ECS position from GPU
        let dx = (gpu_x - pos.x).abs();
        let dy = (gpu_y - pos.y).abs();
        if dx > EPSILON || dy > EPSILON {
            pos.x = gpu_x;
            pos.y = gpu_y;
        }

        // CPU-side arrival detection: check if NPC reached their goal
        if has_target.is_some() && at_dest.is_none() {
            if i * 2 + 1 < targets.len() {
                let goal_x = targets[i * 2];
                let goal_y = targets[i * 2 + 1];
                let dx = gpu_x - goal_x;
                let dy = gpu_y - goal_y;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq <= threshold_sq {
                    commands.entity(entity)
                        .insert(AtDestination)
                        .remove::<HasTarget>();
                }
            }
        }
    }
}
