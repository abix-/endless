// Proof of concept: 5000 NPCs with separation forces + MultiMesh rendering
// Tests whether Rust/ECS can hit 140fps with this workload
//
// NOTE: This is a POC - exact API signatures may need adjustment
// based on the versions of bevy_ecs/godot-rust available at build time.

use bevy_app::{App, Update};
use bevy_ecs::prelude::*;
use godot::prelude::*;
use godot::classes::{INode2D, QuadMesh, MultiMesh, MultiMeshInstance2D, Label};

// ============================================================
// CONSTANTS
// ============================================================

const NPC_COUNT: usize = 5000;
const WORLD_SIZE: f32 = 8000.0;
const CELL_SIZE: f32 = 64.0;
const GRID_WIDTH: usize = 128;  // WORLD_SIZE / CELL_SIZE
const GRID_HEIGHT: usize = 128;
const MAX_PER_CELL: usize = 48;
const GRID_CELLS: usize = GRID_WIDTH * GRID_HEIGHT;
const MIN_SEPARATION: f32 = 16.0;
const MIN_SEP_SQ: f32 = MIN_SEPARATION * MIN_SEPARATION;
const DAMPING: f32 = 0.4;
const DELTA: f32 = 1.0 / 60.0;

// ============================================================
// BEVY ECS COMPONENTS
// ============================================================

#[derive(Component)]
struct Npc {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    sep_x: f32,
    sep_y: f32,
    index: usize,  // stable index for grid lookups
}

// ============================================================
// SHARED RESOURCES
// ============================================================

#[derive(Resource)]
struct SpatialGrid {
    counts: Vec<u16>,
    data: Vec<u32>,
}

impl SpatialGrid {
    fn new() -> Self {
        Self {
            counts: vec![0u16; GRID_CELLS],
            data: vec![0u32; GRID_CELLS * MAX_PER_CELL],
        }
    }

    fn clear(&mut self) {
        self.counts.fill(0);
    }

    fn insert(&mut self, x: f32, y: f32, npc_idx: u32) {
        let cell_idx = Self::cell_index(x, y);
        if cell_idx >= GRID_CELLS { return; }
        let count = self.counts[cell_idx] as usize;
        if count >= MAX_PER_CELL { return; }
        self.data[cell_idx * MAX_PER_CELL + count] = npc_idx;
        self.counts[cell_idx] = (count + 1) as u16;
    }

    fn cell_index(x: f32, y: f32) -> usize {
        let cx = (x / CELL_SIZE).clamp(0.0, (GRID_WIDTH - 1) as f32) as usize;
        let cy = (y / CELL_SIZE).clamp(0.0, (GRID_HEIGHT - 1) as f32) as usize;
        cy * GRID_WIDTH + cx
    }
}

/// Flat position buffer for O(1) lookups during separation
#[derive(Resource)]
struct PositionBuffer {
    x: Vec<f32>,
    y: Vec<f32>,
}

impl PositionBuffer {
    fn new() -> Self {
        Self {
            x: vec![0.0; NPC_COUNT],
            y: vec![0.0; NPC_COUNT],
        }
    }
}

// ============================================================
// BEVY SYSTEMS
// ============================================================

/// Copy positions to flat buffer + rebuild spatial grid
fn grid_rebuild_system(
    query: Query<&Npc>,
    mut grid: ResMut<SpatialGrid>,
    mut pos_buf: ResMut<PositionBuffer>,
) {
    grid.clear();
    for npc in query.iter() {
        pos_buf.x[npc.index] = npc.x;
        pos_buf.y[npc.index] = npc.y;
        grid.insert(npc.x, npc.y, npc.index as u32);
    }
}

/// Compute separation forces from neighbors
fn separation_system(
    mut query: Query<&mut Npc>,
    grid: Res<SpatialGrid>,
    pos_buf: Res<PositionBuffer>,
) {
    for mut npc in query.iter_mut() {
        let mut push_x: f32 = 0.0;
        let mut push_y: f32 = 0.0;

        let cx = (npc.x / CELL_SIZE) as i32;
        let cy = (npc.y / CELL_SIZE) as i32;

        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let nx = cx + dx;
                let ny = cy + dy;
                if nx < 0 || ny < 0 || nx >= GRID_WIDTH as i32 || ny >= GRID_HEIGHT as i32 {
                    continue;
                }
                let cell_idx = (ny as usize) * GRID_WIDTH + (nx as usize);
                let count = grid.counts[cell_idx] as usize;
                let base = cell_idx * MAX_PER_CELL;

                for k in 0..count {
                    let other_idx = grid.data[base + k] as usize;
                    if other_idx == npc.index { continue; }

                    let ddx = npc.x - pos_buf.x[other_idx];
                    let ddy = npc.y - pos_buf.y[other_idx];
                    let dist_sq = ddx * ddx + ddy * ddy;

                    if dist_sq < MIN_SEP_SQ && dist_sq > 0.01 {
                        let dist = dist_sq.sqrt();
                        let overlap = MIN_SEPARATION - dist;
                        let inv_dist = 1.0 / dist;
                        push_x += ddx * inv_dist * overlap * 0.5;
                        push_y += ddy * inv_dist * overlap * 0.5;
                    }
                }
            }
        }

        // Velocity damping
        npc.sep_x += (push_x - npc.sep_x) * DAMPING;
        npc.sep_y += (push_y - npc.sep_y) * DAMPING;
    }
}

/// Apply velocity + separation, wrap world edges
fn navigation_system(mut query: Query<&mut Npc>) {
    for mut npc in query.iter_mut() {
        npc.x += (npc.vx + npc.sep_x) * DELTA;
        npc.y += (npc.vy + npc.sep_y) * DELTA;

        // Wrap
        if npc.x < 0.0 { npc.x += WORLD_SIZE; }
        if npc.x > WORLD_SIZE { npc.x -= WORLD_SIZE; }
        if npc.y < 0.0 { npc.y += WORLD_SIZE; }
        if npc.y > WORLD_SIZE { npc.y -= WORLD_SIZE; }
    }
}

// ============================================================
// GODOT NODE: NPC BENCHMARK
// ============================================================

// Buffer stride: Transform2D (8 floats) + Color (4 floats) = 12 floats per instance
const FLOATS_PER_INSTANCE: usize = 12;

#[derive(GodotClass)]
#[class(base=Node2D)]
struct NpcBenchmark {
    base: Base<Node2D>,
    app: Option<App>,
    multimesh: Option<Gd<MultiMesh>>,
    buffer: Vec<f32>,  // Pre-allocated transform+color buffer
    frame_count: u64,
    fps_timer: f64,
}

#[godot_api]
impl INode2D for NpcBenchmark {
    fn init(base: Base<Node2D>) -> Self {
        // Pre-allocate buffer with identity transforms and green color
        let mut buffer = vec![0.0f32; NPC_COUNT * FLOATS_PER_INSTANCE];
        for i in 0..NPC_COUNT {
            let base_idx = i * FLOATS_PER_INSTANCE;
            // Transform2D identity: [1,0,0,x, 0,1,0,y] format
            buffer[base_idx + 0] = 1.0;  // a.x
            buffer[base_idx + 1] = 0.0;  // b.x
            buffer[base_idx + 2] = 0.0;  // padding
            buffer[base_idx + 3] = 0.0;  // origin.x (will be updated)
            buffer[base_idx + 4] = 0.0;  // a.y
            buffer[base_idx + 5] = 1.0;  // b.y
            buffer[base_idx + 6] = 0.0;  // padding
            buffer[base_idx + 7] = 0.0;  // origin.y (will be updated)
            // Color: green
            buffer[base_idx + 8] = 0.2;   // r
            buffer[base_idx + 9] = 0.8;   // g
            buffer[base_idx + 10] = 0.2;  // b
            buffer[base_idx + 11] = 1.0;  // a
        }
        Self {
            base,
            app: None,
            multimesh: None,
            buffer,
            frame_count: 0,
            fps_timer: 0.0,
        }
    }

    fn ready(&mut self) {
        // Build Bevy app
        let mut app = App::new();
        app.insert_resource(SpatialGrid::new());
        app.insert_resource(PositionBuffer::new());
        app.add_systems(Update, (
            grid_rebuild_system,
            separation_system.after(grid_rebuild_system),
            navigation_system.after(separation_system),
        ));

        // Spawn NPC entities
        let world = app.world_mut();
        let mut seed: u64 = 12345;
        for i in 0..NPC_COUNT {
            // Simple xorshift RNG (no external dep needed for POC)
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let x = (seed as f32 / u64::MAX as f32) * WORLD_SIZE;
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let y = (seed as f32 / u64::MAX as f32) * WORLD_SIZE;
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let speed = 20.0 + (seed as f32 / u64::MAX as f32) * 60.0;
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let angle = (seed as f32 / u64::MAX as f32) * std::f32::consts::TAU;

            world.spawn(Npc {
                x, y,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed,
                sep_x: 0.0,
                sep_y: 0.0,
                index: i,
            });
        }

        self.app = Some(app);

        // Create MultiMesh with proper node (like GDScript version)
        let mut multimesh = MultiMesh::new_gd();
        multimesh.set_transform_format(godot::classes::multi_mesh::TransformFormat::TRANSFORM_2D);
        multimesh.set_use_colors(true);
        multimesh.set_instance_count(NPC_COUNT as i32);

        let mut mesh = QuadMesh::new_gd();
        mesh.set_size(Vector2::new(16.0, 16.0));
        multimesh.set_mesh(&mesh);

        // Create MultiMeshInstance2D and add as child
        let mut mm_instance = MultiMeshInstance2D::new_alloc();
        mm_instance.set_multimesh(&multimesh);
        self.base_mut().add_child(&mm_instance);

        self.multimesh = Some(multimesh);
        godot_print!("[Bevy POC] Spawned {} NPCs", NPC_COUNT);
    }

    fn process(&mut self, delta: f64) {
        // Tick Bevy ECS (runs grid + separation + navigation)
        let multimesh = match self.multimesh.as_mut() {
            Some(mm) => mm,
            None => return,
        };

        if let Some(ref mut app) = self.app {
            app.update();

            // Read positions from PositionBuffer (already updated by grid_rebuild_system)
            let pos_buf = app.world().resource::<PositionBuffer>();

            // Update only the position floats in the buffer (indices 3 and 7)
            for i in 0..NPC_COUNT {
                let base_idx = i * FLOATS_PER_INSTANCE;
                self.buffer[base_idx + 3] = pos_buf.x[i];  // origin.x
                self.buffer[base_idx + 7] = pos_buf.y[i];  // origin.y
            }

            // Single bulk upload
            let packed = PackedFloat32Array::from(self.buffer.as_slice());
            multimesh.set_buffer(&packed);
        }

        // FPS counter
        self.frame_count += 1;
        self.fps_timer += delta;
        if self.fps_timer >= 1.0 {
            let fps = self.frame_count as f64 / self.fps_timer;
            if let Some(mut label) = self.base().try_get_node_as::<Label>("../UI/FPSLabel") {
                label.set_text(&format!("FPS: {:.0} ({} NPCs)", fps, NPC_COUNT));
            }
            self.frame_count = 0;
            self.fps_timer = 0.0;
        }
    }
}

// ============================================================
// GDEXTENSION ENTRY
// ============================================================

struct EndlessEcs;

#[gdextension]
unsafe impl ExtensionLibrary for EndlessEcs {}
