# Phase 11: Clean Architecture

## Overview

Eliminate all 14 remaining static mutexes via proper architecture:

| Before | After |
|--------|-------|
| 14 static Mutex | 0 static Mutex |
| GPU owns positions | Bevy owns all state |
| Lock every position read | Direct component access |
| Sync all 10k positions | Sync only Changed<T> |
| Arrivals via GPU→queue→Bevy | Arrivals in Bevy (distance check) |
| Scattered queues | Two channels (Inbox + Outbox) |

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                              GODOT                                      │
│                                                                         │
│  GDScript                         Rendering                             │
│  - Player input ──────┐     ┌──── MultiMesh (NPCs)                      │
│  - UI events          │     │     Projectile visuals                    │
│                       │     │                                           │
│                       ▼     │                                           │
│              ┌────────────────────┐                                     │
│              │   INBOX CHANNEL    │  (lock-free Sender)                 │
│              │   Godot → Bevy     │                                     │
│              └─────────┬──────────┘                                     │
│                        │                                                │
├────────────────────────│────────────────────────────────────────────────┤
│                        ▼            BEVY ECS                            │
│              ┌──────────────────┐                                       │
│              │  drain_inbox     │  (Step::Drain)                        │
│              └────────┬─────────┘                                       │
│                       │                                                 │
│                       ▼                                                 │
│              ┌──────────────────┐                                       │
│              │  AI / Decisions  │  (Step::AI - parallel)                │
│              └────────┬─────────┘                                       │
│                       │                                                 │
│                       ▼                                                 │
│              ┌──────────────────┐      ┌────────────────────┐           │
│              │  upload_to_gpu   │─────►│    GPU COMPUTE     │           │
│              └──────────────────┘      │  (Step::Gpu)       │           │
│                                        │  - npc_physics     │           │
│              ┌──────────────────┐      │  - combat_target   │           │
│              │  readback_gpu    │◄─────│  - projectiles     │           │
│              └────────┬─────────┘      └────────────────────┘           │
│                       │                                                 │
│                       ▼                                                 │
│              ┌──────────────────┐                                       │
│              │  apply_results   │  (Step::Apply)                        │
│              └────────┬─────────┘                                       │
│                       │                                                 │
│                       ▼                                                 │
│              ┌──────────────────┐                                       │
│              │  emit_sync       │  (Step::Sync) Changed<T> only         │
│              └────────┬─────────┘                                       │
│                       │                                                 │
├───────────────────────│─────────────────────────────────────────────────┤
│                       ▼                                                 │
│              ┌────────────────────┐                                     │
│              │   OUTBOX CHANNEL   │  (lock-free Receiver)               │
│              │   Bevy → Godot     │                                     │
│              └─────────┬──────────┘                                     │
│                        │                                                │
│                        ▼                                                │
│              ┌──────────────────────────────────────────┐               │
│              │  _process() drain outbox                 │               │
│              │  - SyncTransform → update GPU buffer     │               │
│              │  - SpawnView → set slot active           │               │
│              │  - FireProjectile → GPU projectile       │               │
│              └──────────────────────────────────────────┘               │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

## Ownership Model

| Data | Owner | GPU Role |
|------|-------|----------|
| Position | Bevy (Position component) | Computes new position, Bevy reads back |
| Velocity | Bevy (Velocity component) | Computes from physics |
| Target | Bevy (MoveTarget component) | Uploaded for movement |
| Health | Bevy (Health component) | Uploaded for targeting priority |
| Combat target | Bevy (CombatTarget component) | GPU finds nearest, Bevy reads back |
| AI state | Bevy (marker components) | N/A |
| Arrivals | Bevy (distance check) | GPU can flag, Bevy confirms |

## What GPU Still Computes

GPU remains valuable for parallel work:

| Task | GPU Compute | Bevy Logic |
|------|-------------|------------|
| Movement physics | Move toward target, separation forces | Sets target, speed |
| Combat targeting | Find nearest enemy in range (spatial query) | Decides to attack, applies damage |
| Projectile physics | Move projectiles, hit detection | Spawns projectile, handles hit |
| Arrival detection | Distance check (batch 10k) | State transition on arrival |

---

# Phase 11.1: Channel Infrastructure

## Goal

Create lock-free channels without breaking existing code.

## Dependencies

```toml
# rust/Cargo.toml
[dependencies]
crossbeam-channel = "0.5"
```

## New File: rust/src/channels.rs

```rust
//! Lock-free channels for Godot ↔ Bevy communication

use bevy::prelude::*;
use crossbeam_channel::{Sender, Receiver, unbounded};

// ============================================================================
// INBOX: Godot → Bevy
// ============================================================================

/// Messages from Godot to Bevy
#[derive(Debug, Clone)]
pub enum InboxMsg {
    // Spawn requests
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

    // Player commands
    SetTarget { slot: usize, x: f32, y: f32 },
    PlayerClick { x: f32, y: f32 },
    SelectNpc { slot: i32 },  // -1 to deselect

    // GPU results (projectile hits)
    ApplyDamage { slot: usize, amount: f32 },

    // System commands
    Reset,
    SetPaused(bool),
    SetTimeScale(f32),
}

/// Bevy resource containing inbox receiver
#[derive(Resource)]
pub struct Inbox(pub Receiver<InboxMsg>);

/// Godot-side sender (stored in EcsNpcManager)
#[derive(Clone)]
pub struct InboxSender(pub Sender<InboxMsg>);

// ============================================================================
// OUTBOX: Bevy → Godot
// ============================================================================

/// Messages from Bevy to Godot
#[derive(Debug, Clone)]
pub enum OutboxMsg {
    // View lifecycle
    SpawnView {
        slot: usize,
        job: u8,
        x: f32,
        y: f32,
    },
    DespawnView { slot: usize },

    // Transform sync (only for changed entities)
    SyncTransform {
        slot: usize,
        x: f32,
        y: f32,
    },

    // Visual state sync
    SyncHealth { slot: usize, hp: f32, max_hp: f32 },
    SyncColor { slot: usize, r: f32, g: f32, b: f32, a: f32 },
    SyncSprite { slot: usize, col: f32, row: f32 },

    // GPU projectile commands
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

    // Debug visualization
    DebugLine {
        x1: f32, y1: f32,
        x2: f32, y2: f32,
        color: u32,
    },
}

/// Bevy resource containing outbox sender
#[derive(Resource)]
pub struct Outbox(pub Sender<OutboxMsg>);

/// Godot-side receiver (stored in EcsNpcManager)
#[derive(Clone)]
pub struct OutboxReceiver(pub Receiver<OutboxMsg>);

// ============================================================================
// Channel creation
// ============================================================================

/// Both ends of the communication channels
pub struct ChannelPair {
    pub inbox_sender: InboxSender,
    pub inbox_receiver: Inbox,
    pub outbox_sender: Outbox,
    pub outbox_receiver: OutboxReceiver,
}

/// Create unbounded channels for Godot ↔ Bevy communication
pub fn create_channels() -> ChannelPair {
    let (inbox_tx, inbox_rx) = unbounded();
    let (outbox_tx, outbox_rx) = unbounded();

    ChannelPair {
        inbox_sender: InboxSender(inbox_tx),
        inbox_receiver: Inbox(inbox_rx),
        outbox_sender: Outbox(outbox_tx),
        outbox_receiver: OutboxReceiver(outbox_rx),
    }
}
```

## Changes to rust/src/lib.rs

### Add module and imports

```rust
mod channels;
use channels::*;
```

### Store channel endpoints in EcsNpcManager

```rust
#[derive(GodotClass)]
#[class(base=Node)]
pub struct EcsNpcManager {
    // ... existing fields ...

    /// Godot-side channel endpoints
    inbox_sender: Option<InboxSender>,
    outbox_receiver: Option<OutboxReceiver>,
}
```

### Initialize channels in build_app()

```rust
fn build_app() -> (App, InboxSender, OutboxReceiver) {
    let channels = create_channels();

    let app = App::new()
        // Register channel resources
        .insert_resource(channels.inbox_receiver)
        .insert_resource(channels.outbox_sender)
        // ... rest of existing setup
        ;

    (app, channels.inbox_sender, channels.outbox_receiver)
}
```

### Add GDScript API methods

```rust
// Send spawn request via inbox
#[func]
fn send_spawn(&self, x: f32, y: f32, job: i32, faction: i32, opts: Dictionary) -> i32 {
    let slot = allocate_slot();  // Same slot allocation as before

    if let Some(inbox) = &self.inbox_sender {
        inbox.0.send(InboxMsg::SpawnNpc {
            x, y,
            job: job as u8,
            faction: faction as u8,
            town_idx: opts.get("town_idx").map(|v| v.to::<i32>()).unwrap_or(-1),
            home_x: opts.get("home_x").map(|v| v.to::<f32>()).unwrap_or(x),
            home_y: opts.get("home_y").map(|v| v.to::<f32>()).unwrap_or(y),
            work_x: opts.get("work_x").map(|v| v.to::<f32>()).unwrap_or(-1.0),
            work_y: opts.get("work_y").map(|v| v.to::<f32>()).unwrap_or(-1.0),
            starting_post: opts.get("starting_post").map(|v| v.to::<i32>()).unwrap_or(-1),
            attack_type: opts.get("attack_type").map(|v| v.to::<u8>()).unwrap_or(0),
        }).ok();
    }

    slot as i32
}

// Send target via inbox
#[func]
fn send_target(&self, slot: i32, x: f32, y: f32) {
    if let Some(inbox) = &self.inbox_sender {
        inbox.0.send(InboxMsg::SetTarget {
            slot: slot as usize, x, y
        }).ok();
    }
}

// Poll outbox for next message (returns Variant::nil() when empty)
#[func]
fn poll_outbox(&self) -> Variant {
    if let Some(outbox) = &self.outbox_receiver {
        if let Ok(msg) = outbox.0.try_recv() {
            return outbox_msg_to_variant(msg);
        }
    }
    Variant::nil()
}

// Helper to convert OutboxMsg to GDScript Dictionary
fn outbox_msg_to_variant(msg: OutboxMsg) -> Variant {
    let mut dict = Dictionary::new();

    match msg {
        OutboxMsg::SpawnView { slot, job, x, y } => {
            dict.set("type", "SpawnView");
            dict.set("slot", slot as i32);
            dict.set("job", job as i32);
            dict.set("x", x);
            dict.set("y", y);
        }
        OutboxMsg::DespawnView { slot } => {
            dict.set("type", "DespawnView");
            dict.set("slot", slot as i32);
        }
        OutboxMsg::SyncTransform { slot, x, y } => {
            dict.set("type", "SyncTransform");
            dict.set("slot", slot as i32);
            dict.set("x", x);
            dict.set("y", y);
        }
        OutboxMsg::SyncHealth { slot, hp, max_hp } => {
            dict.set("type", "SyncHealth");
            dict.set("slot", slot as i32);
            dict.set("hp", hp);
            dict.set("max_hp", max_hp);
        }
        OutboxMsg::FireProjectile { from_x, from_y, to_x, to_y, speed, damage, faction, shooter, lifetime } => {
            dict.set("type", "FireProjectile");
            dict.set("from_x", from_x);
            dict.set("from_y", from_y);
            dict.set("to_x", to_x);
            dict.set("to_y", to_y);
            dict.set("speed", speed);
            dict.set("damage", damage);
            dict.set("faction", faction);
            dict.set("shooter", shooter as i32);
            dict.set("lifetime", lifetime);
        }
        // ... other message types
        _ => {
            dict.set("type", "Unknown");
        }
    }

    dict.to_variant()
}
```

## Verification

1. `cargo build` passes
2. Existing game still works (channels unused, old statics still work)
3. `send_spawn()` and `poll_outbox()` APIs callable from GDScript

---

# Phase 11.2: Bevy Position Component

## Goal

NPCs get Position component. Bevy owns positions.

## New Components (rust/src/components.rs)

```rust
/// NPC position - Bevy authoritative
/// Replaces reading from GPU_READ_STATE
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct Position(pub Vec2);

/// NPC velocity for movement (computed by GPU or Bevy)
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct Velocity(pub Vec2);

/// Movement target (where NPC is heading)
/// None = stationary, Some = moving toward target
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct MoveTarget(pub Option<Vec2>);

/// Link to GPU buffer slot
/// Same as NpcIndex but semantic distinction
#[derive(Component, Clone, Copy, Debug)]
pub struct GpuSlot(pub usize);
```

## Changes to spawn_npc_system (rust/src/systems/spawn.rs)

```rust
pub fn spawn_npc_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnNpcMsg>,
    // ... existing params ...
    outbox: Res<Outbox>,  // NEW
) {
    for msg in events.read() {
        let idx = msg.slot_idx;
        let job = Job::from_i32(msg.job);

        // Compute initial target
        let initial_target = if job == Job::Farmer && msg.work_x >= 0.0 {
            Some(Vec2::new(msg.work_x, msg.work_y))
        } else {
            None
        };

        // Spawn with Position component (Bevy-owned)
        let mut ec = commands.spawn((
            NpcIndex(idx),
            GpuSlot(idx),
            Position(Vec2::new(msg.x, msg.y)),       // NEW: Bevy owns
            Velocity(Vec2::ZERO),                     // NEW
            MoveTarget(initial_target),               // NEW
            job,
            TownId(msg.town_idx),
            Speed::default(),
            Health::default(),
            Faction::from_i32(msg.faction),
            Home(Vector2::new(msg.home_x, msg.home_y)),
            // ... rest of components
        ));

        // ... existing job-specific component insertion ...

        // NEW: Notify Godot to create visual
        outbox.0.send(OutboxMsg::SpawnView {
            slot: idx,
            job: msg.job as u8,
            x: msg.x,
            y: msg.y,
        }).ok();

        // ... rest of spawn logic (npc_map, pop_stats, meta cache, etc.)
    }
}
```

## Verification

1. `cargo build` passes
2. NPCs spawn with Position component
3. SpawnView messages appear in outbox (verify with test)

---

# Phase 11.3: GPU Upload/Readback Pattern

## Goal

Replace GPU-as-authority with upload/readback accelerator pattern.

## New File: rust/src/gpu_buffers.rs

```rust
//! GPU buffer management - upload Bevy state, readback compute results

use bevy::prelude::*;

/// CPU-side mirrors of GPU buffers
/// Bevy uploads state → GPU computes → Bevy reads back results
#[derive(Resource)]
pub struct GpuBuffers {
    // ========================================================================
    // UPLOAD BUFFERS (Bevy → GPU)
    // ========================================================================

    /// Current positions (from Position component)
    pub positions: Vec<Vec2>,

    /// Movement targets (from MoveTarget component)
    pub targets: Vec<Vec2>,

    /// Current velocities (from Velocity component)
    pub velocities: Vec<Vec2>,

    /// Max speed per NPC (from Speed component)
    pub speeds: Vec<f32>,

    /// Faction for combat queries (from Faction component)
    pub factions: Vec<i32>,

    /// Health for targeting priority (from Health component)
    pub health: Vec<f32>,

    // ========================================================================
    // READBACK BUFFERS (GPU → Bevy)
    // ========================================================================

    /// New positions after physics (applied to Position component)
    pub new_positions: Vec<Vec2>,

    /// Nearest enemy found by GPU (applied to CombatTarget component)
    pub combat_targets: Vec<i32>,

    /// Arrival flags (triggers Arrived component insertion)
    pub arrival_flags: Vec<bool>,

    /// Damage events from projectile hits (applied to Health)
    pub damage_events: Vec<DamageEvent>,

    // ========================================================================
    // PROJECTILE BUFFERS
    // ========================================================================

    pub proj_positions: Vec<Vec2>,
    pub proj_velocities: Vec<Vec2>,
    pub proj_damages: Vec<f32>,
    pub proj_factions: Vec<i32>,
    pub proj_active: Vec<bool>,
    pub proj_lifetimes: Vec<f32>,

    /// Projectile hit events (proj_idx, target_npc_idx, damage)
    pub proj_hits: Vec<(usize, usize, f32)>,

    // ========================================================================
    // COUNTS
    // ========================================================================

    pub npc_count: usize,
    pub proj_count: usize,

    /// Dirty flag - skip upload if nothing changed
    pub needs_upload: bool,
}

#[derive(Clone, Debug)]
pub struct DamageEvent {
    pub target_slot: usize,
    pub damage: f32,
    pub attacker_slot: usize,
}

impl Default for GpuBuffers {
    fn default() -> Self {
        let npc_cap = 10_000;
        let proj_cap = 1_000;

        Self {
            // Upload
            positions: vec![Vec2::ZERO; npc_cap],
            targets: vec![Vec2::ZERO; npc_cap],
            velocities: vec![Vec2::ZERO; npc_cap],
            speeds: vec![100.0; npc_cap],
            factions: vec![0; npc_cap],
            health: vec![100.0; npc_cap],

            // Readback
            new_positions: vec![Vec2::ZERO; npc_cap],
            combat_targets: vec![-1; npc_cap],
            arrival_flags: vec![false; npc_cap],
            damage_events: Vec::with_capacity(1000),

            // Projectiles
            proj_positions: vec![Vec2::ZERO; proj_cap],
            proj_velocities: vec![Vec2::ZERO; proj_cap],
            proj_damages: vec![0.0; proj_cap],
            proj_factions: vec![0; proj_cap],
            proj_active: vec![false; proj_cap],
            proj_lifetimes: vec![0.0; proj_cap],
            proj_hits: Vec::with_capacity(100),

            npc_count: 0,
            proj_count: 0,
            needs_upload: false,
        }
    }
}
```

## New File: rust/src/systems/gpu_sync.rs

```rust
//! GPU synchronization systems - upload state, readback results, sync to Godot

use bevy::prelude::*;
use crate::components::*;
use crate::channels::*;
use crate::gpu_buffers::*;
use crate::resources::*;

// ============================================================================
// UPLOAD: Bevy components → GPU buffers
// ============================================================================

/// Upload Bevy component state to GPU buffers (runs before GPU compute)
pub fn upload_to_gpu_system(
    query: Query<(
        &GpuSlot,
        &Position,
        &MoveTarget,
        &Velocity,
        &Speed,
        &Faction,
        &Health,
    )>,
    mut gpu: ResMut<GpuBuffers>,
) {
    let mut max_slot = 0usize;

    for (slot, pos, target, vel, speed, faction, health) in &query {
        let i = slot.0;
        max_slot = max_slot.max(i + 1);

        gpu.positions[i] = pos.0;
        gpu.targets[i] = target.0.unwrap_or(pos.0);  // Target self if no target
        gpu.velocities[i] = vel.0;
        gpu.speeds[i] = speed.0;
        gpu.factions[i] = faction.as_i32();
        gpu.health[i] = health.0;
    }

    gpu.npc_count = max_slot;
    gpu.needs_upload = true;

    // Clear readback buffers for this frame
    gpu.arrival_flags.fill(false);
    gpu.damage_events.clear();
    gpu.proj_hits.clear();
}

// ============================================================================
// READBACK: GPU results → Bevy components
// ============================================================================

/// Apply GPU compute results back to Bevy components
pub fn readback_from_gpu_system(
    mut query: Query<(&GpuSlot, &mut Position, &mut Velocity)>,
    gpu: Res<GpuBuffers>,
    mut commands: Commands,
    npc_map: Res<NpcEntityMap>,
) {
    // Apply new positions from GPU physics
    for (slot, mut pos, mut vel) in &mut query {
        let i = slot.0;
        if i < gpu.npc_count {
            // Compute velocity from position delta
            vel.0 = gpu.new_positions[i] - pos.0;
            // Update position
            pos.0 = gpu.new_positions[i];
        }
    }

    // Handle arrivals flagged by GPU
    for (i, &arrived) in gpu.arrival_flags.iter().enumerate().take(gpu.npc_count) {
        if arrived {
            if let Some(&entity) = npc_map.0.get(&i) {
                commands.entity(entity)
                    .remove::<MoveTarget>()
                    .insert(Arrived);
            }
        }
    }

    // Handle damage events from GPU projectile hits
    for event in &gpu.damage_events {
        if let Some(&entity) = npc_map.0.get(&event.target_slot) {
            commands.entity(entity).insert(PendingDamage(event.damage));
        }
    }
}

/// Apply combat targets found by GPU spatial query
pub fn apply_combat_targets_system(
    mut query: Query<(&GpuSlot, &mut CombatTarget)>,
    gpu: Res<GpuBuffers>,
    npc_map: Res<NpcEntityMap>,
) {
    for (slot, mut combat) in &mut query {
        let i = slot.0;
        if i < gpu.npc_count {
            let target_idx = gpu.combat_targets[i];
            combat.0 = if target_idx >= 0 {
                npc_map.0.get(&(target_idx as usize)).copied()
            } else {
                None
            };
        }
    }
}

// ============================================================================
// SYNC TO GODOT: Changed<T> → Outbox messages
// ============================================================================

/// Emit transform updates only for NPCs that moved
pub fn emit_transform_sync_system(
    query: Query<(&GpuSlot, &Position), Changed<Position>>,
    outbox: Res<Outbox>,
) {
    for (slot, pos) in &query {
        outbox.0.send(OutboxMsg::SyncTransform {
            slot: slot.0,
            x: pos.0.x,
            y: pos.0.y,
        }).ok();
    }
}

/// Emit health updates only for NPCs that took damage
pub fn emit_health_sync_system(
    query: Query<(&GpuSlot, &Health), Changed<Health>>,
    outbox: Res<Outbox>,
) {
    for (slot, health) in &query {
        outbox.0.send(OutboxMsg::SyncHealth {
            slot: slot.0,
            hp: health.0,
            max_hp: 100.0,  // TODO: from MaxHealth component
        }).ok();
    }
}

/// Emit despawn messages for dead NPCs
pub fn emit_despawn_system(
    query: Query<&GpuSlot, Added<Dead>>,
    outbox: Res<Outbox>,
) {
    for slot in &query {
        outbox.0.send(OutboxMsg::DespawnView { slot: slot.0 }).ok();
    }
}

/// Emit projectile fire requests
pub fn emit_projectile_system(
    query: Query<(&GpuSlot, &Position, &AttackStats, &CombatTarget, &Faction), Added<Attacking>>,
    target_query: Query<&Position>,
    outbox: Res<Outbox>,
) {
    for (slot, pos, stats, combat, faction) in &query {
        if let Some(target_entity) = combat.0 {
            if let Ok(target_pos) = target_query.get(target_entity) {
                outbox.0.send(OutboxMsg::FireProjectile {
                    from_x: pos.0.x,
                    from_y: pos.0.y,
                    to_x: target_pos.0.x,
                    to_y: target_pos.0.y,
                    speed: stats.projectile_speed,
                    damage: stats.damage,
                    faction: faction.as_i32(),
                    shooter: slot.0,
                    lifetime: stats.projectile_lifetime,
                }).ok();
            }
        }
    }
}
```

## Changes to rust/src/lib.rs

### Register resources and systems

```rust
// In build_app()
app
    .init_resource::<GpuBuffers>()

    // System ordering
    .add_systems(Update, (
        drain_inbox_system,
    ).in_set(Step::Drain))

    .add_systems(Update, (
        // AI systems (parallel, use components directly)
        npc_decision_system,
        patrol_system,
        farmer_work_system,
        raider_behavior_system,
    ).in_set(Step::AI))

    .add_systems(Update, (
        upload_to_gpu_system,
    ).in_set(Step::GpuUpload))

    // Note: GPU compute dispatch happens in Godot process() between upload and readback

    .add_systems(Update, (
        readback_from_gpu_system,
        apply_combat_targets_system,
    ).in_set(Step::GpuReadback))

    .add_systems(Update, (
        apply_damage_system,
        death_system,
        death_cleanup_system,
    ).in_set(Step::Apply))

    .add_systems(Update, (
        emit_transform_sync_system,
        emit_health_sync_system,
        emit_despawn_system,
        emit_projectile_system,
    ).in_set(Step::Sync))

    .configure_sets(Update, (
        Step::Drain,
        Step::AI,
        Step::GpuUpload,
        Step::GpuReadback,
        Step::Apply,
        Step::Sync,
    ).chain())
```

## Verification

1. `cargo build` passes
2. Upload system fills GPU buffers from components
3. Readback system applies GPU results to components
4. Changed<Position> triggers SyncTransform in outbox

---

# Phase 11.4: Drain Inbox System

## Goal

Process all incoming messages from Godot via inbox channel.

## New System: rust/src/systems/inbox.rs

```rust
//! Inbox drain system - process Godot → Bevy messages

use bevy::prelude::*;
use crate::channels::*;
use crate::components::*;
use crate::resources::*;

/// Component bundle for newly spawned NPCs
#[derive(Bundle)]
pub struct NpcBundle {
    pub index: NpcIndex,
    pub slot: GpuSlot,
    pub position: Position,
    pub velocity: Velocity,
    pub target: MoveTarget,
    pub job: Job,
    pub faction: Faction,
    pub health: Health,
    pub town: TownId,
    pub home: Home,
    pub speed: Speed,
}

/// Drain inbox channel and apply messages to ECS
pub fn drain_inbox_system(
    mut commands: Commands,
    inbox: Res<Inbox>,
    outbox: Res<Outbox>,
    mut game_time: ResMut<GameTime>,
    mut selected: ResMut<SelectedNpc>,
    npc_map: Res<NpcEntityMap>,
    mut slot_allocator: ResMut<SlotAllocator>,
) {
    while let Ok(msg) = inbox.0.try_recv() {
        match msg {
            InboxMsg::SpawnNpc {
                x, y, job, faction, town_idx,
                home_x, home_y, work_x, work_y,
                starting_post, attack_type,
            } => {
                // Allocate slot
                let slot = slot_allocator.allocate();

                // Compute initial target
                let initial_target = if Job::from_u8(job) == Job::Farmer && work_x >= 0.0 {
                    Some(Vec2::new(work_x, work_y))
                } else {
                    None
                };

                // Spawn entity with all components
                let mut ec = commands.spawn(NpcBundle {
                    index: NpcIndex(slot),
                    slot: GpuSlot(slot),
                    position: Position(Vec2::new(x, y)),
                    velocity: Velocity(Vec2::ZERO),
                    target: MoveTarget(initial_target),
                    job: Job::from_u8(job),
                    faction: Faction::from_u8(faction),
                    health: Health(100.0),
                    town: TownId(town_idx),
                    home: Home(Vector2::new(home_x, home_y)),
                    speed: Speed(100.0),
                });

                // Add job-specific components
                match Job::from_u8(job) {
                    Job::Farmer => {
                        ec.insert(Farmer);
                        ec.insert(Energy::default());
                        if work_x >= 0.0 {
                            ec.insert(WorkPosition(Vector2::new(work_x, work_y)));
                            ec.insert(GoingToWork);
                        }
                    }
                    Job::Guard => {
                        ec.insert(Guard);
                        ec.insert(Energy::default());
                        ec.insert((AttackStats::melee(), AttackTimer(0.0)));
                        if starting_post >= 0 {
                            // Build patrol route...
                        }
                    }
                    Job::Raider => {
                        ec.insert(Energy::default());
                        ec.insert((AttackStats::melee(), AttackTimer(0.0)));
                        ec.insert(Stealer);
                        ec.insert(FleeThreshold { pct: 0.50 });
                        ec.insert(LeashRange { distance: 400.0 });
                        ec.insert(WoundedThreshold { pct: 0.25 });
                    }
                    Job::Fighter => {
                        let stats = if attack_type == 1 {
                            AttackStats::ranged()
                        } else {
                            AttackStats::melee()
                        };
                        ec.insert((stats, AttackTimer(0.0)));
                    }
                }

                // Tell Godot to create visual
                outbox.0.send(OutboxMsg::SpawnView {
                    slot,
                    job,
                    x,
                    y,
                }).ok();
            }

            InboxMsg::SetTarget { slot, x, y } => {
                if let Some(&entity) = npc_map.0.get(&slot) {
                    commands.entity(entity)
                        .insert(MoveTarget(Some(Vec2::new(x, y))));
                }
            }

            InboxMsg::ApplyDamage { slot, amount } => {
                if let Some(&entity) = npc_map.0.get(&slot) {
                    commands.entity(entity).insert(PendingDamage(amount));
                }
            }

            InboxMsg::SelectNpc { slot } => {
                selected.0 = slot;
            }

            InboxMsg::PlayerClick { x, y } => {
                // Could do spatial query here for click selection
            }

            InboxMsg::Reset => {
                // Queue full reset
                commands.insert_resource(ResetFlag(true));
            }

            InboxMsg::SetPaused(paused) => {
                game_time.paused = paused;
            }

            InboxMsg::SetTimeScale(scale) => {
                game_time.time_scale = scale;
            }
        }
    }
}

/// Slot allocator resource (replaces NPC_SLOT_COUNTER + FREE_SLOTS statics)
#[derive(Resource)]
pub struct SlotAllocator {
    next_slot: usize,
    free_slots: Vec<usize>,
}

impl Default for SlotAllocator {
    fn default() -> Self {
        Self {
            next_slot: 0,
            free_slots: Vec::with_capacity(1000),
        }
    }
}

impl SlotAllocator {
    pub fn allocate(&mut self) -> usize {
        self.free_slots.pop().unwrap_or_else(|| {
            let slot = self.next_slot;
            self.next_slot += 1;
            slot
        })
    }

    pub fn free(&mut self, slot: usize) {
        self.free_slots.push(slot);
    }
}
```

## Verification

1. `cargo build` passes
2. Inbox messages create entities correctly
3. Replaces drain_spawn_queue, drain_target_queue functionality

---

# Phase 11.5: Godot Outbox Drain

## Goal

GDScript drains outbox and applies visual updates.

## GDScript: ecs_manager.gd (or main.gd)

```gdscript
extends Node

var ecs: EcsNpcManager

func _process(_delta):
    _drain_outbox()

func _drain_outbox():
    while true:
        var msg = ecs.poll_outbox()
        if msg == null or msg.is_empty():
            break

        var msg_type = msg.get("type", "")

        match msg_type:
            "SpawnView":
                _on_spawn_view(msg.slot, msg.job, msg.x, msg.y)

            "DespawnView":
                _on_despawn_view(msg.slot)

            "SyncTransform":
                _on_sync_transform(msg.slot, msg.x, msg.y)

            "SyncHealth":
                _on_sync_health(msg.slot, msg.hp, msg.max_hp)

            "FireProjectile":
                _on_fire_projectile(msg)

func _on_spawn_view(slot: int, job: int, x: float, y: float):
    # Set GPU buffer slot as active
    ecs.gpu_set_position(slot, x, y)
    ecs.gpu_set_active(slot, true)
    ecs.gpu_set_sprite(slot, job)

func _on_despawn_view(slot: int):
    # Hide in GPU buffer
    ecs.gpu_set_active(slot, false)
    ecs.gpu_set_position(slot, -9999, -9999)

func _on_sync_transform(slot: int, x: float, y: float):
    # Update GPU buffer position
    ecs.gpu_set_position(slot, x, y)

func _on_sync_health(slot: int, hp: float, max_hp: float):
    # Update GPU buffer health (for HP bar rendering)
    ecs.gpu_set_health(slot, hp / max_hp)

func _on_fire_projectile(msg: Dictionary):
    # Spawn GPU projectile
    ecs.fire_projectile(
        msg.from_x, msg.from_y,
        msg.to_x, msg.to_y,
        msg.speed, msg.damage,
        msg.faction, msg.lifetime
    )

# Public API for GDScript to send to Bevy
func spawn_npc(x: float, y: float, job: int, faction: int, opts: Dictionary = {}) -> int:
    return ecs.send_spawn(x, y, job, faction, opts)

func set_target(slot: int, x: float, y: float):
    ecs.send_target(slot, x, y)

func set_paused(paused: bool):
    ecs.send_paused(paused)

func set_time_scale(scale: float):
    ecs.send_time_scale(scale)
```

## Verification

1. Outbox messages received in GDScript
2. GPU buffers update from SyncTransform
3. Visuals appear/disappear correctly

---

# Phase 11.6: System Schedule

## Goal

Correct system ordering with explicit phases.

## Step Enum (rust/src/lib.rs)

```rust
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    /// Drain inbox, apply player input
    Drain,

    /// AI decisions (parallel, no GPU)
    AI,

    /// Upload component state to GPU buffers
    GpuUpload,

    /// Readback GPU results to components
    GpuReadback,

    /// Apply damage, deaths, arrivals
    Apply,

    /// Sync changed components to Godot
    Sync,
}
```

## Full Schedule

```rust
app
    // Drain inbox
    .add_systems(Update, drain_inbox_system.in_set(Step::Drain))

    // AI (parallel)
    .add_systems(Update, (
        npc_decision_system,
        patrol_system,
        farmer_work_system,
        guard_behavior_system,
        raider_behavior_system,
        tired_system,
        resume_patrol_system,
        resume_work_system,
    ).in_set(Step::AI))

    // GPU upload
    .add_systems(Update, upload_to_gpu_system.in_set(Step::GpuUpload))

    // GPU readback
    .add_systems(Update, (
        readback_from_gpu_system,
        apply_combat_targets_system,
    ).in_set(Step::GpuReadback))

    // Apply results
    .add_systems(Update, (
        apply_damage_system,
        death_system,
        death_cleanup_system,
        arrival_handler_system,
    ).in_set(Step::Apply))

    // Sync to Godot
    .add_systems(Update, (
        emit_transform_sync_system,
        emit_health_sync_system,
        emit_despawn_system,
        emit_projectile_system,
    ).in_set(Step::Sync))

    // Ordering
    .configure_sets(Update, (
        Step::Drain,
        Step::AI,
        Step::GpuUpload,
        Step::GpuReadback,
        Step::Apply,
        Step::Sync,
    ).chain())
```

---

# Phase 11.7: Delete All Statics

## Goal

Remove all 14 remaining mutexes.

## Statics to Delete

| Static | Replaced By |
|--------|-------------|
| SPAWN_QUEUE | InboxMsg::SpawnNpc |
| TARGET_QUEUE | InboxMsg::SetTarget |
| ARRIVAL_QUEUE | GPU readback → Arrived component |
| DAMAGE_QUEUE | InboxMsg::ApplyDamage or GPU readback |
| GPU_UPDATE_QUEUE | upload_to_gpu_system |
| GPU_READ_STATE | readback_from_gpu_system |
| GPU_DISPATCH_COUNT | GpuBuffers.npc_count |
| PROJECTILE_FIRE_QUEUE | OutboxMsg::FireProjectile |
| RESET_BEVY | InboxMsg::Reset |
| NPC_SLOT_COUNTER | SlotAllocator Resource |
| FREE_SLOTS | SlotAllocator Resource |
| FREE_PROJ_SLOTS | ProjSlotAllocator Resource |
| FOOD_STORAGE | FoodStorage Resource (stays) |
| GAME_CONFIG_STAGING | InboxMsg or direct Resource init |

## Files to Modify

1. **rust/src/messages.rs** - Delete all static declarations
2. **rust/src/lib.rs** - Remove all .lock() calls, use resources
3. **rust/src/systems/*.rs** - Use Res/ResMut instead of statics

## Verification

1. `cargo build` passes with no static mutex usage
2. `grep "\.lock()"` returns no results in systems
3. Game fully functional

---

# Performance Comparison

| Metric | Before (Static Mutex) | After (Channels + Resources) |
|--------|----------------------|------------------------------|
| Position reads/frame | 10k mutex lock + clone | Direct Query<&Position> |
| Position syncs/frame | 10k (all) | ~500 (Changed only) |
| Mutex contentions | High (14 statics, hot paths) | None (lock-free channels) |
| Arrival detection | GPU→Queue→Bevy round-trip | Bevy distance check (or GPU flag) |
| System parallelism | Blocked by shared locks | Full Bevy scheduler parallelism |
| Code clarity | Scattered .lock() calls | Clear data flow via channels |

---

# Migration Order

| Phase | Risk | Duration | Dependencies |
|-------|------|----------|--------------|
| 11.1 Channels | Low | 1-2 hours | None |
| 11.2 Position | Low | 1 hour | 11.1 |
| 11.3 Upload/Readback | Medium | 3-4 hours | 11.2 |
| 11.4 Drain Inbox | Medium | 2-3 hours | 11.1, 11.2 |
| 11.5 Godot Drain | Medium | 2-3 hours | 11.4 |
| 11.6 Schedule | Low | 1 hour | 11.3, 11.4 |
| 11.7 Delete Statics | High | 2-3 hours | All above |

**Total: ~15-20 hours of focused work**

Each phase should be a separate commit with passing tests.
