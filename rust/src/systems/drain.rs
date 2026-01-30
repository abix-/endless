//! Queue drain systems - Move messages from static queues to Bevy events

use godot_bevy::prelude::bevy_ecs_prelude::{MessageWriter, MessageReader, Res, ResMut};
use crate::channels::{GodotToBevy, GodotToBevyMsg};
use crate::messages::*;
use crate::resources::{self, ResetFlag};

// Legacy drain functions removed in Phase 11.7:
// - drain_spawn_queue → godot_to_bevy_read
// - drain_target_queue → godot_to_bevy_read
// - drain_damage_queue → godot_to_bevy_read

/// Drain arrival queue and convert to Bevy messages.
/// Still needed: lib.rs pushes arrivals from GPU readback.
pub fn drain_arrival_queue(mut messages: MessageWriter<ArrivalMsg>) {
    if let Ok(mut queue) = ARRIVAL_QUEUE.lock() {
        for msg in queue.drain(..) {
            messages.write(msg);
        }
    }
}

/// Drain game config staging into Bevy Resource (one-shot).
pub fn drain_game_config(mut config: ResMut<crate::resources::GameConfig>) {
    if let Ok(mut staging) = GAME_CONFIG_STAGING.lock() {
        if let Some(new_config) = staging.take() {
            *config = new_config;
        }
    }
}

/// Collect GPU update messages from all systems into the static queue.
/// Runs at end of Behavior phase - single lock point for all GPU writes.
/// Still needed: lib.rs reads GPU_UPDATE_QUEUE to update GPU buffers.
pub fn collect_gpu_updates(mut messages: MessageReader<GpuUpdateMsg>) {
    if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
        for msg in messages.read() {
            queue.push(msg.0.clone());
        }
    }
}

/// Read from GodotToBevy inbox and dispatch to Bevy messages/resources.
/// Phase 11: Replaces static mutex queues with lock-free channel.
pub fn godot_to_bevy_read(
    inbox: Option<Res<GodotToBevy>>,
    mut spawn_msgs: MessageWriter<SpawnNpcMsg>,
    mut target_msgs: MessageWriter<SetTargetMsg>,
    mut damage_msgs: MessageWriter<DamageMsg>,
    mut game_time: ResMut<resources::GameTime>,
    mut selected: ResMut<resources::SelectedNpc>,
    mut reset_flag: ResMut<ResetFlag>,
) {
    let inbox = match inbox {
        Some(r) => r,
        None => return,
    };
    while let Ok(msg) = inbox.0.try_recv() {
        match msg {
            GodotToBevyMsg::SpawnNpc {
                slot_idx, x, y, job, faction, town_idx,
                home_x, home_y, work_x, work_y,
                starting_post, attack_type,
            } => {
                // Slot pre-allocated by lib.rs
                spawn_msgs.write(SpawnNpcMsg {
                    slot_idx,
                    x, y,
                    job: job as i32,
                    faction: faction as i32,
                    town_idx,
                    home_x, home_y,
                    work_x, work_y,
                    starting_post,
                    attack_type: attack_type as i32,
                });
            }
            GodotToBevyMsg::SetTarget { slot, x, y } => {
                target_msgs.write(SetTargetMsg { npc_index: slot, x, y });
            }
            GodotToBevyMsg::ApplyDamage { slot, amount } => {
                damage_msgs.write(DamageMsg { npc_index: slot, amount });
            }
            GodotToBevyMsg::SelectNpc { slot } => {
                selected.0 = slot;
            }
            GodotToBevyMsg::PlayerClick { x: _, y: _ } => {
                // TODO: implement click-to-select in Bevy
            }
            GodotToBevyMsg::Reset => {
                reset_flag.0 = true;
            }
            GodotToBevyMsg::SetPaused(paused) => {
                game_time.paused = paused;
            }
            GodotToBevyMsg::SetTimeScale(scale) => {
                game_time.time_scale = scale;
            }
        }
    }
}
