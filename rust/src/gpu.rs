//! GPU Compute Module - wgpu-based NPC physics via Bevy's render graph.
//!
//! Follows Bevy 0.18's compute_shader_game_of_life.rs pattern.
//! Three-phase dispatch per frame: clear grid → insert NPCs → main logic.
//!
//! Data flow (zero-clone architecture):
//! - Main world: Systems write GpuUpdateMsg → GPU_UPDATE_QUEUE
//! - PostUpdate: populate_gpu_state drains queue → NpcGpuState
//! - PostUpdate: build_visual_upload packs ECS + NpcGpuState → NpcVisualUpload
//! - Extract: extract_npc_data reads both via Extract<Res<T>> (immutable, zero clone)
//!   → writes compute data per-dirty-index to NpcGpuBuffers
//!   → writes visual/equip data in bulk to NpcVisualBuffers

use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::{
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        gpu_readback::{Readback, ReadbackComplete},
        render_asset::RenderAssets,
        render_graph::{self, RenderGraph, RenderLabel},
        render_resource::{
            binding_types::{storage_buffer, storage_buffer_read_only, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        storage::{GpuShaderStorageBuffer, ShaderStorageBuffer},
        Render, RenderApp, RenderStartup, RenderSystems,
    },
    shader::PipelineCacheError,
};
use std::borrow::Cow;

use crate::components::{NpcIndex, Faction, Job, Healing, Activity, EquippedWeapon, EquippedHelmet, EquippedArmor, Dead};
use crate::constants::{FOOD_SPRITE, GOLD_SPRITE};
use crate::messages::{GpuUpdate, GPU_UPDATE_QUEUE, ProjGpuUpdate, PROJ_GPU_UPDATE_QUEUE};
use crate::resources::{GameTime, GpuReadState, ProjHitState, ProjPositionState, SlotAllocator, SystemTimings};
use crate::systems::stats::{self, TownUpgrades};
use crate::world::WorldData;

// =============================================================================
// CONSTANTS
// =============================================================================

const SHADER_ASSET_PATH: &str = "shaders/npc_compute.wgsl";
const PROJ_SHADER_ASSET_PATH: &str = "shaders/projectile_compute.wgsl";
const WORKGROUP_SIZE: u32 = 64;
const MAX_NPCS: u32 = 100000;
const MAX_PROJECTILES: u32 = 50_000;
/// 256×256 cells × 128px = 32,768px — covers max 1000×1000 world (32,000px).
const GRID_WIDTH: u32 = 256;
const GRID_HEIGHT: u32 = 256;
const MAX_PER_CELL: u32 = 48;
const HIT_HALF_LENGTH: f32 = 12.0;
const HIT_HALF_WIDTH: f32 = 4.0;

// =============================================================================
// RESOURCES (Main World)
// =============================================================================

/// GPU data extracted to render world each frame. Also serves as the compute uniform buffer.
#[derive(Resource, Clone, ExtractResource, ShaderType)]
pub struct NpcGpuData {
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
    pub proj_max_per_cell: u32,
    pub dodge_unlocked: u32,
    pub threat_radius: f32,
}

impl Default for NpcGpuData {
    fn default() -> Self {
        Self {
            count: 0,
            separation_radius: 20.0,
            separation_strength: 100.0,
            delta: 0.016,
            grid_width: GRID_WIDTH,
            grid_height: GRID_HEIGHT,
            cell_size: 128.0,
            max_per_cell: MAX_PER_CELL,
            arrival_threshold: 8.0,
            mode: 0,
            combat_range: 300.0,
            proj_max_per_cell: MAX_PER_CELL,
            dodge_unlocked: 0,
            threat_radius: 200.0,
        }
    }
}

/// All persistent per-NPC GPU data: compute fields + visual state + dirty tracking.
/// Read via `Extract<Res<NpcGpuState>>` in Extract phase (zero clone, immutable reference).
/// NOT Clone/ExtractResource — never cloned to render world.
#[derive(Resource)]
pub struct NpcGpuState {
    // --- Compute fields (written by game systems via GPU_UPDATE_QUEUE) ---
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
    // --- Visual state (sprite frames + flash, updated by messages) ---
    /// Sprite indices: [col, row, atlas, 0] per NPC, stride 4
    pub sprite_indices: Vec<f32>,
    /// Damage flash intensity: 0.0-1.0 per NPC (decays at 5.0/s)
    pub flash_values: Vec<f32>,
    // --- Flags (bit 0: combat scan enabled) ---
    pub npc_flags: Vec<u32>,
    // --- Per-buffer dirty flags (compute only — visual is rebuilt each frame) ---
    pub dirty_positions: bool,
    pub dirty_targets: bool,
    pub dirty_speeds: bool,
    pub dirty_factions: bool,
    pub dirty_healths: bool,
    pub dirty_arrivals: bool,
    pub dirty_flags: bool,
    // --- Per-index tracking for GPU-authoritative buffers (positions + arrivals) ---
    // GPU compute writes these every frame; CPU only touches them on spawn/teleport/hide/retarget.
    pub position_dirty_indices: Vec<usize>,
    pub arrival_dirty_indices: Vec<usize>,
}

/// GPU-ready packed arrays for NPC visual/equip data. Rebuilt each frame by build_visual_upload.
/// Read via `Extract<Res<NpcVisualUpload>>` in Extract phase (zero clone).
#[derive(Resource, Default)]
pub struct NpcVisualUpload {
    /// [sprite_col, sprite_row, atlas, flash, r, g, b, a] per NPC — matches NpcVisual in npc_render.wgsl
    pub visual_data: Vec<f32>,
    /// [col, row, atlas, pad] × 6 layers per NPC — matches EquipSlot in npc_render.wgsl
    pub equip_data: Vec<f32>,
    /// Number of NPCs packed
    pub npc_count: usize,
}

impl Default for NpcGpuState {
    fn default() -> Self {
        let max = MAX_NPCS as usize;
        Self {
            positions: vec![-9999.0; max * 2],
            targets: vec![0.0; max * 2],
            speeds: vec![100.0; max],
            factions: vec![-1; max],
            healths: vec![0.0; max],
            arrivals: vec![0; max],
            sprite_indices: vec![0.0; max * 4],
            flash_values: vec![0.0; max],
            npc_flags: vec![0; max],
            dirty_positions: false,
            dirty_targets: false,
            dirty_speeds: false,
            dirty_factions: false,
            dirty_healths: false,
            dirty_arrivals: false,
            dirty_flags: false,
            position_dirty_indices: Vec::new(),
            arrival_dirty_indices: Vec::new(),
        }
    }
}

impl NpcGpuState {
    /// Apply a GPU update to the state.
    pub fn apply(&mut self, update: &GpuUpdate) {
        match update {
            GpuUpdate::SetPosition { idx, x, y } => {
                let i = *idx * 2;
                if i + 1 < self.positions.len() {
                    self.positions[i] = *x;
                    self.positions[i + 1] = *y;
                    self.dirty_positions = true;
                    self.position_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::SetTarget { idx, x, y } => {
                let i = *idx * 2;
                if i + 1 < self.targets.len() {
                    self.targets[i] = *x;
                    self.targets[i + 1] = *y;
                    self.dirty_targets = true;
                }
                // Reset arrival flag so GPU resumes movement toward new target
                if *idx < self.arrivals.len() {
                    self.arrivals[*idx] = 0;
                    self.dirty_arrivals = true;
                    self.arrival_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::SetSpeed { idx, speed } => {
                if *idx < self.speeds.len() {
                    self.speeds[*idx] = *speed;
                    self.dirty_speeds = true;
                }
            }
            GpuUpdate::SetFaction { idx, faction } => {
                if *idx < self.factions.len() {
                    self.factions[*idx] = *faction;
                    self.dirty_factions = true;
                }
            }
            GpuUpdate::SetHealth { idx, health } => {
                if *idx < self.healths.len() {
                    self.healths[*idx] = *health;
                    self.dirty_healths = true;
                }
            }
            GpuUpdate::ApplyDamage { idx, amount } => {
                if *idx < self.healths.len() {
                    self.healths[*idx] = (self.healths[*idx] - amount).max(0.0);
                    self.dirty_healths = true;
                }
            }
            GpuUpdate::HideNpc { idx } => {
                let i = *idx * 2;
                if i + 1 < self.positions.len() {
                    self.positions[i] = -9999.0;
                    self.positions[i + 1] = -9999.0;
                    self.dirty_positions = true;
                    self.position_dirty_indices.push(*idx);
                }
            }
            // Visual-only messages — no compute dirty flag
            GpuUpdate::SetSpriteFrame { idx, col, row, atlas } => {
                let i = *idx * 4;
                if i + 3 < self.sprite_indices.len() {
                    self.sprite_indices[i] = *col;
                    self.sprite_indices[i + 1] = *row;
                    self.sprite_indices[i + 2] = *atlas;
                }
            }
            GpuUpdate::SetDamageFlash { idx, intensity } => {
                if *idx < self.flash_values.len() {
                    self.flash_values[*idx] = *intensity;
                }
            }
            GpuUpdate::SetFlags { idx, flags } => {
                if *idx < self.npc_flags.len() {
                    self.npc_flags[*idx] = *flags;
                    self.dirty_flags = true;
                }
            }
        }
    }
}

/// Pack NPC visual + equipment data into GPU-ready arrays for direct upload.
/// Replaces sync_visual_sprites + prepare_npc_buffers visual repack.
/// Runs in PostUpdate after populate_gpu_state (chained).
pub fn build_visual_upload(
    gpu_state: Res<NpcGpuState>,
    gpu_data: Res<NpcGpuData>,
    mut upload: ResMut<NpcVisualUpload>,
    all_npcs: Query<(
        &NpcIndex, &Faction, &Job, &Activity,
        Option<&Healing>,
        Option<&EquippedWeapon>, Option<&EquippedHelmet>, Option<&EquippedArmor>,
    ), Without<Dead>>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("build_visual_upload");
    let npc_count = gpu_data.count as usize;
    upload.npc_count = npc_count;

    // Resize (reuses allocation if already large enough), fill with sentinels
    upload.visual_data.resize(npc_count * 8, -1.0);
    upload.equip_data.resize(npc_count * 24, -1.0);

    // Reset to -1.0 sentinels (phantom slots like waypoints have no ECS entity,
    // so the all_npcs loop below never overwrites them — shader hides when col < 0)
    upload.visual_data[..npc_count * 8].fill(-1.0);
    upload.equip_data[..npc_count * 24].fill(-1.0);

    for (npc_idx, faction, job, activity, healing, weapon, helmet, armor) in all_npcs.iter() {
        let idx = npc_idx.0;
        if idx >= npc_count { continue; }

        // --- Visual data: [sprite_col, sprite_row, atlas, flash, r, g, b, a] ---
        let base = idx * 8;
        upload.visual_data[base]     = gpu_state.sprite_indices.get(idx * 4).copied().unwrap_or(0.0);
        upload.visual_data[base + 1] = gpu_state.sprite_indices.get(idx * 4 + 1).copied().unwrap_or(0.0);
        upload.visual_data[base + 2] = gpu_state.sprite_indices.get(idx * 4 + 2).copied().unwrap_or(0.0);
        upload.visual_data[base + 3] = gpu_state.flash_values.get(idx).copied().unwrap_or(0.0);
        let (r, g, b, a) = if faction.0 == 0 {
            job.color()
        } else {
            crate::constants::raider_faction_color(faction.0)
        };
        upload.visual_data[base + 4] = r;
        upload.visual_data[base + 5] = g;
        upload.visual_data[base + 6] = b;
        upload.visual_data[base + 7] = a;

        // --- Equip data: 6 layers × [col, row, atlas, pad] ---
        let eq = idx * 24;

        // Layer 0: Armor
        let (ac, ar) = armor.map(|a| (a.0, a.1)).unwrap_or((-1.0, 0.0));
        upload.equip_data[eq]     = ac;
        upload.equip_data[eq + 1] = ar;
        upload.equip_data[eq + 2] = 0.0;
        upload.equip_data[eq + 3] = 0.0;

        // Layer 1: Helmet
        let (hc, hr) = helmet.map(|h| (h.0, h.1)).unwrap_or((-1.0, 0.0));
        upload.equip_data[eq + 4] = hc;
        upload.equip_data[eq + 5] = hr;
        upload.equip_data[eq + 6] = 0.0;
        upload.equip_data[eq + 7] = 0.0;

        // Layer 2: Weapon
        let (wc, wr) = weapon.map(|w| (w.0, w.1)).unwrap_or((-1.0, 0.0));
        upload.equip_data[eq + 8] = wc;
        upload.equip_data[eq + 9] = wr;
        upload.equip_data[eq + 10] = 0.0;
        upload.equip_data[eq + 11] = 0.0;

        // Layer 3: Item (food on returning raiders)
        let (ic, ir, ia) = match activity {
            Activity::Returning { has_food: true, .. } => (FOOD_SPRITE.0, FOOD_SPRITE.1, 1.0),
            Activity::Returning { gold, .. } if *gold > 0 => (GOLD_SPRITE.0, GOLD_SPRITE.1, 1.0),
            _ => (-1.0, 0.0, 0.0),
        };
        upload.equip_data[eq + 12] = ic;
        upload.equip_data[eq + 13] = ir;
        upload.equip_data[eq + 14] = ia;
        upload.equip_data[eq + 15] = 0.0;

        // Layer 4: Status (sleep icon)
        let (sc, sr, sa) = if matches!(activity, Activity::Resting) {
            (0.0, 0.0, 3.0)
        } else {
            (-1.0, 0.0, 0.0)
        };
        upload.equip_data[eq + 16] = sc;
        upload.equip_data[eq + 17] = sr;
        upload.equip_data[eq + 18] = sa;
        upload.equip_data[eq + 19] = 0.0;

        // Layer 5: Healing (heal halo)
        let (hlc, hla) = if healing.is_some() { (0.0, 2.0) } else { (-1.0, 0.0) };
        upload.equip_data[eq + 20] = hlc;
        upload.equip_data[eq + 21] = 0.0;
        upload.equip_data[eq + 22] = hla;
        upload.equip_data[eq + 23] = 0.0;
    }

    // Building slots: no ECS entity, sprite data from SetSpriteFrame in sprite_indices
    for idx in 0..npc_count {
        let base = idx * 8;
        if upload.visual_data[base] >= 0.0 { continue; } // already written by NPC loop
        let si = idx * 4;
        let col = gpu_state.sprite_indices.get(si).copied().unwrap_or(-1.0);
        if col < 0.0 { continue; } // truly empty slot
        upload.visual_data[base]     = col;
        upload.visual_data[base + 1] = gpu_state.sprite_indices.get(si + 1).copied().unwrap_or(0.0);
        upload.visual_data[base + 2] = gpu_state.sprite_indices.get(si + 2).copied().unwrap_or(0.0);
        upload.visual_data[base + 3] = 0.0; // no flash
        upload.visual_data[base + 4] = 1.0; // r (white tint)
        upload.visual_data[base + 5] = 1.0; // g
        upload.visual_data[base + 6] = 1.0; // b
        upload.visual_data[base + 7] = 1.0; // a
    }
}

/// Drain GPU_UPDATE_QUEUE and apply updates to NpcGpuState.
/// Runs in main world each frame before extraction.
pub fn populate_gpu_state(mut state: ResMut<NpcGpuState>, time: Res<Time>, slots: Res<SlotAllocator>, timings: Res<SystemTimings>) {
    let _t = timings.scope("populate_gpu");
    // Reset per-buffer dirty flags + GPU-authoritative dirty indices
    state.dirty_positions = false;
    state.dirty_targets = false;
    state.dirty_speeds = false;
    state.dirty_factions = false;
    state.dirty_healths = false;
    state.dirty_arrivals = false;
    state.dirty_flags = false;
    state.position_dirty_indices.clear();
    state.arrival_dirty_indices.clear();

    if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
        for update in queue.drain(..) {
            state.apply(&update);
        }
    }

    // Decay damage flash values (1.0 → 0.0 in ~0.2s)
    let dt = time.delta_secs();
    const FLASH_DECAY_RATE: f32 = 5.0;
    let active = slots.count().min(state.flash_values.len());
    for flash in state.flash_values[..active].iter_mut() {
        if *flash > 0.0 {
            *flash = (*flash - dt * FLASH_DECAY_RATE).max(0.0);
        }
    }
}

// =============================================================================
// PROJECTILE RESOURCES (Main World)
// =============================================================================

/// Projectile GPU data extracted to render world. Also serves as the compute uniform buffer.
#[derive(Resource, Clone, ExtractResource, ShaderType)]
pub struct ProjGpuData {
    pub proj_count: u32,
    pub npc_count: u32,
    pub delta: f32,
    pub hit_half_length: f32,
    pub hit_half_width: f32,
    pub grid_width: u32,
    pub grid_height: u32,
    pub cell_size: f32,
    pub max_per_cell: u32,
    pub mode: u32,
}

impl Default for ProjGpuData {
    fn default() -> Self {
        Self {
            proj_count: 0,
            npc_count: 0,
            delta: 0.016,
            hit_half_length: HIT_HALF_LENGTH,
            hit_half_width: HIT_HALF_WIDTH,
            grid_width: GRID_WIDTH,
            grid_height: GRID_HEIGHT,
            cell_size: 128.0,
            max_per_cell: MAX_PER_CELL,
            mode: 0,
        }
    }
}

/// Projectile buffer data to upload to GPU each frame.
/// Read during Extract via Extract<Res<T>> (zero-clone).
#[derive(Resource)]
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
pub fn populate_proj_buffer_writes(mut writes: ResMut<ProjBufferWrites>, timings: Res<SystemTimings>) {
    let _t = timings.scope("populate_proj");
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
// READBACK (Bevy async GPU→CPU via ShaderStorageBuffer assets)
// =============================================================================

/// Handles to ShaderStorageBuffer assets used as readback targets.
/// Extracted to render world so compute nodes can copy into them.
#[derive(Resource, ExtractResource, Clone)]
pub struct ReadbackHandles {
    pub npc_positions: Handle<ShaderStorageBuffer>,
    pub combat_targets: Handle<ShaderStorageBuffer>,
    pub npc_factions: Handle<ShaderStorageBuffer>,
    pub npc_health: Handle<ShaderStorageBuffer>,
    pub threat_counts: Handle<ShaderStorageBuffer>,
    pub proj_hits: Handle<ShaderStorageBuffer>,
    pub proj_positions: Handle<ShaderStorageBuffer>,
}

/// Create ShaderStorageBuffer readback targets and spawn Readback entities with observers.
fn setup_readback_buffers(
    mut commands: Commands,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
) {
    // Create readback target buffers (COPY_DST for compute→copy, COPY_SRC for Readback to map)
    let npc_pos_buf = {
        let init_pos: Vec<f32> = vec![-9999.0; MAX_NPCS as usize * 2];
        let mut buf = ShaderStorageBuffer::new(bytemuck::cast_slice(&init_pos), RenderAssetUsages::RENDER_WORLD);
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };
    let combat_target_buf = {
        // Initialize with -1 per slot so zeroed memory isn't misread as "target NPC 0"
        let init_targets: Vec<i32> = vec![-1; MAX_NPCS as usize];
        let mut buf = ShaderStorageBuffer::new(bytemuck::cast_slice(&init_targets), RenderAssetUsages::RENDER_WORLD);
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };
    let npc_faction_buf = {
        // Initialize with -1 so unspawned slots aren't misread as faction 0 (player)
        let init_factions: Vec<i32> = vec![-1; MAX_NPCS as usize];
        let mut buf = ShaderStorageBuffer::new(bytemuck::cast_slice(&init_factions), RenderAssetUsages::RENDER_WORLD);
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };
    let npc_health_buf = {
        let mut buf = ShaderStorageBuffer::new(&vec![0u8; MAX_NPCS as usize * 4], RenderAssetUsages::RENDER_WORLD);
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };
    let threat_count_buf = {
        let mut buf = ShaderStorageBuffer::new(&vec![0u8; MAX_NPCS as usize * 4], RenderAssetUsages::RENDER_WORLD);
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };
    let proj_hit_buf = {
        // Initialize with [-1, 0] per slot so zeroed memory isn't misread as "hit NPC 0"
        let init_hits: Vec<[i32; 2]> = vec![[-1, 0]; MAX_PROJECTILES as usize];
        let mut buf = ShaderStorageBuffer::new(bytemuck::cast_slice(&init_hits), RenderAssetUsages::RENDER_WORLD);
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };
    let proj_pos_buf = {
        let mut buf = ShaderStorageBuffer::new(&vec![0u8; MAX_PROJECTILES as usize * 8], RenderAssetUsages::RENDER_WORLD);
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };

    let handles = ReadbackHandles {
        npc_positions: npc_pos_buf.clone(),
        combat_targets: combat_target_buf.clone(),
        npc_factions: npc_faction_buf.clone(),
        npc_health: npc_health_buf.clone(),
        threat_counts: threat_count_buf.clone(),
        proj_hits: proj_hit_buf.clone(),
        proj_positions: proj_pos_buf.clone(),
    };
    commands.insert_resource(handles);

    // Spawn Readback entities — Bevy async-reads each frame, triggers ReadbackComplete
    commands.spawn(Readback::buffer(npc_pos_buf))
        .observe(|event: On<ReadbackComplete>, mut state: ResMut<GpuReadState>| {
            let data: Vec<f32> = event.to_shader_type();
            // Don't overwrite npc_count — buffer is MAX-sized, actual count comes from SlotAllocator
            state.positions = data;
        });

    commands.spawn(Readback::buffer(combat_target_buf))
        .observe(|event: On<ReadbackComplete>, mut state: ResMut<GpuReadState>| {
            let data: Vec<i32> = event.to_shader_type();
            state.combat_targets = data;
        });

    commands.spawn(Readback::buffer(npc_faction_buf))
        .observe(|event: On<ReadbackComplete>, mut state: ResMut<GpuReadState>| {
            let data: Vec<i32> = event.to_shader_type();
            state.factions = data;
        });

    commands.spawn(Readback::buffer(npc_health_buf))
        .observe(|event: On<ReadbackComplete>, mut state: ResMut<GpuReadState>| {
            let data: Vec<f32> = event.to_shader_type();
            state.health = data;
        });

    commands.spawn(Readback::buffer(threat_count_buf))
        .observe(|event: On<ReadbackComplete>, mut state: ResMut<GpuReadState>| {
            let data: Vec<u32> = event.to_shader_type();
            state.threat_counts = data;
        });

    commands.spawn(Readback::buffer(proj_hit_buf))
        .observe(|event: On<ReadbackComplete>, mut state: ResMut<ProjHitState>| {
            let data: Vec<[i32; 2]> = event.to_shader_type();
            state.0 = data;
        });

    commands.spawn(Readback::buffer(proj_pos_buf))
        .observe(|event: On<ReadbackComplete>, mut state: ResMut<ProjPositionState>| {
            let data: Vec<f32> = event.to_shader_type();
            state.0 = data;
        });
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
            .init_resource::<NpcGpuState>()
            .init_resource::<NpcVisualUpload>()
            .init_resource::<NpcSpriteTexture>()
            .add_systems(Update, update_gpu_data)
            .add_systems(PostUpdate, (populate_gpu_state, build_visual_upload).chain());

        // Initialize projectile resources in main world
        app.init_resource::<ProjGpuData>()
            .init_resource::<ProjBufferWrites>()
            .add_systems(Update, update_proj_gpu_data)
            .add_systems(PostUpdate, populate_proj_buffer_writes);

        // Async readback: create ShaderStorageBuffer assets + Readback entities
        app.add_systems(Startup, setup_readback_buffers);

        // Extract resources to render world
        // NpcGpuState + NpcVisualUpload + ProjBufferWrites + ProjPositionState use Extract<Res<T>> (zero-clone)
        app.add_plugins((
            ExtractResourcePlugin::<NpcGpuData>::default(),
            ExtractResourcePlugin::<NpcSpriteTexture>::default(),
            ExtractResourcePlugin::<ProjGpuData>::default(),
            ExtractResourcePlugin::<ReadbackHandles>::default(),
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
                (prepare_npc_bind_groups, prepare_proj_bind_groups).in_set(RenderSystems::PrepareBindGroups),
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
    slots: Res<SlotAllocator>,
    time: Res<Time>,
    game_time: Res<GameTime>,
    upgrades: Res<TownUpgrades>,
    world_data: Res<WorldData>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("update_gpu_data");
    let dt = if game_time.paused { 0.0 } else { time.delta_secs() };
    gpu_data.count = slots.count() as u32;
    gpu_data.delta = dt;

    let player_town_idx = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);
    let levels = upgrades.town_levels(player_town_idx);
    gpu_data.dodge_unlocked = if stats::dodge_unlocked(&levels) { 1 } else { 0 };
}

// =============================================================================
// RENDER WORLD RESOURCES
// =============================================================================

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
    pub threat_counts: Buffer,
    pub npc_flags: Buffer,
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
    pub heal_handle: Option<Handle<Image>>,
    pub sleep_handle: Option<Handle<Image>>,
    pub arrow_handle: Option<Handle<Image>>,
    pub building_handle: Option<Handle<Image>>,
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
    pub grid_counts: Buffer,
    pub grid_data: Buffer,
}

/// Bind groups for projectile compute pass (3 modes: clear grid, build grid, movement+collision).
#[derive(Resource)]
struct ProjBindGroups {
    mode0: BindGroup,
    mode1: BindGroup,
    mode2: BindGroup,
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
        positions: render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("npc_positions"),
            contents: bytemuck::cast_slice(&vec![-9999.0f32; MAX_NPCS as usize * 2]),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
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
        factions: render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("factions"),
            contents: bytemuck::cast_slice(&vec![-1i32; MAX_NPCS as usize]),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
        }),
        healths: render_device.create_buffer(&BufferDescriptor {
            label: Some("healths"),
            size: (MAX_NPCS as usize * std::mem::size_of::<f32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        combat_targets: render_device.create_buffer(&BufferDescriptor {
            label: Some("combat_targets"),
            size: (MAX_NPCS as usize * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        threat_counts: render_device.create_buffer(&BufferDescriptor {
            label: Some("threat_counts"),
            size: (MAX_NPCS as usize * std::mem::size_of::<u32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        npc_flags: render_device.create_buffer(&BufferDescriptor {
            label: Some("npc_flags"),
            size: (MAX_NPCS as usize * std::mem::size_of::<u32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
    };

    commands.insert_resource(buffers);

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
                uniform_buffer::<NpcGpuData>(false),
                // 11-12: projectile spatial grid (read only from NPC perspective)
                storage_buffer_read_only::<Vec<i32>>(false),  // proj_grid_counts
                storage_buffer_read_only::<Vec<i32>>(false),  // proj_grid_data
                // 13-15: projectile data (read only)
                storage_buffer_read_only::<Vec<[f32; 2]>>(false), // proj_positions
                storage_buffer_read_only::<Vec<[f32; 2]>>(false), // proj_velocities
                storage_buffer_read_only::<Vec<i32>>(false),      // proj_factions
                // 16: threat counts output (packed enemies<<16 | allies)
                storage_buffer::<Vec<u32>>(false),
                // 17: npc_flags (bit 0: combat scan enabled)
                storage_buffer_read_only::<Vec<u32>>(false),
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
    proj_buffers: Option<Res<ProjGpuBuffers>>,
    gpu_data: Option<Res<NpcGpuData>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline_cache: Res<PipelineCache>,
) {
    use std::sync::atomic::Ordering;
    use crate::messages::{RENDER_PROFILING, RENDER_TIMINGS, RT_NPC_BINDS};
    let profiling = RENDER_PROFILING.load(Ordering::Relaxed);
    let start = if profiling { Some(std::time::Instant::now()) } else { None };

    let Some(pipeline) = pipeline else { return };
    let Some(buffers) = buffers else { return };
    let Some(proj) = proj_buffers else { return };
    let Some(params) = gpu_data else { return };

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

    // Projectile grid + data bindings (read-only from NPC compute)
    let proj_bind = (
        proj.grid_counts.as_entire_buffer_binding(),
        proj.grid_data.as_entire_buffer_binding(),
        proj.positions.as_entire_buffer_binding(),
        proj.velocities.as_entire_buffer_binding(),
        proj.factions.as_entire_buffer_binding(),
    );

    let threat_bind = buffers.threat_counts.as_entire_buffer_binding();
    let flags_bind = buffers.npc_flags.as_entire_buffer_binding();

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
            proj_bind.0.clone(), proj_bind.1.clone(),
            proj_bind.2.clone(), proj_bind.3.clone(), proj_bind.4.clone(),
            threat_bind.clone(),
            flags_bind.clone(),
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
            proj_bind.0.clone(), proj_bind.1.clone(),
            proj_bind.2.clone(), proj_bind.3.clone(), proj_bind.4.clone(),
            threat_bind.clone(),
            flags_bind.clone(),
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
            proj_bind.0.clone(), proj_bind.1.clone(),
            proj_bind.2.clone(), proj_bind.3.clone(), proj_bind.4.clone(),
            threat_bind.clone(),
            flags_bind.clone(),
        )),
    );

    commands.insert_resource(NpcBindGroups { mode0, mode1, mode2 });

    if let Some(s) = start {
        RENDER_TIMINGS[RT_NPC_BINDS].store((s.elapsed().as_secs_f64() as f32 * 1000.0).to_bits(), Ordering::Relaxed);
    }
}


// write_npc_buffers DELETED — logic moved to extract_npc_data (npc_render.rs, ExtractSchedule)

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
        use std::sync::atomic::Ordering;
        use crate::messages::{RENDER_PROFILING, RENDER_TIMINGS, RT_GPU_COMPUTE};
        let profiling = RENDER_PROFILING.load(Ordering::Relaxed);
        let start = if profiling { Some(std::time::Instant::now()) } else { None };

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

        let npc_count = gpu_data.count;
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

        // Copy positions + combat_targets → readback ShaderStorageBuffer assets
        // Bevy's Readback component will async-read these and fire ReadbackComplete
        let buffers = world.resource::<NpcGpuBuffers>();
        let handles = world.resource::<ReadbackHandles>();
        let render_assets = world.resource::<RenderAssets<GpuShaderStorageBuffer>>();

        let pos_copy_size = (npc_count as u64) * std::mem::size_of::<[f32; 2]>() as u64;
        let ct_copy_size = (npc_count as u64) * std::mem::size_of::<i32>() as u64;

        if let Some(rb_pos) = render_assets.get(&handles.npc_positions) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &buffers.positions, 0, &rb_pos.buffer, 0, pos_copy_size,
            );
        }
        if let Some(rb_ct) = render_assets.get(&handles.combat_targets) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &buffers.combat_targets, 0, &rb_ct.buffer, 0, ct_copy_size,
            );
        }

        let i32_copy_size = (npc_count as u64) * std::mem::size_of::<i32>() as u64;
        let f32_copy_size = (npc_count as u64) * std::mem::size_of::<f32>() as u64;

        if let Some(rb_fac) = render_assets.get(&handles.npc_factions) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &buffers.factions, 0, &rb_fac.buffer, 0, i32_copy_size,
            );
        }
        if let Some(rb_hp) = render_assets.get(&handles.npc_health) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &buffers.healths, 0, &rb_hp.buffer, 0, f32_copy_size,
            );
        }
        let u32_copy_size = (npc_count as u64) * std::mem::size_of::<u32>() as u64;
        if let Some(rb_tc) = render_assets.get(&handles.threat_counts) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &buffers.threat_counts, 0, &rb_tc.buffer, 0, u32_copy_size,
            );
        }

        if let Some(s) = start {
            RENDER_TIMINGS[RT_GPU_COMPUTE].store((s.elapsed().as_secs_f64() as f32 * 1000.0).to_bits(), Ordering::Relaxed);
        }
        Ok(())
    }
}

// =============================================================================
// PROJECTILE COMPUTE
// =============================================================================

/// Update projectile GPU data from ECS each frame.
fn update_proj_gpu_data(
    mut proj_data: ResMut<ProjGpuData>,
    slots: Res<SlotAllocator>,
    proj_alloc: Res<crate::resources::ProjSlotAllocator>,
    time: Res<Time>,
    game_time: Res<GameTime>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("update_proj_gpu");
    let dt = if game_time.paused { 0.0 } else { time.delta_secs() };
    proj_data.proj_count = proj_alloc.next as u32;
    proj_data.npc_count = slots.count() as u32;
    proj_data.delta = dt;
}

fn init_proj_compute_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let max = MAX_PROJECTILES as usize;
    let grid_cells = (GRID_WIDTH * GRID_HEIGHT) as usize;
    let grid_data_size = grid_cells * MAX_PER_CELL as usize;

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
        grid_counts: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_grid_counts"),
            size: (grid_cells * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE,
            mapped_at_creation: false,
        }),
        grid_data: render_device.create_buffer(&BufferDescriptor {
            label: Some("proj_grid_data"),
            size: (grid_data_size * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE,
            mapped_at_creation: false,
        }),
    };

    commands.insert_resource(buffers);

    // 16 bindings: 8 proj (rw) + 3 NPC (ro) + 2 NPC grid (ro) + 1 uniform + 2 proj grid (rw)
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
                // 11-12: NPC spatial grid (read only)
                storage_buffer_read_only::<Vec<i32>>(false),      // grid_counts
                storage_buffer_read_only::<Vec<i32>>(false),      // grid_data
                // 13: uniform params
                uniform_buffer::<ProjGpuData>(false),
                // 14-15: projectile spatial grid (read_write)
                storage_buffer::<Vec<i32>>(false),                // proj_grid_counts
                storage_buffer::<Vec<i32>>(false),                // proj_grid_data
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
    params: Option<Res<ProjGpuData>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline_cache: Res<PipelineCache>,
) {
    use std::sync::atomic::Ordering;
    use crate::messages::{RENDER_PROFILING, RENDER_TIMINGS, RT_PROJ_BINDS};
    let profiling = RENDER_PROFILING.load(Ordering::Relaxed);
    let start = if profiling { Some(std::time::Instant::now()) } else { None };

    let Some(pipeline) = pipeline else { return };
    let Some(proj) = proj_buffers else { return };
    let Some(npc) = npc_buffers else { return };
    let Some(params) = params else { return };

    let layout = &pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout);
    let storage_bindings = (
        proj.positions.as_entire_buffer_binding(),
        proj.velocities.as_entire_buffer_binding(),
        proj.damages.as_entire_buffer_binding(),
        proj.factions.as_entire_buffer_binding(),
        proj.shooters.as_entire_buffer_binding(),
        proj.lifetimes.as_entire_buffer_binding(),
        proj.active.as_entire_buffer_binding(),
        proj.hits.as_entire_buffer_binding(),
        npc.positions.as_entire_buffer_binding(),
        npc.factions.as_entire_buffer_binding(),
        npc.healths.as_entire_buffer_binding(),
        npc.grid_counts.as_entire_buffer_binding(),
        npc.grid_data.as_entire_buffer_binding(),
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
        Some("proj_compute_bg_mode0"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(), storage_bindings.1.clone(),
            storage_bindings.2.clone(), storage_bindings.3.clone(),
            storage_bindings.4.clone(), storage_bindings.5.clone(),
            storage_bindings.6.clone(), storage_bindings.7.clone(),
            storage_bindings.8.clone(), storage_bindings.9.clone(),
            storage_bindings.10.clone(), storage_bindings.11.clone(),
            storage_bindings.12.clone(),
            &ub0,
            proj.grid_counts.as_entire_buffer_binding(),
            proj.grid_data.as_entire_buffer_binding(),
        )),
    );
    let mode1 = render_device.create_bind_group(
        Some("proj_compute_bg_mode1"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(), storage_bindings.1.clone(),
            storage_bindings.2.clone(), storage_bindings.3.clone(),
            storage_bindings.4.clone(), storage_bindings.5.clone(),
            storage_bindings.6.clone(), storage_bindings.7.clone(),
            storage_bindings.8.clone(), storage_bindings.9.clone(),
            storage_bindings.10.clone(), storage_bindings.11.clone(),
            storage_bindings.12.clone(),
            &ub1,
            proj.grid_counts.as_entire_buffer_binding(),
            proj.grid_data.as_entire_buffer_binding(),
        )),
    );
    let mode2 = render_device.create_bind_group(
        Some("proj_compute_bg_mode2"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(), storage_bindings.1.clone(),
            storage_bindings.2.clone(), storage_bindings.3.clone(),
            storage_bindings.4.clone(), storage_bindings.5.clone(),
            storage_bindings.6.clone(), storage_bindings.7.clone(),
            storage_bindings.8.clone(), storage_bindings.9.clone(),
            storage_bindings.10.clone(), storage_bindings.11.clone(),
            storage_bindings.12.clone(),
            &ub2,
            proj.grid_counts.as_entire_buffer_binding(),
            proj.grid_data.as_entire_buffer_binding(),
        )),
    );

    commands.insert_resource(ProjBindGroups { mode0, mode1, mode2 });

    if let Some(s) = start {
        RENDER_TIMINGS[RT_PROJ_BINDS].store((s.elapsed().as_secs_f64() as f32 * 1000.0).to_bits(), Ordering::Relaxed);
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
        use std::sync::atomic::Ordering;
        use crate::messages::{RENDER_PROFILING, RENDER_TIMINGS, RT_PROJ_COMPUTE};
        let profiling = RENDER_PROFILING.load(Ordering::Relaxed);
        let start = if profiling { Some(std::time::Instant::now()) } else { None };

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

        let grid_cells = GRID_WIDTH * GRID_HEIGHT;
        let grid_wg = (grid_cells + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
        let proj_wg = (proj_count + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;

        // Pass 0: Clear projectile spatial grid
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_bind_group(0, &bind_groups.mode0, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(grid_wg, 1, 1);
        }

        // Pass 1: Build projectile spatial grid (insert active projectiles)
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_bind_group(0, &bind_groups.mode1, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(proj_wg, 1, 1);
        }

        // Pass 2: Movement + collision detection
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_bind_group(0, &bind_groups.mode2, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(proj_wg, 1, 1);
        }

        // Copy hits + positions → readback ShaderStorageBuffer assets
        let proj_buffers = world.resource::<ProjGpuBuffers>();
        let handles = world.resource::<ReadbackHandles>();
        let render_assets = world.resource::<RenderAssets<GpuShaderStorageBuffer>>();

        let hit_copy_size = (proj_count as u64) * std::mem::size_of::<[i32; 2]>() as u64;
        if let Some(rb_hits) = render_assets.get(&handles.proj_hits) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &proj_buffers.hits, 0, &rb_hits.buffer, 0, hit_copy_size,
            );
        }
        let pos_copy_size = (proj_count as u64) * std::mem::size_of::<[f32; 2]>() as u64;
        if let Some(rb_pos) = render_assets.get(&handles.proj_positions) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &proj_buffers.positions, 0, &rb_pos.buffer, 0, pos_copy_size,
            );
        }

        if let Some(s) = start {
            RENDER_TIMINGS[RT_PROJ_COMPUTE].store((s.elapsed().as_secs_f64() as f32 * 1000.0).to_bits(), Ordering::Relaxed);
        }
        Ok(())
    }
}
