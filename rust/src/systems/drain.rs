//! Queue drain systems - Move messages from static queues to Bevy events

use godot_bevy::prelude::bevy_ecs_prelude::MessageWriter;
use crate::messages::*;

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

/// Drain the damage queue.
pub fn drain_damage_queue(mut messages: MessageWriter<DamageMsg>) {
    if let Ok(mut queue) = DAMAGE_QUEUE.lock() {
        for msg in queue.drain(..) {
            messages.write(msg);
        }
    }
}
