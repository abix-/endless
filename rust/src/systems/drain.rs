//! Queue drain systems - Move messages from static queues to Bevy events

use godot_bevy::prelude::bevy_ecs_prelude::{MessageWriter, MessageReader, Res, ResMut};
use crate::channels::{GodotToBevy, GodotToBevyMsg};
use crate::constants::MAX_NPC_COUNT;
use crate::messages::*;
use crate::resources;

/// Drain the spawn queue.
pub fn drain_spawn_queue(mut messages: MessageWriter<SpawnNpcMsg>) {
    if let Ok(mut queue) = SPAWN_QUEUE.lock() {
        for msg in queue.drain(..) {
            messages.write(msg);
        }
    }
}

/// Drain the target queue.
pub fn drain_target_queue(mut messages: MessageWriter<SetTargetMsg>) {
    if let Ok(mut queue) = TARGET_QUEUE.lock() {
        for msg in queue.drain(..) {
            messages.write(msg);
        }
    }
}

/// Drain arrival queue and convert to Bevy messages.
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

/// Drain the damage queue.
pub fn drain_damage_queue(mut messages: MessageWriter<DamageMsg>) {
    if let Ok(mut queue) = DAMAGE_QUEUE.lock() {
        for msg in queue.drain(..) {
            messages.write(msg);
        }
    }
}

/// Collect GPU update messages from all systems into the static queue.
/// Runs at end of Behavior phase - single lock point for all GPU writes.
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
) {
    let inbox = match inbox {
        Some(r) => r,
        None => return,
    };
    while let Ok(msg) = inbox.0.try_recv() {
        match msg {
            GodotToBevyMsg::SpawnNpc {
                x, y, job, faction, town_idx,
                home_x, home_y, work_x, work_y,
                starting_post, attack_type,
            } => {
                // Allocate slot (still uses static for now)
                let slot = allocate_slot();
                if let Some(idx) = slot {
                    spawn_msgs.write(SpawnNpcMsg {
                        slot_idx: idx,
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
                if let Ok(mut flag) = RESET_BEVY.lock() {
                    *flag = true;
                }
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

/// Allocate an NPC slot: reuse a free slot or allocate new.
fn allocate_slot() -> Option<usize> {
    if let Ok(mut free) = FREE_SLOTS.lock() {
        if let Some(recycled) = free.pop() {
            return Some(recycled);
        }
    }
    if let Ok(mut counter) = NPC_SLOT_COUNTER.lock() {
        if *counter < MAX_NPC_COUNT {
            let idx = *counter;
            *counter += 1;
            return Some(idx);
        }
    }
    None
}
