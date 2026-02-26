//! Shared SystemParam bundles used across gameplay systems.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::messages::GpuUpdateMsg;
use crate::resources::*;
use crate::messages::DirtyWriters;

/// Mutable world resources commonly edited together by gameplay systems.
#[derive(SystemParam)]
pub struct WorldState<'w> {
    pub grid: ResMut<'w, crate::world::WorldGrid>,
    pub world_data: ResMut<'w, crate::world::WorldData>,
    pub town_grids: ResMut<'w, crate::world::TownGrids>,
    pub building_occupancy: ResMut<'w, crate::world::BuildingOccupancy>,
    pub dirty_writers: DirtyWriters<'w>,
    pub entity_slots: ResMut<'w, EntitySlots>,
    pub building_slots: ResMut<'w, BuildingEntityMap>,
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
        crate::world::place_building(
            &mut self.grid, &self.world_data,
            food_storage, &mut self.entity_slots, &mut self.building_slots,
            &mut self.dirty_writers, kind, town_data_idx, world_pos, cost,
            &self.town_grids, gpu_updates, commands,
        )
    }

    pub fn destroy_building(
        &mut self,
        combat_log: &mut MessageWriter<crate::messages::CombatLogMsg>,
        game_time: &GameTime,
        row: i32, col: i32,
        town_center: Vec2,
        reason: &str,
        gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
    ) -> Result<(), &'static str> {
        crate::world::destroy_building(
            &mut self.grid, &self.world_data,
            &mut self.building_slots, combat_log, game_time,
            row, col, town_center, reason, gpu_updates,
        )
    }
}

/// Mutable economy resources shared by gameplay systems.
#[derive(SystemParam)]
pub struct EconomyState<'w> {
    pub food_storage: ResMut<'w, FoodStorage>,
    pub gold_storage: ResMut<'w, GoldStorage>,
    pub pop_stats: ResMut<'w, PopulationStats>,
}
