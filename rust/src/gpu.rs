//! GPU Compute Module - wgpu-based NPC physics via Bevy's render graph.
//!
//! Follows Bevy 0.18's compute_shader_game_of_life.rs pattern.
//! Three-phase dispatch per frame: clear grid → insert NPCs → main logic.
//!
//! Data flow:
//! - Main world: Systems write GpuUpdateMsg → GPU_UPDATE_QUEUE
//! - Main world: populate_buffer_writes drains queue → NpcBufferWrites
//! - Extract: NpcBufferWrites copied to render world
//! - Render: write_npc_buffers uploads data to GPU

use bevy::{
    prelude::*,
    render::{
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_graph::{self, RenderGraph, RenderLabel},
        render_resource::{
            binding_types::{storage_buffer, storage_buffer_read_only, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        Render, RenderApp, RenderStartup, RenderSystems,
    },
    shader::PipelineCacheError,
};
use std::borrow::Cow;

use crate::components::{NpcIndex, Faction, Job, Healing, Activity, EquippedWeapon, EquippedHelmet, EquippedArmor, Dead};
use crate::constants::{HEAL_SPRITE, SLEEP_SPRITE, FOOD_SPRITE};
use crate::messages::{GpuUpdate, GPU_UPDATE_QUEUE, GPU_READ_STATE, ProjGpuUpdate, PROJ_GPU_UPDATE_QUEUE, PROJ_HIT_STATE};
use crate::resources::NpcCount;

// =============================================================================
// CONSTANTS
// =============================================================================

const SHADER_ASSET_PATH: &str = "shaders/npc_compute.wgsl";
const PROJ_SHADER_ASSET_PATH: &str = "shaders/projectile_compute.wgsl";
const WORKGROUP_SIZE: u32 = 64;
const MAX_NPCS: u32 = 16384;
const MAX_PROJECTILES: u32 = 50_000;
const GRID_WIDTH: u32 = 128;
const GRID_HEIGHT: u32 = 128;
const MAX_PER_CELL: u32 = 48;
const HIT_RADIUS: f32 = 10.0;

// =============================================================================
// RESOURCES (Main World)
// =============================================================================

/// GPU buffer data extracted to render world each frame.
#[derive(Resource, Clone, ExtractResource)]
pub struct NpcGpuData {
    /// Number of active NPCs
    pub npc_count: u32,
    /// Frame delta time
    pub delta: f32,
}

impl Default for NpcGpuData {
    fn default() -> Self {
        Self {
            npc_count: 0,
            delta: 0.016,
        }
    }
}

/// GPU compute parameters (uniform buffer).
#[derive(Resource, Clone, ExtractResource, ShaderType)]
pub struct NpcComputeParams {
    pub count: u32,
    pub separation_radius: f32,
    pub separation_strength: f32,
    pub delta: f32,
    pub grid_width: u32,
    pub grid_height: u32,
    pub cell_size: f32,
    pub max_per_cell: u32,
    pub arrival_threshold: f32,
    pub mode: u32,
    pub combat_range: f32,
    pub _pad2: f32,
}

impl Default for NpcComputeParams {
    fn default() -> Self {
        Self {
            count: 0,
            separation_radius: 20.0,
            separation_strength: 100.0,
            delta: 0.016,
            grid_width: GRID_WIDTH,
            grid_height: GRID_HEIGHT,
            cell_size: 64.0,
            max_per_cell: MAX_PER_CELL,
            arrival_threshold: 8.0,
            mode: 0,
            combat_range: 300.0,
            _pad2: 0.0,
        }
    }
}

/// Buffer data to upload to GPU each frame.
/// Populated in main world, extracted to render world.
#[derive(Resource, Clone, ExtractResource)]
pub struct NpcBufferWrites {
    /// Position buffer: [x0, y0, x1, y1, ...] flattened
    pub positions: Vec<f32>,
    /// Target buffer: [x0, y0, x1, y1, ...] flattened
    pub targets: Vec<f32>,
    /// Speed buffer: one f32 per NPC
    pub speeds: Vec<f32>,
    /// Faction buffer: one i32 per NPC
    pub factions: Vec<i32>,
    /// Health buffer: one f32 per NPC
    pub healths: Vec<f32>,
    /// Arrival flags: one i32 per NPC (0 = moving, 1 = settled)
    pub arrivals: Vec<i32>,
    /// Sprite indices: [col, row, 0, 0] per NPC (vec4 for alignment)
    pub sprite_indices: Vec<f32>,
    /// Colors: [r, g, b, a] per NPC
    pub colors: Vec<f32>,
    /// Damage flash intensity: 0.0-1.0 per NPC (decays each frame)
    pub flash_values: Vec<f32>,
    /// Equipment sprites per layer: [col, row] per NPC, stride 2. -1.0 col = unequipped.
    pub armor_sprites: Vec<f32>,
    pub helmet_sprites: Vec<f32>,
    pub weapon_sprites: Vec<f32>,
    pub item_sprites: Vec<f32>,
    /// Status indicator sprites per NPC: [col, row], -1.0 = none. Layer 5 (sleep icon, etc.)
    pub status_sprites: Vec<f32>,
    /// Healing indicator sprites per NPC: [col, row], -1.0 = none. Layer 6 (healing glow)
    pub healing_sprites: Vec<f32>,
    /// Whether any data changed this frame (skip upload if false)
    pub dirty: bool,
    /// Per-field dirty indices — only these NPC slots get uploaded to GPU
    pub position_dirty_indices: Vec<usize>,
    pub target_dirty_indices: Vec<usize>,
    pub speed_dirty_indices: Vec<usize>,
    pub faction_dirty_indices: Vec<usize>,
    pub health_dirty_indices: Vec<usize>,
    pub arrival_dirty_indices: Vec<usize>,
}

impl Default for NpcBufferWrites {
    fn default() -> Self {
        // Pre-allocate for MAX_NPCS
        let max = MAX_NPCS as usize;
        Self {
            positions: vec![0.0; max * 2],
            targets: vec![0.0; max * 2],
            speeds: vec![100.0; max],
            factions: vec![0; max],
            healths: vec![100.0; max],
            arrivals: vec![0; max],  // 0 = wants to move
            sprite_indices: vec![0.0; max * 4], // vec4 per NPC
            colors: vec![1.0; max * 4],          // RGBA, default white
            flash_values: vec![0.0; max],
            armor_sprites: vec![-1.0; max * 2],
            helmet_sprites: vec![-1.0; max * 2],
            weapon_sprites: vec![-1.0; max * 2],
            item_sprites: vec![-1.0; max * 2],
            status_sprites: vec![-1.0; max * 2],
            healing_sprites: vec![-1.0; max * 2],
            dirty: false,
            position_dirty_indices: Vec::new(),
            target_dirty_indices: Vec::new(),
            speed_dirty_indices: Vec::new(),
            faction_dirty_indices: Vec::new(),
            health_dirty_indices: Vec::new(),
            arrival_dirty_indices: Vec::new(),
        }
    }
}

impl NpcBufferWrites {
    /// Apply a GPU update to the buffer data.
    pub fn apply(&mut self, update: &GpuUpdate) {
        match update {
            GpuUpdate::SetPosition { idx, x, y } => {
                let i = *idx * 2;
                if i + 1 < self.positions.len() {
                    self.positions[i] = *x;
                    self.positions[i + 1] = *y;
                    self.dirty = true;
                    self.position_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::SetTarget { idx, x, y } => {
                let i = *idx * 2;
                if i + 1 < self.targets.len() {
                    self.targets[i] = *x;
                    self.targets[i + 1] = *y;
                    self.dirty = true;
                    self.target_dirty_indices.push(*idx);
                }
                // Reset arrival flag so GPU resumes movement toward new target
                if *idx < self.arrivals.len() {
                    self.arrivals[*idx] = 0;
                    self.arrival_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::SetSpeed { idx, speed } => {
                if *idx < self.speeds.len() {
                    self.speeds[*idx] = *speed;
                    self.dirty = true;
                    self.speed_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::SetFaction { idx, faction } => {
                if *idx < self.factions.len() {
                    self.factions[*idx] = *faction;
                    self.dirty = true;
                    self.faction_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::SetHealth { idx, health } => {
                if *idx < self.healths.len() {
                    self.healths[*idx] = *health;
                    self.dirty = true;
                    self.health_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::ApplyDamage { idx, amount } => {
                if *idx < self.healths.len() {
                    self.healths[*idx] = (self.healths[*idx] - amount).max(0.0);
                    self.dirty = true;
                    self.health_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::HideNpc { idx } => {
                // Move to offscreen position
                let i = *idx * 2;
                if i + 1 < self.positions.len() {
                    self.positions[i] = -9999.0;
                    self.positions[i + 1] = -9999.0;
                    self.dirty = true;
                    self.position_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::SetSpriteFrame { idx, col, row } => {
                let i = *idx * 4;
                if i + 3 < self.sprite_indices.len() {
                    self.sprite_indices[i] = *col;
                    self.sprite_indices[i + 1] = *row;
                    // zw unused, leave as 0
                    self.dirty = true;
                }
            }
            GpuUpdate::SetDamageFlash { idx, intensity } => {
                if *idx < self.flash_values.len() {
                    self.flash_values[*idx] = *intensity;
                    self.dirty = true;
                }
            }
        }
    }
}

/// Derive visual sprite state (colors, equipment, indicators) from ECS components.
/// Single source of truth — replaces deferred SetColor/SetEquipSprite/SetHealing/SetSleeping messages.
/// Runs in Update after Step::Behavior so the buffer is in sync when tests read it.
pub fn sync_visual_sprites(
    mut buffer: ResMut<NpcBufferWrites>,
    all_npcs: Query<(
        &NpcIndex, &Faction, &Job, &Activity,
        Option<&Healing>,
        Option<&EquippedWeapon>, Option<&EquippedHelmet>, Option<&EquippedArmor>,
    ), Without<Dead>>,
) {
    // Single pass: write ALL visual fields per alive NPC (defaults where no component).
    // Dead NPCs are skipped by the renderer (x < -9000), so stale data is harmless.
    for (npc_idx, faction, job, activity, healing, weapon, helmet, armor) in all_npcs.iter() {
        let idx = npc_idx.0;
        let j = idx * 2;

        // Color: raiders use faction palette, others use job color
        let c = idx * 4;
        if c + 3 < buffer.colors.len() {
            let (r, g, b, a) = if *job == Job::Raider {
                crate::constants::raider_faction_color(faction.0)
            } else {
                job.color()
            };
            buffer.colors[c] = r;
            buffer.colors[c + 1] = g;
            buffer.colors[c + 2] = b;
            buffer.colors[c + 3] = a;
        }

        if j + 1 >= buffer.weapon_sprites.len() { continue; }

        // Equipment (write -1.0 sentinel when unequipped)
        let (wc, wr) = weapon.map(|w| (w.0, w.1)).unwrap_or((-1.0, 0.0));
        buffer.weapon_sprites[j] = wc;
        buffer.weapon_sprites[j + 1] = wr;

        let (hc, hr) = helmet.map(|h| (h.0, h.1)).unwrap_or((-1.0, 0.0));
        buffer.helmet_sprites[j] = hc;
        buffer.helmet_sprites[j + 1] = hr;

        let (ac, ar) = armor.map(|a| (a.0, a.1)).unwrap_or((-1.0, 0.0));
        buffer.armor_sprites[j] = ac;
        buffer.armor_sprites[j + 1] = ar;

        // Carried item (food)
        let (ic, ir) = if matches!(activity, Activity::Returning { has_food: true }) {
            (FOOD_SPRITE.0, FOOD_SPRITE.1)
        } else {
            (-1.0, 0.0)
        };
        buffer.item_sprites[j] = ic;
        buffer.item_sprites[j + 1] = ir;

        // Healing indicator
        let (hlc, hlr) = if healing.is_some() { (HEAL_SPRITE.0, HEAL_SPRITE.1) } else { (-1.0, 0.0) };
        buffer.healing_sprites[j] = hlc;
        buffer.healing_sprites[j + 1] = hlr;

        // Sleep indicator
        let (sc, sr) = if matches!(activity, Activity::Resting { .. }) {
            (SLEEP_SPRITE.0, SLEEP_SPRITE.1)
        } else {
            (-1.0, 0.0)
        };
        buffer.status_sprites[j] = sc;
        buffer.status_sprites[j + 1] = sr;
    }

    buffer.dirty = true;
}

/// Drain GPU_UPDATE_QUEUE and apply updates to NpcBufferWrites.
/// Runs in main world each frame before extraction.
pub fn populate_buffer_writes(mut buffer_writes: ResMut<NpcBufferWrites>, time: Res<Time>, npc_count: Res<NpcCount>) {
    // Reset dirty flags - will be set if any updates applied
    buffer_writes.dirty = false;
    buffer_writes.position_dirty_indices.clear();
    buffer_writes.target_dirty_indices.clear();
    buffer_writes.speed_dirty_indices.clear();
    buffer_writes.faction_dirty_indices.clear();
    buffer_writes.health_dirty_indices.clear();
    buffer_writes.arrival_dirty_indices.clear();

    if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
        for update in queue.drain(..) {
            buffer_writes.apply(&update);
        }
    }

    // Decay damage flash values (1.0 → 0.0 in ~0.2s)
    let dt = time.delta_secs();
    const FLASH_DECAY_RATE: f32 = 5.0;
    let mut any_flash = false;
    let active = npc_count.0.min(buffer_writes.flash_values.len());
    for flash in buffer_writes.flash_values[..active].iter_mut() {
        if *flash > 0.0 {
            *flash = (*flash - dt * FLASH_DECAY_RATE).max(0.0);
            any_flash = true;
        }
    }
    if any_flash {
        buffer_writes.dirty = true;
    }
}

// =============================================================================
// PROJECTILE RESOURCES (Main World)
// =============================================================================

/// Projectile GPU data extracted to render world each frame.
#[derive(Resource, Clone, ExtractResource)]
pub struct ProjGpuData {
    pub proj_count: u32,
    pub npc_count: u32,
    pub delta: f32,
}

impl Default for ProjGpuData {
    fn default() -> Self {
        Self { proj_count: 0, npc_count: 0, delta: 0.016 }
    }
}

/// Projectile compute parameters (uniform buffer).
#[derive(Resource, Clone, ExtractResource, ShaderType)]
pub struct ProjComputeParams {
    pub proj_count: u32,
    pub npc_count: u32,
    pub delta: f32,
    pub hit_radius: f32,
    pub grid_width: u32,
    pub grid_height: u32,
    pub cell_size: f32,
    pub max_per_cell: u32,
}

impl Default for ProjComputeParams {
    fn default() -> Self {
        Self {
            proj_count: 0,
            npc_count: 0,
            delta: 0.016,
            hit_radius: HIT_RADIUS,
            grid_width: GRID_WIDTH,
            grid_height: GRID_HEIGHT,
            cell_size: 64.0,
            max_per_cell: MAX_PER_CELL,
        }
    }
}

/// Projectile buffer data to upload to GPU each frame.
#[derive(Resource, Clone, ExtractResource)]
pub struct ProjBufferWrites {
    pub positions: Vec<f32>,   // [x, y] per proj
    pub velocities: Vec<f32>,  // [vx, vy] per proj
    pub damages: Vec<f32>,
    pub factions: Vec<i32>,
    pub shooters: Vec<i32>,
    pub lifetimes: Vec<f32>,
    pub active: Vec<i32>,
    pub hits: Vec<i32>,        // [npc_idx, processed] per proj
    pub dirty: bool,
    /// Per-slot dirty tracking: Spawn writes all fields, Deactivate writes active+hits
    pub spawn_dirty_indices: Vec<usize>,
    pub deactivate_dirty_indices: Vec<usize>,
}

impl Default for ProjBufferWrites {
    fn default() -> Self {
        let max = MAX_PROJECTILES as usize;
        Self {
            positions: vec![0.0; max * 2],
            velocities: vec![0.0; max * 2],
            damages: vec![0.0; max],
            factions: vec![0; max],
            shooters: vec![-1; max],
            lifetimes: vec![0.0; max],
            active: vec![0; max],
            hits: vec![-1; max * 2],   // -1 = no hit
            dirty: false,
            spawn_dirty_indices: Vec::new(),
            deactivate_dirty_indices: Vec::new(),
        }
    }
}

impl ProjBufferWrites {
    pub fn apply(&mut self, update: &ProjGpuUpdate) {
        match update {
            ProjGpuUpdate::Spawn { idx, x, y, vx, vy, damage, faction, shooter, lifetime } => {
                let i2 = *idx * 2;
                if i2 + 1 < self.positions.len() {
                    self.positions[i2] = *x;
                    self.positions[i2 + 1] = *y;
                    self.velocities[i2] = *vx;
                    self.velocities[i2 + 1] = *vy;
                    self.damages[*idx] = *damage;
                    self.factions[*idx] = *faction;
                    self.shooters[*idx] = *shooter;
                    self.lifetimes[*idx] = *lifetime;
                    self.active[*idx] = 1;
                    self.hits[i2] = -1;
                    self.hits[i2 + 1] = 0;
                    self.dirty = true;
                    self.spawn_dirty_indices.push(*idx);
                }
            }
            ProjGpuUpdate::Deactivate { idx } => {
                if *idx < self.active.len() {
                    self.active[*idx] = 0;
                    // Reset hit record so GPU doesn't re-trigger
                    let i2 = *idx * 2;
                    if i2 + 1 < self.hits.len() {
                        self.hits[i2] = -1;
                        self.hits[i2 + 1] = 0;
                    }
                    self.dirty = true;
                    self.deactivate_dirty_indices.push(*idx);
                }
            }
        }
    }
}

/// Drain PROJ_GPU_UPDATE_QUEUE and apply updates to ProjBufferWrites.
pub fn populate_proj_buffer_writes(mut writes: ResMut<ProjBufferWrites>) {
    writes.dirty = false;
    writes.spawn_dirty_indices.clear();
    writes.deactivate_dirty_indices.clear();
    if let Ok(mut queue) = PROJ_GPU_UPDATE_QUEUE.lock() {
        for update in queue.drain(..) {
            writes.apply(&update);
        }
    }
}

// =============================================================================
// PLUGIN
// =============================================================================

pub struct GpuComputePlugin;

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct NpcComputeLabel;

impl Plugin for GpuComputePlugin {
    fn build(&self, app: &mut App) {
        // Initialize NPC resources in main world
        app.init_resource::<NpcGpuData>()
            .init_resource::<NpcComputeParams>()
            .init_resource::<NpcBufferWrites>()
            .init_resource::<NpcSpriteTexture>()
            .add_systems(Update, update_gpu_data)
            .add_systems(PostUpdate, populate_buffer_writes);

        // Initialize projectile resources in main world
        app.init_resource::<ProjGpuData>()
            .init_resource::<ProjComputeParams>()
            .init_resource::<ProjBufferWrites>()
            .add_systems(Update, update_proj_gpu_data)
            .add_systems(PostUpdate, populate_proj_buffer_writes);

        // Extract resources to render world
        app.add_plugins((
            ExtractResourcePlugin::<NpcGpuData>::default(),
            ExtractResourcePlugin::<NpcComputeParams>::default(),
            ExtractResourcePlugin::<NpcBufferWrites>::default(),
            ExtractResourcePlugin::<NpcSpriteTexture>::default(),
            ExtractResourcePlugin::<ProjGpuData>::default(),
            ExtractResourcePlugin::<ProjComputeParams>::default(),
            ExtractResourcePlugin::<ProjBufferWrites>::default(),
        ));

        // Set up render world systems
        let render_app = match app.get_sub_app_mut(RenderApp) {
            Some(ra) => ra,
            None => {
                warn!("RenderApp not available - GPU compute disabled");
                return;
            }
        };

        render_app
            .add_systems(RenderStartup, (init_npc_compute_pipeline, init_proj_compute_pipeline))
            .add_systems(
                Render,
                (
                    (write_npc_buffers, write_proj_buffers).in_set(RenderSystems::PrepareResources),
                    (prepare_npc_bind_groups, prepare_proj_bind_groups).in_set(RenderSystems::PrepareBindGroups),
                    readback_all.in_set(RenderSystems::Cleanup),
                ),
            );

        // Add compute nodes to render graph
        // NPC compute → Projectile compute → Camera driver
        {
            let mut render_graph = render_app.world_mut().resource_mut::<RenderGraph>();
            render_graph.add_node(NpcComputeLabel, NpcComputeNode::default());
            render_graph.add_node(ProjectileComputeLabel, ProjectileComputeNode::default());
            render_graph.add_node_edge(NpcComputeLabel, ProjectileComputeLabel);
            render_graph.add_node_edge(ProjectileComputeLabel, bevy::render::graph::CameraDriverLabel);
        }

        info!("GPU compute plugin initialized");
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct ProjectileComputeLabel;

/// Update GPU data from ECS each frame.
fn update_gpu_data(
    mut gpu_data: ResMut<NpcGpuData>,
    mut params: ResMut<NpcComputeParams>,
    npc_count: Res<crate::resources::NpcCount>,
    time: Res<Time>,
) {
    gpu_data.npc_count = npc_count.0 as u32;
    gpu_data.delta = time.delta_secs();
    params.delta = time.delta_secs();
    params.count = npc_count.0 as u32;
}

// =============================================================================
// RENDER WORLD RESOURCES
// =============================================================================

/// Ping-pong index for double-buffered staging readback.
/// Frame N writes to staging[current], readback reads staging[1-current] (previous frame's data).
#[derive(Resource, Default)]
struct StagingIndex {
    current: usize,  // 0 or 1
    has_previous: bool,  // false on first frame (no previous data to read)
}

/// GPU buffers for NPC compute and rendering.
#[derive(Resource)]
pub struct NpcGpuBuffers {
    // Compute buffers
    pub positions: Buffer,
    pub targets: Buffer,
    pub speeds: Buffer,
    pub grid_counts: Buffer,
    pub grid_data: Buffer,
    pub arrivals: Buffer,
    pub backoff: Buffer,
    pub factions: Buffer,
    pub healths: Buffer,
    pub combat_targets: Buffer,
    /// Double-buffered staging for CPU readback of positions (MAP_READ | COPY_DST)
    pub position_staging: [Buffer; 2],
    /// Double-buffered staging for CPU readback of combat targets (MAP_READ | COPY_DST)
    pub combat_target_staging: [Buffer; 2],
}

/// Bind groups for compute passes (one per mode, different uniform buffer).
#[derive(Resource)]
struct NpcBindGroups {
    mode0: BindGroup,  // Clear grid
    mode1: BindGroup,  // Build grid
    mode2: BindGroup,  // Movement + targeting
}

/// Pipeline resources for compute.
#[derive(Resource)]
struct NpcComputePipeline {
    bind_group_layout: BindGroupLayoutDescriptor,
    pipeline_id: CachedComputePipelineId,
}

/// Handle to the NPC sprite texture (main world).
/// Set by the render module after loading sprite sheets.
#[derive(Resource, Clone, ExtractResource, Default)]
pub struct NpcSpriteTexture {
    pub handle: Option<Handle<Image>>,
    pub world_handle: Option<Handle<Image>>,
}

/// GPU buffers for projectile compute.
#[derive(Resource)]
pub struct ProjGpuBuffers {
    pub positions: Buffer,
    pub velocities: Buffer,
    pub damages: Buffer,
    pub factions: Buffer,
    pub shooters: Buffer,
    pub lifetimes: Buffer,
    pub active: Buffer,
    pub hits: Buffer,
    /// Double-buffered staging for CPU readback of hit results (MAP_READ | COPY_DST)
    pub hit_staging: [Buffer; 2],
    /// Double-buffered staging for CPU readback of projectile positions (MAP_READ | COPY_DST)
    pub position_staging: [Buffer; 2],
}

/// Bind groups for projectile compute pass.
#[derive(Resource)]
struct ProjBindGroups {
    bind_group: BindGroup,
}

/// Pipeline resources for projectile compute.
#[derive(Resource)]
struct ProjComputePipeline {
    bind_group_layout: BindGroupLayoutDescriptor,
    pipeline_id: CachedComputePipelineId,
}

// =============================================================================
// PIPELINE INITIALIZATION
// =============================================================================

fn init_npc_compute_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let grid_cells = (GRID_WIDTH * GRID_HEIGHT) as usize;
    let grid_data_size = grid_cells * MAX_PER_CELL as usize;

    // Create GPU buffers
    let buffers = NpcGpuBuffers {
        positions: render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_positions"),
            size: (MAX_NPCS as usize * std::mem::size_of::<[f32; 2]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        targets: render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_targets"),
            size: (MAX_NPCS as usize * std::mem::size_of::<[f32; 2]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        speeds: render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_speeds"),
            size: (MAX_NPCS as usize * std::mem::size_of::<f32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        grid_counts: render_device.create_buffer(&BufferDescriptor {
            label: Some("grid_counts"),
            size: (grid_cells * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE,
            mapped_at_creation: false,
        }),
        grid_data: render_device.create_buffer(&BufferDescriptor {
            label: Some("grid_data"),
            size: (grid_data_size * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE,
            mapped_at_creation: false,
        }),
        arrivals: render_device.create_buffer(&BufferDescriptor {
            label: Some("arrivals"),
            size: (MAX_NPCS as usize * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        backoff: render_device.create_buffer(&BufferDescriptor {
            label: Some("backoff"),
            size: (MAX_NPCS as usize * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE,
            mapped_at_creation: false,
        }),
        factions: render_device.create_buffer(&BufferDescriptor {
            label: Some("factions"),
            size: (MAX_NPCS as usize * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        healths: render_device.create_buffer(&BufferDescriptor {
            label: Some("healths"),
            size: (MAX_NPCS as usize * std::mem::size_of::<f32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        combat_targets: render_device.create_buffer(&BufferDescriptor {
            label: Some("combat_targets"),
            size: (MAX_NPCS as usize * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        position_staging: [
            render_device.create_buffer(&BufferDescriptor {
                label: Some("npc_position_staging_0"),
                size: (MAX_NPCS as usize * std::mem::size_of::<[f32; 2]>()) as u64,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            render_device.create_buffer(&BufferDescriptor {
                label: Some("npc_position_staging_1"),
                size: (MAX_NPCS as usize * std::mem::size_of::<[f32; 2]>()) as u64,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        ],
        combat_target_staging: [
            render_device.create_buffer(&BufferDescriptor {
                label: Some("npc_combat_target_staging_0"),
                size: (MAX_NPCS as usize * std::mem::size_of::<i32>()) as u64,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            render_device.create_buffer(&BufferDescriptor {
                label: Some("npc_combat_target_staging_1"),
                size: (MAX_NPCS as usize * std::mem::size_of::<i32>()) as u64,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        ],
    };

    commands.insert_resource(buffers);
    commands.insert_resource(StagingIndex::default());

    // Define bind group layout (all storage buffers are read_write for simplicity)
    let bind_group_layout = BindGroupLayoutDescriptor::new(
        "NpcComputeLayout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                // 0: positions
                storage_buffer::<Vec<[f32; 2]>>(false),
                // 1: goals (targets)
                storage_buffer::<Vec<[f32; 2]>>(false),
                // 2: speeds
                storage_buffer::<Vec<f32>>(false),
                // 3: grid_counts
                storage_buffer::<Vec<i32>>(false),
                // 4: grid_data
                storage_buffer::<Vec<i32>>(false),
                // 5: arrivals
                storage_buffer::<Vec<i32>>(false),
                // 6: backoff
                storage_buffer::<Vec<i32>>(false),
                // 7: factions
                storage_buffer::<Vec<i32>>(false),
                // 8: healths
                storage_buffer::<Vec<f32>>(false),
                // 9: combat_targets
                storage_buffer::<Vec<i32>>(false),
                // 10: params (uniform)
                uniform_buffer::<NpcComputeParams>(false),
            ),
        ),
    );

    // Queue compute pipeline
    let shader = asset_server.load(SHADER_ASSET_PATH);
    let pipeline_id = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some(Cow::from("npc_compute_pipeline")),
        layout: vec![bind_group_layout.clone()],
        shader,
        entry_point: Some(Cow::from("main")),
        ..default()
    });

    commands.insert_resource(NpcComputePipeline {
        bind_group_layout,
        pipeline_id,
    });

    info!("NPC compute pipeline queued");
}


// =============================================================================
// BIND GROUP PREPARATION
// =============================================================================

fn prepare_npc_bind_groups(
    mut commands: Commands,
    pipeline: Option<Res<NpcComputePipeline>>,
    buffers: Option<Res<NpcGpuBuffers>>,
    params: Option<Res<NpcComputeParams>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline_cache: Res<PipelineCache>,
) {
    let Some(pipeline) = pipeline else { return };
    let Some(buffers) = buffers else { return };
    let Some(params) = params else { return };

    // Create 3 uniform buffers (one per mode) for multi-dispatch
    let layout = &pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout);
    let storage_bindings = (
        buffers.positions.as_entire_buffer_binding(),
        buffers.targets.as_entire_buffer_binding(),
        buffers.speeds.as_entire_buffer_binding(),
        buffers.grid_counts.as_entire_buffer_binding(),
        buffers.grid_data.as_entire_buffer_binding(),
        buffers.arrivals.as_entire_buffer_binding(),
        buffers.backoff.as_entire_buffer_binding(),
        buffers.factions.as_entire_buffer_binding(),
        buffers.healths.as_entire_buffer_binding(),
        buffers.combat_targets.as_entire_buffer_binding(),
    );

    let mut p0 = params.clone(); p0.mode = 0;
    let mut p1 = params.clone(); p1.mode = 1;
    let mut p2 = params.clone(); p2.mode = 2;

    let mut ub0 = UniformBuffer::from(p0);
    let mut ub1 = UniformBuffer::from(p1);
    let mut ub2 = UniformBuffer::from(p2);
    ub0.write_buffer(&render_device, &render_queue);
    ub1.write_buffer(&render_device, &render_queue);
    ub2.write_buffer(&render_device, &render_queue);

    let mode0 = render_device.create_bind_group(
        Some("npc_compute_bg_mode0"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(), storage_bindings.1.clone(),
            storage_bindings.2.clone(), storage_bindings.3.clone(),
            storage_bindings.4.clone(), storage_bindings.5.clone(),
            storage_bindings.6.clone(), storage_bindings.7.clone(),
            storage_bindings.8.clone(), storage_bindings.9.clone(),
            &ub0,
        )),
    );
    let mode1 = render_device.create_bind_group(
        Some("npc_compute_bg_mode1"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(), storage_bindings.1.clone(),
            storage_bindings.2.clone(), storage_bindings.3.clone(),
            storage_bindings.4.clone(), storage_bindings.5.clone(),
            storage_bindings.6.clone(), storage_bindings.7.clone(),
            storage_bindings.8.clone(), storage_bindings.9.clone(),
            &ub1,
        )),
    );
    let mode2 = render_device.create_bind_group(
        Some("npc_compute_bg_mode2"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(), storage_bindings.1.clone(),
            storage_bindings.2.clone(), storage_bindings.3.clone(),
            storage_bindings.4.clone(), storage_bindings.5.clone(),
            storage_bindings.6.clone(), storage_bindings.7.clone(),
            storage_bindings.8.clone(), storage_bindings.9.clone(),
            &ub2,
        )),
    );

    commands.insert_resource(NpcBindGroups { mode0, mode1, mode2 });
}


// =============================================================================
// BUFFER WRITING
// =============================================================================

/// Write NPC data from extracted resource to GPU buffers.
/// Runs in render world before bind group preparation.
fn write_npc_buffers(
    buffers: Option<Res<NpcGpuBuffers>>,
    buffer_writes: Option<Res<NpcBufferWrites>>,
    render_queue: Res<RenderQueue>,
) {
    let Some(buffers) = buffers else { return };
    let Some(writes) = buffer_writes else { return };

    // Skip if no changes this frame
    if !writes.dirty {
        return;
    }

    // Debug: log first 5 NPCs data once
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        for i in 0..5 {
            let px = writes.positions.get(i * 2).copied().unwrap_or(-1.0);
            let py = writes.positions.get(i * 2 + 1).copied().unwrap_or(-1.0);
            let sc = writes.sprite_indices.get(i * 4).copied().unwrap_or(-1.0);
            let sr = writes.sprite_indices.get(i * 4 + 1).copied().unwrap_or(-1.0);
            let cr = writes.colors.get(i * 4).copied().unwrap_or(-1.0);
            let cg = writes.colors.get(i * 4 + 1).copied().unwrap_or(-1.0);
            let cb = writes.colors.get(i * 4 + 2).copied().unwrap_or(-1.0);
            info!("NPC[{}] pos=({:.0},{:.0}) sprite=({:.0},{:.0}) color=({:.2},{:.2},{:.2})",
                  i, px, py, sc, sr, cr, cg, cb);
        }
    }

    // Per-index uploads — only write the NPC slots that actually changed
    for &idx in &writes.position_dirty_indices {
        let start = idx * 2;
        if start + 2 <= writes.positions.len() {
            let byte_offset = (start * std::mem::size_of::<f32>()) as u64;
            render_queue.write_buffer(
                &buffers.positions, byte_offset,
                bytemuck::cast_slice(&writes.positions[start..start + 2]),
            );
        }
    }

    for &idx in &writes.target_dirty_indices {
        let start = idx * 2;
        if start + 2 <= writes.targets.len() {
            let byte_offset = (start * std::mem::size_of::<f32>()) as u64;
            render_queue.write_buffer(
                &buffers.targets, byte_offset,
                bytemuck::cast_slice(&writes.targets[start..start + 2]),
            );
        }
    }

    for &idx in &writes.speed_dirty_indices {
        if idx < writes.speeds.len() {
            let byte_offset = (idx * std::mem::size_of::<f32>()) as u64;
            render_queue.write_buffer(
                &buffers.speeds, byte_offset,
                bytemuck::cast_slice(&writes.speeds[idx..idx + 1]),
            );
        }
    }

    for &idx in &writes.faction_dirty_indices {
        if idx < writes.factions.len() {
            let byte_offset = (idx * std::mem::size_of::<i32>()) as u64;
            render_queue.write_buffer(
                &buffers.factions, byte_offset,
                bytemuck::cast_slice(&writes.factions[idx..idx + 1]),
            );
        }
    }

    for &idx in &writes.health_dirty_indices {
        if idx < writes.healths.len() {
            let byte_offset = (idx * std::mem::size_of::<f32>()) as u64;
            render_queue.write_buffer(
                &buffers.healths, byte_offset,
                bytemuck::cast_slice(&writes.healths[idx..idx + 1]),
            );
        }
    }

    for &idx in &writes.arrival_dirty_indices {
        if idx < writes.arrivals.len() {
            let byte_offset = (idx * std::mem::size_of::<i32>()) as u64;
            render_queue.write_buffer(
                &buffers.arrivals, byte_offset,
                bytemuck::cast_slice(&writes.arrivals[idx..idx + 1]),
            );
        }
    }

}

/// Double-buffered readback: read PREVIOUS frame's staging data, single poll for all buffers.
/// Compute nodes copy to staging[current] this frame; we read staging[1-current] (already done).
/// The poll returns near-instantly since GPU finished last frame's copy before this frame started.
fn readback_all(
    npc_buffers: Option<Res<NpcGpuBuffers>>,
    gpu_data: Option<Res<NpcGpuData>>,
    proj_buffers: Option<Res<ProjGpuBuffers>>,
    proj_data: Option<Res<ProjGpuData>>,
    mut staging_index: ResMut<StagingIndex>,
    render_device: Res<RenderDevice>,
) {
    let read_idx = 1 - staging_index.current;

    // Skip first frame — no previous data to read yet
    if !staging_index.has_previous {
        staging_index.has_previous = true;
        staging_index.current = 1 - staging_index.current;
        return;
    }

    // Map all staging buffers from the PREVIOUS frame (up to 4 maps, single poll)
    let (tx_all, rx_all) = std::sync::mpsc::sync_channel(4);

    // NPC staging maps
    let npc_count = gpu_data.as_ref().map(|d| d.npc_count as usize).unwrap_or(0);
    let has_npc = npc_count > 0 && npc_buffers.is_some();
    let npc_pos_slice;
    let npc_ct_slice;
    if has_npc {
        let buffers = npc_buffers.as_ref().unwrap();
        let pos_size = npc_count * std::mem::size_of::<[f32; 2]>();
        let ct_size = npc_count * std::mem::size_of::<i32>();
        npc_pos_slice = Some(buffers.position_staging[read_idx].slice(..pos_size as u64));
        npc_ct_slice = Some(buffers.combat_target_staging[read_idx].slice(..ct_size as u64));

        let tx = tx_all.clone();
        npc_pos_slice.as_ref().unwrap().map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(("npc_pos", r)); });
        let tx = tx_all.clone();
        npc_ct_slice.as_ref().unwrap().map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(("npc_ct", r)); });
    } else {
        npc_pos_slice = None;
        npc_ct_slice = None;
    }

    // Projectile staging maps
    let proj_count = proj_data.as_ref().map(|d| d.proj_count as usize).unwrap_or(0);
    let has_proj = proj_count > 0 && proj_buffers.is_some();
    let proj_hit_slice;
    let proj_pos_slice;
    if has_proj {
        let buffers = proj_buffers.as_ref().unwrap();
        let hit_size = proj_count * std::mem::size_of::<[i32; 2]>();
        let pos_size = proj_count * std::mem::size_of::<[f32; 2]>();
        proj_hit_slice = Some(buffers.hit_staging[read_idx].slice(..hit_size as u64));
        proj_pos_slice = Some(buffers.position_staging[read_idx].slice(..pos_size as u64));

        let tx = tx_all.clone();
        proj_hit_slice.as_ref().unwrap().map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(("proj_hit", r)); });
        let tx = tx_all.clone();
        proj_pos_slice.as_ref().unwrap().map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(("proj_pos", r)); });
    } else {
        proj_hit_slice = None;
        proj_pos_slice = None;
    }
    drop(tx_all);

    let expected = (if has_npc { 2 } else { 0 }) + (if has_proj { 2 } else { 0 });
    if expected == 0 {
        staging_index.current = 1 - staging_index.current;
        return;
    }

    // Single poll flushes all pending map_async calls
    let _ = render_device.poll(wgpu::PollType::wait_indefinitely());

    // Collect results
    let mut npc_pos_ok = false;
    let mut npc_ct_ok = false;
    let mut proj_hit_ok = false;
    let mut proj_pos_ok = false;
    for _ in 0..expected {
        if let Ok((tag, result)) = rx_all.recv() {
            let ok = result.is_ok();
            match tag {
                "npc_pos" => npc_pos_ok = ok,
                "npc_ct" => npc_ct_ok = ok,
                "proj_hit" => proj_hit_ok = ok,
                "proj_pos" => proj_pos_ok = ok,
                _ => {}
            }
        }
    }

    // Process NPC readback
    if npc_pos_ok && npc_ct_ok {
        let pos_data = npc_pos_slice.as_ref().unwrap().get_mapped_range();
        let ct_data = npc_ct_slice.as_ref().unwrap().get_mapped_range();
        let positions: &[f32] = bytemuck::cast_slice(&pos_data);
        let combat_targets: &[i32] = bytemuck::cast_slice(&ct_data);

        if let Ok(mut state) = GPU_READ_STATE.lock() {
            state.positions.clear();
            state.positions.extend_from_slice(&positions[..npc_count * 2]);
            state.combat_targets.clear();
            state.combat_targets.extend_from_slice(&combat_targets[..npc_count]);
            state.npc_count = npc_count;
        }

        drop(pos_data);
        drop(ct_data);
    }
    if has_npc {
        let buffers = npc_buffers.as_ref().unwrap();
        if npc_pos_ok { buffers.position_staging[read_idx].unmap(); }
        if npc_ct_ok { buffers.combat_target_staging[read_idx].unmap(); }
    }

    // Process projectile readback
    if proj_hit_ok {
        let mapped = proj_hit_slice.as_ref().unwrap().get_mapped_range();
        let hits: &[[i32; 2]] = bytemuck::cast_slice(&mapped);
        if let Ok(mut state) = PROJ_HIT_STATE.lock() {
            state.clear();
            state.extend_from_slice(&hits[..proj_count]);
        }
        drop(mapped);
    }
    if proj_pos_ok {
        let mapped = proj_pos_slice.as_ref().unwrap().get_mapped_range();
        let positions: &[f32] = bytemuck::cast_slice(&mapped);
        if let Ok(mut state) = crate::messages::PROJ_POSITION_STATE.lock() {
            state.clear();
            state.extend_from_slice(&positions[..proj_count * 2]);
        }
        drop(mapped);
    }
    if has_proj {
        let buffers = proj_buffers.as_ref().unwrap();
        if proj_hit_ok { buffers.hit_staging[read_idx].unmap(); }
        if proj_pos_ok { buffers.position_staging[read_idx].unmap(); }
    }

    // Flip staging index for next frame
    staging_index.current = 1 - staging_index.current;
}

// =============================================================================
// RENDER GRAPH NODE
// =============================================================================

enum NpcComputeState {
    Loading,
    Ready,
}

struct NpcComputeNode {
    state: NpcComputeState,
}

impl Default for NpcComputeNode {
    fn default() -> Self {
        Self {
            state: NpcComputeState::Loading,
        }
    }
}

impl render_graph::Node for NpcComputeNode {
    fn update(&mut self, world: &mut World) {
        let Some(pipeline) = world.get_resource::<NpcComputePipeline>() else {
            return;
        };
        let pipeline_cache = world.resource::<PipelineCache>();

        match self.state {
            NpcComputeState::Loading => {
                match pipeline_cache.get_compute_pipeline_state(pipeline.pipeline_id) {
                    CachedPipelineState::Ok(_) => {
                        self.state = NpcComputeState::Ready;
                        info!("NPC compute pipeline ready");
                    }
                    CachedPipelineState::Err(PipelineCacheError::ShaderNotLoaded(_)) => {}
                    CachedPipelineState::Err(err) => {
                        panic!("NPC compute shader error: {err}")
                    }
                    _ => {}
                }
            }
            NpcComputeState::Ready => {}
        }
    }

    fn run(
        &self,
        _graph: &mut render_graph::RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), render_graph::NodeRunError> {
        // Only run if ready
        if !matches!(self.state, NpcComputeState::Ready) {
            return Ok(());
        }

        let Some(bind_groups) = world.get_resource::<NpcBindGroups>() else {
            return Ok(());
        };
        let Some(gpu_data) = world.get_resource::<NpcGpuData>() else {
            return Ok(());
        };
        let pipeline_cache = world.resource::<PipelineCache>();
        let pipeline = world.resource::<NpcComputePipeline>();

        let npc_count = gpu_data.npc_count;
        if npc_count == 0 {
            return Ok(());
        }

        let Some(compute_pipeline) = pipeline_cache.get_compute_pipeline(pipeline.pipeline_id)
        else {
            return Ok(());
        };

        let grid_cells = GRID_WIDTH * GRID_HEIGHT;
        let grid_wg = (grid_cells + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
        let npc_wg = (npc_count + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;

        // Pass 0: Clear spatial grid
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_bind_group(0, &bind_groups.mode0, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(grid_wg, 1, 1);
        }

        // Pass 1: Build spatial grid (insert NPCs into cells)
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_bind_group(0, &bind_groups.mode1, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(npc_wg, 1, 1);
        }

        // Pass 2: Movement + combat targeting
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_bind_group(0, &bind_groups.mode2, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(npc_wg, 1, 1);
        }

        // Copy positions + combat_targets → staging[current] for CPU readback next frame
        let buffers = world.resource::<NpcGpuBuffers>();
        let si = world.resource::<StagingIndex>().current;
        let pos_copy_size = (npc_count as u64) * std::mem::size_of::<[f32; 2]>() as u64;
        let ct_copy_size = (npc_count as u64) * std::mem::size_of::<i32>() as u64;

        render_context.command_encoder().copy_buffer_to_buffer(
            &buffers.positions, 0, &buffers.position_staging[si], 0, pos_copy_size,
        );
        render_context.command_encoder().copy_buffer_to_buffer(
            &buffers.combat_targets, 0, &buffers.combat_target_staging[si], 0, ct_copy_size,
        );

        Ok(())
    }
}

// =============================================================================
// PROJECTILE COMPUTE
// =============================================================================

/// Update projectile GPU data from ECS each frame.
fn update_proj_gpu_data(
    mut proj_data: ResMut<ProjGpuData>,
    mut proj_params: ResMut<ProjComputeParams>,
    npc_count: Res<crate::resources::NpcCount>,
    proj_alloc: Res<crate::resources::ProjSlotAllocator>,
    time: Res<Time>,
) {
    let pc = proj_alloc.next as u32;
    let nc = npc_count.0 as u32;
    let dt = time.delta_secs();
    proj_data.proj_count = pc;
    proj_data.npc_count = nc;
    proj_data.delta = dt;
    proj_params.proj_count = pc;
    proj_params.npc_count = nc;
    proj_params.delta = dt;
}

fn init_proj_compute_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let max = MAX_PROJECTILES as usize;

    let buffers = ProjGpuBuffers {
        positions: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_positions"),
            size: (max * std::mem::size_of::<[f32; 2]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        velocities: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_velocities"),
            size: (max * std::mem::size_of::<[f32; 2]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        damages: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_damages"),
            size: (max * std::mem::size_of::<f32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        factions: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_factions"),
            size: (max * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        shooters: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_shooters"),
            size: (max * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        lifetimes: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_lifetimes"),
            size: (max * std::mem::size_of::<f32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        active: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_active"),
            size: (max * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        hits: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_hits"),
            size: (max * std::mem::size_of::<[i32; 2]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        hit_staging: [
            render_device.create_buffer(&BufferDescriptor {
                label: Some("proj_hit_staging_0"),
                size: (max * std::mem::size_of::<[i32; 2]>()) as u64,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            render_device.create_buffer(&BufferDescriptor {
                label: Some("proj_hit_staging_1"),
                size: (max * std::mem::size_of::<[i32; 2]>()) as u64,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        ],
        position_staging: [
            render_device.create_buffer(&BufferDescriptor {
                label: Some("proj_position_staging_0"),
                size: (max * std::mem::size_of::<[f32; 2]>()) as u64,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            render_device.create_buffer(&BufferDescriptor {
                label: Some("proj_position_staging_1"),
                size: (max * std::mem::size_of::<[f32; 2]>()) as u64,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        ],
    };

    commands.insert_resource(buffers);

    // 14 bindings: 8 proj (rw) + 3 NPC (ro) + 2 grid (ro) + 1 uniform
    let bind_group_layout = BindGroupLayoutDescriptor::new(
        "ProjComputeLayout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                // 0-7: projectile buffers (read_write)
                storage_buffer::<Vec<[f32; 2]>>(false),  // positions
                storage_buffer::<Vec<[f32; 2]>>(false),  // velocities
                storage_buffer::<Vec<f32>>(false),        // damages
                storage_buffer::<Vec<i32>>(false),        // factions
                storage_buffer::<Vec<i32>>(false),        // shooters
                storage_buffer::<Vec<f32>>(false),        // lifetimes
                storage_buffer::<Vec<i32>>(false),        // active
                storage_buffer::<Vec<[i32; 2]>>(false),   // hits
                // 8-10: NPC buffers (read only)
                storage_buffer_read_only::<Vec<[f32; 2]>>(false), // npc_positions
                storage_buffer_read_only::<Vec<i32>>(false),      // npc_factions
                storage_buffer_read_only::<Vec<f32>>(false),      // npc_healths
                // 11-12: spatial grid (read only)
                storage_buffer_read_only::<Vec<i32>>(false),      // grid_counts
                storage_buffer_read_only::<Vec<i32>>(false),      // grid_data
                // 13: uniform params
                uniform_buffer::<ProjComputeParams>(false),
            ),
        ),
    );

    let shader = asset_server.load(PROJ_SHADER_ASSET_PATH);
    let pipeline_id = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some(Cow::from("proj_compute_pipeline")),
        layout: vec![bind_group_layout.clone()],
        shader,
        entry_point: Some(Cow::from("main")),
        ..default()
    });

    commands.insert_resource(ProjComputePipeline {
        bind_group_layout,
        pipeline_id,
    });

    info!("Projectile compute pipeline queued");
}

fn prepare_proj_bind_groups(
    mut commands: Commands,
    pipeline: Option<Res<ProjComputePipeline>>,
    proj_buffers: Option<Res<ProjGpuBuffers>>,
    npc_buffers: Option<Res<NpcGpuBuffers>>,
    params: Option<Res<ProjComputeParams>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline_cache: Res<PipelineCache>,
) {
    let Some(pipeline) = pipeline else { return };
    let Some(proj) = proj_buffers else { return };
    let Some(npc) = npc_buffers else { return };
    let Some(params) = params else { return };

    let mut uniform_buffer = UniformBuffer::from(params.clone());
    uniform_buffer.write_buffer(&render_device, &render_queue);

    let bind_group = render_device.create_bind_group(
        Some("proj_compute_bind_group"),
        &pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout),
        &BindGroupEntries::sequential((
            // 0-7: projectile buffers
            proj.positions.as_entire_buffer_binding(),
            proj.velocities.as_entire_buffer_binding(),
            proj.damages.as_entire_buffer_binding(),
            proj.factions.as_entire_buffer_binding(),
            proj.shooters.as_entire_buffer_binding(),
            proj.lifetimes.as_entire_buffer_binding(),
            proj.active.as_entire_buffer_binding(),
            proj.hits.as_entire_buffer_binding(),
            // 8-10: NPC buffers (shared, read only)
            npc.positions.as_entire_buffer_binding(),
            npc.factions.as_entire_buffer_binding(),
            npc.healths.as_entire_buffer_binding(),
            // 11-12: spatial grid (shared, read only)
            npc.grid_counts.as_entire_buffer_binding(),
            npc.grid_data.as_entire_buffer_binding(),
            // 13: uniform
            &uniform_buffer,
        )),
    );

    commands.insert_resource(ProjBindGroups { bind_group });
}

/// Write projectile data from extracted resource to GPU buffers.
fn write_proj_buffers(
    buffers: Option<Res<ProjGpuBuffers>>,
    writes: Option<Res<ProjBufferWrites>>,
    render_queue: Res<RenderQueue>,
) {
    let Some(buffers) = buffers else { return };
    let Some(writes) = writes else { return };

    if !writes.dirty {
        return;
    }

    // Spawn: write all fields for each new projectile slot
    for &idx in &writes.spawn_dirty_indices {
        let i2 = idx * 2;
        if i2 + 2 <= writes.positions.len() {
            let byte2 = (i2 * std::mem::size_of::<f32>()) as u64;
            render_queue.write_buffer(&buffers.positions, byte2, bytemuck::cast_slice(&writes.positions[i2..i2 + 2]));
            render_queue.write_buffer(&buffers.velocities, byte2, bytemuck::cast_slice(&writes.velocities[i2..i2 + 2]));
        }
        if idx < writes.damages.len() {
            let byte1f = (idx * std::mem::size_of::<f32>()) as u64;
            let byte1i = (idx * std::mem::size_of::<i32>()) as u64;
            render_queue.write_buffer(&buffers.damages, byte1f, bytemuck::cast_slice(&writes.damages[idx..idx + 1]));
            render_queue.write_buffer(&buffers.factions, byte1i, bytemuck::cast_slice(&writes.factions[idx..idx + 1]));
            render_queue.write_buffer(&buffers.shooters, byte1i, bytemuck::cast_slice(&writes.shooters[idx..idx + 1]));
            render_queue.write_buffer(&buffers.lifetimes, byte1f, bytemuck::cast_slice(&writes.lifetimes[idx..idx + 1]));
            render_queue.write_buffer(&buffers.active, byte1i, bytemuck::cast_slice(&writes.active[idx..idx + 1]));
        }
        if i2 + 2 <= writes.hits.len() {
            let byte2i = (i2 * std::mem::size_of::<i32>()) as u64;
            render_queue.write_buffer(&buffers.hits, byte2i, bytemuck::cast_slice(&writes.hits[i2..i2 + 2]));
        }
    }

    // Deactivate: write only active flag + hit reset
    for &idx in &writes.deactivate_dirty_indices {
        if idx < writes.active.len() {
            let byte1i = (idx * std::mem::size_of::<i32>()) as u64;
            render_queue.write_buffer(&buffers.active, byte1i, bytemuck::cast_slice(&writes.active[idx..idx + 1]));
        }
        let i2 = idx * 2;
        if i2 + 2 <= writes.hits.len() {
            let byte2i = (i2 * std::mem::size_of::<i32>()) as u64;
            render_queue.write_buffer(&buffers.hits, byte2i, bytemuck::cast_slice(&writes.hits[i2..i2 + 2]));
        }
    }
}

enum ProjComputeState {
    Loading,
    Ready,
}

struct ProjectileComputeNode {
    state: ProjComputeState,
}

impl Default for ProjectileComputeNode {
    fn default() -> Self {
        Self { state: ProjComputeState::Loading }
    }
}

impl render_graph::Node for ProjectileComputeNode {
    fn update(&mut self, world: &mut World) {
        let Some(pipeline) = world.get_resource::<ProjComputePipeline>() else {
            return;
        };
        let pipeline_cache = world.resource::<PipelineCache>();

        match self.state {
            ProjComputeState::Loading => {
                match pipeline_cache.get_compute_pipeline_state(pipeline.pipeline_id) {
                    CachedPipelineState::Ok(_) => {
                        self.state = ProjComputeState::Ready;
                        info!("Projectile compute pipeline ready");
                    }
                    CachedPipelineState::Err(PipelineCacheError::ShaderNotLoaded(_)) => {}
                    CachedPipelineState::Err(err) => {
                        panic!("Projectile compute shader error: {err}")
                    }
                    _ => {}
                }
            }
            ProjComputeState::Ready => {}
        }
    }

    fn run(
        &self,
        _graph: &mut render_graph::RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), render_graph::NodeRunError> {
        if !matches!(self.state, ProjComputeState::Ready) {
            return Ok(());
        }

        let Some(bind_groups) = world.get_resource::<ProjBindGroups>() else {
            return Ok(());
        };
        let Some(proj_data) = world.get_resource::<ProjGpuData>() else {
            return Ok(());
        };
        let pipeline_cache = world.resource::<PipelineCache>();
        let pipeline = world.resource::<ProjComputePipeline>();

        let proj_count = proj_data.proj_count;
        if proj_count == 0 {
            return Ok(());
        }

        let Some(compute_pipeline) = pipeline_cache.get_compute_pipeline(pipeline.pipeline_id)
        else {
            return Ok(());
        };

        // Scope compute pass so it drops before we use the encoder for copy
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());

            pass.set_bind_group(0, &bind_groups.bind_group, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(
                (proj_count + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE,
                1,
                1,
            );
        }

        // Copy hits + positions → staging[current] for CPU readback next frame
        let proj_buffers = world.resource::<ProjGpuBuffers>();
        let si = world.resource::<StagingIndex>().current;
        let hit_copy_size = (proj_count as u64) * std::mem::size_of::<[i32; 2]>() as u64;
        render_context.command_encoder().copy_buffer_to_buffer(
            &proj_buffers.hits, 0, &proj_buffers.hit_staging[si], 0, hit_copy_size,
        );
        let pos_copy_size = (proj_count as u64) * std::mem::size_of::<[f32; 2]>() as u64;
        render_context.command_encoder().copy_buffer_to_buffer(
            &proj_buffers.positions, 0, &proj_buffers.position_staging[si], 0, pos_copy_size,
        );

        Ok(())
    }
}
