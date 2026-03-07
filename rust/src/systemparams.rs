//! Shared SystemParam bundles used across gameplay systems.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::messages::DirtyWriters;
use crate::messages::GpuUpdateMsg;
use crate::resources::*;

/// Mutable world resources commonly edited together by gameplay systems.
#[derive(SystemParam)]
pub struct WorldState<'w> {
    pub grid: ResMut<'w, crate::world::WorldGrid>,
    pub world_data: ResMut<'w, crate::world::WorldData>,
    pub dirty_writers: DirtyWriters<'w>,
    pub entity_slots: ResMut<'w, GpuSlotPool>,
    pub entity_map: ResMut<'w, EntityMap>,
    pub uid_alloc: ResMut<'w, NextEntityUid>,
}

impl WorldState<'_> {
    pub fn place_building(
        &mut self,
        food_storage: &mut FoodStorage,
        kind: crate::world::BuildingKind,
        town_data_idx: usize,
        world_pos: Vec2,
        cost: i32,
        gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
        commands: &mut Commands,
    ) -> Result<(), &'static str> {
        // 1-per-town limit
        if kind == crate::world::BuildingKind::Merchant
            && self.entity_map.count_for_town(kind, town_data_idx as u32) >= 1
        {
            return Err("Only one Merchant per town");
        }
        let faction = self
            .world_data
            .towns
            .get(town_data_idx)
            .map(|t| t.faction)
            .unwrap_or(0);
        let patrol_order = if kind == crate::world::BuildingKind::Waypoint {
            self.entity_map
                .count_for_town(crate::world::BuildingKind::Waypoint, town_data_idx as u32)
                as u32
        } else {
            0
        };
        let wall_level = if kind == crate::world::BuildingKind::Wall {
            1
        } else {
            0
        };
        crate::world::place_building(
            &mut self.entity_slots,
            &mut self.entity_map,
            &mut self.uid_alloc,
            commands,
            gpu_updates,
            kind,
            world_pos,
            town_data_idx as u32,
            faction,
            patrol_order,
            wall_level,
            None,
            None,
            Some(crate::world::BuildContext {
                grid: &mut self.grid,
                world_data: &self.world_data,
                food_storage,
                cost,
            }),
            Some(&mut self.dirty_writers),
        )
        .map(|_| ())
    }

    /// Upgrade an existing road at world_pos to a higher tier.
    /// Returns Ok if upgrade succeeded, Err if no upgradeable road found.
    pub fn upgrade_road(
        &mut self,
        food_storage: &mut FoodStorage,
        new_kind: crate::world::BuildingKind,
        town_data_idx: usize,
        world_pos: Vec2,
        gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
        commands: &mut Commands,
    ) -> Result<(), &'static str> {
        let (gc, gr) = self.grid.world_to_grid(world_pos);
        let snapped = self.grid.grid_to_world(gc, gr);
        let inst = self.entity_map.get_at_grid(gc as i32, gr as i32)
            .ok_or("no building at position")?;
        let old_kind = inst.kind;
        let old_town = inst.town_idx;
        if !old_kind.is_road() {
            return Err("not a road");
        }
        let old_tier = old_kind.road_tier().unwrap_or(0);
        let new_tier = new_kind.road_tier().ok_or("target is not a road")?;
        if new_tier <= old_tier {
            return Err("already same or higher tier");
        }
        if old_town != town_data_idx as u32 {
            return Err("road belongs to different town");
        }

        // Cost = new road cost minus old road cost
        let upgrade_cost = crate::constants::building_cost(new_kind)
            - crate::constants::building_cost(old_kind);
        let food = food_storage.food.get_mut(town_data_idx).ok_or("invalid town")?;
        if *food < upgrade_cost {
            return Err("not enough food");
        }

        // Remove old road
        let slot = inst.slot;
        if let Some(&entity) = self.entity_map.entities.get(&slot) {
            commands.entity(entity).despawn();
        }
        self.entity_map.remove_by_slot(slot);
        self.entity_slots.free(slot);
        gpu_updates.write(crate::messages::GpuUpdateMsg(
            crate::messages::GpuUpdate::Hide { idx: slot },
        ));

        // Deduct cost
        *food -= upgrade_cost;

        // Place new road (no validation context — we already validated)
        let faction = self.world_data.towns.get(town_data_idx)
            .map(|t| t.faction).unwrap_or(0);
        crate::world::place_building(
            &mut self.entity_slots,
            &mut self.entity_map,
            &mut self.uid_alloc,
            commands,
            gpu_updates,
            new_kind,
            snapped,
            town_data_idx as u32,
            faction,
            0,
            0,
            None,
            None,
            None, // no BuildContext — skip validation (already done)
            Some(&mut self.dirty_writers),
        )
        .map(|_| ())
    }

    pub fn destroy_building(
        &mut self,
        combat_log: &mut MessageWriter<crate::messages::CombatLogMsg>,
        game_time: &GameTime,
        row: i32,
        col: i32,
        town_center: Vec2,
        reason: &str,
        gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
    ) -> Result<(), &'static str> {
        crate::world::destroy_building(
            &mut self.grid,
            &self.world_data,
            &mut self.entity_map,
            combat_log,
            game_time,
            row,
            col,
            town_center,
            reason,
            gpu_updates,
        )
    }
}

/// Mutable economy resources shared by gameplay systems.
#[derive(SystemParam)]
pub struct EconomyState<'w> {
    pub food_storage: ResMut<'w, FoodStorage>,
    pub gold_storage: ResMut<'w, GoldStorage>,
    pub pop_stats: ResMut<'w, PopulationStats>,
    pub town_inventory: ResMut<'w, crate::resources::TownInventory>,
}
