// Endless ECS - Chunk 1: Bevy owns NPCs, renders to MultiMesh
// Architecture: GDScript spawns → Bevy entities → MultiMesh rendering

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;
use godot_bevy::prelude::*;
use godot::classes::{RenderingServer, QuadMesh, INode2D};
use bevy::prelude::{App, Update};

// ============================================================
// ECS COMPONENTS
// ============================================================

/// NPC position in world space
#[derive(Component, Default, Clone, Copy)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

/// NPC job type (determines sprite color)
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Job {
    Farmer,
    Guard,
    Raider,
}

impl Job {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Job::Guard,
            2 => Job::Raider,
            _ => Job::Farmer,
        }
    }

    pub fn color(&self) -> (f32, f32, f32) {
        match self {
            Job::Farmer => (0.2, 0.8, 0.2),  // Green
            Job::Guard => (0.2, 0.4, 0.9),   // Blue
            Job::Raider => (0.9, 0.2, 0.2),  // Red
        }
    }
}

// ============================================================
// ECS MESSAGES (Bevy 0.17+ uses Message for buffered events)
// ============================================================

/// Message: spawn an NPC (sent from GDScript via bridge)
#[derive(Message, Clone)]
pub struct SpawnNpcMsg {
    pub x: f32,
    pub y: f32,
    pub job: i32,
}

// ============================================================
// ECS RESOURCES
// ============================================================

/// Holds the MultiMesh RID for rendering
#[derive(Resource)]
pub struct MultiMeshResource {
    pub rid: godot::prelude::Rid,
    pub canvas_item: godot::prelude::Rid,
    pub max_instances: usize,
}

impl Default for MultiMeshResource {
    fn default() -> Self {
        Self {
            rid: godot::prelude::Rid::Invalid,
            canvas_item: godot::prelude::Rid::Invalid,
            max_instances: 0,
        }
    }
}

/// NPC count for tracking
#[derive(Resource, Default)]
pub struct NpcCount(pub usize);

// ============================================================
// ECS SYSTEMS
// ============================================================

/// Processes spawn events, creates entities (can run parallel)
fn spawn_npc_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnNpcMsg>,
    mut count: ResMut<NpcCount>,
) {
    for event in events.read() {
        commands.spawn((
            Position { x: event.x, y: event.y },
            Job::from_i32(event.job),
        ));
        count.0 += 1;
    }
    // Update static for GDScript to read
    if let Ok(mut npc_count) = NPC_COUNT.lock() {
        *npc_count = count.0;
    }
}

/// Renders all NPCs to MultiMesh (must run on main thread)
fn render_npc_system(
    query: Query<(&Position, &Job)>,
    _godot: GodotAccess,  // Forces main thread execution
) {
    // Get MultiMesh RID and max count from statics
    let (rid, max_count) = {
        let rid_guard = MULTIMESH_RID.lock().unwrap();
        let max_guard = MULTIMESH_MAX.lock().unwrap();
        match *rid_guard {
            Some(rid) => (rid, *max_guard),
            None => return,
        }
    };

    if max_count == 0 {
        return;
    }

    const FLOATS_PER_INSTANCE: usize = 12;

    // Always write full buffer (required by Godot)
    let mut buffer = vec![0.0f32; max_count * FLOATS_PER_INSTANCE];

    // Initialize all with identity transform but alpha=0 (invisible)
    for i in 0..max_count {
        let base = i * FLOATS_PER_INSTANCE;
        buffer[base + 0] = 1.0;  // scale x
        buffer[base + 5] = 1.0;  // scale y
        // alpha stays 0 = invisible
    }

    // Write actual NPC data
    for (i, (pos, job)) in query.iter().enumerate() {
        let base = i * FLOATS_PER_INSTANCE;
        let (r, g, b) = job.color();

        buffer[base + 3] = pos.x;    // position x
        buffer[base + 7] = pos.y;    // position y
        buffer[base + 8] = r;
        buffer[base + 9] = g;
        buffer[base + 10] = b;
        buffer[base + 11] = 1.0;     // alpha = visible
    }

    let packed = PackedFloat32Array::from(buffer.as_slice());
    RenderingServer::singleton().multimesh_set_buffer(rid, &packed);
}

// ============================================================
// BEVY APP
// ============================================================

#[bevy_app]
fn build_app(app: &mut App) {
    app.add_message::<SpawnNpcMsg>()
       .init_resource::<MultiMeshResource>()
       .init_resource::<NpcCount>()
       .add_systems(Update, (drain_spawn_queue, spawn_npc_system, render_npc_system).chain());

    godot_print!("[ECS] Chunk 1: Bevy NPC rendering initialized");
}

// ============================================================
// GDSCRIPT BRIDGE (EcsNpcManager)
// ============================================================

use std::sync::Mutex;

/// Queue of spawn events (GDScript → Bevy)
static SPAWN_QUEUE: Mutex<Vec<SpawnNpcMsg>> = Mutex::new(Vec::new());

/// Shared MultiMesh RID and max count (set by EcsNpcManager, read by render system)
static MULTIMESH_RID: Mutex<Option<godot::prelude::Rid>> = Mutex::new(None);
static MULTIMESH_MAX: Mutex<usize> = Mutex::new(0);

/// NPC count (updated by spawn system, read by GDScript)
static NPC_COUNT: Mutex<usize> = Mutex::new(0);

/// Queues a spawn event (called from GDScript)
fn queue_spawn(x: f32, y: f32, job: i32) {
    if let Ok(mut queue) = SPAWN_QUEUE.lock() {
        queue.push(SpawnNpcMsg { x, y, job });
    }
}

/// Drains spawn queue into Bevy messages (called from system)
fn drain_spawn_queue(mut messages: MessageWriter<SpawnNpcMsg>) {
    if let Ok(mut queue) = SPAWN_QUEUE.lock() {
        for msg in queue.drain(..) {
            messages.write(msg);
        }
    }
}

#[derive(GodotClass)]
#[class(base=Node2D)]
pub struct EcsNpcManager {
    base: Base<Node2D>,
    initialized: bool,
}

#[godot_api]
impl INode2D for EcsNpcManager {
    fn init(base: Base<Node2D>) -> Self {
        Self {
            base,
            initialized: false,
        }
    }

    fn ready(&mut self) {
        self.setup_multimesh(10000);  // Max 10K NPCs
        godot_print!("[EcsNpcManager] Ready - MultiMesh created");
    }
}

#[godot_api]
impl EcsNpcManager {
    /// Initialize MultiMesh for rendering
    #[func]
    fn setup_multimesh(&mut self, max_count: i32) {
        let mut rs = RenderingServer::singleton();

        // Create MultiMesh
        let multimesh_rid = rs.multimesh_create();

        // Create quad mesh
        let mut mesh = QuadMesh::new_gd();
        mesh.set_size(Vector2::new(16.0, 16.0));
        let mesh_rid = mesh.get_rid();
        rs.multimesh_set_mesh(multimesh_rid, mesh_rid);

        // Allocate instance data (use_indirect required for buffer updates)
        rs.multimesh_allocate_data_ex(
            multimesh_rid,
            max_count,
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
        ).color_format(true).use_indirect(true).done();

        // Initialize ALL instances with identity transform + alpha (required before canvas attach)
        const FLOATS_PER_INSTANCE: usize = 12;
        let count = max_count as usize;
        let mut init_buffer = vec![0.0f32; count * FLOATS_PER_INSTANCE];
        for i in 0..count {
            let base = i * FLOATS_PER_INSTANCE;
            init_buffer[base + 0] = 1.0;  // scale x
            init_buffer[base + 5] = 1.0;  // scale y
            init_buffer[base + 11] = 1.0; // alpha
        }
        let packed = PackedFloat32Array::from(init_buffer.as_slice());
        rs.multimesh_set_buffer(multimesh_rid, &packed);

        // Attach to scene AFTER buffer init
        let canvas_item = rs.canvas_item_create();
        let parent_canvas = self.base().get_canvas_item();
        rs.canvas_item_set_parent(canvas_item, parent_canvas);
        rs.canvas_item_add_multimesh(canvas_item, multimesh_rid);

        // Store RID and max count in statics for render system
        {
            let mut guard = MULTIMESH_RID.lock().unwrap();
            *guard = Some(multimesh_rid);
        }
        {
            let mut guard = MULTIMESH_MAX.lock().unwrap();
            *guard = max_count as usize;
        }

        std::mem::forget(mesh);

        self.initialized = true;
        godot_print!("[EcsNpcManager] MultiMesh RID: {} allocated for {} instances", multimesh_rid.to_u64(), max_count);
    }

    /// Spawn an NPC at position with job type
    /// job: 0=Farmer, 1=Guard, 2=Raider
    #[func]
    fn spawn_npc(&self, x: f32, y: f32, job: i32) {
        queue_spawn(x, y, job);
    }

    /// Spawn multiple NPCs (batch)
    #[func]
    fn spawn_npcs(&self, positions: PackedVector2Array, jobs: PackedInt32Array) {
        let count = positions.len().min(jobs.len());
        for i in 0..count {
            if let (Some(pos), Some(job)) = (positions.get(i), jobs.get(i)) {
                queue_spawn(pos.x, pos.y, job);
            }
        }
    }

    /// Get current NPC count
    #[func]
    fn get_npc_count(&self) -> i32 {
        if let Ok(count) = NPC_COUNT.lock() {
            *count as i32
        } else {
            0
        }
    }
}
