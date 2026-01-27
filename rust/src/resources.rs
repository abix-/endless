//! ECS Resources - Shared state accessible by all systems

use godot_bevy::prelude::bevy_ecs_prelude::*;
use std::collections::HashMap;
use crate::constants::MAX_NPC_COUNT;

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

/// CPU-side copy of GPU data, used for uploading to GPU buffers.
/// When `dirty` is true, the data needs to be re-uploaded.
#[derive(Resource)]
pub struct GpuData {
    /// Position data: [x0, y0, x1, y1, ...] - 2 floats per NPC
    pub positions: Vec<f32>,
    /// Target positions: [tx0, ty0, tx1, ty1, ...] - 2 floats per NPC
    pub targets: Vec<f32>,
    /// Colors: [r0, g0, b0, a0, r1, g1, b1, a1, ...] - 4 floats per NPC
    pub colors: Vec<f32>,
    /// Movement speeds: one float per NPC
    pub speeds: Vec<f32>,
    /// Faction data: 0=Villager, 1=Raider - one i32 per NPC
    pub factions: Vec<i32>,
    /// Health data: current HP - one f32 per NPC
    pub healths: Vec<f32>,
    /// Combat targets from GPU: -1 = no target, else NPC index
    pub combat_targets: Vec<i32>,
    /// Current NPC count
    pub npc_count: usize,
    /// True if data changed and needs GPU upload
    pub dirty: bool,
}

impl Default for GpuData {
    fn default() -> Self {
        Self {
            positions: vec![0.0; MAX_NPC_COUNT * 2],
            targets: vec![0.0; MAX_NPC_COUNT * 2],
            colors: vec![0.0; MAX_NPC_COUNT * 4],
            speeds: vec![0.0; MAX_NPC_COUNT],
            factions: vec![0; MAX_NPC_COUNT],
            healths: vec![100.0; MAX_NPC_COUNT],
            combat_targets: vec![-1; MAX_NPC_COUNT],
            npc_count: 0,
            dirty: false,
        }
    }
}
