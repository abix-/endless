//! ECS Resources - Shared state accessible by all systems

use crate::constants::{MAX_ENTITIES, MAX_NPC_COUNT, MAX_PROJECTILES};
use bevy::prelude::*;
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Profiling resource: frame timing + render-world timing drain + tracing capture.
/// Auto-capture via SystemTimingLayer handles all main-world systems.
/// Render-world timings still use record() via atomic drain in frame_timer_start.
const EMA_ALPHA: f32 = 0.1;

#[derive(Resource)]
pub struct SystemTimings {
    data: Mutex<HashMap<&'static str, f32>>,
    /// Tracing-captured timings (Bevy auto-spans, feature-gated behind `trace`).
    traced: Mutex<HashMap<String, f32>>,
    pub frame_ms: Mutex<f32>,
    pub enabled: bool,
}

impl Default for SystemTimings {
    fn default() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
            traced: Mutex::new(HashMap::new()),
            frame_ms: Mutex::new(0.0),
            enabled: false,
        }
    }
}

impl SystemTimings {
    /// Record true frame time from Bevy's Time::delta (captures render + vsync + everything).
    pub fn record_frame_delta(&self, dt_secs: f32) {
        if self.enabled {
            let ms = dt_secs * 1000.0;
            if let Ok(mut fm) = self.frame_ms.lock() {
                *fm = *fm * (1.0 - EMA_ALPHA) + ms * EMA_ALPHA;
            }
        }
    }

    /// Record a timing value directly (same EMA as scope guard).
    /// Use for accumulated sub-section timings recorded after a loop.
    pub fn record(&self, name: &'static str, ms: f32) {
        if let Ok(mut data) = self.data.lock() {
            let entry = data.entry(name).or_insert(0.0);
            *entry = *entry * (1.0 - EMA_ALPHA) + ms * EMA_ALPHA;
        }
    }

    /// Record a tracing-captured timing (from Bevy auto-spans).
    pub fn record_traced(&self, name: &str, ms: f32) {
        if let Ok(mut traced) = self.traced.lock() {
            let entry = traced.entry(name.to_string()).or_insert(0.0);
            // Already EMA-smoothed by the tracing layer; just copy the latest value.
            *entry = ms;
        }
    }

    pub fn get_timings(&self) -> HashMap<&'static str, f32> {
        self.data.lock().map(|d| d.clone()).unwrap_or_default()
    }

    pub fn get_traced_timings(&self) -> HashMap<String, f32> {
        self.traced.lock().map(|d| d.clone()).unwrap_or_default()
    }

    pub fn get_frame_ms(&self) -> f32 {
        self.frame_ms.lock().map(|f| *f).unwrap_or(0.0)
    }
}

/// Delta time for the current frame (seconds).
#[derive(Resource, Default)]
pub struct DeltaTime(pub f32);

/// Monotonically increasing UID allocator. Starts at 1 (0 is reserved as "none").
#[derive(Resource)]
pub struct NextEntityUid(pub u64);

impl Default for NextEntityUid {
    fn default() -> Self {
        Self(1)
    }
}

impl NextEntityUid {
    /// Allocate the next UID. Never returns EntityUid(0).
    pub fn next(&mut self) -> crate::components::EntityUid {
        let uid = crate::components::EntityUid(self.0);
        self.0 += 1;
        uid
    }
}

/// NPC decision throttling config. Controls how often non-combat decisions are evaluated.
#[derive(Resource)]
pub struct NpcDecisionConfig {
    pub interval: f32, // seconds between decision evaluations (default 2.0)
    pub max_decisions_per_frame: usize, // max Tier 3 decisions per frame (adaptive bucket floor)
}

impl Default for NpcDecisionConfig {
    fn default() -> Self {
        Self {
            interval: 2.0,
            max_decisions_per_frame: 300,
        }
    }
}

/// Unified entity registry — ALL entities (NPCs + buildings) slot→entity mapping,
/// plus building-specific instance data, spatial grid, and indexes.
/// Populated on NPC spawn and building placement, used by damage/combat/rendering for entity lookup.
#[derive(Resource, Default)]
pub struct EntityMap {
    /// ALL entities (NPCs + buildings) — unified slot→entity
    pub entities: HashMap<usize, Entity>,

    // UID bidirectional maps — stable identity for gameplay cross-references
    uid_to_slot: HashMap<crate::components::EntityUid, usize>,
    slot_to_uid: HashMap<usize, crate::components::EntityUid>,
    uid_to_entity: HashMap<crate::components::EntityUid, Entity>,
    entity_to_uid: HashMap<Entity, crate::components::EntityUid>,

    // Building-specific data
    instances: HashMap<usize, BuildingInstance>,
    by_kind: HashMap<crate::world::BuildingKind, Vec<usize>>,
    by_kind_town: HashMap<(crate::world::BuildingKind, u32), Vec<usize>>,
    by_grid_cell: HashMap<(i32, i32), usize>,

    // NPC-specific data (index-only — gameplay state on ECS components)
    npcs: HashMap<usize, NpcEntry>,
    npc_by_town: HashMap<i32, Vec<usize>>,

    // Spatial grid
    spatial_cell_size: f32,
    spatial_width: usize,
    spatial_cells: Vec<Vec<usize>>,

    // Kind-filtered spatial indexes (for worksite queries)
    // Key: (kind, town_idx, cell_index) → slots matching kind+town in that cell
    spatial_kind_town: HashMap<(crate::world::BuildingKind, u32, usize), Vec<usize>>,
    // Key: (kind, cell_index) → slots matching kind (any town) in that cell
    spatial_kind_cell: HashMap<(crate::world::BuildingKind, usize), Vec<usize>>,
    // Back-index: slot → bucket positions for O(1) swap-remove
    spatial_bucket_idx: HashMap<usize, SpatialBucketRef>,
}

/// Back-index for O(1) swap-remove from kind-filtered spatial buckets.
#[derive(Clone, Debug)]
struct SpatialBucketRef {
    kind: crate::world::BuildingKind,
    town_idx: u32,
    cell_idx: usize,
    /// Index in spatial_kind_town[(kind, town, cell)] vec
    kind_town_pos: usize,
    /// Index in spatial_kind_cell[(kind, cell)] vec
    kind_cell_pos: usize,
}

impl EntityMap {
    // ── UID API ───────────────────────────────────────────────────────

    /// Register a UID↔slot↔entity mapping. Called at NPC spawn time.
    pub fn register_uid(&mut self, slot: usize, uid: crate::components::EntityUid, entity: Entity) {
        debug_assert!(uid.0 != 0, "EntityUid(0) is reserved");
        self.uid_to_slot.insert(uid, slot);
        self.slot_to_uid.insert(slot, uid);
        self.uid_to_entity.insert(uid, entity);
        self.entity_to_uid.insert(entity, uid);
        #[cfg(debug_assertions)]
        self.debug_assert_uid_bijection();
    }

    /// Register UID↔slot only (no entity yet). Used for buildings before ECS entity exists.
    pub fn register_uid_slot_only(&mut self, slot: usize, uid: crate::components::EntityUid) {
        debug_assert!(uid.0 != 0, "EntityUid(0) is reserved");
        self.uid_to_slot.insert(uid, slot);
        self.slot_to_uid.insert(slot, uid);
    }

    /// Bind a UID to an ECS entity. Called when ECS entity is created for a building.
    pub fn bind_uid_entity(&mut self, uid: crate::components::EntityUid, entity: Entity) {
        self.uid_to_entity.insert(uid, entity);
        self.entity_to_uid.insert(entity, uid);
    }

    /// Unregister UID mappings for a slot. Called at death/despawn.
    pub fn unregister_uid(&mut self, slot: usize) {
        if let Some(uid) = self.slot_to_uid.remove(&slot) {
            self.uid_to_slot.remove(&uid);
            if let Some(entity) = self.uid_to_entity.remove(&uid) {
                self.entity_to_uid.remove(&entity);
            }
        }
        #[cfg(debug_assertions)]
        self.debug_assert_uid_bijection();
    }

    /// Clear all UID mappings. Called on world reset/load.
    pub fn clear_uids(&mut self) {
        self.uid_to_slot.clear();
        self.slot_to_uid.clear();
        self.uid_to_entity.clear();
        self.entity_to_uid.clear();
    }

    pub fn uid_for_slot(&self, slot: usize) -> Option<crate::components::EntityUid> {
        self.slot_to_uid.get(&slot).copied()
    }

    pub fn slot_for_uid(&self, uid: crate::components::EntityUid) -> Option<usize> {
        self.uid_to_slot.get(&uid).copied()
    }

    pub fn entity_by_uid(&self, uid: crate::components::EntityUid) -> Option<Entity> {
        self.uid_to_entity.get(&uid).copied()
    }

    pub fn uid_by_entity(&self, entity: Entity) -> Option<crate::components::EntityUid> {
        self.entity_to_uid.get(&entity).copied()
    }

    /// Look up a building instance by UID (resolves UID→slot→instance).
    pub fn instance_by_uid(&self, uid: crate::components::EntityUid) -> Option<&BuildingInstance> {
        self.uid_to_slot
            .get(&uid)
            .and_then(|&slot| self.instances.get(&slot))
    }

    #[cfg(debug_assertions)]
    fn debug_assert_uid_bijection(&self) {
        debug_assert_eq!(
            self.uid_to_slot.len(),
            self.slot_to_uid.len(),
            "UID↔slot map length mismatch: {} vs {}",
            self.uid_to_slot.len(),
            self.slot_to_uid.len()
        );
        debug_assert_eq!(
            self.uid_to_entity.len(),
            self.entity_to_uid.len(),
            "UID↔entity map length mismatch: {} vs {}",
            self.uid_to_entity.len(),
            self.entity_to_uid.len()
        );
        for (&uid, &slot) in &self.uid_to_slot {
            debug_assert_eq!(
                self.slot_to_uid.get(&slot),
                Some(&uid),
                "UID→slot→UID round-trip failed for uid={:?} slot={}",
                uid,
                slot
            );
        }
        for (&uid, &entity) in &self.uid_to_entity {
            debug_assert_eq!(
                self.entity_to_uid.get(&entity),
                Some(&uid),
                "UID→entity→UID round-trip failed for uid={:?}",
                uid
            );
        }
    }

    // ── Building instance API ──────────────────────────────────────────

    /// Remove a building by its slot. Removes entity mapping, UID mapping, AND instance data.
    pub fn remove_by_slot(&mut self, slot: usize) {
        self.entities.remove(&slot);
        self.unregister_uid(slot);
        self.remove_instance(slot);
    }

    /// Clear all building data (instances, indexes, spatial grid). Does NOT clear entities.
    pub fn clear_buildings(&mut self) {
        self.instances.clear();
        self.by_kind.clear();
        self.by_kind_town.clear();
        self.by_grid_cell.clear();
        self.spatial_cells.iter_mut().for_each(|c| c.clear());
        self.spatial_kind_town.clear();
        self.spatial_kind_cell.clear();
        self.spatial_bucket_idx.clear();
        self.clear_uids();
    }

    /// Number of building instances.
    pub fn building_count(&self) -> usize {
        self.instances.len()
    }

    /// Iterate all entity instance slot keys.
    pub fn all_entity_slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.instances.keys().copied()
    }

    /// Add or update a building instance. Updates all indexes.
    /// If the slot already exists, removes old index entries first to avoid duplicates.
    pub fn add_instance(&mut self, inst: BuildingInstance) {
        let slot = inst.slot;
        let kind = inst.kind;
        // Remove old index entries if updating an existing slot
        if let Some(old) = self.instances.remove(&slot) {
            if let Some(slots) = self.by_kind.get_mut(&old.kind) {
                slots.retain(|&s| s != slot);
            }
            if let Some(slots) = self.by_kind_town.get_mut(&(old.kind, old.town_idx)) {
                slots.retain(|&s| s != slot);
            }
            let old_gc = (old.position.x / 32.0).floor() as i32;
            let old_gr = (old.position.y / 32.0).floor() as i32;
            self.by_grid_cell.remove(&(old_gc, old_gr));
            self.spatial_remove(slot, old.position);
        }
        self.by_kind.entry(kind).or_default().push(slot);
        self.by_kind_town
            .entry((kind, inst.town_idx))
            .or_default()
            .push(slot);
        let gc = (inst.position.x / 32.0).floor() as i32;
        let gr = (inst.position.y / 32.0).floor() as i32;
        self.by_grid_cell.insert((gc, gr), slot);
        let pos = inst.position;
        self.instances.insert(slot, inst);
        self.spatial_insert(slot, pos);
    }

    /// Remove an instance by slot. Returns removed instance if any.
    fn remove_instance(&mut self, slot: usize) -> Option<BuildingInstance> {
        if let Some(inst) = self.instances.remove(&slot) {
            if let Some(slots) = self.by_kind.get_mut(&inst.kind) {
                slots.retain(|&s| s != slot);
            }
            if let Some(slots) = self.by_kind_town.get_mut(&(inst.kind, inst.town_idx)) {
                slots.retain(|&s| s != slot);
            }
            let gc = (inst.position.x / 32.0).floor() as i32;
            let gr = (inst.position.y / 32.0).floor() as i32;
            self.by_grid_cell.remove(&(gc, gr));
            self.spatial_remove(slot, inst.position);
            Some(inst)
        } else {
            None
        }
    }

    pub fn get_instance(&self, slot: usize) -> Option<&BuildingInstance> {
        self.instances.get(&slot)
    }

    pub fn get_instance_mut(&mut self, slot: usize) -> Option<&mut BuildingInstance> {
        self.instances.get_mut(&slot)
    }

    pub fn iter_instances(&self) -> impl Iterator<Item = &BuildingInstance> {
        self.instances.values()
    }

    pub fn iter_instances_mut(&mut self) -> impl Iterator<Item = &mut BuildingInstance> {
        self.instances.values_mut()
    }

    pub fn iter_kind(
        &self,
        kind: crate::world::BuildingKind,
    ) -> impl Iterator<Item = &BuildingInstance> {
        let slots = self.by_kind.get(&kind);
        let instances = &self.instances;
        slots
            .into_iter()
            .flat_map(|v| v.iter())
            .filter_map(move |&s| instances.get(&s))
    }

    pub fn iter_kind_for_town(
        &self,
        kind: crate::world::BuildingKind,
        town_idx: u32,
    ) -> impl Iterator<Item = &BuildingInstance> {
        let slots = self.by_kind_town.get(&(kind, town_idx));
        let instances = &self.instances;
        slots
            .into_iter()
            .flat_map(|v| v.iter())
            .filter_map(move |&s| instances.get(&s))
    }

    pub fn count_for_town(&self, kind: crate::world::BuildingKind, town_idx: u32) -> usize {
        self.iter_kind_for_town(kind, town_idx).count()
    }

    pub fn building_counts(&self, town_idx: u32) -> HashMap<crate::world::BuildingKind, usize> {
        let mut counts = HashMap::new();
        for (kind, slots) in &self.by_kind {
            let count = slots
                .iter()
                .filter(|&&s| {
                    self.instances
                        .get(&s)
                        .is_some_and(|i| i.town_idx == town_idx)
                })
                .count();
            if count > 0 {
                counts.insert(*kind, count);
            }
        }
        counts
    }

    pub fn gold_mine_index(&self, pos: Vec2) -> Option<usize> {
        self.iter_kind(crate::world::BuildingKind::GoldMine)
            .enumerate()
            .find(|(_, inst)| (inst.position - pos).length() < 1.0)
            .map(|(i, _)| i)
    }

    pub fn find_by_position(&self, pos: Vec2) -> Option<&BuildingInstance> {
        let gc = (pos.x / 32.0).floor() as i32;
        let gr = (pos.y / 32.0).floor() as i32;
        self.by_grid_cell
            .get(&(gc, gr))
            .and_then(|&s| self.instances.get(&s))
    }

    pub fn find_by_position_mut(&mut self, pos: Vec2) -> Option<&mut BuildingInstance> {
        let gc = (pos.x / 32.0).floor() as i32;
        let gr = (pos.y / 32.0).floor() as i32;
        let slot = self.by_grid_cell.get(&(gc, gr)).copied()?;
        self.instances.get_mut(&slot)
    }

    pub fn find_farm_at(&self, pos: Vec2) -> Option<&BuildingInstance> {
        self.find_by_position(pos)
            .filter(|i| i.kind == crate::world::BuildingKind::Farm)
    }

    pub fn find_farm_at_mut(&mut self, pos: Vec2) -> Option<&mut BuildingInstance> {
        self.find_by_position_mut(pos)
            .filter(|i| i.kind == crate::world::BuildingKind::Farm)
    }

    pub fn find_mine_at(&self, pos: Vec2) -> Option<&BuildingInstance> {
        self.find_by_position(pos)
            .filter(|i| i.kind == crate::world::BuildingKind::GoldMine)
    }

    pub fn find_mine_at_mut(&mut self, pos: Vec2) -> Option<&mut BuildingInstance> {
        self.find_by_position_mut(pos)
            .filter(|i| i.kind == crate::world::BuildingKind::GoldMine)
    }

    pub fn iter_growable(&self) -> impl Iterator<Item = &BuildingInstance> {
        self.iter_kind(crate::world::BuildingKind::Farm)
            .chain(self.iter_kind(crate::world::BuildingKind::GoldMine))
    }

    pub fn slot_at_position(&self, pos: Vec2) -> Option<usize> {
        let gc = (pos.x / 32.0).floor() as i32;
        let gr = (pos.y / 32.0).floor() as i32;
        self.by_grid_cell.get(&(gc, gr)).copied()
    }

    pub fn has_building_at(&self, gc: i32, gr: i32) -> bool {
        self.by_grid_cell.contains_key(&(gc, gr))
    }

    pub fn get_at_grid(&self, gc: i32, gr: i32) -> Option<&BuildingInstance> {
        self.by_grid_cell
            .get(&(gc, gr))
            .and_then(|&s| self.instances.get(&s))
    }

    // ── Occupancy ─────────────────────────────────────────────────────

    pub fn release(&mut self, slot: usize) {
        if let Some(inst) = self.instances.get_mut(&slot) {
            inst.occupants = inst.occupants.saturating_sub(1);
        }
    }

    pub fn occupant_count(&self, slot: usize) -> i32 {
        self.instances.get(&slot).map_or(0, |i| i.occupants as i32)
    }

    pub fn is_occupied(&self, slot: usize) -> bool {
        self.instances.get(&slot).is_some_and(|i| i.occupants >= 1)
    }

    // ── NPC instance API ───────────────────────────────────────────────

    /// Register an NPC slot→entity mapping (index-only, no gameplay state).
    pub fn register_npc(
        &mut self,
        slot: usize,
        entity: Entity,
        job: crate::components::Job,
        faction: i32,
        town_idx: i32,
    ) {
        debug_assert!(
            !self.npcs.contains_key(&slot),
            "duplicate NPC slot {}",
            slot
        );
        self.entities.insert(slot, entity);
        self.npc_by_town.entry(town_idx).or_default().push(slot);
        self.npcs.insert(
            slot,
            NpcEntry {
                slot,
                entity,
                job,
                faction,
                town_idx,
                dead: false,
            },
        );
    }

    /// Unregister an NPC slot. Removes entity mapping, UID mapping, and NPC entry.
    pub fn unregister_npc(&mut self, slot: usize) -> Option<NpcEntry> {
        debug_assert!(
            self.npcs.contains_key(&slot),
            "removing absent NPC slot {}",
            slot
        );
        self.entities.remove(&slot);
        self.unregister_uid(slot);
        if let Some(entry) = self.npcs.remove(&slot) {
            if let Some(slots) = self.npc_by_town.get_mut(&entry.town_idx) {
                slots.retain(|&s| s != slot);
            }
            Some(entry)
        } else {
            None
        }
    }

    pub fn get_npc(&self, slot: usize) -> Option<&NpcEntry> {
        self.npcs.get(&slot)
    }

    pub fn get_npc_mut(&mut self, slot: usize) -> Option<&mut NpcEntry> {
        self.npcs.get_mut(&slot)
    }

    pub fn iter_npcs(&self) -> impl Iterator<Item = &NpcEntry> {
        self.npcs.values()
    }

    pub fn npcs_for_town(&self, town_idx: i32) -> impl Iterator<Item = &NpcEntry> {
        let npcs = &self.npcs;
        self.npc_by_town
            .get(&town_idx)
            .into_iter()
            .flat_map(|v| v.iter())
            .filter_map(move |&s| npcs.get(&s))
    }

    pub fn npc_count(&self) -> usize {
        self.npcs.len()
    }

    pub fn clear_npcs(&mut self) {
        for &slot in self.npcs.keys() {
            self.entities.remove(&slot);
            // UID cleanup: remove slot's UID mappings
            if let Some(uid) = self.slot_to_uid.remove(&slot) {
                self.uid_to_slot.remove(&uid);
                if let Some(entity) = self.uid_to_entity.remove(&uid) {
                    self.entity_to_uid.remove(&entity);
                }
            }
        }
        self.npcs.clear();
        self.npc_by_town.clear();
    }

    /// Check if a slot is an NPC (vs building).
    pub fn is_npc(&self, slot: usize) -> bool {
        self.npcs.contains_key(&slot)
    }

    // ── Spatial grid ───────────────────────────────────────────────────

    pub fn init_spatial(&mut self, world_size_px: f32) {
        self.spatial_cell_size = 256.0;
        self.spatial_width = (world_size_px / self.spatial_cell_size).ceil() as usize + 1;
        let total = self.spatial_width * self.spatial_width;
        self.spatial_cells.resize_with(total, Vec::new);
    }

    pub fn rebuild_spatial(&mut self) {
        for cell in &mut self.spatial_cells {
            cell.clear();
        }
        self.spatial_kind_town.clear();
        self.spatial_kind_cell.clear();
        self.spatial_bucket_idx.clear();
        let slots: Vec<(usize, Vec2)> = self
            .instances
            .values()
            .map(|i| (i.slot, i.position))
            .collect();
        for (slot, pos) in slots {
            self.spatial_insert(slot, pos);
        }
        #[cfg(debug_assertions)]
        self.validate_spatial_indexes();
    }

    fn spatial_insert(&mut self, slot: usize, pos: Vec2) {
        if self.spatial_width == 0 {
            return;
        }
        let cx = (pos.x / self.spatial_cell_size) as usize;
        let cy = (pos.y / self.spatial_cell_size) as usize;
        if cx < self.spatial_width && cy < self.spatial_width {
            let cell_idx = cy * self.spatial_width + cx;
            self.spatial_cells[cell_idx].push(slot);

            // Kind-filtered buckets
            if let Some(inst) = self.instances.get(&slot) {
                let kind = inst.kind;
                let town = inst.town_idx;

                let kt_bucket = self
                    .spatial_kind_town
                    .entry((kind, town, cell_idx))
                    .or_default();
                let kt_pos = kt_bucket.len();
                kt_bucket.push(slot);

                let kc_bucket = self.spatial_kind_cell.entry((kind, cell_idx)).or_default();
                let kc_pos = kc_bucket.len();
                kc_bucket.push(slot);

                self.spatial_bucket_idx.insert(
                    slot,
                    SpatialBucketRef {
                        kind,
                        town_idx: town,
                        cell_idx,
                        kind_town_pos: kt_pos,
                        kind_cell_pos: kc_pos,
                    },
                );
            }
        }
    }

    fn spatial_remove(&mut self, slot: usize, pos: Vec2) {
        if self.spatial_width == 0 {
            return;
        }
        let cx = (pos.x / self.spatial_cell_size) as usize;
        let cy = (pos.y / self.spatial_cell_size) as usize;
        if cx < self.spatial_width && cy < self.spatial_width {
            let idx = cy * self.spatial_width + cx;
            self.spatial_cells[idx].retain(|&s| s != slot);
        }

        // Kind-filtered bucket swap-remove
        if let Some(bucket_ref) = self.spatial_bucket_idx.remove(&slot) {
            // Remove from kind+town bucket
            let kt_key = (bucket_ref.kind, bucket_ref.town_idx, bucket_ref.cell_idx);
            if let Some(kt_bucket) = self.spatial_kind_town.get_mut(&kt_key) {
                let pos_in_vec = bucket_ref.kind_town_pos;
                if pos_in_vec < kt_bucket.len() {
                    kt_bucket.swap_remove(pos_in_vec);
                    // Update the swapped element's back-index
                    if pos_in_vec < kt_bucket.len() {
                        let swapped_slot = kt_bucket[pos_in_vec];
                        if let Some(swapped_ref) = self.spatial_bucket_idx.get_mut(&swapped_slot) {
                            swapped_ref.kind_town_pos = pos_in_vec;
                        }
                    }
                }
                if kt_bucket.is_empty() {
                    self.spatial_kind_town.remove(&kt_key);
                }
            }

            // Remove from kind+cell bucket
            let kc_key = (bucket_ref.kind, bucket_ref.cell_idx);
            if let Some(kc_bucket) = self.spatial_kind_cell.get_mut(&kc_key) {
                let pos_in_vec = bucket_ref.kind_cell_pos;
                if pos_in_vec < kc_bucket.len() {
                    kc_bucket.swap_remove(pos_in_vec);
                    if pos_in_vec < kc_bucket.len() {
                        let swapped_slot = kc_bucket[pos_in_vec];
                        if let Some(swapped_ref) = self.spatial_bucket_idx.get_mut(&swapped_slot) {
                            swapped_ref.kind_cell_pos = pos_in_vec;
                        }
                    }
                }
                if kc_bucket.is_empty() {
                    self.spatial_kind_cell.remove(&kc_key);
                }
            }
        }
    }

    pub fn spatial_cell_size(&self) -> f32 {
        self.spatial_cell_size
    }

    pub fn for_each_nearby(&self, pos: Vec2, radius: f32, mut f: impl FnMut(&BuildingInstance)) {
        if self.spatial_width == 0 {
            return;
        }
        let cs = self.spatial_cell_size;
        let min_cx = ((pos.x - radius).max(0.0) / cs) as usize;
        let max_cx = (((pos.x + radius) / cs) as usize).min(self.spatial_width - 1);
        let min_cy = ((pos.y - radius).max(0.0) / cs) as usize;
        let max_cy = (((pos.y + radius) / cs) as usize).min(self.spatial_width - 1);
        for cy in min_cy..=max_cy {
            let row = cy * self.spatial_width;
            for cx in min_cx..=max_cx {
                for &slot in &self.spatial_cells[row + cx] {
                    if let Some(inst) = self.instances.get(&slot) {
                        f(inst);
                    }
                }
            }
        }
    }

    // ── Kind-filtered spatial queries ─────────────────────────────────

    /// Convert pixel radius to cell radius from a center cell.
    fn cell_radius(&self, px_radius: f32) -> usize {
        if self.spatial_cell_size <= 0.0 {
            return 0;
        }
        (px_radius / self.spatial_cell_size).ceil() as usize
    }

    /// Iterate buildings of a specific kind+town in cells within radius of pos.
    pub fn for_each_nearby_kind_town(
        &self,
        pos: Vec2,
        radius: f32,
        kind: crate::world::BuildingKind,
        town_idx: u32,
        mut f: impl FnMut(&BuildingInstance),
    ) {
        if self.spatial_width == 0 {
            return;
        }
        let cs = self.spatial_cell_size;
        let min_cx = ((pos.x - radius).max(0.0) / cs) as usize;
        let max_cx = (((pos.x + radius) / cs) as usize).min(self.spatial_width - 1);
        let min_cy = ((pos.y - radius).max(0.0) / cs) as usize;
        let max_cy = (((pos.y + radius) / cs) as usize).min(self.spatial_width - 1);
        for cy in min_cy..=max_cy {
            for cx in min_cx..=max_cx {
                let cell_idx = cy * self.spatial_width + cx;
                if let Some(bucket) = self.spatial_kind_town.get(&(kind, town_idx, cell_idx)) {
                    for &slot in bucket {
                        if let Some(inst) = self.instances.get(&slot) {
                            f(inst);
                        }
                    }
                }
            }
        }
    }

    /// Iterate buildings of a specific kind (any town) in cells within radius of pos.
    pub fn for_each_nearby_kind(
        &self,
        pos: Vec2,
        radius: f32,
        kind: crate::world::BuildingKind,
        mut f: impl FnMut(&BuildingInstance),
    ) {
        if self.spatial_width == 0 {
            return;
        }
        let cs = self.spatial_cell_size;
        let min_cx = ((pos.x - radius).max(0.0) / cs) as usize;
        let max_cx = (((pos.x + radius) / cs) as usize).min(self.spatial_width - 1);
        let min_cy = ((pos.y - radius).max(0.0) / cs) as usize;
        let max_cy = (((pos.y + radius) / cs) as usize).min(self.spatial_width - 1);
        for cy in min_cy..=max_cy {
            for cx in min_cx..=max_cx {
                let cell_idx = cy * self.spatial_width + cx;
                if let Some(bucket) = self.spatial_kind_cell.get(&(kind, cell_idx)) {
                    for &slot in bucket {
                        if let Some(inst) = self.instances.get(&slot) {
                            f(inst);
                        }
                    }
                }
            }
        }
    }

    /// Cell-ring query: iterate kind+town buildings only in cells between inner and outer radii.
    /// inner_cell_r=0, outer_cell_r=0 visits only the center cell.
    /// Each cell is visited exactly once across successive ring expansions.
    pub fn for_each_ring_kind_town(
        &self,
        pos: Vec2,
        inner_cell_r: usize,
        outer_cell_r: usize,
        kind: crate::world::BuildingKind,
        town_idx: u32,
        mut f: impl FnMut(&BuildingInstance),
    ) {
        if self.spatial_width == 0 {
            return;
        }
        let cs = self.spatial_cell_size;
        let center_cx = (pos.x / cs) as usize;
        let center_cy = (pos.y / cs) as usize;
        let w = self.spatial_width;

        let outer = outer_cell_r;
        let min_cx = center_cx.saturating_sub(outer).min(w - 1);
        let max_cx = (center_cx + outer).min(w - 1);
        let min_cy = center_cy.saturating_sub(outer).min(w - 1);
        let max_cy = (center_cy + outer).min(w - 1);

        for cy in min_cy..=max_cy {
            for cx in min_cx..=max_cx {
                // Skip cells in the inner region (already visited)
                if inner_cell_r > 0 {
                    let dx = if cx >= center_cx {
                        cx - center_cx
                    } else {
                        center_cx - cx
                    };
                    let dy = if cy >= center_cy {
                        cy - center_cy
                    } else {
                        center_cy - cy
                    };
                    if dx < inner_cell_r && dy < inner_cell_r {
                        continue;
                    }
                }
                let cell_idx = cy * w + cx;
                if let Some(bucket) = self.spatial_kind_town.get(&(kind, town_idx, cell_idx)) {
                    for &slot in bucket {
                        if let Some(inst) = self.instances.get(&slot) {
                            f(inst);
                        }
                    }
                }
            }
        }
    }

    /// Cell-ring query: iterate kind buildings (any town) only in cells between inner and outer radii.
    pub fn for_each_ring_kind(
        &self,
        pos: Vec2,
        inner_cell_r: usize,
        outer_cell_r: usize,
        kind: crate::world::BuildingKind,
        mut f: impl FnMut(&BuildingInstance),
    ) {
        if self.spatial_width == 0 {
            return;
        }
        let cs = self.spatial_cell_size;
        let center_cx = (pos.x / cs) as usize;
        let center_cy = (pos.y / cs) as usize;
        let w = self.spatial_width;

        let outer = outer_cell_r;
        let min_cx = center_cx.saturating_sub(outer).min(w - 1);
        let max_cx = (center_cx + outer).min(w - 1);
        let min_cy = center_cy.saturating_sub(outer).min(w - 1);
        let max_cy = (center_cy + outer).min(w - 1);

        for cy in min_cy..=max_cy {
            for cx in min_cx..=max_cx {
                if inner_cell_r > 0 {
                    let dx = if cx >= center_cx {
                        cx - center_cx
                    } else {
                        center_cx - cx
                    };
                    let dy = if cy >= center_cy {
                        cy - center_cy
                    } else {
                        center_cy - cy
                    };
                    if dx < inner_cell_r && dy < inner_cell_r {
                        continue;
                    }
                }
                let cell_idx = cy * w + cx;
                if let Some(bucket) = self.spatial_kind_cell.get(&(kind, cell_idx)) {
                    for &slot in bucket {
                        if let Some(inst) = self.instances.get(&slot) {
                            f(inst);
                        }
                    }
                }
            }
        }
    }

    // ── Worksite query API ────────────────────────────────────────────

    /// Find nearest worksite using cell-ring expansion with kind-filtered spatial index.
    /// `score` returns `Option<S>` — `None` rejects, `Some(s)` accepts.
    /// **Lower S wins** (min-order). Use tuples like `(priority: u8, dist2_bits: u32)`.
    /// Faction filtering (if needed) is applied by the caller inside the score closure.
    pub fn find_nearest_worksite<S: Ord>(
        &self,
        from: Vec2,
        kind: crate::world::BuildingKind,
        town_idx: u32,
        fallback: WorksiteFallback,
        max_radius: f32,
        mut score: impl FnMut(&BuildingInstance) -> Option<S>,
    ) -> Option<WorksiteResult> {
        debug_assert!(town_idx != u32::MAX, "town_idx looks like -1 as u32");
        let max_cell_r = self.cell_radius(max_radius);
        let mut best: Option<(S, usize, Vec2)> = None;

        // Town-scoped expanding ring search
        let mut prev_r: usize = 0;
        let mut cell_r: usize = 0; // start with center cell (r=0)
        loop {
            self.for_each_ring_kind_town(from, prev_r, cell_r, kind, town_idx, |inst| {
                if let Some(s) = score(inst) {
                    if best.is_none() || s < best.as_ref().unwrap().0 {
                        best = Some((s, inst.slot, inst.position));
                    }
                }
            });
            if best.is_some() || cell_r >= max_cell_r {
                break;
            }
            prev_r = cell_r + 1;
            cell_r = if cell_r == 0 {
                1
            } else {
                (cell_r * 2).min(max_cell_r)
            };
        }

        // AnyTown fallback
        if best.is_none() && matches!(fallback, WorksiteFallback::AnyTown) {
            prev_r = 0;
            cell_r = 0;
            loop {
                self.for_each_ring_kind(from, prev_r, cell_r, kind, |inst| {
                    if let Some(s) = score(inst) {
                        if best.is_none() || s < best.as_ref().unwrap().0 {
                            best = Some((s, inst.slot, inst.position));
                        }
                    }
                });
                if best.is_some() || cell_r >= max_cell_r {
                    break;
                }
                prev_r = cell_r + 1;
                cell_r = if cell_r == 0 {
                    1
                } else {
                    (cell_r * 2).min(max_cell_r)
                };
            }
        }

        best.map(|(_, slot, position)| WorksiteResult {
            slot,
            position,
            radius_used: cell_r as f32 * self.spatial_cell_size,
        })
    }

    /// Validate and claim a worksite slot. Returns None if stale/invalid.
    /// Single authority point for all worksite claims.
    pub fn try_claim_worksite(
        &mut self,
        slot: usize,
        expected_kind: crate::world::BuildingKind,
        expected_town: Option<u32>,
        max_occupants: i32,
    ) -> Option<ClaimedWorksite> {
        let valid = self.instances.get(&slot).is_some_and(|inst| {
            inst.kind == expected_kind
                && expected_town.is_none_or(|t| inst.town_idx == t)
                && (inst.occupants as i32) < max_occupants
        });
        if valid {
            let inst = self.instances.get_mut(&slot).unwrap();
            inst.occupants += 1;
            Some(ClaimedWorksite {
                slot,
                position: inst.position,
            })
        } else {
            None
        }
    }

    // ── Debug validation ──────────────────────────────────────────────

    /// Verify all kind-filtered spatial indexes are consistent with back-index.
    #[cfg(debug_assertions)]
    fn validate_spatial_indexes(&self) {
        // Every slot in bucket_idx must exist in both corresponding buckets
        for (&slot, bref) in &self.spatial_bucket_idx {
            let kt_key = (bref.kind, bref.town_idx, bref.cell_idx);
            let kt_bucket = self.spatial_kind_town.get(&kt_key).unwrap_or_else(|| {
                panic!(
                    "spatial_bucket_idx slot {} references missing kind_town bucket {:?}",
                    slot, kt_key
                )
            });
            assert!(
                bref.kind_town_pos < kt_bucket.len(),
                "slot {} kind_town_pos {} >= bucket len {}",
                slot,
                bref.kind_town_pos,
                kt_bucket.len()
            );
            assert_eq!(
                kt_bucket[bref.kind_town_pos], slot,
                "slot {} kind_town_pos {} points to slot {}",
                slot, bref.kind_town_pos, kt_bucket[bref.kind_town_pos]
            );

            let kc_key = (bref.kind, bref.cell_idx);
            let kc_bucket = self.spatial_kind_cell.get(&kc_key).unwrap_or_else(|| {
                panic!(
                    "spatial_bucket_idx slot {} references missing kind_cell bucket {:?}",
                    slot, kc_key
                )
            });
            assert!(
                bref.kind_cell_pos < kc_bucket.len(),
                "slot {} kind_cell_pos {} >= bucket len {}",
                slot,
                bref.kind_cell_pos,
                kc_bucket.len()
            );
            assert_eq!(
                kc_bucket[bref.kind_cell_pos], slot,
                "slot {} kind_cell_pos {} points to slot {}",
                slot, bref.kind_cell_pos, kc_bucket[bref.kind_cell_pos]
            );
        }

        // Every slot in every bucket must have a back-index entry
        for (key, bucket) in &self.spatial_kind_town {
            for (pos, &slot) in bucket.iter().enumerate() {
                let bref = self.spatial_bucket_idx.get(&slot).unwrap_or_else(|| {
                    panic!(
                        "kind_town bucket {:?} pos {} slot {} has no back-index",
                        key, pos, slot
                    )
                });
                assert_eq!(
                    bref.kind_town_pos, pos,
                    "slot {} back-index kind_town_pos {} != actual pos {}",
                    slot, bref.kind_town_pos, pos
                );
            }
        }
        for (key, bucket) in &self.spatial_kind_cell {
            for (pos, &slot) in bucket.iter().enumerate() {
                let bref = self.spatial_bucket_idx.get(&slot).unwrap_or_else(|| {
                    panic!(
                        "kind_cell bucket {:?} pos {} slot {} has no back-index",
                        key, pos, slot
                    )
                });
                assert_eq!(
                    bref.kind_cell_pos, pos,
                    "slot {} back-index kind_cell_pos {} != actual pos {}",
                    slot, bref.kind_cell_pos, pos
                );
            }
        }
    }
}

/// Result from `find_nearest_worksite`. Slot must be re-validated via `try_claim_worksite`.
pub struct WorksiteResult {
    pub slot: usize,
    pub position: Vec2,
    pub radius_used: f32,
}

/// Fallback policy when town-scoped worksite search finds nothing.
#[derive(Clone, Copy)]
pub enum WorksiteFallback {
    TownOnly,
    AnyTown,
}

/// Returned by `try_claim_worksite` after successful validation and claim.
pub struct ClaimedWorksite {
    pub slot: usize,
    pub position: Vec2,
}

/// Population counts per (job_id, clan_id).
#[derive(Default, Clone)]
pub struct PopStats {
    pub alive: i32,
    pub working: i32,
    pub dead: i32,
}

/// Aggregated population stats, updated incrementally at spawn/death/state transitions.
#[derive(Resource, Default)]
pub struct PopulationStats(pub HashMap<(i32, i32), PopStats>);

/// Game config pushed from GDScript at startup.
#[derive(Resource)]
pub struct GameConfig {
    /// Per-job home count (mirrors WorldGenConfig.npc_counts).
    pub npc_counts: std::collections::BTreeMap<crate::components::Job, i32>,
    pub spawn_interval_hours: i32,
    pub food_per_work_hour: i32,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            npc_counts: crate::constants::NPC_REGISTRY
                .iter()
                .map(|d| (d.job, d.default_count as i32))
                .collect(),
            spawn_interval_hours: 4,
            food_per_work_hour: 1,
        }
    }
}

/// Game time tracking - Bevy-owned, uses PhysicsDelta from godot-bevy.
/// Only total_seconds is mutable. Day/hour/minute are derived on demand.
#[derive(Resource)]
pub struct GameTime {
    pub total_seconds: f32, // Only mutable state - accumulates from PhysicsDelta
    pub seconds_per_hour: f32, // Game speed: 5.0 = 1 game-hour per 5 real seconds
    pub start_hour: i32,    // Hour at game start (6 = 6am)
    pub time_scale: f32,    // 1.0 = normal, 2.0 = 2x speed
    pub paused: bool,
    pub last_hour: i32,    // Previous hour (for detecting hour ticks)
    pub hour_ticked: bool, // True if hour just changed this frame
}

impl GameTime {
    /// True when gameplay should be frozen.
    /// `time_scale <= 0` is treated the same as paused.
    pub fn is_paused(&self) -> bool {
        self.paused || self.time_scale <= 0.0
    }

    /// Gameplay-scaled delta. Zero when paused, multiplied by time_scale otherwise.
    pub fn delta(&self, time: &Time) -> f32 {
        if self.is_paused() {
            0.0
        } else {
            time.delta_secs() * self.time_scale
        }
    }

    pub fn total_hours(&self) -> i32 {
        (self.total_seconds / self.seconds_per_hour) as i32
    }

    pub fn day(&self) -> i32 {
        (self.start_hour + self.total_hours()) / 24 + 1
    }

    pub fn hour(&self) -> i32 {
        (self.start_hour + self.total_hours()) % 24
    }

    pub fn minute(&self) -> i32 {
        let seconds_into_hour = self.total_seconds % self.seconds_per_hour;
        ((seconds_into_hour / self.seconds_per_hour) * 60.0) as i32
    }

    pub fn is_daytime(&self) -> bool {
        let h = self.hour();
        h >= 6 && h < 20
    }
}

impl Default for GameTime {
    fn default() -> Self {
        Self {
            total_seconds: 0.0,
            seconds_per_hour: 5.0,
            start_hour: 6,
            time_scale: 1.0,
            last_hour: 0,
            hour_ticked: false,
            paused: false,
        }
    }
}

// ============================================================================
// UI STATE RESOURCES
// ============================================================================

/// Kill statistics for UI display.
#[derive(Resource, Clone, Default)]
pub struct KillStats {
    pub archer_kills: i32,   // Raiders killed by archers
    pub villager_kills: i32, // Villagers (farmers/archers) killed by raiders
}

/// Currently selected NPC index (-1 = none).
#[derive(Resource)]
pub struct SelectedNpc(pub i32);
impl Default for SelectedNpc {
    fn default() -> Self {
        Self(-1)
    }
}

/// Currently selected building (grid cell). `active = false` means no building selected.
#[derive(Resource, Default)]
pub struct SelectedBuilding {
    pub col: usize,
    pub row: usize,
    pub active: bool,
    pub slot: Option<usize>,
    pub kind: Option<crate::world::BuildingKind>,
}

/// Camera follow mode — when true, camera tracks the selected NPC.
#[derive(Resource, Default)]
pub struct FollowSelected(pub bool);

// ============================================================================
// DEBUG RESOURCES
// ============================================================================

/// Toggleable debug log flags. Controlled via pause menu settings.
#[derive(Resource)]
pub struct DebugFlags {
    /// Log GPU readback positions each tick
    pub readback: bool,
    /// Log combat stats each tick
    pub combat: bool,
    /// Log spawn/death events
    pub spawns: bool,
    /// Log behavior state changes
    pub behavior: bool,
}

impl Default for DebugFlags {
    fn default() -> Self {
        Self {
            readback: false,
            combat: false,
            spawns: false,
            behavior: false,
        }
    }
}

/// Health system debug info - updated by damage/death systems, read by GDScript.
#[derive(Resource, Default)]
pub struct HealthDebug {
    pub damage_processed: usize,
    pub deaths_this_frame: usize,
    pub despawned_this_frame: usize,
    pub bevy_entity_count: usize,
    pub health_samples: Vec<(usize, f32)>,
    // Healing debug
    pub healing_npcs_checked: usize,
    pub healing_positions_len: usize,
    pub healing_towns_count: usize,
    pub healing_in_zone_count: usize,
    pub healing_healed_count: usize,
    pub healing_active_count: usize,
    pub healing_enter_checks: usize,
    pub healing_exits: usize,
}

/// Combat system debug info - updated by cooldown/attack systems, read by GDScript.
#[derive(Resource)]
pub struct CombatDebug {
    pub attackers_queried: usize,
    pub targets_found: usize,
    pub attacks_made: usize,
    pub chases_started: usize,
    pub in_combat_added: usize,
    pub sample_target_idx: i32,
    pub positions_len: usize,
    pub combat_targets_len: usize,
    pub bounds_failures: usize,
    pub sample_dist: f32,
    pub in_range_count: usize,
    pub timer_ready_count: usize,
    pub sample_timer: f32,
    pub cooldown_entities: usize,
    pub frame_delta: f32,
    pub sample_combat_target_0: i32,
    pub sample_combat_target_1: i32,
    pub sample_pos_0: (f32, f32),
    pub sample_pos_1: (f32, f32),
}

impl Default for CombatDebug {
    fn default() -> Self {
        Self {
            attackers_queried: 0,
            targets_found: 0,
            attacks_made: 0,
            chases_started: 0,
            in_combat_added: 0,
            sample_target_idx: -99,
            positions_len: 0,
            combat_targets_len: 0,
            bounds_failures: 0,
            sample_dist: -1.0,
            in_range_count: 0,
            timer_ready_count: 0,
            sample_timer: -1.0,
            cooldown_entities: 0,
            frame_delta: 0.0,
            sample_combat_target_0: -99,
            sample_combat_target_1: -99,
            sample_pos_0: (0.0, 0.0),
            sample_pos_1: (0.0, 0.0),
        }
    }
}

/// Runtime metric for target intent thrashing.
/// Tracks per-NPC SetTarget reason flips within the current game minute.
#[derive(Resource, Default)]
pub struct NpcTargetThrashDebug {
    pub minute_key: i32,
    pub sink_window_key: i64,
    pub writes_this_minute: Vec<u16>,
    pub reason_flips_this_minute: Vec<u16>,
    pub target_changes_this_minute: Vec<u16>,
    pub ping_pong_this_minute: Vec<u16>,
    pub last_reason: Vec<String>,
    pub last_target_q: Vec<(i32, i32)>,
    pub prev_target_q: Vec<(i32, i32)>,
    pub sink_writes_this_minute: Vec<u16>,
    pub sink_target_changes_this_minute: Vec<u16>,
    pub sink_ping_pong_this_minute: Vec<u16>,
    pub sink_last_target: Vec<(f32, f32)>,
    pub sink_prev_target: Vec<(f32, f32)>,
    pub sink_has_target: Vec<bool>,
}

impl NpcTargetThrashDebug {
    #[inline]
    fn target_delta_sq(a: (f32, f32), b: (f32, f32)) -> f32 {
        let dx = a.0 - b.0;
        let dy = a.1 - b.1;
        dx * dx + dy * dy
    }

    pub fn record(&mut self, idx: usize, reason: &'static str, minute_key: i32, x: f32, y: f32) {
        if self.minute_key != minute_key {
            self.minute_key = minute_key;
            self.writes_this_minute.fill(0);
            self.reason_flips_this_minute.fill(0);
            self.target_changes_this_minute.fill(0);
            self.ping_pong_this_minute.fill(0);
            self.sink_writes_this_minute.fill(0);
            self.sink_target_changes_this_minute.fill(0);
            self.sink_ping_pong_this_minute.fill(0);
        }
        self.ensure_len(idx + 1);
        self.writes_this_minute[idx] = self.writes_this_minute[idx].saturating_add(1);

        let q = (x.round() as i32, y.round() as i32);
        let last_q = self.last_target_q[idx];
        if last_q != (0, 0) && last_q != q {
            self.target_changes_this_minute[idx] =
                self.target_changes_this_minute[idx].saturating_add(1);
            if self.prev_target_q[idx] == q {
                self.ping_pong_this_minute[idx] = self.ping_pong_this_minute[idx].saturating_add(1);
            }
        }
        self.prev_target_q[idx] = last_q;
        self.last_target_q[idx] = q;

        if self.last_reason[idx] != reason {
            if !self.last_reason[idx].is_empty() {
                self.reason_flips_this_minute[idx] =
                    self.reason_flips_this_minute[idx].saturating_add(1);
            }
            self.last_reason[idx].clear();
            self.last_reason[idx].push_str(reason);
        }
    }

    pub fn record_sink(&mut self, idx: usize, window_key: i64, x: f32, y: f32) {
        if self.sink_window_key != window_key {
            self.sink_window_key = window_key;
            self.sink_writes_this_minute.fill(0);
            self.sink_target_changes_this_minute.fill(0);
            self.sink_ping_pong_this_minute.fill(0);
            self.sink_has_target.fill(false);
        }
        self.ensure_len(idx + 1);
        self.sink_writes_this_minute[idx] = self.sink_writes_this_minute[idx].saturating_add(1);
        let curr = (x, y);
        if self.sink_has_target[idx] {
            let last = self.sink_last_target[idx];
            // Tiny epsilon to avoid float jitter noise while still catching visible flips.
            if Self::target_delta_sq(last, curr) > 0.01 {
                self.sink_target_changes_this_minute[idx] =
                    self.sink_target_changes_this_minute[idx].saturating_add(1);
                let prev = self.sink_prev_target[idx];
                if Self::target_delta_sq(prev, curr) <= 0.01 {
                    self.sink_ping_pong_this_minute[idx] =
                        self.sink_ping_pong_this_minute[idx].saturating_add(1);
                }
            }
            self.sink_prev_target[idx] = last;
        } else {
            self.sink_has_target[idx] = true;
        }
        self.sink_last_target[idx] = curr;
    }

    pub fn top_offenders(&self, top_n: usize) -> Vec<(usize, u16, u16, u16, u16, &str)> {
        let mut rows: Vec<(usize, u16, u16, u16, u16, &str)> = self
            .sink_target_changes_this_minute
            .iter()
            .enumerate()
            .filter_map(|(idx, &sink_changes)| {
                if sink_changes == 0 {
                    return None;
                }
                let reason_flips = self.reason_flips_this_minute.get(idx).copied().unwrap_or(0);
                let ping_pong = self
                    .sink_ping_pong_this_minute
                    .get(idx)
                    .copied()
                    .unwrap_or(0);
                let writes = self.sink_writes_this_minute.get(idx).copied().unwrap_or(0);
                let reason = self.last_reason.get(idx).map(|s| s.as_str()).unwrap_or("");
                Some((idx, sink_changes, ping_pong, reason_flips, writes, reason))
            })
            .collect();
        rows.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| b.2.cmp(&a.2))
                .then_with(|| b.4.cmp(&a.4))
        });
        rows.truncate(top_n);
        rows
    }

    fn ensure_len(&mut self, len: usize) {
        if self.writes_this_minute.len() < len {
            self.writes_this_minute.resize(len, 0);
        }
        if self.reason_flips_this_minute.len() < len {
            self.reason_flips_this_minute.resize(len, 0);
        }
        if self.target_changes_this_minute.len() < len {
            self.target_changes_this_minute.resize(len, 0);
        }
        if self.ping_pong_this_minute.len() < len {
            self.ping_pong_this_minute.resize(len, 0);
        }
        if self.last_reason.len() < len {
            self.last_reason.resize_with(len, String::new);
        }
        if self.last_target_q.len() < len {
            self.last_target_q.resize(len, (0, 0));
        }
        if self.prev_target_q.len() < len {
            self.prev_target_q.resize(len, (0, 0));
        }
        if self.sink_writes_this_minute.len() < len {
            self.sink_writes_this_minute.resize(len, 0);
        }
        if self.sink_target_changes_this_minute.len() < len {
            self.sink_target_changes_this_minute.resize(len, 0);
        }
        if self.sink_ping_pong_this_minute.len() < len {
            self.sink_ping_pong_this_minute.resize(len, 0);
        }
        if self.sink_last_target.len() < len {
            self.sink_last_target.resize(len, (0.0, 0.0));
        }
        if self.sink_prev_target.len() < len {
            self.sink_prev_target.resize(len, (0.0, 0.0));
        }
        if self.sink_has_target.len() < len {
            self.sink_has_target.resize(len, false);
        }
    }
}

// ============================================================================
// MOVEMENT INTENT — Single-owner arbitration for NPC SetTarget
// ============================================================================

/// Priority ladder for movement intent resolution.
/// Higher value wins. Derive Ord so `max()` picks the winner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MovementPriority {
    Wander = 0,
    JobRoute = 1,
    Squad = 2,
    Combat = 3,
    Survival = 4,
    ManualTarget = 5,
    DirectControl = 6,
}

/// A single movement intent submitted by a gameplay system.
#[derive(Clone, Debug)]
pub struct MovementIntent {
    pub target: Vec2,
    pub priority: MovementPriority,
    pub source: &'static str,
}

/// Per-NPC intent map. Keyed by Entity, cleared every frame.
/// Sparse — only NPCs whose target changes get an entry.
#[derive(Resource, Default)]
pub struct MovementIntents {
    intents: HashMap<Entity, MovementIntent>,
}

impl MovementIntents {
    /// Submit a movement intent. Keeps the highest-priority intent per entity.
    #[inline]
    pub fn submit(
        &mut self,
        entity: Entity,
        target: Vec2,
        priority: MovementPriority,
        source: &'static str,
    ) {
        match self.intents.entry(entity) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                if priority > e.get().priority {
                    *e.get_mut() = MovementIntent {
                        target,
                        priority,
                        source,
                    };
                }
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(MovementIntent {
                    target,
                    priority,
                    source,
                });
            }
        }
    }

    /// Drain all intents for resolution. Clears the map but keeps allocation.
    pub fn drain(&mut self) -> std::collections::hash_map::Drain<'_, Entity, MovementIntent> {
        self.intents.drain()
    }
}

// ============================================================================
// UI CACHE RESOURCES
// ============================================================================

const NPC_LOG_CAPACITY: usize = 100;

/// Per-NPC metadata for UI display (names, levels, traits).
#[derive(Clone, Default)]
pub struct NpcMeta {
    pub name: String,
    pub level: i32,
    pub xp: i32,
    pub trait_id: i32,
    pub town_id: i32,
    pub job: i32,
}

/// A single log entry for an NPC's activity history.
#[derive(Clone)]
pub struct NpcLogEntry {
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub message: Cow<'static, str>,
}

/// Per-NPC metadata cache (names, levels, traits). Indexed by slot.
#[derive(Resource)]
pub struct NpcMetaCache(pub Vec<NpcMeta>);

impl Default for NpcMetaCache {
    fn default() -> Self {
        Self(vec![NpcMeta::default(); MAX_NPC_COUNT])
    }
}

/// Per-town NPC lists for O(1) roster queries. Index = town_id, value = Vec of NPC slots.
#[derive(Resource, Default)]
pub struct NpcsByTownCache(pub Vec<Vec<usize>>);

/// Per-NPC activity logs. Indexed by slot. 500 entries max per NPC.
#[derive(Resource)]
pub struct NpcLogCache {
    pub logs: Vec<VecDeque<NpcLogEntry>>,
    /// Filtering mode (synced from UserSettings each frame).
    pub mode: crate::settings::NpcLogMode,
    /// Currently selected NPC slot (-1 = none).
    pub selected: i32,
    /// Player faction id (for Faction mode filtering).
    pub player_faction: i32,
    /// Per-slot faction cache (set from decision_system iteration).
    slot_factions: Vec<i32>,
}

impl Default for NpcLogCache {
    fn default() -> Self {
        Self {
            logs: (0..MAX_NPC_COUNT).map(|_| VecDeque::new()).collect(),
            mode: crate::settings::NpcLogMode::SelectedOnly,
            selected: -1,
            player_faction: 0,
            slot_factions: vec![-1; MAX_NPC_COUNT],
        }
    }
}

impl NpcLogCache {
    /// Record a slot's faction (called during decision_system iteration).
    #[inline]
    pub fn set_slot_faction(&mut self, idx: usize, faction: i32) {
        if idx < self.slot_factions.len() {
            self.slot_factions[idx] = faction;
        }
    }

    /// Update selected NPC, clearing stale logs from previously selected NPC.
    pub fn update_selected(&mut self, new_selected: i32) {
        if new_selected != self.selected {
            // Clear previous selection's log when in SelectedOnly mode
            if self.mode == crate::settings::NpcLogMode::SelectedOnly {
                let old = self.selected as usize;
                if old < self.logs.len() {
                    self.logs[old].clear();
                }
            }
            self.selected = new_selected;
        }
    }

    /// Push a log message for an NPC with timestamp.
    /// Filtered by current mode — early-returns for NPCs outside the active scope.
    pub fn push(
        &mut self,
        idx: usize,
        day: i32,
        hour: i32,
        minute: i32,
        message: impl Into<Cow<'static, str>>,
    ) {
        if idx >= MAX_NPC_COUNT {
            return;
        }

        // Gate by mode
        match self.mode {
            crate::settings::NpcLogMode::SelectedOnly => {
                if self.selected < 0 || idx != self.selected as usize {
                    return;
                }
            }
            crate::settings::NpcLogMode::Faction => {
                if idx < self.slot_factions.len() && self.slot_factions[idx] != self.player_faction
                {
                    return;
                }
            }
            crate::settings::NpcLogMode::All => {}
        }

        let entry = NpcLogEntry {
            day,
            hour,
            minute,
            message: message.into(),
        };
        if let Some(log) = self.logs.get_mut(idx) {
            if log.len() >= NPC_LOG_CAPACITY {
                log.pop_front();
            }
            log.push_back(entry);
        }
    }
}

// ============================================================================
// PHASE 11.7: RESOURCES REPLACING STATICS
// ============================================================================

/// Shared slot allocator logic. Wraps a free-list allocator with configurable max.
pub struct SlotPool {
    pub next: usize,
    pub max: usize,
    pub free: Vec<usize>,
}

impl SlotPool {
    pub fn new(max: usize) -> Self {
        Self {
            next: 0,
            max,
            free: Vec::new(),
        }
    }
    pub fn alloc(&mut self) -> Option<usize> {
        self.free.pop().or_else(|| {
            if self.next < self.max {
                let idx = self.next;
                self.next += 1;
                Some(idx)
            } else {
                None
            }
        })
    }
    pub fn free(&mut self, slot: usize) {
        self.free.push(slot);
    }
    /// High-water mark: max slot index ever allocated. Use for GPU dispatch bounds.
    pub fn count(&self) -> usize {
        self.next
    }
    /// Currently alive: allocated minus freed. Use for UI display counts.
    pub fn alive(&self) -> usize {
        self.next - self.free.len()
    }
    pub fn reset(&mut self) {
        self.next = 0;
        self.free.clear();
    }
}

/// Unified entity slot allocator. NPCs and buildings share the same slot namespace.
/// Slot = GPU index (no offset arithmetic). Manages 0..MAX_ENTITIES with free list.
/// Every allocation queues a GPU state reset (drained by `populate_gpu_state`).
#[derive(Resource)]
pub struct GpuSlotPool {
    pool: SlotPool,
    pending_resets: Vec<usize>,
    pending_frees: Vec<usize>,
}

impl Default for GpuSlotPool {
    fn default() -> Self {
        Self {
            pool: SlotPool::new(MAX_ENTITIES),
            pending_resets: Vec::new(),
            pending_frees: Vec::new(),
        }
    }
}

impl GpuSlotPool {
    /// Allocate a slot and queue a full GPU state reset (prevents stale data from previous occupant).
    pub fn alloc_reset(&mut self) -> Option<usize> {
        let slot = self.pool.alloc()?;
        self.pending_resets.push(slot);
        Some(slot)
    }
    pub fn free(&mut self, slot: usize) {
        self.pool.free(slot);
        self.pending_frees.push(slot);
    }
    /// High-water mark: max slot index ever allocated.
    pub fn count(&self) -> usize {
        self.pool.count()
    }
    /// Currently alive: allocated minus freed.
    pub fn alive(&self) -> usize {
        self.pool.alive()
    }
    pub fn reset(&mut self) {
        self.pool.reset();
        self.pending_resets.clear();
        self.pending_frees.clear();
    }
    /// Drain slots needing GPU state reset. Called by `populate_gpu_state`.
    pub fn take_pending_resets(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.pending_resets)
    }
    /// Drain slots needing GPU hide cleanup. Called by `populate_gpu_state`.
    pub fn take_pending_frees(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.pending_frees)
    }
    /// Direct access for save/load that rebuilds allocator state.
    pub fn set_next(&mut self, n: usize) {
        self.pool.next = n;
    }
    /// Direct access to free list for save/load.
    pub fn free_list_mut(&mut self) -> &mut Vec<usize> {
        &mut self.pool.free
    }
    /// Read-only access to free list for debug display.
    pub fn free_list(&self) -> &[usize] {
        &self.pool.free
    }
    /// High-water mark (alias for debug display).
    pub fn next(&self) -> usize {
        self.pool.next
    }
}

/// Projectile slot allocator. Wraps SlotPool like GpuSlotPool.
#[derive(Resource)]
pub struct ProjSlotAllocator(pub SlotPool);

impl Default for ProjSlotAllocator {
    fn default() -> Self {
        Self(SlotPool::new(MAX_PROJECTILES))
    }
}

impl std::ops::Deref for ProjSlotAllocator {
    type Target = SlotPool;
    fn deref(&self) -> &SlotPool {
        &self.0
    }
}

impl std::ops::DerefMut for ProjSlotAllocator {
    fn deref_mut(&mut self) -> &mut SlotPool {
        &mut self.0
    }
}

/// GPU readback state. Populated by ReadbackComplete observers, read by main-world Bevy systems.
#[derive(Resource)]
pub struct GpuReadState {
    pub positions: Vec<f32>,      // [x0, y0, x1, y1, ...]
    pub combat_targets: Vec<i32>, // target index per NPC (-1 = none)
    pub health: Vec<f32>,
    pub factions: Vec<i32>,
    pub threat_counts: Vec<u32>, // packed (enemies << 16 | allies) per NPC
    pub npc_count: usize,
}

impl Default for GpuReadState {
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            combat_targets: Vec::new(),
            health: Vec::new(),
            factions: Vec::new(),
            threat_counts: Vec::new(),
            npc_count: 0,
        }
    }
}

/// GPU→CPU readback of projectile hit results. Each entry is [npc_idx, processed].
/// Populated by ReadbackComplete observer, read by process_proj_hits.
#[derive(Resource, Default)]
pub struct ProjHitState(pub Vec<[i32; 2]>);

/// GPU→CPU readback of projectile positions. [x0, y0, x1, y1, ...] flattened.
/// Populated by ReadbackComplete observer, read by extract_proj_data (ExtractSchedule).
#[derive(Resource, Default)]
pub struct ProjPositionState(pub Vec<f32>);

/// Food storage per location. Replaces FOOD_STORAGE static.
#[derive(Resource, Default)]
pub struct FoodStorage {
    pub food: Vec<i32>, // One entry per clan/location
}

impl FoodStorage {
    pub fn init(&mut self, count: usize) {
        self.food = vec![0; count];
    }
}

/// Gold storage per town. Mirrors FoodStorage.
#[derive(Resource, Default)]
pub struct GoldStorage {
    pub gold: Vec<i32>,
}

impl GoldStorage {
    pub fn init(&mut self, count: usize) {
        self.gold = vec![0; count];
    }
}

/// Per-faction statistics.
#[derive(Clone, Default)]
pub struct FactionStat {
    pub alive: i32,
    pub dead: i32,
    pub kills: i32,
}

/// Stats for all factions. Index 0 = player/villagers, 1+ = raider towns.
#[derive(Resource, Default)]
pub struct FactionStats {
    pub stats: Vec<FactionStat>,
}

/// Raider town state for respawning and foraging.
/// Faction 1+ are raider towns. Index 0 in this struct = faction 1.
#[derive(Resource, Default)]
pub struct RaiderState {
    /// Max raiders per town (set from config at init).
    pub max_pop: Vec<i32>,
    /// Hours accumulated since last respawn check.
    pub respawn_timers: Vec<f32>,
    /// Hours accumulated since last forage tick.
    pub forage_timers: Vec<f32>,
}

impl RaiderState {
    /// Initialize raider state for N towns.
    pub fn init(&mut self, count: usize, max_pop: i32) {
        self.max_pop = vec![max_pop; count];
        self.respawn_timers = vec![0.0; count];
        self.forage_timers = vec![0.0; count];
    }

    /// Get raider index from faction (faction 1 = index 0, etc).
    pub fn faction_to_idx(faction: i32) -> Option<usize> {
        if faction > 0 {
            Some((faction - 1) as usize)
        } else {
            None
        }
    }
}

impl FactionStats {
    pub fn init(&mut self, count: usize) {
        self.stats = vec![FactionStat::default(); count];
    }

    pub fn inc_alive(&mut self, faction: i32) {
        if let Some(s) = self.stats.get_mut(faction as usize) {
            s.alive += 1;
        }
    }

    pub fn dec_alive(&mut self, faction: i32) {
        if let Some(s) = self.stats.get_mut(faction as usize) {
            s.alive = (s.alive - 1).max(0);
        }
    }

    pub fn inc_dead(&mut self, faction: i32) {
        if let Some(s) = self.stats.get_mut(faction as usize) {
            s.dead += 1;
        }
    }

    pub fn inc_kills(&mut self, faction: i32) {
        if let Some(s) = self.stats.get_mut(faction as usize) {
            s.kills += 1;
        }
    }
}

// ============================================================================
// UI STATE
// ============================================================================

/// Active tab in the left panel.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum LeftPanelTab {
    #[default]
    Roster,
    Upgrades,
    Policies,
    Patrols,
    Squads,
    Factions,
    Profiler,
    Help,
}

/// Active category in the pause-menu settings panel.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum PauseSettingsTab {
    #[default]
    Interface,
    Video,
    Camera,
    Controls,
    Audio,
    Logs,
    Debug,
    SaveGame,
    LoadGame,
}

impl PauseSettingsTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Interface => "Interface",
            Self::Video => "Video",
            Self::Camera => "Camera",
            Self::Controls => "Controls",
            Self::Audio => "Audio",
            Self::Logs => "Logs",
            Self::Debug => "Debug",
            Self::SaveGame => "Save Game",
            Self::LoadGame => "Load Game",
        }
    }

    pub fn title_subtitle(self) -> (&'static str, &'static str) {
        match self {
            Self::Interface => ("Interface", "UI size, text readability, and display behavior."),
            Self::Video => ("Video", "Window resolution, vsync, and display behavior."),
            Self::Camera => ("Camera", "Panning, zoom speed, and sprite-detail transitions."),
            Self::Controls => ("Controls", "View and rebind keyboard shortcuts."),
            Self::Audio => ("Audio", "Music and sound effect levels."),
            Self::Logs => ("Logs", "Control what gets written to combat and activity logs."),
            Self::Debug => ("Debug", "Developer visibility and diagnostics toggles."),
            Self::SaveGame => ("Save Game", "Quicksave instantly or save manually by filename."),
            Self::LoadGame => ("Load Game", "Quickload or load a named/manual save file."),
        }
    }
}

/// Which UI panels are open. Toggled by keyboard shortcuts and HUD buttons.
#[derive(Resource)]
pub struct UiState {
    pub build_menu_open: bool,
    pub pause_menu_open: bool,
    pub pause_settings_tab: PauseSettingsTab,
    pub left_panel_open: bool,
    pub left_panel_tab: LeftPanelTab,
    pub combat_log_visible: bool,
    /// MinerHome building data index — next click assigns a gold mine.
    pub assigning_mine: Option<usize>,
    /// Currently selected faction in the Factions tab (for world overlays).
    pub factions_overlay_faction: Option<i32>,
    /// Preferred inspector tab after latest click when both NPC and building are selected.
    pub inspector_prefer_npc: bool,
    /// Monotonic click counter for inspector tab auto-focus application.
    pub inspector_click_seq: u64,
    /// True when the player's fountain has been destroyed — shows lose screen.
    pub game_over: bool,
    /// Tower upgrade popup — Some(slot) when open for a specific tower.
    pub tower_upgrade_slot: Option<usize>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            build_menu_open: false,
            pause_menu_open: false,
            pause_settings_tab: PauseSettingsTab::default(),
            left_panel_open: false,
            left_panel_tab: LeftPanelTab::default(),
            combat_log_visible: true,
            assigning_mine: None,
            factions_overlay_faction: None,
            inspector_prefer_npc: true,
            inspector_click_seq: 0,
            game_over: false,
            tower_upgrade_slot: None,
        }
    }
}

impl UiState {
    /// Toggle left panel to a specific tab, or close if already showing that tab.
    pub fn toggle_left_tab(&mut self, tab: LeftPanelTab) {
        if self.left_panel_open && self.left_panel_tab == tab {
            self.left_panel_open = false;
        } else {
            self.left_panel_open = true;
            self.left_panel_tab = tab;
        }
    }
}

// ============================================================================
// BUILD MENU STATE
// ============================================================================

/// Context for build palette + placement mode.
#[derive(Resource)]
pub struct BuildMenuContext {
    /// Which town in WorldData.towns this placement targets.
    pub town_data_idx: Option<usize>,
    /// Active building selection for click-to-place mode.
    pub selected_build: Option<crate::world::BuildingKind>,
    /// Destroy mode — click to remove buildings.
    pub destroy_mode: bool,
    /// Last hovered snapped world position (for indicators/tooltips).
    pub hover_world_pos: Vec2,
    /// Drag-line start slot in town-grid coordinates (row, col).
    pub drag_start_slot: Option<(i32, i32)>,
    /// Drag-line current/end slot in town-grid coordinates (row, col).
    pub drag_current_slot: Option<(i32, i32)>,
    /// Show the mouse-follow build hint sprite (hidden when snapped over a valid build slot).
    pub show_cursor_hint: bool,
    /// Bevy image handles for ghost preview sprites (populated by build_menu init).
    pub ghost_sprites: std::collections::HashMap<crate::world::BuildingKind, Handle<Image>>,
    /// Active build menu category tab.
    pub build_tab: crate::constants::DisplayCategory,
}

impl Default for BuildMenuContext {
    fn default() -> Self {
        Self {
            town_data_idx: None,
            selected_build: None,
            destroy_mode: false,
            hover_world_pos: Vec2::ZERO,
            drag_start_slot: None,
            drag_current_slot: None,
            show_cursor_hint: true,
            ghost_sprites: std::collections::HashMap::new(),
            build_tab: crate::constants::DisplayCategory::Economy,
        }
    }
}

impl BuildMenuContext {
    #[inline]
    pub fn clear_drag(&mut self) {
        self.drag_start_slot = None;
        self.drag_current_slot = None;
    }
}

// ============================================================================
// COMBAT LOG
// ============================================================================

/// Event type for combat log color coding.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CombatEventKind {
    Kill,
    Spawn,
    Raid,
    Harvest,
    LevelUp,
    Ai,
    BuildingDamage,
    Loot,
}

impl CombatEventKind {
    const COUNT: usize = 8;

    fn index(self) -> usize {
        match self {
            Self::Kill => 0,
            Self::Spawn => 1,
            Self::Raid => 2,
            Self::Harvest => 3,
            Self::LevelUp => 4,
            Self::Ai => 5,
            Self::BuildingDamage => 6,
            Self::Loot => 7,
        }
    }
}

/// A single combat log entry.
#[derive(Clone)]
pub struct CombatLogEntry {
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub kind: CombatEventKind,
    pub faction: i32,
    pub message: String,
    /// Optional world position — rendered as a clickable camera-pan button in the log.
    pub location: Option<bevy::math::Vec2>,
}

const COMBAT_LOG_PER_KIND: usize = 200;

/// Global combat event log. Per-kind ring buffers (200 each), newest at back.
#[derive(Resource)]
pub struct CombatLog {
    buffers: [VecDeque<CombatLogEntry>; CombatEventKind::COUNT],
}

impl Default for CombatLog {
    fn default() -> Self {
        Self {
            buffers: std::array::from_fn(|_| VecDeque::new()),
        }
    }
}

impl CombatLog {
    pub fn push(
        &mut self,
        kind: CombatEventKind,
        faction: i32,
        day: i32,
        hour: i32,
        minute: i32,
        message: String,
    ) {
        self.push_at(kind, faction, day, hour, minute, message, None);
    }

    pub fn push_at(
        &mut self,
        kind: CombatEventKind,
        faction: i32,
        day: i32,
        hour: i32,
        minute: i32,
        message: String,
        location: Option<bevy::math::Vec2>,
    ) {
        let buf = &mut self.buffers[kind.index()];
        if buf.len() >= COMBAT_LOG_PER_KIND {
            buf.pop_front();
        }
        buf.push_back(CombatLogEntry {
            day,
            hour,
            minute,
            kind,
            faction,
            message,
            location,
        });
    }

    pub fn iter_all(&self) -> impl Iterator<Item = &CombatLogEntry> {
        self.buffers.iter().flat_map(|b| b.iter())
    }
}

// ============================================================================
// BUILDING TOWER STATE
// ============================================================================

/// Per-building tower state for one building kind.
#[derive(Default)]
pub struct TowerKindState {
    /// Cooldown timer per building (seconds remaining).
    pub timers: Vec<f32>,
    /// Whether auto-attack is enabled per building.
    pub attack_enabled: Vec<bool>,
}

/// Tower state for all building kinds that can shoot.
#[derive(Resource, Default)]
pub struct TowerState {
    pub town: TowerKindState,
    /// Per-slot cooldown for player/AI-built Tower buildings.
    pub tower_cooldowns: std::collections::HashMap<usize, f32>,
}

// ============================================================================
// BUILDING SPAWNERS
// ============================================================================

/// A single placed building instance. All runtime state for one building.
#[derive(Clone)]
pub struct BuildingInstance {
    pub kind: crate::world::BuildingKind,
    pub position: Vec2,
    pub town_idx: u32,
    pub slot: usize,
    pub faction: i32,
    // Kind-specific fields (zero/None for non-applicable kinds)
    pub patrol_order: u32,                             // Waypoint only
    pub assigned_mine: Option<Vec2>,                   // MinerHome only
    pub manual_mine: bool,                             // MinerHome only
    pub wall_level: u8,                                // Wall only
    pub npc_uid: Option<crate::components::EntityUid>, // Spawner buildings only (None = no NPC alive)
    pub respawn_timer: f32,   // Spawner buildings only (-1.0 = not respawning)
    pub growth_ready: bool,   // Farm/Mine only (false = growing, true = ready to harvest)
    pub growth_progress: f32, // Farm/Mine only (0.0 to 1.0)
    pub occupants: i16,       // Farm/Mine only — number of NPCs working here
    pub under_construction: f32, // Seconds remaining; 0.0 = complete, >0 = constructing
    pub kills: i32,              // Tower/Fountain only — kill counter
    pub xp: i32,                 // Tower/Fountain only — XP (same scale as NPC: +100 per kill)
    pub upgrade_levels: Vec<u8>, // Tower only — per-stat upgrade levels (indices match TOWER_UPGRADES)
    pub auto_upgrade_flags: Vec<bool>, // Tower only — per-stat auto-buy flags (indices match TOWER_UPGRADES)
}

impl BuildingInstance {
    /// Harvest a Ready farm/mine. Resets to Growing, returns yield (farm=1 food, mine=MINE_EXTRACT_PER_CYCLE gold). Returns 0 if not Ready.
    pub fn harvest(&mut self) -> i32 {
        if !self.growth_ready {
            return 0;
        }
        self.growth_ready = false;
        self.growth_progress = 0.0;
        match self.kind {
            crate::world::BuildingKind::Farm => 1,
            crate::world::BuildingKind::GoldMine => crate::constants::MINE_EXTRACT_PER_CYCLE,
            _ => 0,
        }
    }

    /// Log message for a harvest event.
    pub fn harvest_log_msg(&self, yield_amount: i32) -> String {
        match self.kind {
            crate::world::BuildingKind::Farm => format!(
                "Farm harvested at ({:.0},{:.0})",
                self.position.x, self.position.y
            ),
            crate::world::BuildingKind::GoldMine => {
                format!("Mine harvested ({} gold)", yield_amount)
            }
            _ => String::new(),
        }
    }
}

/// Per-NPC runtime state. All NPC data lives here — no ECS components except GpuSlot.
/// Parallel to BuildingInstance: both live in EntityMap, shared slot namespace.
#[derive(Clone)]
/// Lightweight NPC index entry in EntityMap. All gameplay state lives on ECS components.
/// This provides slot↔Entity mapping and identity fields for fast iteration/filtering.
pub struct NpcEntry {
    pub slot: usize,
    pub entity: Entity,
    pub job: crate::components::Job,
    pub faction: i32,
    pub town_idx: i32,
    /// Set by death_system; entry removed on despawn.
    pub dead: bool,
}

/// Building HP render data. Read by build_overlay_instances for rendering.
#[derive(Resource, Default)]
pub struct BuildingHpRender {
    pub positions: Vec<Vec2>,
    pub health_pcts: Vec<f32>,
}

/// Per-town auto-upgrade flags. When enabled, upgrades are purchased automatically
/// once per game hour whenever the town has enough food.
#[derive(Resource)]
pub struct AutoUpgrade {
    pub flags: Vec<Vec<bool>>,
}

impl AutoUpgrade {
    /// Ensure flags vec has at least `n` town entries, each sized to current upgrade count.
    pub fn ensure_towns(&mut self, n: usize) {
        let count = crate::systems::stats::upgrade_count();
        while self.flags.len() < n {
            self.flags.push(vec![false; count]);
        }
        for v in &mut self.flags {
            v.resize(count, false);
        }
    }
}

impl Default for AutoUpgrade {
    fn default() -> Self {
        let count = crate::systems::stats::upgrade_count();
        Self {
            flags: vec![vec![false; count]; 16],
        }
    }
}

// ============================================================================
// TOWN POLICIES
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum WorkSchedule {
    #[default]
    Both,
    DayOnly,
    NightOnly,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum OffDutyBehavior {
    #[default]
    GoToBed,
    StayAtFountain,
    WanderTown,
}

fn default_policy_mining_radius() -> f32 {
    crate::constants::DEFAULT_MINING_RADIUS
}

/// Per-town behavior configuration. Controls flee thresholds, work schedules, off-duty behavior.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PolicySet {
    pub eat_food: bool,
    #[serde(alias = "guard_aggressive")]
    pub archer_aggressive: bool,
    #[serde(alias = "guard_leash")]
    pub archer_leash: bool,
    pub farmer_fight_back: bool,
    pub prioritize_healing: bool,
    pub farmer_flee_hp: f32, // 0.0-1.0 percentage
    #[serde(alias = "guard_flee_hp")]
    pub archer_flee_hp: f32,
    pub recovery_hp: f32, // 0.0-1.0 — go rest/heal when below this
    pub farmer_schedule: WorkSchedule,
    #[serde(alias = "guard_schedule")]
    pub archer_schedule: WorkSchedule,
    pub farmer_off_duty: OffDutyBehavior,
    #[serde(alias = "guard_off_duty")]
    pub archer_off_duty: OffDutyBehavior,
    #[serde(default = "default_policy_mining_radius")]
    pub mining_radius: f32,
}

impl Default for PolicySet {
    fn default() -> Self {
        Self {
            eat_food: true,
            archer_aggressive: false,
            archer_leash: true,
            farmer_fight_back: false,
            prioritize_healing: true,
            farmer_flee_hp: 0.30,
            archer_flee_hp: 0.15,
            recovery_hp: 0.80,
            farmer_schedule: WorkSchedule::Both,
            archer_schedule: WorkSchedule::Both,
            farmer_off_duty: OffDutyBehavior::GoToBed,
            archer_off_duty: OffDutyBehavior::GoToBed,
            mining_radius: crate::constants::DEFAULT_MINING_RADIUS,
        }
    }
}

/// Auto-mining cache and per-mine enable state.
#[derive(Resource, Default)]
pub struct MiningPolicy {
    /// Per-town discovered gold mine slots within policy radius.
    pub discovered_mines: Vec<Vec<usize>>,
    /// Per-gold-mine enabled toggle, keyed by EntityMap slot.
    pub mine_enabled: HashMap<usize, bool>,
}

// ============================================================================
// DIFFICULTY
// ============================================================================

/// Difficulty preset values for world gen.
pub struct DifficultyPreset {
    pub farms: usize,
    pub ai_towns: usize,
    pub raider_towns: usize,
    pub gold_mines: usize,
    /// Per-job NPC counts (only jobs listed are overridden; unlisted keep current value).
    pub npc_counts: std::collections::BTreeMap<crate::components::Job, usize>,
    pub endless_mode: bool,
    pub endless_strength: f32,
    pub raider_forage_hours: f32,
}

/// Game difficulty — scales building costs. Selected on main menu, immutable during play.
#[derive(
    Clone, Copy, PartialEq, Eq, Debug, Default, Resource, serde::Serialize, serde::Deserialize,
)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];

    pub fn label(self) -> &'static str {
        match self {
            Difficulty::Easy => "Easy",
            Difficulty::Normal => "Normal",
            Difficulty::Hard => "Hard",
        }
    }

    /// World gen presets. Overrides listed explicitly; unlisted jobs reset to NPC_REGISTRY defaults.
    pub fn presets(self) -> DifficultyPreset {
        use crate::components::Job;
        let (farms, ai_towns, raider_towns, gold_mines, endless_mode, endless_strength, raider_forage_hours, overrides) =
            match self {
                Difficulty::Easy => (
                    4,
                    2,
                    2,
                    3,
                    true,
                    0.5,
                    12.0,
                    vec![(Job::Farmer, 4), (Job::Archer, 8), (Job::Raider, 0)],
                ),
                Difficulty::Normal => (
                    2,
                    5,
                    5,
                    2,
                    true,
                    0.75,
                    6.0,
                    vec![(Job::Farmer, 2), (Job::Archer, 4), (Job::Raider, 1)],
                ),
                Difficulty::Hard => (
                    1,
                    20,
                    20,
                    1,
                    true,
                    1.25,
                    3.0,
                    vec![(Job::Farmer, 0), (Job::Archer, 2), (Job::Raider, 2)],
                ),
            };
        // Start from registry defaults, then apply preset overrides
        let mut npc_counts: std::collections::BTreeMap<Job, usize> = crate::constants::NPC_REGISTRY
            .iter()
            .map(|d| (d.job, d.default_count as usize))
            .collect();
        for (job, count) in overrides {
            npc_counts.insert(job, count);
        }
        DifficultyPreset {
            farms,
            ai_towns,
            raider_towns,
            gold_mines,
            npc_counts,
            endless_mode,
            endless_strength,
            raider_forage_hours,
        }
    }

    /// Migration group scaling: extra raiders per N player villagers.
    pub fn migration_scaling(self) -> i32 {
        match self {
            Difficulty::Easy => 6,
            Difficulty::Normal => 4,
            Difficulty::Hard => 2,
        }
    }
}

/// Per-town policy settings. Index matches WorldData.towns.
#[derive(Resource)]
pub struct TownPolicies {
    pub policies: Vec<PolicySet>,
}

impl Default for TownPolicies {
    fn default() -> Self {
        Self {
            policies: vec![PolicySet::default(); 16],
        }
    }
}

// ============================================================================
// SQUADS
// ============================================================================

/// Who controls a squad — player or an AI town.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum SquadOwner {
    #[default]
    Player,
    Town(usize), // town_data_idx
}

/// Returns true if the NPC's town matches the squad owner.
pub fn npc_matches_owner(owner: SquadOwner, npc_town_id: i32, player_town: i32) -> bool {
    match owner {
        SquadOwner::Player => npc_town_id == player_town,
        SquadOwner::Town(tdi) => npc_town_id == tdi as i32,
    }
}

/// A squad of combat units (player-controlled or AI-commanded).
#[derive(Clone)]
pub struct Squad {
    /// NPC UIDs assigned to this squad (stable across slot reuse).
    pub members: Vec<crate::components::EntityUid>,
    /// Squad target position. None = no target, guards patrol normally.
    pub target: Option<Vec2>,
    /// Desired member count. 0 = manual mode (no auto-recruit).
    pub target_size: usize,
    /// If true, squad members patrol waypoints when no squad target is set.
    pub patrol_enabled: bool,
    /// If true, squad members go home to rest when tired.
    pub rest_when_tired: bool,
    /// Wave state: true while this squad is actively attacking a target.
    pub wave_active: bool,
    /// Member count at wave start, used to detect heavy casualties.
    pub wave_start_count: usize,
    /// Minimum members required before a new wave can start.
    pub wave_min_start: usize,
    /// End wave when alive members drop below this percent of `wave_start_count`.
    pub wave_retreat_below_pct: usize,
    /// Squad owner: Player (indices 0..MAX_SQUADS) or AI Town (appended after).
    pub owner: SquadOwner,
    /// Hold fire: when true, members only attack their ManualTarget (no auto-engage).
    pub hold_fire: bool,
}

impl Squad {
    pub fn is_player(&self) -> bool {
        self.owner == SquadOwner::Player
    }
}

impl Default for Squad {
    fn default() -> Self {
        Self {
            members: Vec::new(),
            target: None,
            target_size: 0,
            patrol_enabled: true,
            rest_when_tired: true,
            wave_active: false,
            wave_start_count: 0,
            wave_min_start: 0,
            wave_retreat_below_pct: 50,
            owner: SquadOwner::Player,
            hold_fire: false,
        }
    }
}

/// All squads + UI state. First MAX_SQUADS are player-reserved; AI squads appended after.
#[derive(Resource)]
pub struct SquadState {
    pub squads: Vec<Squad>,
    /// Currently selected squad in UI (-1 = none).
    pub selected: i32,
    /// When true, next left-click sets the selected squad's target.
    pub placing_target: bool,
    /// Box-select drag: world-space start position (None = not dragging).
    pub drag_start: Option<Vec2>,
    /// True while mouse is held and drag exceeds threshold (5px).
    pub box_selecting: bool,
    /// DC NPCs keep fighting after looting instead of returning home.
    pub dc_no_return: bool,
}

impl Default for SquadState {
    fn default() -> Self {
        Self {
            squads: (0..crate::constants::MAX_SQUADS)
                .map(|_| Squad::default())
                .collect(),
            selected: 0,
            placing_target: false,
            drag_start: None,
            box_selecting: false,
            dc_no_return: false,
        }
    }
}

impl SquadState {
    /// Allocate a new squad with the given owner. Returns the squad index.
    pub fn alloc_squad(&mut self, owner: SquadOwner) -> usize {
        let idx = self.squads.len();
        self.squads.push(Squad {
            owner,
            ..Default::default()
        });
        idx
    }

    /// Iterate squads owned by a specific AI town.
    pub fn squads_for_town(&self, tdi: usize) -> impl Iterator<Item = (usize, &Squad)> {
        self.squads
            .iter()
            .enumerate()
            .filter(move |(_, s)| s.owner == SquadOwner::Town(tdi))
    }
}

// ============================================================================
// HELP CATALOG
// ============================================================================

/// In-game help tooltips. Flat map of topic key → help text.
/// Single source of truth for all "?" tooltip content.
#[derive(Resource)]
pub struct HelpCatalog(pub HashMap<&'static str, &'static str>);

impl HelpCatalog {
    pub fn new() -> Self {
        let mut m = HashMap::new();

        // Top bar stats
        m.insert("food", "Farmers grow food at farms. Spend it on buildings (right-click green '+' slots) and upgrades (U key). Build more Houses to get more farmers.");
        m.insert("gold", "Gold mines appear between towns. Set your miner count in the Roster tab (R key) using the Miners slider. Miners walk to the nearest mine, dig gold, and bring it back.");
        m.insert("pop", "Living NPCs / spawner buildings. Build Farmer Homes and Archer Homes to grow your town. Dead NPCs respawn after 12 game-hours.");
        m.insert("farmers", "Each Farmer Home spawns 1 farmer who works at the nearest free farm. Build farms first, then Farmer Homes to staff them.");
        m.insert("archers", "Each Archer Home spawns 1 archer who patrols waypoints. Build Waypoints to create a patrol route, then Archer Homes to staff them.");
        m.insert("raiders", "Enemy raiders steal food from your farms. Build archers and waypoints near farms to defend them.");
        m.insert("time", "Default: Space = pause/unpause. +/- = speed up/slow down (0x, 0.25x to 128x). 0x behaves as pause. Rebind in ESC > Settings > Controls.");

        // Left panel tabs
        m.insert("tab_roster", "Filter, sort, click to inspect. F to follow.");
        m.insert("tab_upgrades", "Spend food and gold on permanent upgrades.");
        m.insert(
            "tab_policies",
            "Work schedules, off-duty behavior, flee and aggro settings.",
        );
        m.insert(
            "tab_patrols",
            "Guard post patrol order. Use arrows to reorder.",
        );
        m.insert("tab_squads", "Set squad sizes and map targets. Default hotkeys are 1-9/0 (rebind in ESC > Settings > Controls).");
        m.insert(
            "tab_profiler",
            "Per-system timings. Enable in ESC > Settings > Debug.",
        );

        // Build menu
        m.insert(
            "build_farm",
            "Grows food over time. Build a Farmer Home nearby to assign a farmer to harvest it.",
        );
        m.insert(
            "build_farmer_home",
            "Spawns 1 farmer. Farmer works at the nearest free farm. Build farms first!",
        );
        m.insert(
            "build_archer_home",
            "Spawns 1 archer. Archer patrols nearby waypoints and fights enemies.",
        );
        m.insert(
            "build_waypoint",
            "Patrol waypoint for guards. Guards patrol between nearby waypoints and fight enemies.",
        );
        m.insert(
            "build_tent",
            "Spawns 1 raider. Raiders steal food from enemy farms and bring it back to their town.",
        );
        m.insert(
            "build_miner_home",
            "Spawns 1 miner. Miner works at the nearest gold mine.",
        );
        m.insert(
            "unlock_slot",
            "Pay food to unlock this grid slot. Then right-click it again to build.",
        );
        m.insert(
            "destroy",
            "Remove this building. Its NPC dies and the slot becomes empty.",
        );

        // Inspector (NPC)
        m.insert("npc_state", "What this NPC is currently doing. Working = at their job. Resting = recovering energy at home. Fighting = in combat.");
        m.insert("npc_energy", "Energy drains while active, recovers while resting at home. NPCs go rest when energy drops below 50, resume at 80.");
        m.insert("npc_trait", "Personality trait. 40% of NPCs spawn with one. Brave = never flees. Swift = +25% speed. Hardy = +25% HP.");
        m.insert(
            "npc_level",
            "Archers level up from kills. +1% all stats per level. XP needed = (level+1)^2 x 100.",
        );

        // Getting started
        m.insert("getting_started", "Welcome! Right-click green '+' slots to build.\n- Build Farms + Farmer Homes for food\n- Build Waypoints + Archer Homes for defense\n- Raiders will attack your farms\nKeys: R=roster, U=upgrades, P=policies, T=patrols, Q=squads, H=help");

        Self(m)
    }
}

// ============================================================================
// TUTORIAL STATE
// ============================================================================

/// Guided tutorial state machine. Step 0 = not started, 1-10 = active, 255 = done.
#[derive(Resource)]
pub struct TutorialState {
    pub step: u8,
    pub initial_farms: usize,
    pub initial_farmer_homes: usize,
    pub initial_waypoints: usize,
    pub initial_archer_homes: usize,
    pub initial_miner_homes: usize,
    pub camera_start: Vec2,
    /// Wall-clock seconds when tutorial started (for 10-minute auto-end).
    pub start_time: f64,
}

impl Default for TutorialState {
    fn default() -> Self {
        Self {
            step: 0,
            initial_farms: 0,
            initial_farmer_homes: 0,
            initial_waypoints: 0,
            initial_archer_homes: 0,
            initial_miner_homes: 0,
            camera_start: Vec2::ZERO,
            start_time: 0.0,
        }
    }
}

// ============================================================================
// MIGRATION STATE
// ============================================================================

/// Active migration group: boat → walk → settle lifecycle.
/// Phase 1 (boat): boat_slot is Some, member_slots empty, town_data_idx None
/// Phase 2 (walk): boat_slot None, member_slots filled, town_data_idx None
/// Phase 3 (settle): town created, NPCs get Home, migration cleared
pub struct MigrationGroup {
    // Boat phase
    pub boat_slot: Option<usize>,
    pub boat_pos: Vec2,
    /// Where the AI wants to settle (picked at boat spawn, far from existing towns).
    pub settle_target: Vec2,
    // Intent (from PendingAiSpawn)
    pub is_raider: bool,
    pub upgrade_levels: Vec<u8>,
    pub starting_food: i32,
    pub starting_gold: i32,
    // Set at disembark
    pub member_slots: Vec<usize>,
    pub faction: i32,
    // Set at settle
    pub town_data_idx: Option<usize>,
    pub grid_idx: usize,
}

/// Tracks dynamic raider town migrations.
#[derive(Resource, Default)]
pub struct MigrationState {
    pub active: Option<MigrationGroup>,
    pub check_timer: f32,
    /// Debug: force-spawn a migration group next frame (ignores cooldown/population checks).
    pub debug_spawn: bool,
}

/// Pending AI respawn queued by endless mode after a town is defeated.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct PendingAiSpawn {
    pub delay_remaining: f32,
    pub is_raider: bool,
    pub upgrade_levels: Vec<u8>,
    pub starting_food: i32,
    pub starting_gold: i32,
}

/// Endless mode: defeated AI enemies are replaced by new ones scaled to player strength.
#[derive(Resource)]
pub struct EndlessMode {
    pub enabled: bool,
    /// Fraction of player strength for replacement AI (0.25–1.5)
    pub strength_fraction: f32,
    pub pending_spawns: Vec<PendingAiSpawn>,
}

impl Default for EndlessMode {
    fn default() -> Self {
        Self {
            enabled: false,
            strength_fraction: 0.75,
            pending_spawns: Vec::new(),
        }
    }
}

/// Pre-computed healing zone per town, indexed by faction for O(1) lookup.
pub struct HealingZone {
    pub center: Vec2,
    pub enter_radius_sq: f32,
    pub exit_radius_sq: f32,
    pub heal_rate: f32,
    pub town_idx: usize,
    pub faction: i32,
}

/// Faction-indexed healing zone cache. Rebuilt when HealingZonesDirtyMsg is received.
#[derive(Resource, Default)]
pub struct HealingZoneCache {
    pub by_faction: Vec<Vec<HealingZone>>,
}

/// Tracks whether any buildings are damaged and need fountain healing.
/// Separate resource because this is persistent state (stays true while damage exists),
/// unlike the one-shot dirty signals which are now Bevy Messages.
#[derive(Resource, Default)]
pub struct BuildingHealState {
    pub needs_healing: bool,
}

/// Tracks NPC slots currently in a healing zone. Sustain-check iterates only these.
#[derive(Resource)]
pub struct ActiveHealingSlots {
    pub slots: Vec<usize>,
    pub mark: Vec<u8>,
}

impl Default for ActiveHealingSlots {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            mark: vec![0u8; crate::constants::MAX_ENTITIES],
        }
    }
}

// ============================================================================
// AUDIO
// ============================================================================

/// Runtime audio state — volume levels and loaded track handles.
#[derive(Resource)]
pub struct GameAudio {
    pub music_volume: f32,
    pub sfx_volume: f32,
    pub tracks: Vec<Handle<AudioSource>>,
    pub last_track: Option<usize>,
    pub loop_current: bool,
    /// UI-requested track — set by jukebox dropdown, consumed by jukebox_system.
    pub play_next: Option<usize>,
    /// Playback speed multiplier (0.25-2.0, default 1.0).
    pub music_speed: f32,
    /// SFX variant handles keyed by kind — multiple variants per kind for random selection.
    pub sfx_handles: std::collections::HashMap<SfxKind, Vec<Handle<AudioSource>>>,
    /// Whether arrow shoot SFX plays (disabled by default — the sound is rough).
    pub sfx_shoot_enabled: bool,
}

impl Default for GameAudio {
    fn default() -> Self {
        Self {
            music_volume: 0.3,
            sfx_volume: 0.15,
            tracks: Vec::new(),
            last_track: None,
            loop_current: false,
            play_next: None,
            music_speed: 1.0,
            sfx_handles: std::collections::HashMap::new(),
            sfx_shoot_enabled: false,
        }
    }
}

/// Marker component for the currently playing music entity.
#[derive(Component)]
pub struct MusicTrack;

/// Sound effect categories.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum SfxKind {
    ArrowShoot,
    Death,
    Build,
    Click,
    Upgrade,
}

/// Fire-and-forget SFX trigger message. Position enables spatial culling (None = always play).
#[derive(Message, Clone)]
pub struct PlaySfxMsg {
    pub kind: SfxKind,
    pub position: Option<Vec2>,
}

// Test12 relocated to src/tests/vertical_slice.rs — uses shared TestState resource.
