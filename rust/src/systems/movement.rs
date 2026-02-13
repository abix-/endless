//! Movement systems - Target tracking, arrival detection, position sync

use bevy::prelude::*;

use crate::components::*;
use crate::constants::ARRIVAL_THRESHOLD;
use crate::gpu::NpcBufferWrites;
use crate::resources::{GpuReadState, SystemTimings};

/// Read positions from GPU and update Bevy Position components.
/// Also detects arrivals: if NPC is in a transit Activity and within ARRIVAL_THRESHOLD
/// of their goal, add AtDestination marker for decision_system to handle.
pub fn gpu_position_readback(
    mut commands: Commands,
    mut query: Query<(Entity, &NpcIndex, &mut Position, &Activity, Option<&AtDestination>)>,
    gpu_state: Res<GpuReadState>,
    buffer_writes: Res<NpcBufferWrites>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("gpu_position_readback");
    let positions = &gpu_state.positions;
    let targets = &buffer_writes.targets;
    let threshold_sq = ARRIVAL_THRESHOLD * ARRIVAL_THRESHOLD;

    const EPSILON: f32 = 0.01;

    for (entity, npc_idx, mut pos, activity, at_dest) in query.iter_mut() {
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
        if activity.is_transit() && at_dest.is_none() {
            if i * 2 + 1 < targets.len() {
                let goal_x = targets[i * 2];
                let goal_y = targets[i * 2 + 1];
                let dx = gpu_x - goal_x;
                let dy = gpu_y - goal_y;
                let dist_sq = dx * dx + dy * dy;

                if dist_sq <= threshold_sq {
                    commands.entity(entity)
                        .insert(AtDestination);
                }
            }
        }
    }
}
