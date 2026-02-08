//! API - Query and UI methods exposed to GDScript.
//! Uses #[godot_api(secondary)] to allow splitting impl blocks across files.

use godot::prelude::*;

// Import generated registration types for secondary impl blocks
use crate::__registration_methods_EcsNpcManager;
use crate::__registration_constants_EcsNpcManager;
use crate::__godot_EcsNpcManager_Funcs;

use crate::{EcsNpcManager, GPU_READ_STATE, ARRIVAL_QUEUE};
use crate::resources::{self, NpcEntityMap};
use crate::components;
use crate::world;
use crate::{derive_npc_state, job_name, trait_name};

#[godot_api(secondary)]
impl EcsNpcManager {
    // ========================================================================
    // UI QUERY API (Phase 9.4)
    // ========================================================================

    /// Get population statistics for UI display.
    #[func]
    pub fn get_population_stats(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let mut farmers_alive = 0i32;
        let mut guards_alive = 0i32;
        let mut raiders_alive = 0i32;

        // Count alive NPCs from NpcsByTownCache + GPU health
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let (Some(by_town), Some(meta)) = (
                    app.world().get_resource::<resources::NpcsByTownCache>(),
                    app.world().get_resource::<resources::NpcMetaCache>(),
                ) {
                    if let Ok(state) = GPU_READ_STATE.lock() {
                        for town_npcs in by_town.0.iter() {
                            for &idx in town_npcs {
                                if idx < state.health.len() && state.health[idx] > 0.0 {
                                    match meta.0[idx].job {
                                        0 => farmers_alive += 1,
                                        1 => guards_alive += 1,
                                        2 => raiders_alive += 1,
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(kills) = app.world().get_resource::<resources::KillStats>() {
                    dict.set("guard_kills", kills.guard_kills);
                    dict.set("villager_kills", kills.villager_kills);
                }
                // Get dead counts from PopulationStats (tracks by job)
                if let Some(pop) = app.world().get_resource::<resources::PopulationStats>() {
                    let mut farmers_dead = 0i32;
                    let mut guards_dead = 0i32;
                    let mut raiders_dead = 0i32;
                    for ((job, _clan), stats) in pop.0.iter() {
                        match job {
                            0 => farmers_dead += stats.dead,
                            1 => guards_dead += stats.dead,
                            2 => raiders_dead += stats.dead,
                            _ => {}
                        }
                    }
                    dict.set("farmers_dead", farmers_dead);
                    dict.set("guards_dead", guards_dead);
                    dict.set("raiders_dead", raiders_dead);
                }
            }
        }

        dict.set("farmers_alive", farmers_alive);
        dict.set("guards_alive", guards_alive);
        dict.set("raiders_alive", raiders_alive);
        dict
    }

    /// Get population for a specific town.
    #[func]
    pub fn get_town_population(&self, town_idx: i32) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let mut farmer_count = 0i32;
        let mut guard_count = 0i32;
        let mut raider_count = 0i32;

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let (Some(by_town), Some(meta)) = (
                    app.world().get_resource::<resources::NpcsByTownCache>(),
                    app.world().get_resource::<resources::NpcMetaCache>(),
                ) {
                    if let Ok(state) = GPU_READ_STATE.lock() {
                        if (town_idx as usize) < by_town.0.len() {
                            for &idx in &by_town.0[town_idx as usize] {
                                if idx < state.health.len() && state.health[idx] > 0.0 {
                                    match meta.0[idx].job {
                                        0 => farmer_count += 1,
                                        1 => guard_count += 1,
                                        2 => raider_count += 1,
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        dict.set("farmer_count", farmer_count);
        dict.set("guard_count", guard_count);
        dict.set("raider_count", raider_count);
        dict
    }

    /// Get detailed info for a single NPC (for inspector panel).
    #[func]
    pub fn get_npc_info(&self, idx: i32) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let i = idx as usize;

        // GPU data: positions, targets, factions (these come from GPU compute)
        if let Ok(state) = GPU_READ_STATE.lock() {
            if i < state.npc_count {
                dict.set("x", state.positions.get(i * 2).copied().unwrap_or(0.0));
                dict.set("y", state.positions.get(i * 2 + 1).copied().unwrap_or(0.0));
                dict.set("faction", state.factions.get(i).copied().unwrap_or(0));
                dict.set("target_idx", state.combat_targets.get(i).copied().unwrap_or(-1));
            }
        }

        // Bevy data: components and resources
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                // Meta cache (name, level, trait, etc.)
                if let Some(meta) = app.world().get_resource::<resources::NpcMetaCache>() {
                    if i < meta.0.len() {
                        dict.set("name", GString::from(&meta.0[i].name));
                        dict.set("level", meta.0[i].level);
                        dict.set("xp", meta.0[i].xp);
                        dict.set("trait", GString::from(trait_name(meta.0[i].trait_id)));
                        dict.set("town_id", meta.0[i].town_id);
                        dict.set("job", GString::from(job_name(meta.0[i].job)));
                    }
                }
                // Entity components (HP, Energy, State)
                if let Some(npc_map) = app.world().get_resource::<NpcEntityMap>() {
                    if let Some(&entity) = npc_map.0.get(&i) {
                        dict.set("state", GString::from(derive_npc_state(app.world(), entity)));
                        // Read HP directly from component
                        if let Some(health) = app.world().get::<components::Health>(entity) {
                            dict.set("hp", health.0);
                        }
                        // Read Energy directly from component
                        if let Some(energy) = app.world().get::<components::Energy>(entity) {
                            dict.set("energy", energy.0);
                        }
                    }
                }
            }
        }

        dict.set("max_hp", 100.0);
        dict
    }

    /// Get activity log for an NPC (decisions, state changes, combat events).
    /// Returns array of dicts with {day, hour, minute, message} for last N entries.
    #[func]
    pub fn get_npc_log(&self, idx: i32, limit: i32) -> Array<VarDictionary> {
        let mut result = Array::new();
        let i = idx as usize;
        let limit = limit.max(1) as usize;

        let Some(bevy_app) = self.get_bevy_app() else { return result; };
        let app_ref = bevy_app.bind();
        let Some(app) = app_ref.get_app() else { return result; };
        let Some(logs) = app.world().get_resource::<resources::NpcLogCache>() else { return result; };

        if let Some(log) = logs.0.get(i) {
            let entries: Vec<_> = log.iter().collect();
            let start = entries.len().saturating_sub(limit);
            for entry in entries[start..].iter().rev() {
                let mut entry_dict = VarDictionary::new();
                entry_dict.set("day", entry.day);
                entry_dict.set("hour", entry.hour);
                entry_dict.set("minute", entry.minute);
                entry_dict.set("message", GString::from(&entry.message));
                result.push(&entry_dict);
            }
        }

        result
    }

    /// Get list of NPCs in a town (for roster panel).
    #[func]
    pub fn get_npcs_by_town(&self, town_idx: i32, filter: i32) -> Array<VarDictionary> {
        let mut result = Array::new();

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let (Some(by_town), Some(meta), Some(npc_map)) = (
                    app.world().get_resource::<resources::NpcsByTownCache>(),
                    app.world().get_resource::<resources::NpcMetaCache>(),
                    app.world().get_resource::<NpcEntityMap>(),
                ) {
                    if let Ok(gpu_state) = GPU_READ_STATE.lock() {
                        if (town_idx as usize) < by_town.0.len() {
                            for &idx in &by_town.0[town_idx as usize] {
                                // Skip dead NPCs
                                if idx >= gpu_state.health.len() || gpu_state.health[idx] <= 0.0 {
                                    continue;
                                }

                                // Apply job filter (-1 = all)
                                let job = meta.0[idx].job;
                                if filter >= 0 && job != filter {
                                    continue;
                                }

                                let state = npc_map.0.get(&idx)
                                    .map(|&e| derive_npc_state(app.world(), e))
                                    .unwrap_or("Idle");

                                let mut npc_dict = VarDictionary::new();
                                npc_dict.set("idx", idx as i32);
                                npc_dict.set("name", GString::from(&meta.0[idx].name));
                                npc_dict.set("job", GString::from(job_name(job)));
                                npc_dict.set("level", meta.0[idx].level);
                                npc_dict.set("hp", gpu_state.health[idx]);
                                npc_dict.set("max_hp", 100.0f32);
                                npc_dict.set("state", GString::from(state));
                                npc_dict.set("trait", GString::from(trait_name(meta.0[idx].trait_id)));

                                result.push(&npc_dict);
                            }
                        }
                    }
                }
            }
        }

        result
    }

    /// Get selected NPC data: { idx, position, target } in one FFI call.
    #[func]
    pub fn get_selected_npc(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let mut idx = -1i32;
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(selected) = app.world().get_resource::<resources::SelectedNpc>() {
                    idx = selected.0;
                }
            }
        }
        dict.set("idx", idx);
        if idx >= 0 {
            dict.set("position", self.get_npc_position(idx));
            dict.set("target", self.get_npc_target(idx));
        }
        dict
    }

    /// Set currently selected NPC index.
    #[func]
    pub fn set_selected_npc(&mut self, idx: i32) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut selected) = app.world_mut().get_resource_mut::<resources::SelectedNpc>() {
                    selected.0 = idx;
                }
            }
        }
    }

    /// Get NPC name by index.
    #[func]
    pub fn get_npc_name(&self, idx: i32) -> GString {
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(meta) = app.world().get_resource::<resources::NpcMetaCache>() {
                    if (idx as usize) < meta.0.len() {
                        return GString::from(&meta.0[idx as usize].name);
                    }
                }
            }
        }
        GString::new()
    }

    /// Get NPC trait by index.
    #[func]
    pub fn get_npc_trait(&self, idx: i32) -> i32 {
        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(meta) = app.world().get_resource::<resources::NpcMetaCache>() {
                    if (idx as usize) < meta.0.len() {
                        return meta.0[idx as usize].trait_id;
                    }
                }
            }
        }
        0
    }

    /// Set NPC name (for rename feature).
    #[func]
    pub fn set_npc_name(&mut self, idx: i32, name: GString) {
        if let Some(mut bevy_app) = self.get_bevy_app() {
            if let Some(app) = bevy_app.bind_mut().get_app_mut() {
                if let Some(mut meta) = app.world_mut().get_resource_mut::<resources::NpcMetaCache>() {
                    if (idx as usize) < meta.0.len() {
                        meta.0[idx as usize].name = name.to_string();
                    }
                }
            }
        }
    }

    /// Find nearest NPC at a position within radius (for click selection).
    #[func]
    pub fn get_npc_at_position(&self, x: f32, y: f32, radius: f32) -> i32 {
        let mut best_idx: i32 = -1;
        let mut best_dist = radius;

        // Use Bevy SlotAllocator count (high-water mark) for click detection
        let slot_count = if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                app.world().get_resource::<resources::SlotAllocator>()
                    .map(|s| s.count())
                    .unwrap_or(0)
            } else { 0 }
        } else { 0 };

        if let Some(gpu) = &self.gpu {
            for i in 0..slot_count {
                // Skip dead NPCs (health <= 0)
                let health = gpu.healths.get(i).copied().unwrap_or(0.0);
                if health <= 0.0 {
                    continue;
                }

                let px = gpu.positions.get(i * 2).copied().unwrap_or(0.0);
                let py = gpu.positions.get(i * 2 + 1).copied().unwrap_or(0.0);
                let dx = px - x;
                let dy = py - y;
                let dist = (dx * dx + dy * dy).sqrt();

                if dist < best_dist {
                    best_dist = dist;
                    best_idx = i as i32;
                }
            }
        }

        best_idx
    }

    /// Find nearest location at a position within radius (for click selection).
    /// Returns: { type: "farm"|"bed"|"guard_post"|"fountain"|"", index: i32, x: f32, y: f32, town_idx: i32 }
    #[func]
    pub fn get_location_at_position(&self, x: f32, y: f32, radius: f32) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("type", "");
        dict.set("index", -1);
        dict.set("x", 0.0f32);
        dict.set("y", 0.0f32);
        dict.set("town_idx", -1);

        let mut best_dist = radius;

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(world_data) = app.world().get_resource::<world::WorldData>() {
                    // Check farms
                    for (i, farm) in world_data.farms.iter().enumerate() {
                        let dx = farm.position.x - x;
                        let dy = farm.position.y - y;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist < best_dist {
                            best_dist = dist;
                            dict.set("type", "farm");
                            dict.set("index", i as i32);
                            dict.set("x", farm.position.x);
                            dict.set("y", farm.position.y);
                            dict.set("town_idx", farm.town_idx as i32);
                        }
                    }
                    // Check beds
                    for (i, bed) in world_data.beds.iter().enumerate() {
                        let dx = bed.position.x - x;
                        let dy = bed.position.y - y;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist < best_dist {
                            best_dist = dist;
                            dict.set("type", "bed");
                            dict.set("index", i as i32);
                            dict.set("x", bed.position.x);
                            dict.set("y", bed.position.y);
                            dict.set("town_idx", bed.town_idx as i32);
                        }
                    }
                    // Check guard posts
                    for (i, post) in world_data.guard_posts.iter().enumerate() {
                        let dx = post.position.x - x;
                        let dy = post.position.y - y;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist < best_dist {
                            best_dist = dist;
                            dict.set("type", "guard_post");
                            dict.set("index", i as i32);
                            dict.set("x", post.position.x);
                            dict.set("y", post.position.y);
                            dict.set("town_idx", post.town_idx as i32);
                        }
                    }
                    // Check town centers (fountains)
                    for (i, town) in world_data.towns.iter().enumerate() {
                        let dx = town.center.x - x;
                        let dy = town.center.y - y;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist < best_dist {
                            best_dist = dist;
                            dict.set("type", "fountain");
                            dict.set("index", i as i32);
                            dict.set("x", town.center.x);
                            dict.set("y", town.center.y);
                            dict.set("town_idx", i as i32);
                        }
                    }
                }
            }
        }

        dict
    }

    /// Get bed statistics for a town.
    #[func]
    pub fn get_bed_stats(&self, town_idx: i32) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let mut total = 0i32;
        let mut free = 0i32;

        if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let (Some(world_data), Some(beds)) = (
                    app.world().get_resource::<world::WorldData>(),
                    app.world().get_resource::<world::BedOccupancy>(),
                ) {
                    for bed in world_data.beds.iter() {
                        if bed.town_idx == town_idx as u32 {
                            total += 1;
                            let key = world::pos_to_key(bed.position);
                            let occupant = beds.occupants.get(&key).copied().unwrap_or(-1);
                            if occupant < 0 {
                                free += 1;
                            }
                        }
                    }
                }
            }
        }

        dict.set("total_beds", total);
        dict.set("free_beds", free);
        dict
    }

    #[func]
    pub fn get_guard_debug(&mut self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        if let Some(gpu) = &mut self.gpu {
            let npc_count = GPU_READ_STATE.lock().map(|s| s.npc_count).unwrap_or(0);

            let arrival_bytes = gpu.rd.buffer_get_data(gpu.arrival_buffer);
            let arrival_slice = arrival_bytes.as_slice();
            let mut arrived_flags = 0;
            for i in 0..npc_count {
                if arrival_slice.len() >= (i + 1) * 4 {
                    let val = i32::from_le_bytes([
                        arrival_slice[i * 4],
                        arrival_slice[i * 4 + 1],
                        arrival_slice[i * 4 + 2],
                        arrival_slice[i * 4 + 3],
                    ]);
                    if val > 0 { arrived_flags += 1; }
                }
            }

            let prev_true = self.prev_arrivals.iter().take(npc_count).filter(|&&x| x).count();
            let queue_len = ARRIVAL_QUEUE.lock().map(|q| q.len()).unwrap_or(0);

            dict.set("arrived_flags", arrived_flags as i32);
            dict.set("prev_arrivals_true", prev_true as i32);
            dict.set("arrival_queue_len", queue_len as i32);
        }
        dict
    }
}
