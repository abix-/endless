//! Movement systems - Target tracking, arrival detection, intent resolution

use bevy::prelude::*;

use crate::components::*;
use crate::constants::ARRIVAL_THRESHOLD;
use crate::gpu::EntityGpuState;
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::resources::{GameTime, GpuReadState, MovementIntents, NpcTargetThrashDebug};

/// Read positions from GPU readback buffer → ECS Position + arrival detection.
/// GPU is movement authority; ECS Position is read-model synced here.
/// Query-first: iterates ECS archetypes, not HashMap.
pub fn gpu_position_readback(
    gpu_state: Res<GpuReadState>,
    buffer_writes: Res<EntityGpuState>,
    mut npc_q: Query<(&GpuSlot, &mut Position, &Activity, &mut NpcFlags)>,
) {
    let positions = &gpu_state.positions;
    let targets = &buffer_writes.targets;
    let threshold_sq = ARRIVAL_THRESHOLD * ARRIVAL_THRESHOLD;

    for (es, mut pos, activity, mut flags) in npc_q.iter_mut() {
        let i = es.0;
        if i * 2 + 1 >= positions.len() { continue; }

        let gpu_x = positions[i * 2];
        let gpu_y = positions[i * 2 + 1];

        if gpu_x < -9000.0 { continue; }

        pos.x = gpu_x;
        pos.y = gpu_y;

        // CPU-side arrival detection
        if activity.is_transit() && !flags.at_destination {
            if i * 2 + 1 < targets.len() {
                let goal_x = targets[i * 2];
                let goal_y = targets[i * 2 + 1];
                let dx = gpu_x - goal_x;
                let dy = gpu_y - goal_y;
                if dx * dx + dy * dy <= threshold_sq {
                    flags.at_destination = true;
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
    npc_query: Query<&GpuSlot>,
    npc_gpu: Res<EntityGpuState>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut target_thrash: ResMut<NpcTargetThrashDebug>,
    game_time: Res<GameTime>,
) {
    if game_time.is_paused() { return; }
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
