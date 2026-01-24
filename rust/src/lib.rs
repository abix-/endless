// Proof of concept: 5000 NPCs with separation forces + MultiMesh rendering
// Tests whether Rust/ECS can hit 140fps with this workload
//
// NOTE: This is a POC - exact API signatures may need adjustment
// based on the versions of bevy_ecs/godot-rust available at build time.

use bevy_app::{App, Update};
use bevy_ecs::prelude::*;
use godot::prelude::*;
use godot::classes::{INode2D, RenderingServer};

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

#[derive(GodotClass)]
#[class(base=Node2D)]
struct NpcBenchmark {
    base: Base<Node2D>,
    app: Option<App>,
    multimesh_rid: Rid,
    frame_count: u64,
    fps_timer: f64,
}

#[godot_api]
impl INode2D for NpcBenchmark {
    fn init(base: Base<Node2D>) -> Self {
        Self {
            base,
            app: None,
            multimesh_rid: Rid::Invalid,
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

        // Create MultiMesh via RenderingServer (no node overhead)
        let mut rs = RenderingServer::singleton();
        let mm_rid = rs.multimesh_create();
        rs.multimesh_set_mesh(mm_rid, rs.get_test_quad());
        rs.multimesh_allocate_data(mm_rid, NPC_COUNT as i32,
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
            true,  // use_colors
            false, // use_custom_data
        );

        // Attach to canvas
        let canvas_item = self.base().get_canvas_item();
        rs.canvas_item_add_multimesh(canvas_item, mm_rid, Rid::Invalid);

        self.multimesh_rid = mm_rid;
        godot_print!("[Bevy POC] Spawned {} NPCs", NPC_COUNT);
    }

    fn process(&mut self, delta: f64) {
        // Tick Bevy ECS (runs grid + separation + navigation)
        if let Some(ref mut app) = self.app {
            app.update();

            // Push positions to MultiMesh
            let mut rs = RenderingServer::singleton();
            let world = app.world_mut();
            let mut query = world.query::<&Npc>();

            for npc in query.iter(world) {
                let t = Transform2D::new(
                    Vector2::new(1.0, 0.0),
                    Vector2::new(0.0, 1.0),
                    Vector2::new(npc.x, npc.y),
                );
                rs.multimesh_instance_set_transform_2d(
                    self.multimesh_rid,
                    npc.index as i32,
                    t,
                );
                rs.multimesh_instance_set_color(
                    self.multimesh_rid,
                    npc.index as i32,
                    Color::from_rgba(0.2, 0.8, 0.2, 1.0),
                );
            }
        }

        // FPS counter
        self.frame_count += 1;
        self.fps_timer += delta;
        if self.fps_timer >= 1.0 {
            let fps = self.frame_count as f64 / self.fps_timer;
            godot_print!("[Bevy POC] FPS: {:.1} ({} NPCs)", fps, NPC_COUNT);
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
