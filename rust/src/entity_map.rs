//! EntityMap — unified entity registry for NPCs and buildings.

use bevy::prelude::*;
use hashbrown::HashMap;

use crate::constants::TOWN_GRID_SPACING;

/// Dense slot map with O(1) insert, O(1) remove, and cache-friendly iteration.
/// Parallel `slots`/`data` arrays + reverse index (slot → position) for swap_remove.
/// For `T = ()`, data is zero-sized (optimized away by compiler).
#[derive(Clone)]
pub struct DenseSlotMap<T: Clone> {
    slots: Vec<usize>,
    data: Vec<T>,
    index: HashMap<usize, usize>,
}

impl<T: Clone> Default for DenseSlotMap<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            data: Vec::new(),
            index: HashMap::new(),
        }
    }
}

impl<T: Clone> DenseSlotMap<T> {
    pub fn insert(&mut self, slot: usize, value: T) {
        if let Some(&idx) = self.index.get(&slot) {
            self.data[idx] = value;
        } else {
            let idx = self.slots.len();
            self.slots.push(slot);
            self.data.push(value);
            self.index.insert(slot, idx);
        }
    }

    pub fn remove(&mut self, slot: usize) -> Option<T> {
        if let Some(idx) = self.index.remove(&slot) {
            self.slots.swap_remove(idx);
            let value = self.data.swap_remove(idx);
            if idx < self.slots.len() {
                self.index.insert(self.slots[idx], idx);
            }
            Some(value)
        } else {
            None
        }
    }

    pub fn get(&self, slot: usize) -> Option<&T> {
        self.index.get(&slot).map(|&idx| &self.data[idx])
    }

    pub fn get_mut(&mut self, slot: usize) -> Option<&mut T> {
        self.index
            .get(&slot)
            .copied()
            .map(move |idx| &mut self.data[idx])
    }

    pub fn slot_slice(&self) -> &[usize] {
        &self.slots
    }

    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.data.iter()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.data.iter_mut()
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn clear(&mut self) {
        self.slots.clear();
        self.data.clear();
        self.index.clear();
    }
}

/// Thin wrapper: DenseSlotMap<()> with slot-only API.
#[derive(Clone, Default)]
pub struct DenseSlotSet(DenseSlotMap<()>);

impl DenseSlotSet {
    pub fn insert(&mut self, slot: usize) {
        self.0.insert(slot, ());
    }
    pub fn remove(&mut self, slot: usize) {
        self.0.remove(slot);
    }
    pub fn as_slice(&self) -> &[usize] {
        self.0.slot_slice()
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Lightweight building index entry in EntityMap. All gameplay state lives on ECS components.
/// Provides slot↔Entity mapping and identity fields for fast iteration/spatial queries.
/// Pure identity + spatial — no gameplay state. Occupancy tracked separately in EntityMap.occupancy.
#[derive(Clone)]
pub struct BuildingInstance {
    pub kind: crate::world::BuildingKind,
    pub position: Vec2,
    pub town_idx: u32,
    pub slot: usize,
    pub faction: i32,
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

/// Unified entity registry — ALL entities (NPCs + buildings) slot→entity mapping,
/// plus building-specific instance data, spatial grid, and indexes.
/// Populated on NPC spawn and building placement, used by damage/combat/rendering for entity lookup.
#[derive(Resource, Default)]
pub struct EntityMap {
    /// ALL entities (NPCs + buildings) — unified slot→entity
    pub entities: HashMap<usize, Entity>,
    /// Reverse index: entity→slot (bijection with `entities`)
    pub(crate) entity_to_slot: HashMap<Entity, usize>,

    // Building-specific data (dense storage for cache-friendly iteration)
    instances: DenseSlotMap<BuildingInstance>,
    by_kind: HashMap<crate::world::BuildingKind, DenseSlotMap<BuildingInstance>>,
    by_kind_town: HashMap<(crate::world::BuildingKind, u32), DenseSlotMap<BuildingInstance>>,
    by_grid_cell: HashMap<(i32, i32), usize>,
    spawner_slots: DenseSlotSet,
    /// Worksite claim counts — slot->i16, incremented at claim time.
    /// Used for capacity checks (prevents double-claims on max_occupants slots).
    pub(crate) occupancy: DenseSlotMap<i16>,
    /// Worksite physical presence counts — slot->i16, incremented on worker arrival.
    /// Used by growth_system/sleeping_sync to gate tended production rates.
    pub(crate) present: DenseSlotMap<i16>,

    // NPC-specific data (index-only — gameplay state on ECS components)
    npcs: HashMap<usize, NpcEntry>,
    npc_by_town: HashMap<i32, DenseSlotSet>,
    // Per-worksite FIFO claim order: slot -> claimer entity queue.
    worksite_claim_queue: HashMap<usize, Vec<Entity>>,

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
    // ── Entity ↔ slot bijection ────────────────────────────────────────

    /// Register a slot↔entity mapping (forward + reverse).
    pub fn set_entity(&mut self, slot: usize, entity: Entity) {
        if let Some(old_entity) = self.entities.insert(slot, entity) {
            self.entity_to_slot.remove(&old_entity);
        }
        self.entity_to_slot.insert(entity, slot);
    }

    /// Remove a slot↔entity mapping (forward + reverse).
    fn remove_entity_mapping(&mut self, slot: usize) {
        if let Some(entity) = self.entities.remove(&slot) {
            self.entity_to_slot.remove(&entity);
        }
    }

    /// Look up the slot for an entity. O(1) via reverse index.
    pub fn slot_for_entity(&self, entity: Entity) -> Option<usize> {
        self.entity_to_slot.get(&entity).copied()
    }

    // ── Building instance API ──────────────────────────────────────────

    /// Look up a building instance by entity (resolves entity→slot→instance).
    pub fn instance_by_entity(&self, entity: Entity) -> Option<&BuildingInstance> {
        self.slot_for_entity(entity)
            .and_then(|slot| self.instances.get(slot))
    }

    /// Remove a building by its slot. Removes entity mapping AND instance data.
    pub fn remove_by_slot(&mut self, slot: usize) {
        self.remove_entity_mapping(slot);
        self.remove_instance(slot);
    }

    /// Clear all building data (instances, indexes, spatial grid). Does NOT clear entities.
    pub fn clear_buildings(&mut self) {
        self.instances.clear();
        self.by_kind.clear();
        self.by_kind_town.clear();
        self.by_grid_cell.clear();
        self.spawner_slots.clear();
        self.worksite_claim_queue.clear();
        self.spatial_cells.iter_mut().for_each(|c| c.clear());
        self.spatial_kind_town.clear();
        self.spatial_kind_cell.clear();
        self.spatial_bucket_idx.clear();
    }

    /// Number of building instances.
    pub fn building_count(&self) -> usize {
        self.instances.len()
    }

    /// Iterate all entity instance slot keys.
    pub fn all_entity_slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.instances.slot_slice().iter().copied()
    }

    /// Add or update a building instance. Updates all indexes.
    /// If the slot already exists, removes old index entries first to avoid duplicates.
    pub fn add_instance(&mut self, inst: BuildingInstance) {
        let slot = inst.slot;
        let kind = inst.kind;
        // Remove old index entries if updating an existing slot
        if let Some(old) = self.instances.remove(slot) {
            if let Some(slots) = self.by_kind.get_mut(&old.kind) {
                slots.remove(slot);
            }
            if let Some(slots) = self.by_kind_town.get_mut(&(old.kind, old.town_idx)) {
                slots.remove(slot);
            }
            let old_gc = (old.position.x / TOWN_GRID_SPACING).floor() as i32;
            let old_gr = (old.position.y / TOWN_GRID_SPACING).floor() as i32;
            self.by_grid_cell.remove(&(old_gc, old_gr));
            self.spatial_remove(slot, old.position);
            self.spawner_slots.remove(slot);
        }
        self.by_kind
            .entry(kind)
            .or_default()
            .insert(slot, inst.clone());
        self.by_kind_town
            .entry((kind, inst.town_idx))
            .or_default()
            .insert(slot, inst.clone());
        let gc = (inst.position.x / TOWN_GRID_SPACING).floor() as i32;
        let gr = (inst.position.y / TOWN_GRID_SPACING).floor() as i32;
        self.by_grid_cell.insert((gc, gr), slot);
        let is_spawner = crate::constants::building_def(inst.kind).spawner.is_some();
        let pos = inst.position;
        self.instances.insert(slot, inst);
        self.occupancy.insert(slot, 0);
        self.present.insert(slot, 0);
        self.spatial_insert(slot, pos);
        if is_spawner {
            self.spawner_slots.insert(slot);
        }
    }

    /// Remove an instance by slot. Returns removed instance if any.
    fn remove_instance(&mut self, slot: usize) -> Option<BuildingInstance> {
        if let Some(inst) = self.instances.remove(slot) {
            if let Some(slots) = self.by_kind.get_mut(&inst.kind) {
                slots.remove(slot);
            }
            if let Some(slots) = self.by_kind_town.get_mut(&(inst.kind, inst.town_idx)) {
                slots.remove(slot);
            }
            let gc = (inst.position.x / TOWN_GRID_SPACING).floor() as i32;
            let gr = (inst.position.y / TOWN_GRID_SPACING).floor() as i32;
            self.by_grid_cell.remove(&(gc, gr));
            self.spatial_remove(slot, inst.position);
            self.spawner_slots.remove(slot);
            self.occupancy.remove(slot);
            self.present.remove(slot);
            self.worksite_claim_queue.remove(&slot);
            Some(inst)
        } else {
            None
        }
    }

    pub fn get_instance(&self, slot: usize) -> Option<&BuildingInstance> {
        self.instances.get(slot)
    }

    pub fn get_instance_mut(&mut self, slot: usize) -> Option<&mut BuildingInstance> {
        self.instances.get_mut(slot)
    }

    pub fn iter_instances(&self) -> impl Iterator<Item = &BuildingInstance> {
        self.instances.values()
    }

    pub fn iter_instances_mut(&mut self) -> impl Iterator<Item = &mut BuildingInstance> {
        self.instances.values_mut()
    }

    pub fn spawner_slots(&self) -> &[usize] {
        self.spawner_slots.as_slice()
    }

    pub fn iter_kind(
        &self,
        kind: crate::world::BuildingKind,
    ) -> impl Iterator<Item = &BuildingInstance> {
        self.by_kind.get(&kind).into_iter().flat_map(|m| m.values())
    }

    pub fn iter_kind_for_town(
        &self,
        kind: crate::world::BuildingKind,
        town_idx: u32,
    ) -> impl Iterator<Item = &BuildingInstance> {
        self.by_kind_town
            .get(&(kind, town_idx))
            .into_iter()
            .flat_map(|m| m.values())
    }

    pub fn count_for_town(&self, kind: crate::world::BuildingKind, town_idx: u32) -> usize {
        self.by_kind_town
            .get(&(kind, town_idx))
            .map_or(0, |m| m.len())
    }

    pub fn building_counts(&self, town_idx: u32) -> HashMap<crate::world::BuildingKind, usize> {
        let mut counts = HashMap::new();
        for (&kind, map) in &self.by_kind {
            let count = map.values().filter(|i| i.town_idx == town_idx).count();
            if count > 0 {
                counts.insert(kind, count);
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
        let gc = (pos.x / TOWN_GRID_SPACING).floor() as i32;
        let gr = (pos.y / TOWN_GRID_SPACING).floor() as i32;
        self.by_grid_cell
            .get(&(gc, gr))
            .and_then(|&s| self.instances.get(s))
    }

    pub fn find_by_position_mut(&mut self, pos: Vec2) -> Option<&mut BuildingInstance> {
        let gc = (pos.x / TOWN_GRID_SPACING).floor() as i32;
        let gr = (pos.y / TOWN_GRID_SPACING).floor() as i32;
        let slot = self.by_grid_cell.get(&(gc, gr)).copied()?;
        self.instances.get_mut(slot)
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

    pub fn slot_at_position(&self, pos: Vec2) -> Option<usize> {
        let gc = (pos.x / TOWN_GRID_SPACING).floor() as i32;
        let gr = (pos.y / TOWN_GRID_SPACING).floor() as i32;
        self.by_grid_cell.get(&(gc, gr)).copied()
    }

    pub fn has_building_at(&self, gc: i32, gr: i32) -> bool {
        self.by_grid_cell.contains_key(&(gc, gr))
    }

    pub fn get_at_grid(&self, gc: i32, gr: i32) -> Option<&BuildingInstance> {
        self.by_grid_cell
            .get(&(gc, gr))
            .and_then(|&s| self.instances.get(s))
    }

    // ── Occupancy ─────────────────────────────────────────────────────

    pub fn release(&mut self, slot: usize) {
        self.release_for(slot, None);
    }

    pub fn release_for(&mut self, slot: usize, claimer: Option<Entity>) {
        if let Some(occ) = self.occupancy.get_mut(slot) {
            *occ = occ.saturating_sub(1);
            if let Some(claimer_entity) = claimer {
                if let Some(queue) = self.worksite_claim_queue.get_mut(&slot) {
                    if let Some(pos) = queue.iter().position(|&u| u == claimer_entity) {
                        queue.remove(pos);
                    }
                    if queue.is_empty() {
                        self.worksite_claim_queue.remove(&slot);
                    }
                }
            } else if *occ <= 0 {
                self.worksite_claim_queue.remove(&slot);
            }
        } else {
            self.worksite_claim_queue.remove(&slot);
        }
        // Decrement physical presence if any (guard against underflow).
        if let Some(p) = self.present.get_mut(slot) {
            if *p > 0 {
                *p -= 1;
            }
        }
    }

    pub fn occupant_count(&self, slot: usize) -> i32 {
        self.occupancy.get(slot).map_or(0, |&occ| occ as i32)
    }

    pub fn is_occupied(&self, slot: usize) -> bool {
        self.occupancy.get(slot).is_some_and(|&occ| occ >= 1)
    }

    /// Directly set occupancy for a slot. Used by tests/benches; gameplay code should
    /// use `try_claim_worksite`/`release` instead.
    pub fn set_occupancy(&mut self, slot: usize, count: i16) {
        self.occupancy.insert(slot, count);
    }

    /// First claimer entity at a worksite (the farmer actively tending it).
    pub fn worksite_claimer(&self, slot: usize) -> Option<Entity> {
        self.worksite_claim_queue
            .get(&slot)
            .and_then(|queue| queue.first().copied())
    }

    /// Increment physical presence count for a slot. Called when a worker physically
    /// arrives at the worksite (transitions to Active/Holding phase).
    pub fn mark_present(&mut self, slot: usize) {
        if let Some(p) = self.present.get_mut(slot) {
            *p += 1;
        }
    }

    /// Number of workers physically present at the worksite. Used by growth_system
    /// to gate tended production rates (as opposed to just claimed/reserved slots).
    pub fn present_count(&self, slot: usize) -> i32 {
        self.present.get(slot).map_or(0, |&p| p as i32)
    }

    /// Directly set present count for a slot. Used by tests; gameplay code should
    /// use `mark_present` via WorkIntent::MarkPresent.
    pub fn set_present(&mut self, slot: usize, count: i16) {
        self.present.insert(slot, count);
    }

    pub fn is_worksite_harvest_turn(&self, slot: usize, claimer: Entity) -> bool {
        self.worksite_claim_queue
            .get(&slot)
            .and_then(|queue| queue.first().copied())
            .is_none_or(|front| front == claimer)
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
        self.set_entity(slot, entity);
        self.npc_by_town.entry(town_idx).or_default().insert(slot);
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
        self.remove_entity_mapping(slot);
        if let Some(entry) = self.npcs.remove(&slot) {
            if let Some(slots) = self.npc_by_town.get_mut(&entry.town_idx) {
                slots.remove(slot);
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

    pub fn slots_for_town(&self, town_idx: i32) -> &[usize] {
        self.npc_by_town
            .get(&town_idx)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn npcs_for_town(&self, town_idx: i32) -> impl Iterator<Item = &NpcEntry> {
        let npcs = &self.npcs;
        self.npc_by_town
            .get(&town_idx)
            .into_iter()
            .flat_map(|v| v.as_slice().iter())
            .filter_map(move |&s| npcs.get(&s))
    }

    pub fn npc_count(&self) -> usize {
        self.npcs.len()
    }

    pub fn clear_npcs(&mut self) {
        let npc_slots: Vec<usize> = self.npcs.keys().copied().collect();
        for slot in npc_slots {
            if let Some(entity) = self.entities.remove(&slot) {
                self.entity_to_slot.remove(&entity);
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
            if let Some(inst) = self.instances.get(slot) {
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

    pub fn for_each_nearby(
        &self,
        pos: Vec2,
        radius: f32,
        mut f: impl FnMut(&BuildingInstance, i16),
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
            let row = cy * self.spatial_width;
            for cx in min_cx..=max_cx {
                for &slot in &self.spatial_cells[row + cx] {
                    if let Some(inst) = self.instances.get(slot) {
                        let occ = self.occupancy.get(slot).copied().unwrap_or(0);
                        f(inst, occ);
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
        mut f: impl FnMut(&BuildingInstance, i16),
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
                        if let Some(inst) = self.instances.get(slot) {
                            let occ = self.occupancy.get(slot).copied().unwrap_or(0);
                            f(inst, occ);
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
        mut f: impl FnMut(&BuildingInstance, i16),
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
                        if let Some(inst) = self.instances.get(slot) {
                            let occ = self.occupancy.get(slot).copied().unwrap_or(0);
                            f(inst, occ);
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
        mut f: impl FnMut(&BuildingInstance, i16),
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
                    let dx = cx.abs_diff(center_cx);
                    let dy = cy.abs_diff(center_cy);
                    if dx < inner_cell_r && dy < inner_cell_r {
                        continue;
                    }
                }
                let cell_idx = cy * w + cx;
                if let Some(bucket) = self.spatial_kind_town.get(&(kind, town_idx, cell_idx)) {
                    for &slot in bucket {
                        if let Some(inst) = self.instances.get(slot) {
                            let occ = self.occupancy.get(slot).copied().unwrap_or(0);
                            f(inst, occ);
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
        mut f: impl FnMut(&BuildingInstance, i16),
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
                    let dx = cx.abs_diff(center_cx);
                    let dy = cy.abs_diff(center_cy);
                    if dx < inner_cell_r && dy < inner_cell_r {
                        continue;
                    }
                }
                let cell_idx = cy * w + cx;
                if let Some(bucket) = self.spatial_kind_cell.get(&(kind, cell_idx)) {
                    for &slot in bucket {
                        if let Some(inst) = self.instances.get(slot) {
                            let occ = self.occupancy.get(slot).copied().unwrap_or(0);
                            f(inst, occ);
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
        mut score: impl FnMut(&BuildingInstance, i16) -> Option<S>,
    ) -> Option<WorksiteResult> {
        debug_assert!(town_idx != u32::MAX, "town_idx looks like -1 as u32");
        let max_cell_r = self.cell_radius(max_radius);
        let mut best: Option<(S, usize, Vec2)> = None;

        // Town-scoped expanding ring search
        let mut prev_r: usize = 0;
        let mut cell_r: usize = 0; // start with center cell (r=0)
        loop {
            self.for_each_ring_kind_town(from, prev_r, cell_r, kind, town_idx, |inst, occ| {
                if let Some(s) = score(inst, occ) {
                    if best.as_ref().is_none_or(|b| s < b.0) {
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
                self.for_each_ring_kind(from, prev_r, cell_r, kind, |inst, occ| {
                    if let Some(s) = score(inst, occ) {
                        if best.as_ref().is_none_or(|b| s < b.0) {
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
        claimer: Option<Entity>,
    ) -> Option<ClaimedWorksite> {
        let occ = self.occupancy.get(slot).copied().unwrap_or(0) as i32;
        let valid = self.instances.get(slot).is_some_and(|inst| {
            inst.kind == expected_kind
                && expected_town.is_none_or(|t| {
                    inst.town_idx == t || inst.town_idx == crate::constants::TOWN_NONE
                })
                && occ < max_occupants
        });
        if valid {
            if let Some(o) = self.occupancy.get_mut(slot) {
                *o += 1;
            }
            if let Some(claimer_entity) = claimer {
                let queue = self.worksite_claim_queue.entry(slot).or_default();
                if !queue.contains(&claimer_entity) {
                    queue.push(claimer_entity);
                }
            }
            let position = self.instances.get(slot).expect("validated above").position;
            Some(ClaimedWorksite { slot, position })
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
