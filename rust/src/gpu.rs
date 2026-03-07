//! GPU Compute Module - wgpu-based NPC physics via Bevy's render graph.
//!
//! Follows Bevy 0.18's compute_shader_game_of_life.rs pattern.
//! Three-phase dispatch per frame: clear grid → insert NPCs → main logic.
//!
//! Data flow (zero-clone architecture):
//! - Main world: Systems write GpuUpdateMsg
//! - PostUpdate: populate_gpu_state reads messages -> EntityGpuState
//! - PostUpdate: build_visual_upload updates dirty visual slots (event-driven, not full rebuild)
//! - Extract: extract_npc_data reads both via Extract<Res<T>> (immutable, zero clone)
//!   → writes compute data per-dirty-index to EntityGpuBuffers
//!   → writes visual/equip data in bulk to NpcVisualBuffers

use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
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
    },
    shader::PipelineCacheError,
};
use std::borrow::Cow;

use crate::components::{Activity, Building, Dead, Faction, GpuSlot, Job};
use crate::constants::{
    FOOD_SPRITE, GOLD_SPRITE, MAX_ENTITIES, MAX_NPC_COUNT,
    MAX_PROJECTILES as MAX_PROJECTILE_COUNT, PROJECTILE_HIT_HALF_LENGTH, PROJECTILE_HIT_HALF_WIDTH,
};
use crate::messages::{GpuUpdate, GpuUpdateMsg, ProjGpuUpdate, ProjGpuUpdateMsg};
use crate::resources::{
    GameTime, GpuReadState, GpuSlotPool, NpcTargetThrashDebug, ProjHitState, ProjPositionState,
};
use crate::systems::stats::{self, TownUpgrades};
use crate::world::WorldData;

// =============================================================================
// CONSTANTS
// =============================================================================

const SHADER_ASSET_PATH: &str = "shaders/npc_compute.wgsl";
const PROJ_SHADER_ASSET_PATH: &str = "shaders/projectile_compute.wgsl";
const WORKGROUP_SIZE: u32 = 64;
/// 256×256 cells × 128px = 32,768px — covers max 1000×1000 world (32,000px).
const GRID_WIDTH: u32 = 256;
const GRID_HEIGHT: u32 = 256;
const MAX_PER_CELL: u32 = 48;

// =============================================================================
// RESOURCES (Main World)
// =============================================================================

/// NPC compute uniform buffer fields. Owned by RenderFrameConfig.
#[derive(Clone, ShaderType)]
pub struct EntityGpuData {
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
    pub tile_grid_width: u32,
    pub tile_grid_height: u32,
    pub tile_cell_size: f32,
    pub entity_count: u32,
}

impl Default for EntityGpuData {
    fn default() -> Self {
        Self {
            count: 0,
            separation_radius: 40.0,
            separation_strength: 200.0,
            delta: 0.016,
            grid_width: GRID_WIDTH,
            grid_height: GRID_HEIGHT,
            cell_size: 128.0,
            max_per_cell: MAX_PER_CELL,
            arrival_threshold: 8.0,
            mode: 0,
            combat_range: 400.0,
            proj_max_per_cell: MAX_PER_CELL,
            dodge_unlocked: 0,
            threat_radius: 400.0,
            tile_grid_width: 0,
            tile_grid_height: 0,
            tile_cell_size: 64.0,
            entity_count: 0,
        }
    }
}

/// Single extracted resource carrying all per-frame render config.
/// Replaces 4 separate ExtractResourcePlugin registrations with 1.
#[derive(Resource, Clone, ExtractResource, Default)]
pub struct RenderFrameConfig {
    pub npc: EntityGpuData,
    pub proj: ProjGpuData,
    pub textures: NpcSpriteTexture,
    pub readback: ReadbackHandles,
    pub tile_flags: Vec<u32>,
}

/// All persistent per-entity GPU data: compute fields + visual state + dirty tracking.
/// Unified buffer: NPCs and buildings share the same arrays at their slot index.
/// Read via `Extract<Res<EntityGpuState>>` in Extract phase (zero clone, immutable reference).
/// NOT Clone/ExtractResource — never cloned to render world.
#[derive(Resource)]
pub struct EntityGpuState {
    // --- Compute fields (written by game systems via GpuUpdateMsg) ---
    /// Position buffer: [x0, y0, x1, y1, ...] flattened
    pub positions: Vec<f32>,
    /// Target buffer: [x0, y0, x1, y1, ...] flattened
    pub targets: Vec<f32>,
    /// Speed buffer: one f32 per NPC
    pub speeds: Vec<f32>,
    /// Faction buffer: one i32 per NPC
    pub factions: Vec<i32>,
    /// Health buffer: one f32 per NPC (normalized 0.0–1.0)
    pub healths: Vec<f32>,
    /// Max health per slot (CPU-only, used to normalize SetHealth values)
    pub max_healths: Vec<f32>,
    /// Arrival flags: one i32 per NPC (0 = moving, 1 = settled)
    pub arrivals: Vec<i32>,
    // --- Visual state (sprite frames + flash, updated by messages) ---
    /// Sprite indices: [col, row, atlas, 0] per NPC, stride 4
    pub sprite_indices: Vec<f32>,
    /// Damage flash intensity: 0.0-1.0 per NPC (decays at 5.0/s)
    pub flash_values: Vec<f32>,
    // --- Flags (bit 0: combat scan enabled) ---
    pub entity_flags: Vec<u32>,
    /// Hitbox half-sizes: [half_w, half_h] per entity (interleaved, stride 2)
    pub half_sizes: Vec<f32>,
    // --- Per-index dirty tracking (all buffers) ---
    // Pre-sorted and deduped in populate_gpu_state for coalesced GPU uploads in extract.
    //
    // AUTHORITY CONTRACT:
    // - positions, arrivals: GPU-AUTHORITATIVE between GpuUpdate events.
    //   CPU array holds only spawn/teleport/hide values. Uploads must never
    //   include non-dirty slots (use strict coalescing, not gap-based).
    // - All other buffers (targets, speeds, factions, healths, flags, half_sizes):
    //   CPU-AUTHORITATIVE. EntityGpuState always holds ground truth.
    //   Gap-based coalescing is safe for these.
    pub dirty_targets: bool,
    pub position_dirty_indices: Vec<usize>,
    pub arrival_dirty_indices: Vec<usize>,
    pub target_dirty_indices: Vec<usize>,
    pub speed_dirty_indices: Vec<usize>,
    pub faction_dirty_indices: Vec<usize>,
    pub health_dirty_indices: Vec<usize>,
    pub flags_dirty_indices: Vec<usize>,
    pub half_size_dirty_indices: Vec<usize>,
    /// Slots hidden this frame — used by build_visual_upload to clear stale visual/equip data.
    pub hidden_indices: Vec<usize>,
    /// Last-known target buffer size for full-upload fallback detection.
    pub target_buffer_size: usize,
    // --- Visual dirty tracking (event-driven visual upload) ---
    /// Slots whose visual/equip data changed this frame (sprite, activity, equipment changes)
    pub visual_dirty_indices: Vec<usize>,
    /// Slots dirty from flash decay only — need visual_data[flash] update but NOT equip
    pub flash_only_indices: Vec<usize>,
    /// Force full visual rebuild (startup, load, reset)
    pub visual_full_rebuild: bool,
}

/// GPU-ready packed arrays for NPC visual/equip data. Persistent across frames; only dirty slots updated.
/// Read via `Extract<Res<NpcVisualUpload>>` in Extract phase (zero clone).
#[derive(Resource, Default)]
pub struct NpcVisualUpload {
    /// [sprite_col, sprite_row, atlas, flash, r, g, b, a] per NPC — matches NpcVisual in npc_render.wgsl
    pub visual_data: Vec<f32>,
    /// [col, row, atlas, pad] × 6 layers per NPC — matches EquipSlot in npc_render.wgsl
    pub equip_data: Vec<f32>,
    /// Number of entities packed
    pub entity_count: usize,
    /// Slots whose visual data was written this frame (for per-index GPU upload)
    pub visual_uploaded_indices: Vec<usize>,
    /// Slots whose equip data was written this frame (subset — excludes flash-only changes)
    pub equip_uploaded_indices: Vec<usize>,
    /// True when full visual rebuild happened (startup/load) — extract should do bulk upload
    pub visual_full_upload: bool,
}

impl Default for EntityGpuState {
    fn default() -> Self {
        let max = MAX_ENTITIES;
        Self {
            positions: vec![-9999.0; max * 2],
            targets: vec![0.0; max * 2],
            speeds: vec![0.0; max],
            factions: vec![-1; max],
            healths: vec![0.0; max],
            max_healths: vec![100.0; max],
            arrivals: vec![0; max],
            sprite_indices: vec![0.0; max * 4],
            flash_values: vec![0.0; max],
            entity_flags: vec![0; max],
            half_sizes: vec![0.0; max * 2],
            dirty_targets: false,
            position_dirty_indices: Vec::new(),
            arrival_dirty_indices: Vec::new(),
            target_dirty_indices: Vec::new(),
            speed_dirty_indices: Vec::new(),
            faction_dirty_indices: Vec::new(),
            health_dirty_indices: Vec::new(),
            flags_dirty_indices: Vec::new(),
            half_size_dirty_indices: Vec::new(),
            hidden_indices: Vec::new(),
            target_buffer_size: 0,
            visual_dirty_indices: Vec::new(),
            flash_only_indices: Vec::new(),
            visual_full_rebuild: true,
        }
    }
}

impl EntityGpuState {
    /// Apply a GPU update to the state.
    pub fn apply(&mut self, update: &GpuUpdate) {
        match update {
            GpuUpdate::SetPosition { idx, x, y } => {
                let i = *idx * 2;
                if i + 1 < self.positions.len() {
                    self.positions[i] = *x;
                    self.positions[i + 1] = *y;
                    self.position_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::SetTarget { idx, x, y } => {
                let i = *idx * 2;
                if i + 1 < self.targets.len() {
                    self.targets[i] = *x;
                    self.targets[i + 1] = *y;
                    self.dirty_targets = true;
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
                    self.speed_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::SetFaction { idx, faction } => {
                if *idx < self.factions.len() {
                    self.factions[*idx] = *faction;
                    self.faction_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::SetMaxHealth { idx, max_health } => {
                if *idx < self.max_healths.len() {
                    self.max_healths[*idx] = *max_health;
                }
            }
            GpuUpdate::SetHealth { idx, health } => {
                if *idx < self.healths.len() {
                    let max = self.max_healths.get(*idx).copied().unwrap_or(100.0).max(1.0);
                    self.healths[*idx] = *health / max;
                    self.health_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::ApplyDamage { idx, amount } => {
                if *idx < self.healths.len() {
                    let max = self.max_healths.get(*idx).copied().unwrap_or(100.0).max(1.0);
                    self.healths[*idx] = (self.healths[*idx] - amount / max).max(0.0);
                    self.health_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::Hide { idx } => {
                let i = *idx * 2;
                if i + 1 < self.positions.len() {
                    self.positions[i] = -9999.0;
                    self.positions[i + 1] = -9999.0;
                    self.position_dirty_indices.push(*idx);
                }
                // Clear sprite/flash state so reused slots don't show ghost visuals
                let si = *idx * 4;
                if si + 3 < self.sprite_indices.len() {
                    self.sprite_indices[si] = -1.0;
                    self.sprite_indices[si + 1] = 0.0;
                    self.sprite_indices[si + 2] = 0.0;
                    self.sprite_indices[si + 3] = 0.0;
                }
                if *idx < self.flash_values.len() {
                    self.flash_values[*idx] = 0.0;
                }
                self.hidden_indices.push(*idx);
                self.visual_dirty_indices.push(*idx);
            }
            // Visual-only messages — no compute dirty flag
            GpuUpdate::SetSpriteFrame {
                idx,
                col,
                row,
                atlas,
            } => {
                let i = *idx * 4;
                if i + 3 < self.sprite_indices.len() {
                    self.sprite_indices[i] = *col;
                    self.sprite_indices[i + 1] = *row;
                    self.sprite_indices[i + 2] = *atlas;
                    self.visual_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::SetDamageFlash { idx, intensity } => {
                if *idx < self.flash_values.len() {
                    self.flash_values[*idx] = *intensity;
                    self.visual_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::MarkVisualDirty { idx } => {
                self.visual_dirty_indices.push(*idx);
            }
            GpuUpdate::SetFlags { idx, flags } => {
                if *idx < self.entity_flags.len() {
                    self.entity_flags[*idx] = *flags;
                    self.flags_dirty_indices.push(*idx);
                }
            }
            GpuUpdate::SetHalfSize {
                idx,
                half_w,
                half_h,
            } => {
                let i = *idx * 2;
                if i + 1 < self.half_sizes.len() {
                    self.half_sizes[i] = *half_w;
                    self.half_sizes[i + 1] = *half_h;
                    self.half_size_dirty_indices.push(*idx);
                }
            }
        }
    }
}

// BuildingGpuState removed — buildings use EntityGpuState at their unified slot index.

/// Write NPC visual + equip data for a single slot into upload buffers.
#[inline]
fn write_npc_visual(
    idx: usize,
    entity: Entity,
    job: &Job,
    faction: i32,
    gpu_state: &EntityGpuState,
    upload: &mut NpcVisualUpload,
    activity_q: &Query<&crate::components::Activity>,
    npc_flags_q: &Query<&crate::components::NpcFlags>,
    equipment_q: &Query<(&crate::components::NpcEquipment, &crate::components::Job)>,
    carried_loot_q: &Query<&crate::components::CarriedLoot>,
) {
    let base = idx * 8;
    if base + 7 >= upload.visual_data.len() {
        return;
    }

    // Visual data: [sprite_col, sprite_row, atlas, flash, r, g, b, a]
    upload.visual_data[base] = gpu_state
        .sprite_indices
        .get(idx * 4)
        .copied()
        .unwrap_or(0.0);
    upload.visual_data[base + 1] = gpu_state
        .sprite_indices
        .get(idx * 4 + 1)
        .copied()
        .unwrap_or(0.0);
    upload.visual_data[base + 2] = gpu_state
        .sprite_indices
        .get(idx * 4 + 2)
        .copied()
        .unwrap_or(0.0);
    upload.visual_data[base + 3] = gpu_state.flash_values.get(idx).copied().unwrap_or(0.0);
    let (r, g, b, a) = if faction == crate::constants::FACTION_PLAYER {
        job.color()
    } else {
        crate::constants::raider_faction_color(faction)
    };
    upload.visual_data[base + 4] = r;
    upload.visual_data[base + 5] = g;
    upload.visual_data[base + 6] = b;
    upload.visual_data[base + 7] = a;

    // Equip data: 7 layers × [col, row, atlas, pad]
    let eq = idx * 28;

    // Layers 0-3: equipment sprites from NpcEquipment (armor, helm, weapon, shield)
    let (ac, ar, hc, hr, wc, wr, sc_eq, sr_eq) = if let Ok((npc_eq, eq_job)) = equipment_q.get(entity) {
        let a = npc_eq.armor_sprite();
        let h = npc_eq.helm_sprite(*eq_job);
        let w = npc_eq.weapon_sprite(*eq_job);
        let s = npc_eq.shield_sprite();
        (a.0, a.1, h.0, h.1, w.0, w.1, s.0, s.1)
    } else {
        (-1.0, 0.0, -1.0, 0.0, -1.0, 0.0, -1.0, 0.0)
    };
    // Layer 0: Armor
    upload.equip_data[eq] = ac;
    upload.equip_data[eq + 1] = ar;
    upload.equip_data[eq + 2] = 0.0;
    upload.equip_data[eq + 3] = 0.0;
    // Layer 1: Helmet
    upload.equip_data[eq + 4] = hc;
    upload.equip_data[eq + 5] = hr;
    upload.equip_data[eq + 6] = 0.0;
    upload.equip_data[eq + 7] = 0.0;
    // Layer 2: Weapon
    upload.equip_data[eq + 8] = wc;
    upload.equip_data[eq + 9] = wr;
    upload.equip_data[eq + 10] = 0.0;
    upload.equip_data[eq + 11] = 0.0;
    // Layer 3: Shield
    upload.equip_data[eq + 12] = sc_eq;
    upload.equip_data[eq + 13] = sr_eq;
    upload.equip_data[eq + 14] = 0.0;
    upload.equip_data[eq + 15] = 0.0;

    // Layer 4: Item (carried loot — gold takes display priority)
    let npc_activity = activity_q.get(entity).ok();
    let (ic, ir, ia) = if let Some(cl) = carried_loot_q.get(entity).ok() {
        if cl.gold > 0 {
            (GOLD_SPRITE.0, GOLD_SPRITE.1, 1.0)
        } else if cl.food > 0 {
            (FOOD_SPRITE.0, FOOD_SPRITE.1, 1.0)
        } else {
            (-1.0, 0.0, 0.0)
        }
    } else {
        (-1.0, 0.0, 0.0)
    };
    upload.equip_data[eq + 16] = ic;
    upload.equip_data[eq + 17] = ir;
    upload.equip_data[eq + 18] = ia;
    upload.equip_data[eq + 19] = 0.0;

    // Layer 5: Status (sleep icon)
    let (sc, sr, sa) = if npc_activity.is_some_and(|a| matches!(*a, Activity::Resting)) {
        (0.0, 0.0, 3.0)
    } else {
        (-1.0, 0.0, 0.0)
    };
    upload.equip_data[eq + 20] = sc;
    upload.equip_data[eq + 21] = sr;
    upload.equip_data[eq + 22] = sa;
    upload.equip_data[eq + 23] = 0.0;

    // Layer 6: Healing (heal halo)
    let is_healing = npc_flags_q.get(entity).is_ok_and(|f| f.healing);
    let (hlc, hla) = if is_healing { (0.0, 2.0) } else { (-1.0, 0.0) };
    upload.equip_data[eq + 24] = hlc;
    upload.equip_data[eq + 25] = 0.0;
    upload.equip_data[eq + 26] = hla;
    upload.equip_data[eq + 27] = 0.0;
}

/// Write building visual data for a single slot into upload buffers.
#[inline]
fn write_building_visual(idx: usize, gpu_state: &EntityGpuState, upload: &mut NpcVisualUpload) {
    let base = idx * 8;
    if base + 7 >= upload.visual_data.len() {
        return;
    }
    let si = idx * 4;
    let col = gpu_state.sprite_indices.get(si).copied().unwrap_or(-1.0);
    if col < 0.0 {
        return;
    } // hidden or uninitialized
    upload.visual_data[base] = col;
    upload.visual_data[base + 1] = gpu_state.sprite_indices.get(si + 1).copied().unwrap_or(0.0);
    upload.visual_data[base + 2] = gpu_state.sprite_indices.get(si + 2).copied().unwrap_or(0.0);
    upload.visual_data[base + 3] = 0.0; // no flash
    upload.visual_data[base + 4] = 1.0; // r (white tint)
    upload.visual_data[base + 5] = 1.0; // g
    upload.visual_data[base + 6] = 1.0; // b
    upload.visual_data[base + 7] = 1.0; // a
    // Wipe stale NPC equip overlays on building slots
    let eq = idx * 28;
    if eq + 27 < upload.equip_data.len() {
        upload.equip_data[eq..eq + 28].fill(-1.0);
    }
}

/// Clear a slot to sentinel values (no visual).
#[inline]
fn clear_visual_slot(idx: usize, upload: &mut NpcVisualUpload) {
    let vbase = idx * 8;
    if vbase + 7 < upload.visual_data.len() {
        upload.visual_data[vbase..vbase + 8].fill(-1.0);
    }
    let ebase = idx * 28;
    if ebase + 27 < upload.equip_data.len() {
        upload.equip_data[ebase..ebase + 28].fill(-1.0);
    }
}

/// Event-driven visual upload: persistent buffers, only dirty slots updated per frame.
/// Full rebuild on startup/load; incremental updates via visual_dirty_indices thereafter.
/// Runs in PostUpdate after populate_gpu_state (chained).
pub fn build_visual_upload(
    mut gpu_state: ResMut<EntityGpuState>,
    slots: Res<GpuSlotPool>,
    mut upload: ResMut<NpcVisualUpload>,
    entity_map: Res<crate::resources::EntityMap>,
    activity_q: Query<&crate::components::Activity>,
    npc_flags_q: Query<&crate::components::NpcFlags>,
    equipment_q: Query<(&crate::components::NpcEquipment, &crate::components::Job)>,
    carried_loot_q: Query<&crate::components::CarriedLoot>,
    npc_q: Query<(Entity, &GpuSlot, &Job, &Faction), (Without<Building>, Without<Dead>)>,
    building_q: Query<&GpuSlot, (With<Building>, Without<Dead>)>,
) {
    // Read live count from authoritative source — not the stale RenderFrameConfig copy
    let entity_count = slots.count();
    upload.entity_count = entity_count;

    // Resize (reuses allocation if already large enough), new tail gets sentinels
    upload.visual_data.resize(entity_count * 8, -1.0);
    upload.equip_data.resize(entity_count * 28, -1.0);

    // Clear hidden slots (despawned entities)
    for &idx in &gpu_state.hidden_indices {
        clear_visual_slot(idx, &mut upload);
    }

    if gpu_state.visual_full_rebuild {
        // Full rebuild: query-first iteration over all live entities
        for (entity, es, job, faction) in npc_q.iter() {
            write_npc_visual(
                es.0,
                entity,
                job,
                faction.0,
                &gpu_state,
                &mut upload,
                &activity_q,
                &npc_flags_q,
                &equipment_q,
                &carried_loot_q,
            );
        }
        for es in building_q.iter() {
            write_building_visual(es.0, &gpu_state, &mut upload);
        }
        gpu_state.visual_full_rebuild = false;
        upload.visual_full_upload = true;
        upload.visual_uploaded_indices.clear();
        upload.equip_uploaded_indices.clear();
    } else {
        // Dirty-only: dedup then update only changed slots
        gpu_state.visual_dirty_indices.sort_unstable();
        gpu_state.visual_dirty_indices.dedup();
        for i in 0..gpu_state.visual_dirty_indices.len() {
            let idx = gpu_state.visual_dirty_indices[i];
            if let Some(npc) = entity_map.get_npc(idx) {
                if npc.dead {
                    clear_visual_slot(idx, &mut upload);
                    continue;
                }
                write_npc_visual(
                    idx,
                    npc.entity,
                    &npc.job,
                    npc.faction,
                    &gpu_state,
                    &mut upload,
                    &activity_q,
                    &npc_flags_q,
                    &equipment_q,
                    &carried_loot_q,
                );
            } else if entity_map.get_instance(idx).is_some() {
                write_building_visual(idx, &gpu_state, &mut upload);
            } else {
                clear_visual_slot(idx, &mut upload);
            }
        }
        upload.visual_full_upload = false;

        // Flash-only slots: update just the flash float in visual_data, skip equip entirely.
        // These are slots whose flash is decaying but nothing else changed (no sprite/activity/equipment change).
        // Use binary_search on the sorted visual_dirty_indices to skip slots already handled above.
        for &idx in &gpu_state.flash_only_indices {
            if gpu_state.visual_dirty_indices.binary_search(&idx).is_ok() {
                continue; // already fully updated above
            }
            let base = idx * 8;
            if base + 3 < upload.visual_data.len() {
                upload.visual_data[base + 3] =
                    gpu_state.flash_values.get(idx).copied().unwrap_or(0.0);
            }
        }

        // Build upload index Vecs: visual includes everything, equip excludes flash-only
        upload.visual_uploaded_indices.clear();
        upload.equip_uploaded_indices.clear();
        // Visual-dirty slots need both visual + equip upload
        upload
            .visual_uploaded_indices
            .extend_from_slice(&gpu_state.visual_dirty_indices);
        upload
            .equip_uploaded_indices
            .extend_from_slice(&gpu_state.visual_dirty_indices);
        // Flash-only slots need visual upload only (not equip)
        for &idx in &gpu_state.flash_only_indices {
            if gpu_state.visual_dirty_indices.binary_search(&idx).is_err() {
                upload.visual_uploaded_indices.push(idx);
            }
        }
        // Hidden indices need both visual + equip upload
        upload
            .visual_uploaded_indices
            .extend_from_slice(&gpu_state.hidden_indices);
        upload
            .equip_uploaded_indices
            .extend_from_slice(&gpu_state.hidden_indices);
        upload.visual_uploaded_indices.sort_unstable();
        upload.visual_uploaded_indices.dedup();
        upload.equip_uploaded_indices.sort_unstable();
        upload.equip_uploaded_indices.dedup();
    }
    gpu_state.visual_dirty_indices.clear();
}

/// Drain GpuUpdateMsg messages and apply updates to EntityGpuState (unified entity state).
/// Runs in main world each frame before extraction.
pub fn populate_gpu_state(
    mut events: MessageReader<GpuUpdateMsg>,
    mut npc_state: ResMut<EntityGpuState>,
    mut target_thrash: ResMut<NpcTargetThrashDebug>,
    _game_time: Res<GameTime>,
    real_time: Res<Time<Real>>,
    time: Res<Time>,
    mut slots: ResMut<GpuSlotPool>,
) {
    let sink_window_key = real_time.elapsed_secs_f64().floor() as i64;
    // Reset dirty flags and per-index dirty tracking
    npc_state.dirty_targets = false;
    npc_state.position_dirty_indices.clear();
    npc_state.arrival_dirty_indices.clear();
    npc_state.target_dirty_indices.clear();
    npc_state.speed_dirty_indices.clear();
    npc_state.faction_dirty_indices.clear();
    npc_state.health_dirty_indices.clear();
    npc_state.flags_dirty_indices.clear();
    npc_state.half_size_dirty_indices.clear();
    npc_state.hidden_indices.clear();

    // Hide freed slots (deallocation cleanup — position=-9999, health=0, speed=0, flags=0)
    for slot in slots.take_pending_frees() {
        let pi = slot * 2;
        if pi + 1 < npc_state.positions.len() {
            npc_state.positions[pi] = -9999.0;
            npc_state.positions[pi + 1] = -9999.0;
            npc_state.position_dirty_indices.push(slot);
            npc_state.hidden_indices.push(slot);
        }
        if slot < npc_state.healths.len() {
            npc_state.healths[slot] = 0.0;
            npc_state.health_dirty_indices.push(slot);
        }
        if slot < npc_state.speeds.len() {
            npc_state.speeds[slot] = 0.0;
            npc_state.speed_dirty_indices.push(slot);
        }
        if slot < npc_state.entity_flags.len() {
            npc_state.entity_flags[slot] = 0;
            npc_state.flags_dirty_indices.push(slot);
        }
    }

    // Reset GPU state for newly allocated slots (prevents stale data from previous occupants)
    for slot in slots.take_pending_resets() {
        let pi = slot * 2;
        if pi + 1 < npc_state.positions.len() {
            npc_state.positions[pi] = -9999.0;
            npc_state.positions[pi + 1] = -9999.0;
            npc_state.position_dirty_indices.push(slot);
        }
        if pi + 1 < npc_state.targets.len() {
            npc_state.targets[pi] = -9999.0;
            npc_state.targets[pi + 1] = -9999.0;
            npc_state.target_dirty_indices.push(slot);
        }
        if slot < npc_state.arrivals.len() {
            npc_state.arrivals[slot] = 0;
            npc_state.arrival_dirty_indices.push(slot);
        }
        if slot < npc_state.speeds.len() {
            npc_state.speeds[slot] = 0.0;
            npc_state.speed_dirty_indices.push(slot);
        }
        if slot < npc_state.factions.len() {
            npc_state.factions[slot] = -1;
            npc_state.faction_dirty_indices.push(slot);
        }
        if slot < npc_state.healths.len() {
            npc_state.healths[slot] = 0.0;
            npc_state.health_dirty_indices.push(slot);
        }
        if slot < npc_state.entity_flags.len() {
            npc_state.entity_flags[slot] = 0;
            npc_state.flags_dirty_indices.push(slot);
        }
        let hi = slot * 2;
        if hi + 1 < npc_state.half_sizes.len() {
            npc_state.half_sizes[hi] = 0.0;
            npc_state.half_sizes[hi + 1] = 0.0;
            npc_state.half_size_dirty_indices.push(slot);
        }
        if slot < npc_state.flash_values.len() {
            npc_state.flash_values[slot] = 0.0;
        }
    }

    for msg in events.read() {
        let update = &msg.0;
        if let GpuUpdate::SetTarget { idx, x, y } = update {
            target_thrash.record_sink(*idx, sink_window_key, *x, *y);
        }
        npc_state.apply(update);
    }

    // Decay damage flash values (1.0 → 0.0 in ~0.2s)
    // Flash-only slots go to flash_only_indices (visual update but no equip upload needed).
    let dt = time.delta_secs();
    const FLASH_DECAY_RATE: f32 = 5.0;
    let active = slots.count().min(npc_state.flash_values.len());
    let mut flash_dirty: Vec<usize> = Vec::new();
    for (slot_idx, flash) in npc_state.flash_values[..active].iter_mut().enumerate() {
        if *flash > 0.0 {
            *flash = (*flash - dt * FLASH_DECAY_RATE).max(0.0);
            flash_dirty.push(slot_idx);
        }
    }
    npc_state.flash_only_indices.clear();
    npc_state.flash_only_indices.extend_from_slice(&flash_dirty);

    // Pre-sort+dedup dirty index Vecs so extract phase receives coalesce-ready data
    macro_rules! sort_dedup {
        ($v:expr) => {
            if $v.len() > 1 {
                $v.sort_unstable();
                $v.dedup();
            }
        };
    }
    sort_dedup!(npc_state.position_dirty_indices);
    sort_dedup!(npc_state.arrival_dirty_indices);
    sort_dedup!(npc_state.target_dirty_indices);
    sort_dedup!(npc_state.speed_dirty_indices);
    sort_dedup!(npc_state.faction_dirty_indices);
    sort_dedup!(npc_state.health_dirty_indices);
    sort_dedup!(npc_state.flags_dirty_indices);
    sort_dedup!(npc_state.half_size_dirty_indices);
}

// =============================================================================
// PROJECTILE RESOURCES (Main World)
// =============================================================================

/// Projectile compute uniform buffer fields. Owned by RenderFrameConfig.
#[derive(Clone, ShaderType)]
pub struct ProjGpuData {
    pub proj_count: u32,
    pub _npc_count: u32, // unused by shader, kept for struct alignment
    pub delta: f32,
    pub hit_half_length: f32,
    pub hit_half_width: f32,
    pub grid_width: u32,
    pub grid_height: u32,
    pub cell_size: f32,
    pub max_per_cell: u32,
    pub mode: u32,
    pub entity_count: u32,
}

impl Default for ProjGpuData {
    fn default() -> Self {
        Self {
            proj_count: 0,
            _npc_count: 0,
            delta: 0.016,
            hit_half_length: PROJECTILE_HIT_HALF_LENGTH,
            hit_half_width: PROJECTILE_HIT_HALF_WIDTH,
            grid_width: GRID_WIDTH,
            grid_height: GRID_HEIGHT,
            cell_size: 128.0,
            max_per_cell: MAX_PER_CELL,
            mode: 0,
            entity_count: 0,
        }
    }
}

/// Projectile buffer data to upload to GPU each frame.
/// Read during Extract via Extract<Res<T>> (zero-clone).
#[derive(Resource)]
pub struct ProjBufferWrites {
    pub positions: Vec<f32>,  // [x, y] per proj
    pub velocities: Vec<f32>, // [vx, vy] per proj
    pub damages: Vec<f32>,
    pub factions: Vec<i32>,
    pub shooters: Vec<i32>,
    pub lifetimes: Vec<f32>,
    pub active: Vec<i32>,
    pub hits: Vec<i32>, // [npc_idx, processed] per proj
    pub dirty: bool,
    /// Per-slot dirty tracking: Spawn writes all fields, Deactivate writes active+hits
    pub spawn_dirty_indices: Vec<usize>,
    pub deactivate_dirty_indices: Vec<usize>,
    /// Currently active projectile indices — iterate this instead of 0..proj_count.
    pub active_set: Vec<usize>,
}

impl Default for ProjBufferWrites {
    fn default() -> Self {
        let max = MAX_PROJECTILE_COUNT;
        Self {
            positions: vec![0.0; max * 2],
            velocities: vec![0.0; max * 2],
            damages: vec![0.0; max],
            factions: vec![0; max],
            shooters: vec![-1; max],
            lifetimes: vec![0.0; max],
            active: vec![0; max],
            hits: vec![-1; max * 2], // -1 = no hit
            dirty: false,
            spawn_dirty_indices: Vec::new(),
            deactivate_dirty_indices: Vec::new(),
            active_set: Vec::new(),
        }
    }
}

impl ProjBufferWrites {
    pub fn apply(&mut self, update: &ProjGpuUpdate) {
        match update {
            ProjGpuUpdate::Spawn {
                idx,
                x,
                y,
                vx,
                vy,
                damage,
                faction,
                shooter,
                lifetime,
            } => {
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
                    self.active_set.push(*idx);
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
                    if let Some(pos) = self.active_set.iter().position(|&s| s == *idx) {
                        self.active_set.swap_remove(pos);
                    }
                }
            }
        }
    }
}

/// Apply projectile GPU updates from Bevy messages to ProjBufferWrites.
pub fn populate_proj_buffer_writes(
    mut events: MessageReader<ProjGpuUpdateMsg>,
    mut writes: ResMut<ProjBufferWrites>,
) {
    writes.dirty = false;
    writes.spawn_dirty_indices.clear();
    writes.deactivate_dirty_indices.clear();
    for msg in events.read() {
        writes.apply(&msg.0);
    }
}

// =============================================================================
// READBACK (Bevy async GPU→CPU via ShaderStorageBuffer assets)
// =============================================================================

/// Handles to ShaderStorageBuffer assets used as readback targets.
/// Owned by RenderFrameConfig, extracted to render world so compute nodes can copy into them.
#[derive(Clone, Default)]
pub struct ReadbackHandles {
    pub npc_positions: Handle<ShaderStorageBuffer>,
    pub combat_targets: Handle<ShaderStorageBuffer>,
    pub npc_factions: Handle<ShaderStorageBuffer>,
    pub npc_health: Handle<ShaderStorageBuffer>,
    pub threat_counts: Handle<ShaderStorageBuffer>,
    pub proj_hits: Handle<ShaderStorageBuffer>,
    pub proj_positions: Handle<ShaderStorageBuffer>,
}

/// Round up to next power-of-2 (min 1024) for readback buffer_range sizing.
fn readback_bucket(count: usize) -> usize {
    count.max(1024).next_power_of_two()
}

/// Main-world-only state for dynamic readback entity management.
/// NOT extracted to render world — buckets and entity tracking stay on the CPU side.
#[derive(Resource, Default)]
pub struct ReadbackState {
    pub npc_bucket: usize,
    pub entity_bucket: usize,
    pub proj_bucket: usize,
    /// Always-on readback entities (positions, combat_targets, health, proj_hits, proj_positions).
    /// Only respawned on bucket change.
    pub always_entities: Vec<Entity>,
    /// Throttled readback entities (factions, threat_counts). Despawned 2 frames after spawn
    /// to allow async readback to complete (GPU copy frame N, CPU read frame N+1).
    pub throttled_entities: Vec<(Entity, u32)>, // (entity, frames_alive)
    pub faction_frame_counter: u32,
    pub threat_frame_counter: u32,
}

/// Create ShaderStorageBuffer readback targets (MAX-sized for compute copy destination).
/// Readback entities are spawned dynamically by `sync_readback_ranges`.
fn setup_readback_buffers(
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    mut config: ResMut<RenderFrameConfig>,
) {
    // Create readback target buffers (COPY_DST for compute→copy, COPY_SRC for Readback to map)
    let npc_pos_buf = {
        let init_pos: Vec<f32> = vec![-9999.0; MAX_NPC_COUNT * 2];
        let mut buf = ShaderStorageBuffer::new(
            bytemuck::cast_slice(&init_pos),
            RenderAssetUsages::RENDER_WORLD,
        );
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };
    let combat_target_buf = {
        let init_targets: Vec<i32> = vec![-1; MAX_ENTITIES];
        let mut buf = ShaderStorageBuffer::new(
            bytemuck::cast_slice(&init_targets),
            RenderAssetUsages::RENDER_WORLD,
        );
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };
    let npc_faction_buf = {
        let init_factions: Vec<i32> = vec![-1; MAX_NPC_COUNT];
        let mut buf = ShaderStorageBuffer::new(
            bytemuck::cast_slice(&init_factions),
            RenderAssetUsages::RENDER_WORLD,
        );
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };
    let npc_health_buf = {
        let mut buf = ShaderStorageBuffer::new(
            &vec![0u8; MAX_NPC_COUNT * 4],
            RenderAssetUsages::RENDER_WORLD,
        );
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };
    let threat_count_buf = {
        let mut buf = ShaderStorageBuffer::new(
            &vec![0u8; MAX_NPC_COUNT * 4],
            RenderAssetUsages::RENDER_WORLD,
        );
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };
    let proj_hit_buf = {
        let init_hits: Vec<[i32; 2]> = vec![[-1, 0]; MAX_PROJECTILE_COUNT];
        let mut buf = ShaderStorageBuffer::new(
            bytemuck::cast_slice(&init_hits),
            RenderAssetUsages::RENDER_WORLD,
        );
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };
    let proj_pos_buf = {
        let mut buf = ShaderStorageBuffer::new(
            &vec![0u8; MAX_PROJECTILE_COUNT * 8],
            RenderAssetUsages::RENDER_WORLD,
        );
        buf.buffer_description.usage |= BufferUsages::COPY_DST | BufferUsages::COPY_SRC;
        buffers.add(buf)
    };

    config.readback = ReadbackHandles {
        npc_positions: npc_pos_buf,
        combat_targets: combat_target_buf,
        npc_factions: npc_faction_buf,
        npc_health: npc_health_buf,
        threat_counts: threat_count_buf,
        proj_hits: proj_hit_buf,
        proj_positions: proj_pos_buf,
    };
}

/// Dynamically spawn/despawn Readback entities with buffer_range sized to current counts.
/// Quantized to power-of-2 buckets to avoid per-frame respawn churn.
/// Factions read every 60 frames, threat_counts every 30 frames (stale-tolerant).
fn sync_readback_ranges(
    mut commands: Commands,
    config: Res<RenderFrameConfig>,
    mut rb_state: ResMut<ReadbackState>,
    slots: Res<GpuSlotPool>,
    proj_alloc: Res<crate::resources::ProjSlotAllocator>,
) {
    let entity_count = slots.count();
    let proj_count = proj_alloc.next;

    let new_npc = readback_bucket(entity_count);
    let new_entity = readback_bucket(entity_count);
    let new_proj = readback_bucket(proj_count);

    let bucket_changed = new_npc != rb_state.npc_bucket
        || new_entity != rb_state.entity_bucket
        || new_proj != rb_state.proj_bucket;

    rb_state.faction_frame_counter += 1;
    rb_state.threat_frame_counter += 1;
    let faction_due = rb_state.faction_frame_counter >= 60;
    let threat_due = rb_state.threat_frame_counter >= 30;

    let sz = |count: usize, elem: usize| -> u64 { (count * elem) as u64 };
    let rb = &config.readback;

    // Always-on readbacks: only respawn when bucket changes or first frame
    if bucket_changed || rb_state.always_entities.is_empty() {
        for entity in rb_state.always_entities.drain(..) {
            if let Ok(mut cmds) = commands.get_entity(entity) {
                cmds.despawn();
            }
        }
        // Throttled entities also have stale bucket sizes — clear them too
        for (entity, _) in rb_state.throttled_entities.drain(..) {
            if let Ok(mut cmds) = commands.get_entity(entity) {
                cmds.despawn();
            }
        }

        rb_state.always_entities.push(
            commands
                .spawn(Readback::buffer_range(
                    rb.npc_positions.clone(),
                    0,
                    sz(new_npc, 8),
                ))
                .observe(|e: On<ReadbackComplete>, mut s: ResMut<GpuReadState>| {
                    s.positions = e.to_shader_type();
                })
                .id(),
        );

        rb_state.always_entities.push(
            commands
                .spawn(Readback::buffer_range(
                    rb.combat_targets.clone(),
                    0,
                    sz(new_entity, 4),
                ))
                .observe(|e: On<ReadbackComplete>, mut s: ResMut<GpuReadState>| {
                    s.combat_targets = e.to_shader_type();
                })
                .id(),
        );

        rb_state.always_entities.push(
            commands
                .spawn(Readback::buffer_range(
                    rb.npc_health.clone(),
                    0,
                    sz(new_npc, 4),
                ))
                .observe(|e: On<ReadbackComplete>, mut s: ResMut<GpuReadState>| {
                    s.health = e.to_shader_type();
                })
                .id(),
        );

        rb_state.always_entities.push(
            commands
                .spawn(Readback::buffer_range(
                    rb.proj_hits.clone(),
                    0,
                    sz(new_proj, 8),
                ))
                .observe(|e: On<ReadbackComplete>, mut s: ResMut<ProjHitState>| {
                    s.0 = e.to_shader_type();
                })
                .id(),
        );

        rb_state.always_entities.push(
            commands
                .spawn(Readback::buffer_range(
                    rb.proj_positions.clone(),
                    0,
                    sz(new_proj, 8),
                ))
                .observe(
                    |e: On<ReadbackComplete>, mut s: ResMut<ProjPositionState>| {
                        s.0 = e.to_shader_type();
                    },
                )
                .id(),
        );

        rb_state.npc_bucket = new_npc;
        rb_state.entity_bucket = new_entity;
        rb_state.proj_bucket = new_proj;
    }

    // Throttled readbacks: despawn after 2 frames (GPU copies frame N, CPU reads frame N+1).
    // Without despawn, Readback entities read every frame — defeating throttling.
    rb_state.throttled_entities.retain_mut(|(entity, age)| {
        *age += 1;
        if *age >= 3 {
            if let Ok(mut cmds) = commands.get_entity(*entity) {
                cmds.despawn();
            }
            false
        } else {
            true
        }
    });

    if faction_due || (bucket_changed && rb_state.faction_frame_counter > 0) {
        rb_state.throttled_entities.push((
            commands
                .spawn(Readback::buffer_range(
                    rb.npc_factions.clone(),
                    0,
                    sz(new_npc, 4),
                ))
                .observe(|e: On<ReadbackComplete>, mut s: ResMut<GpuReadState>| {
                    s.factions = e.to_shader_type();
                })
                .id(),
            0,
        ));
        rb_state.faction_frame_counter = 0;
    }

    if threat_due || (bucket_changed && rb_state.threat_frame_counter > 0) {
        rb_state.throttled_entities.push((
            commands
                .spawn(Readback::buffer_range(
                    rb.threat_counts.clone(),
                    0,
                    sz(new_npc, 4),
                ))
                .observe(|e: On<ReadbackComplete>, mut s: ResMut<GpuReadState>| {
                    s.threat_counts = e.to_shader_type();
                })
                .id(),
            0,
        ));
        rb_state.threat_frame_counter = 0;
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
        // Initialize resources in main world
        app.init_resource::<RenderFrameConfig>()
            .init_resource::<EntityGpuState>()
            .init_resource::<NpcVisualUpload>()
            .init_resource::<ProjBufferWrites>()
            .init_resource::<ReadbackState>()
            .add_systems(
                FixedUpdate,
                (
                    update_gpu_data,
                    update_proj_gpu_data,
                    populate_tile_flags,
                    sync_readback_ranges,
                ),
            )
            .add_systems(
                PostUpdate,
                (populate_gpu_state, build_visual_upload).chain(),
            )
            .add_systems(PostUpdate, populate_proj_buffer_writes);

        // Async readback: create ShaderStorageBuffer assets (Readback entities spawned by sync_readback_ranges)
        app.add_systems(Startup, setup_readback_buffers);

        // Extract resources to render world
        // EntityGpuState + NpcVisualUpload + ProjBufferWrites + ProjPositionState use Extract<Res<T>> (zero-clone)
        app.add_plugins(ExtractResourcePlugin::<RenderFrameConfig>::default());

        // Set up render world systems
        let render_app = match app.get_sub_app_mut(RenderApp) {
            Some(ra) => ra,
            None => {
                warn!("RenderApp not available - GPU compute disabled");
                return;
            }
        };

        render_app
            .add_systems(
                RenderStartup,
                (init_npc_compute_pipeline, init_proj_compute_pipeline),
            )
            .add_systems(
                Render,
                (prepare_npc_bind_groups, prepare_proj_bind_groups)
                    .in_set(RenderSystems::PrepareBindGroups),
            );

        // Add compute nodes to render graph
        // NPC compute → Projectile compute → Camera driver
        {
            let mut render_graph = render_app.world_mut().resource_mut::<RenderGraph>();
            render_graph.add_node(NpcComputeLabel, NpcComputeNode::default());
            render_graph.add_node(ProjectileComputeLabel, ProjectileComputeNode::default());
            render_graph.add_node_edge(NpcComputeLabel, ProjectileComputeLabel);
            render_graph.add_node_edge(
                ProjectileComputeLabel,
                bevy::render::graph::CameraDriverLabel,
            );
        }

        info!("GPU compute plugin initialized");
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct ProjectileComputeLabel;

/// Update GPU data from ECS each frame.
fn update_gpu_data(
    mut config: ResMut<RenderFrameConfig>,
    slots: Res<GpuSlotPool>,
    time: Res<Time>,
    game_time: Res<GameTime>,
    upgrades: Res<TownUpgrades>,
    world_data: Res<WorldData>,
) {
    let dt = game_time.delta(&time);
    config.npc.count = slots.count() as u32;
    config.npc.entity_count = slots.count() as u32;
    config.npc.delta = dt;

    let player_town_idx = world_data
        .towns
        .iter()
        .position(|t| t.faction == crate::constants::FACTION_PLAYER)
        .unwrap_or(0);
    let levels = upgrades.town_levels(player_town_idx);
    config.npc.dodge_unlocked = if stats::dodge_unlocked(&levels) { 1 } else { 0 };
}

/// Populate tile_flags vec from WorldGrid for GPU upload.
/// Only rebuilds when buildings have changed.
fn populate_tile_flags(
    mut config: ResMut<RenderFrameConfig>,
    grid: Res<crate::world::WorldGrid>,
    world_data: Res<crate::world::WorldData>,
    entity_map: Res<crate::resources::EntityMap>,
    mut grid_dirty: MessageReader<crate::messages::BuildingGridDirtyMsg>,
) {
    // Set grid dimensions every frame (cheap)
    config.npc.tile_grid_width = grid.width as u32;
    config.npc.tile_grid_height = grid.height as u32;
    config.npc.tile_cell_size = grid.cell_size;

    // Only rebuild flags vec when buildings changed
    if grid_dirty.read().count() == 0 && !config.tile_flags.is_empty() {
        return;
    }
    let total = grid.width * grid.height;
    if total == 0 {
        return;
    }
    let mut flags = vec![0u32; total];
    // Terrain pass
    for row in 0..grid.height {
        for col in 0..grid.width {
            if let Some(cell) = grid.cell(col, row) {
                let idx = row * grid.width + col;
                flags[idx] = match cell.terrain {
                    crate::world::Biome::Grass => crate::constants::TILE_GRASS,
                    crate::world::Biome::Forest => crate::constants::TILE_FOREST,
                    crate::world::Biome::Water => crate::constants::TILE_WATER,
                    crate::world::Biome::Rock => crate::constants::TILE_ROCK,
                    crate::world::Biome::Dirt => crate::constants::TILE_DIRT,
                };
            }
        }
    }
    // Building pass — iterate instances instead of all grid cells
    for inst in entity_map.iter_instances() {
        let (gc, gr) = grid.world_to_grid(inst.position);
        let idx = gr * grid.width + gc;
        if idx >= total {
            continue;
        }
        if inst.kind.is_road() {
            flags[idx] |= crate::constants::TILE_ROAD;
        }
        if inst.kind == crate::world::BuildingKind::Wall {
            let faction = world_data
                .towns
                .get(inst.town_idx as usize)
                .map(|t| t.faction as u32)
                .unwrap_or(0);
            flags[idx] |= crate::constants::TILE_WALL
                | ((faction & crate::constants::WALL_FACTION_MASK)
                    << crate::constants::WALL_FACTION_SHIFT);
        }
    }
    config.tile_flags = flags;
}

// =============================================================================
// RENDER WORLD RESOURCES
// =============================================================================

/// GPU buffers for NPC compute and rendering.
#[derive(Resource)]
pub struct EntityGpuBuffers {
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
    pub entity_flags: Buffer,
    pub tile_flags: Buffer,
    /// Per-entity hitbox half-sizes [half_w, half_h] for projectile collision.
    pub half_sizes: Buffer,
}

/// Bind groups for compute passes (one per mode, different uniform buffer).
#[derive(Resource)]
struct NpcBindGroups {
    mode0: BindGroup, // Clear grid
    mode1: BindGroup, // Build grid
    mode2: BindGroup, // Movement + targeting
}

/// Pipeline resources for compute.
#[derive(Resource)]
struct NpcComputePipeline {
    bind_group_layout: BindGroupLayoutDescriptor,
    pipeline_id: CachedComputePipelineId,
}

/// NPC sprite texture handles. Owned by RenderFrameConfig.
/// Set by the render module after loading sprite sheets.
#[derive(Clone, Default)]
pub struct NpcSpriteTexture {
    pub handle: Option<Handle<Image>>,
    pub world_handle: Option<Handle<Image>>,
    pub building_handle: Option<Handle<Image>>,
    pub extras_handle: Option<Handle<Image>>,
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

    // Create GPU buffers — entity-sized for unified NPC + building collision
    let max_ents = MAX_ENTITIES as usize;
    let buffers = EntityGpuBuffers {
        positions: render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("entity_positions"),
            contents: bytemuck::cast_slice(&vec![-9999.0f32; max_ents * 2]),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
        }),
        targets: render_device.create_buffer(&BufferDescriptor {
            label: Some("entity_targets"),
            size: (max_ents * std::mem::size_of::<[f32; 2]>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        speeds: render_device.create_buffer(&BufferDescriptor {
            label: Some("entity_speeds"),
            size: (max_ents * std::mem::size_of::<f32>()) as u64,
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
            label: Some("entity_arrivals"),
            size: (max_ents * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        backoff: render_device.create_buffer(&BufferDescriptor {
            label: Some("entity_backoff"),
            size: (max_ents * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE,
            mapped_at_creation: false,
        }),
        factions: render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("entity_factions"),
            contents: bytemuck::cast_slice(&vec![-1i32; max_ents]),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
        }),
        healths: render_device.create_buffer(&BufferDescriptor {
            label: Some("entity_healths"),
            size: (max_ents * std::mem::size_of::<f32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        combat_targets: render_device.create_buffer(&BufferDescriptor {
            label: Some("entity_combat_targets"),
            size: (max_ents * std::mem::size_of::<i32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        threat_counts: render_device.create_buffer(&BufferDescriptor {
            label: Some("entity_threat_counts"),
            size: (max_ents * std::mem::size_of::<u32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        entity_flags: render_device.create_buffer(&BufferDescriptor {
            label: Some("entity_flags"),
            size: (max_ents * std::mem::size_of::<u32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        tile_flags: render_device.create_buffer(&BufferDescriptor {
            label: Some("tile_flags"),
            // Max world: 32000px / 32px = 1000 cells per side → 1024×1024 buffer
            size: (1024 * 1024 * std::mem::size_of::<u32>()) as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        half_sizes: render_device.create_buffer(&BufferDescriptor {
            label: Some("entity_half_sizes"),
            size: (max_ents * std::mem::size_of::<[f32; 2]>()) as u64,
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
                uniform_buffer::<EntityGpuData>(false),
                // 11-12: projectile spatial grid (read only from NPC perspective)
                storage_buffer_read_only::<Vec<i32>>(false), // proj_grid_counts
                storage_buffer_read_only::<Vec<i32>>(false), // proj_grid_data
                // 13-15: projectile data (read only)
                storage_buffer_read_only::<Vec<[f32; 2]>>(false), // proj_positions
                storage_buffer_read_only::<Vec<[f32; 2]>>(false), // proj_velocities
                storage_buffer_read_only::<Vec<i32>>(false),      // proj_factions
                // 16: threat counts output (packed enemies<<16 | allies)
                storage_buffer::<Vec<u32>>(false),
                // 17: entity_flags (bit 0: combat scan, bit 1: building)
                storage_buffer_read_only::<Vec<u32>>(false),
                // 18: tile_flags (bitfield per world grid cell: bit 0=road)
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
    buffers: Option<Res<EntityGpuBuffers>>,
    proj_buffers: Option<Res<ProjGpuBuffers>>,
    config: Option<Res<RenderFrameConfig>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline_cache: Res<PipelineCache>,
) {
    use crate::messages::{RENDER_PROFILING, RENDER_TIMINGS, RT_NPC_BINDS};
    use std::sync::atomic::Ordering;
    let profiling = RENDER_PROFILING.load(Ordering::Relaxed);
    let start = if profiling {
        Some(std::time::Instant::now())
    } else {
        None
    };

    let Some(pipeline) = pipeline else { return };
    let Some(buffers) = buffers else { return };
    let Some(proj) = proj_buffers else { return };
    let Some(config) = config else { return };
    let params = &config.npc;

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

    let mut p0 = params.clone();
    p0.mode = 0;
    let mut p1 = params.clone();
    p1.mode = 1;
    let mut p2 = params.clone();
    p2.mode = 2;

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
    let flags_bind = buffers.entity_flags.as_entire_buffer_binding();
    let tile_bind = buffers.tile_flags.as_entire_buffer_binding();

    let mode0 = render_device.create_bind_group(
        Some("npc_compute_bg_mode0"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(),
            storage_bindings.1.clone(),
            storage_bindings.2.clone(),
            storage_bindings.3.clone(),
            storage_bindings.4.clone(),
            storage_bindings.5.clone(),
            storage_bindings.6.clone(),
            storage_bindings.7.clone(),
            storage_bindings.8.clone(),
            storage_bindings.9.clone(),
            &ub0,
            proj_bind.0.clone(),
            proj_bind.1.clone(),
            proj_bind.2.clone(),
            proj_bind.3.clone(),
            proj_bind.4.clone(),
            threat_bind.clone(),
            flags_bind.clone(),
            tile_bind.clone(),
        )),
    );
    let mode1 = render_device.create_bind_group(
        Some("npc_compute_bg_mode1"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(),
            storage_bindings.1.clone(),
            storage_bindings.2.clone(),
            storage_bindings.3.clone(),
            storage_bindings.4.clone(),
            storage_bindings.5.clone(),
            storage_bindings.6.clone(),
            storage_bindings.7.clone(),
            storage_bindings.8.clone(),
            storage_bindings.9.clone(),
            &ub1,
            proj_bind.0.clone(),
            proj_bind.1.clone(),
            proj_bind.2.clone(),
            proj_bind.3.clone(),
            proj_bind.4.clone(),
            threat_bind.clone(),
            flags_bind.clone(),
            tile_bind.clone(),
        )),
    );
    let mode2 = render_device.create_bind_group(
        Some("npc_compute_bg_mode2"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(),
            storage_bindings.1.clone(),
            storage_bindings.2.clone(),
            storage_bindings.3.clone(),
            storage_bindings.4.clone(),
            storage_bindings.5.clone(),
            storage_bindings.6.clone(),
            storage_bindings.7.clone(),
            storage_bindings.8.clone(),
            storage_bindings.9.clone(),
            &ub2,
            proj_bind.0.clone(),
            proj_bind.1.clone(),
            proj_bind.2.clone(),
            proj_bind.3.clone(),
            proj_bind.4.clone(),
            threat_bind.clone(),
            flags_bind.clone(),
            tile_bind.clone(),
        )),
    );

    commands.insert_resource(NpcBindGroups {
        mode0,
        mode1,
        mode2,
    });

    if let Some(s) = start {
        RENDER_TIMINGS[RT_NPC_BINDS].store(
            (s.elapsed().as_secs_f64() as f32 * 1000.0).to_bits(),
            Ordering::Relaxed,
        );
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
        use crate::messages::{RENDER_PROFILING, RENDER_TIMINGS, RT_GPU_COMPUTE};
        use std::sync::atomic::Ordering;
        let profiling = RENDER_PROFILING.load(Ordering::Relaxed);
        let start = if profiling {
            Some(std::time::Instant::now())
        } else {
            None
        };

        // Only run if ready
        if !matches!(self.state, NpcComputeState::Ready) {
            return Ok(());
        }

        let Some(bind_groups) = world.get_resource::<NpcBindGroups>() else {
            return Ok(());
        };
        let Some(config) = world.get_resource::<RenderFrameConfig>() else {
            return Ok(());
        };
        let pipeline_cache = world.resource::<PipelineCache>();
        let pipeline = world.resource::<NpcComputePipeline>();

        let npc_count = config.npc.count;
        let entity_count = config.npc.entity_count;
        if entity_count == 0 {
            return Ok(());
        }

        let Some(compute_pipeline) = pipeline_cache.get_compute_pipeline(pipeline.pipeline_id)
        else {
            return Ok(());
        };

        let grid_cells = GRID_WIDTH * GRID_HEIGHT;
        let grid_wg = (grid_cells + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
        let entity_wg = (entity_count + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;

        // Pass 0: Clear spatial grid
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_bind_group(0, &bind_groups.mode0, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(grid_wg, 1, 1);
        }

        // Pass 1: Build spatial grid (insert all entities into cells)
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_bind_group(0, &bind_groups.mode1, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(entity_wg, 1, 1);
        }

        // Pass 2: Movement (NPCs) + combat targeting (NPCs + towers)
        {
            let mut pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_bind_group(0, &bind_groups.mode2, &[]);
            pass.set_pipeline(compute_pipeline);
            pass.dispatch_workgroups(entity_wg, 1, 1);
        }

        // Copy positions + combat_targets → readback ShaderStorageBuffer assets
        // Bevy's Readback component will async-read these and fire ReadbackComplete
        let buffers = world.resource::<EntityGpuBuffers>();
        let handles = &world.resource::<RenderFrameConfig>().readback;
        let render_assets = world.resource::<RenderAssets<GpuShaderStorageBuffer>>();

        let pos_copy_size = (npc_count as u64) * std::mem::size_of::<[f32; 2]>() as u64;
        let ct_copy_size = (entity_count as u64) * std::mem::size_of::<i32>() as u64; // entity_count: includes tower targets

        if let Some(rb_pos) = render_assets.get(&handles.npc_positions) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &buffers.positions,
                0,
                &rb_pos.buffer,
                0,
                pos_copy_size,
            );
        }
        if let Some(rb_ct) = render_assets.get(&handles.combat_targets) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &buffers.combat_targets,
                0,
                &rb_ct.buffer,
                0,
                ct_copy_size,
            );
        }

        let i32_copy_size = (npc_count as u64) * std::mem::size_of::<i32>() as u64;
        let f32_copy_size = (npc_count as u64) * std::mem::size_of::<f32>() as u64;

        if let Some(rb_fac) = render_assets.get(&handles.npc_factions) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &buffers.factions,
                0,
                &rb_fac.buffer,
                0,
                i32_copy_size,
            );
        }
        if let Some(rb_hp) = render_assets.get(&handles.npc_health) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &buffers.healths,
                0,
                &rb_hp.buffer,
                0,
                f32_copy_size,
            );
        }
        let u32_copy_size = (npc_count as u64) * std::mem::size_of::<u32>() as u64;
        if let Some(rb_tc) = render_assets.get(&handles.threat_counts) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &buffers.threat_counts,
                0,
                &rb_tc.buffer,
                0,
                u32_copy_size,
            );
        }

        if let Some(s) = start {
            RENDER_TIMINGS[RT_GPU_COMPUTE].store(
                (s.elapsed().as_secs_f64() as f32 * 1000.0).to_bits(),
                Ordering::Relaxed,
            );
        }
        Ok(())
    }
}

// =============================================================================
// PROJECTILE COMPUTE
// =============================================================================

/// Update projectile GPU data from ECS each frame.
fn update_proj_gpu_data(
    mut config: ResMut<RenderFrameConfig>,
    slots: Res<GpuSlotPool>,
    proj_alloc: Res<crate::resources::ProjSlotAllocator>,
    time: Res<Time>,
    game_time: Res<GameTime>,
) {
    let dt = game_time.delta(&time);
    config.proj.proj_count = proj_alloc.next as u32;
    config.proj._npc_count = slots.count() as u32;
    config.proj.entity_count = slots.count() as u32;
    config.proj.delta = dt;
}

fn init_proj_compute_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let max = MAX_PROJECTILE_COUNT;
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

    // 18 bindings: 8 proj (rw) + 3 NPC (ro) + 2 NPC grid (ro) + 1 uniform + 2 proj grid (rw) + 1 half_sizes (ro) + 1 entity_flags (ro)
    let bind_group_layout = BindGroupLayoutDescriptor::new(
        "ProjComputeLayout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                // 0-7: projectile buffers (read_write)
                storage_buffer::<Vec<[f32; 2]>>(false), // positions
                storage_buffer::<Vec<[f32; 2]>>(false), // velocities
                storage_buffer::<Vec<f32>>(false),      // damages
                storage_buffer::<Vec<i32>>(false),      // factions
                storage_buffer::<Vec<i32>>(false),      // shooters
                storage_buffer::<Vec<f32>>(false),      // lifetimes
                storage_buffer::<Vec<i32>>(false),      // active
                storage_buffer::<Vec<[i32; 2]>>(false), // hits
                // 8-10: NPC buffers (read only)
                storage_buffer_read_only::<Vec<[f32; 2]>>(false), // npc_positions
                storage_buffer_read_only::<Vec<i32>>(false),      // npc_factions
                storage_buffer_read_only::<Vec<f32>>(false),      // npc_healths
                // 11-12: NPC spatial grid (read only)
                storage_buffer_read_only::<Vec<i32>>(false), // grid_counts
                storage_buffer_read_only::<Vec<i32>>(false), // grid_data
                // 13: uniform params
                uniform_buffer::<ProjGpuData>(false),
                // 14-15: projectile spatial grid (read_write)
                storage_buffer::<Vec<i32>>(false), // proj_grid_counts
                storage_buffer::<Vec<i32>>(false), // proj_grid_data
                // 16: entity hitbox half-sizes (read only)
                storage_buffer_read_only::<Vec<[f32; 2]>>(false), // entity_half_sizes
                // 17: entity flags (read only — UNTARGETABLE skip)
                storage_buffer_read_only::<Vec<u32>>(false), // entity_flags
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
    entity_buffers: Option<Res<EntityGpuBuffers>>,
    config: Option<Res<RenderFrameConfig>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline_cache: Res<PipelineCache>,
) {
    use crate::messages::{RENDER_PROFILING, RENDER_TIMINGS, RT_PROJ_BINDS};
    use std::sync::atomic::Ordering;
    let profiling = RENDER_PROFILING.load(Ordering::Relaxed);
    let start = if profiling {
        Some(std::time::Instant::now())
    } else {
        None
    };

    let Some(pipeline) = pipeline else { return };
    let Some(proj) = proj_buffers else { return };
    let Some(ent) = entity_buffers else { return };
    let Some(config) = config else { return };

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
        ent.positions.as_entire_buffer_binding(),
        ent.factions.as_entire_buffer_binding(),
        ent.healths.as_entire_buffer_binding(),
        ent.grid_counts.as_entire_buffer_binding(),
        ent.grid_data.as_entire_buffer_binding(),
    );

    let mut p0 = config.proj.clone();
    p0.mode = 0;
    let mut p1 = config.proj.clone();
    p1.mode = 1;
    let mut p2 = config.proj.clone();
    p2.mode = 2;

    let mut ub0 = UniformBuffer::from(p0);
    let mut ub1 = UniformBuffer::from(p1);
    let mut ub2 = UniformBuffer::from(p2);
    ub0.write_buffer(&render_device, &render_queue);
    ub1.write_buffer(&render_device, &render_queue);
    ub2.write_buffer(&render_device, &render_queue);

    let half_sizes_bind = ent.half_sizes.as_entire_buffer_binding();
    let entity_flags_bind = ent.entity_flags.as_entire_buffer_binding();

    let mode0 = render_device.create_bind_group(
        Some("proj_compute_bg_mode0"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(),
            storage_bindings.1.clone(),
            storage_bindings.2.clone(),
            storage_bindings.3.clone(),
            storage_bindings.4.clone(),
            storage_bindings.5.clone(),
            storage_bindings.6.clone(),
            storage_bindings.7.clone(),
            storage_bindings.8.clone(),
            storage_bindings.9.clone(),
            storage_bindings.10.clone(),
            storage_bindings.11.clone(),
            storage_bindings.12.clone(),
            &ub0,
            proj.grid_counts.as_entire_buffer_binding(),
            proj.grid_data.as_entire_buffer_binding(),
            half_sizes_bind.clone(),
            entity_flags_bind.clone(),
        )),
    );
    let mode1 = render_device.create_bind_group(
        Some("proj_compute_bg_mode1"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(),
            storage_bindings.1.clone(),
            storage_bindings.2.clone(),
            storage_bindings.3.clone(),
            storage_bindings.4.clone(),
            storage_bindings.5.clone(),
            storage_bindings.6.clone(),
            storage_bindings.7.clone(),
            storage_bindings.8.clone(),
            storage_bindings.9.clone(),
            storage_bindings.10.clone(),
            storage_bindings.11.clone(),
            storage_bindings.12.clone(),
            &ub1,
            proj.grid_counts.as_entire_buffer_binding(),
            proj.grid_data.as_entire_buffer_binding(),
            half_sizes_bind.clone(),
            entity_flags_bind.clone(),
        )),
    );
    let mode2 = render_device.create_bind_group(
        Some("proj_compute_bg_mode2"),
        layout,
        &BindGroupEntries::sequential((
            storage_bindings.0.clone(),
            storage_bindings.1.clone(),
            storage_bindings.2.clone(),
            storage_bindings.3.clone(),
            storage_bindings.4.clone(),
            storage_bindings.5.clone(),
            storage_bindings.6.clone(),
            storage_bindings.7.clone(),
            storage_bindings.8.clone(),
            storage_bindings.9.clone(),
            storage_bindings.10.clone(),
            storage_bindings.11.clone(),
            storage_bindings.12.clone(),
            &ub2,
            proj.grid_counts.as_entire_buffer_binding(),
            proj.grid_data.as_entire_buffer_binding(),
            half_sizes_bind.clone(),
            entity_flags_bind.clone(),
        )),
    );

    commands.insert_resource(ProjBindGroups {
        mode0,
        mode1,
        mode2,
    });

    if let Some(s) = start {
        RENDER_TIMINGS[RT_PROJ_BINDS].store(
            (s.elapsed().as_secs_f64() as f32 * 1000.0).to_bits(),
            Ordering::Relaxed,
        );
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
        Self {
            state: ProjComputeState::Loading,
        }
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
        use crate::messages::{RENDER_PROFILING, RENDER_TIMINGS, RT_PROJ_COMPUTE};
        use std::sync::atomic::Ordering;
        let profiling = RENDER_PROFILING.load(Ordering::Relaxed);
        let start = if profiling {
            Some(std::time::Instant::now())
        } else {
            None
        };

        if !matches!(self.state, ProjComputeState::Ready) {
            return Ok(());
        }

        let Some(bind_groups) = world.get_resource::<ProjBindGroups>() else {
            return Ok(());
        };
        let Some(config) = world.get_resource::<RenderFrameConfig>() else {
            return Ok(());
        };
        let pipeline_cache = world.resource::<PipelineCache>();
        let pipeline = world.resource::<ProjComputePipeline>();

        let proj_count = config.proj.proj_count;
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
        let handles = &world.resource::<RenderFrameConfig>().readback;
        let render_assets = world.resource::<RenderAssets<GpuShaderStorageBuffer>>();

        let hit_copy_size = (proj_count as u64) * std::mem::size_of::<[i32; 2]>() as u64;
        if let Some(rb_hits) = render_assets.get(&handles.proj_hits) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &proj_buffers.hits,
                0,
                &rb_hits.buffer,
                0,
                hit_copy_size,
            );
        }
        let pos_copy_size = (proj_count as u64) * std::mem::size_of::<[f32; 2]>() as u64;
        if let Some(rb_pos) = render_assets.get(&handles.proj_positions) {
            render_context.command_encoder().copy_buffer_to_buffer(
                &proj_buffers.positions,
                0,
                &rb_pos.buffer,
                0,
                pos_copy_size,
            );
        }

        if let Some(s) = start {
            RENDER_TIMINGS[RT_PROJ_COMPUTE].store(
                (s.elapsed().as_secs_f64() as f32 * 1000.0).to_bits(),
                Ordering::Relaxed,
            );
        }
        Ok(())
    }
}
