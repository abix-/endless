//! ECS Resources - Shared state accessible by all systems

use godot_bevy::prelude::bevy_ecs_prelude::*;
use std::collections::HashMap;

/// Tracks total number of active NPCs.
#[derive(Resource, Default)]
pub struct NpcCount(pub usize);

/// Delta time for the current frame (seconds).
#[derive(Resource, Default)]
pub struct DeltaTime(pub f32);

/// O(1) lookup from NPC slot index to Bevy Entity.
/// Populated on spawn, used by damage_system for fast entity lookup.
#[derive(Resource, Default)]
pub struct NpcEntityMap(pub HashMap<usize, Entity>);
