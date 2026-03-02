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
    pub town_grids: ResMut<'w, crate::world::TownGrids>,
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
                town_grids: &self.town_grids,
                cost,
            }),
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
