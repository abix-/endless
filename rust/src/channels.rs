//! Lock-free channels for Godot ↔ Bevy communication
//!
//! Replaces static Mutex queues with proper message passing:
//! - GodotToBevy: spawn requests, player commands, GPU projectile hits
//! - BevyToGodot: view updates, transform sync, projectile fire commands
//!
//! Uses crossbeam-channel because Sender is Sync (allows parallel Bevy systems).

use crossbeam_channel::{unbounded, Receiver, Sender};
use godot_bevy::prelude::bevy_ecs_prelude::*;

// ============================================================================
// GODOT → BEVY
// ============================================================================

/// Messages from Godot to Bevy
#[derive(Debug, Clone)]
pub enum GodotToBevyMsg {
    /// Spawn a new NPC
    SpawnNpc {
        x: f32,
        y: f32,
        job: u8,
        faction: u8,
        town_idx: i32,
        home_x: f32,
        home_y: f32,
        work_x: f32,
        work_y: f32,
        starting_post: i32,
        attack_type: u8,
    },

    /// Set movement target for an NPC
    SetTarget { slot: usize, x: f32, y: f32 },

    /// Player clicked at position (for selection)
    PlayerClick { x: f32, y: f32 },

    /// Select an NPC (-1 to deselect)
    SelectNpc { slot: i32 },

    /// Apply damage to an NPC (from GPU projectile hits)
    ApplyDamage { slot: usize, amount: f32 },

    /// Reset the entire ECS world
    Reset,

    /// Pause/unpause game time
    SetPaused(bool),

    /// Set game time scale
    SetTimeScale(f32),
}

/// Bevy resource - receives messages from Godot
#[derive(Resource)]
pub struct GodotToBevy(pub Receiver<GodotToBevyMsg>);

/// Godot-side sender (stored in EcsNpcManager)
#[derive(Clone)]
pub struct GodotToBevySender(pub Sender<GodotToBevyMsg>);

// ============================================================================
// BEVY → GODOT
// ============================================================================

/// Messages from Bevy to Godot
#[derive(Debug, Clone)]
pub enum BevyToGodotMsg {
    /// Create visual for a new NPC
    SpawnView {
        slot: usize,
        job: u8,
        x: f32,
        y: f32,
    },

    /// Remove visual for a dead NPC
    DespawnView { slot: usize },

    /// Update NPC position (only sent for Changed<Position>)
    SyncTransform { slot: usize, x: f32, y: f32 },

    /// Update NPC health (only sent for Changed<Health>)
    SyncHealth { slot: usize, hp: f32, max_hp: f32 },

    /// Update NPC color
    SyncColor {
        slot: usize,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    },

    /// Update NPC sprite frame
    SyncSprite { slot: usize, col: f32, row: f32 },

    /// Fire a projectile (GPU projectile system)
    FireProjectile {
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        speed: f32,
        damage: f32,
        faction: i32,
        shooter: usize,
        lifetime: f32,
    },
}

/// Bevy resource - sends messages to Godot (Sync - safe for parallel systems)
#[derive(Resource, Clone)]
pub struct BevyToGodot(pub Sender<BevyToGodotMsg>);

/// Godot-side receiver (stored in EcsNpcManager)
pub struct BevyToGodotReceiver(pub Receiver<BevyToGodotMsg>);

// ============================================================================
// Channel creation
// ============================================================================

/// Both ends of the communication channels
pub struct ChannelPair {
    /// Godot holds this to send to Bevy
    pub godot_to_bevy_sender: GodotToBevySender,
    /// Bevy resource - receives from Godot
    pub godot_to_bevy_receiver: GodotToBevy,
    /// Bevy resource - sends to Godot
    pub bevy_to_godot_sender: BevyToGodot,
    /// Godot holds this to receive from Bevy
    pub bevy_to_godot_receiver: BevyToGodotReceiver,
}

/// Create unbounded channels for Godot ↔ Bevy communication
pub fn create_channels() -> ChannelPair {
    let (g2b_tx, g2b_rx) = unbounded();
    let (b2g_tx, b2g_rx) = unbounded();

    ChannelPair {
        godot_to_bevy_sender: GodotToBevySender(g2b_tx),
        godot_to_bevy_receiver: GodotToBevy(g2b_rx),
        bevy_to_godot_sender: BevyToGodot(b2g_tx),
        bevy_to_godot_receiver: BevyToGodotReceiver(b2g_rx),
    }
}
