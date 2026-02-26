//! Movement systems - Target tracking, arrival detection, intent resolution

use bevy::prelude::*;

use crate::components::*;
use crate::constants::ARRIVAL_THRESHOLD;
use crate::gpu::NpcGpuState;
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::resources::{GameTime, GpuReadState, MovementIntents, NpcTargetThrashDebug, SystemTimings};

/// Read positions from GPU and update Bevy Position components.
/// Also detects arrivals: if NPC is in a transit Activity and within ARRIVAL_THRESHOLD
/// of their goal, add AtDestination marker for decision_system to handle.
pub fn gpu_position_readback(
    mut commands: Commands,
    mut query: Query<(Entity, &NpcIndex, &mut Position, &Activity, Option<&AtDestination>)>,
    gpu_state: Res<GpuReadState>,
    buffer_writes: Res<NpcGpuState>,
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

/// Resolve movement intents: pick the highest-priority intent per NPC,
/// emit exactly one SetTarget when the target actually changed.
/// Runs after all intent-producing systems (decision, combat, health, render).
pub fn resolve_movement_system(
    mut intents: ResMut<MovementIntents>,
    npc_query: Query<&NpcIndex>,
    npc_gpu: Res<NpcGpuState>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut target_thrash: ResMut<NpcTargetThrashDebug>,
    game_time: Res<GameTime>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("resolve_movement");
    let targets = &npc_gpu.targets;
    let minute_key = game_time.day() * 24 * 60 + game_time.hour() * 60 + game_time.minute();

    for (entity, intent) in intents.drain() {
        let Ok(npc_idx) = npc_query.get(entity) else { continue };
        let idx = npc_idx.0;

        // Skip if target unchanged (same check as combat's target_changed)
        let i = idx * 2;
        if i + 1 < targets.len() {
            let dx = targets[i] - intent.target.x;
            let dy = targets[i + 1] - intent.target.y;
            if dx * dx + dy * dy <= 1.0 { continue; }
        }

        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
            idx, x: intent.target.x, y: intent.target.y,
        }));
        target_thrash.record(idx, intent.source, minute_key, intent.target.x, intent.target.y);
    }
}
