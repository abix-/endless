//! Sync system - Send all changed state to Godot via channels (Phase 11)
//!
//! Single system handles all Changed<T> queries, keeping outbox writes in one place.

use godot_bevy::prelude::bevy_ecs_prelude::*;

use crate::channels::{BevyToGodot, BevyToGodotMsg};
use crate::components::{NpcIndex, Position, Health};

/// Write changed NPC state to BevyToGodot outbox.
/// Runs once per frame, only sends messages for components that actually changed.
pub fn bevy_to_godot_write(
    positions: Query<(&NpcIndex, &Position), Changed<Position>>,
    healths: Query<(&NpcIndex, &Health), Changed<Health>>,
    outbox: Option<Res<BevyToGodot>>,
) {
    let outbox = match outbox {
        Some(o) => o,
        None => return,
    };

    // Sync changed positions
    for (npc_idx, pos) in positions.iter() {
        let _ = outbox.0.send(BevyToGodotMsg::SyncTransform {
            slot: npc_idx.0,
            x: pos.x,
            y: pos.y,
        });
    }

    // Sync changed health
    for (npc_idx, health) in healths.iter() {
        let _ = outbox.0.send(BevyToGodotMsg::SyncHealth {
            slot: npc_idx.0,
            hp: health.0,
            max_hp: 100.0,
        });
    }
}
