//! Sync system - Send changed state to Godot via channels (Phase 11)
//!
//! Single system handles Changed<T> queries, keeping outbox writes in one place.

use godot_bevy::prelude::bevy_ecs_prelude::*;

use crate::channels::{BevyToGodot, BevyToGodotMsg};
use crate::components::{NpcIndex, Health};

/// Write changed NPC state to BevyToGodot outbox.
/// Runs once per frame, only sends messages for components that actually changed.
pub fn bevy_to_godot_write(
    healths: Query<(&NpcIndex, &Health), Changed<Health>>,
    outbox: Option<Res<BevyToGodot>>,
) {
    let outbox = match outbox {
        Some(o) => o,
        None => return,
    };

    // Sync changed health
    for (npc_idx, health) in healths.iter() {
        let _ = outbox.0.send(BevyToGodotMsg::SyncHealth {
            slot: npc_idx.0,
            hp: health.0,
            max_hp: 100.0,
        });
    }
}
