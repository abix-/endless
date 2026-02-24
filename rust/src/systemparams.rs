//! Shared SystemParam bundles used across gameplay systems.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::resources::*;

/// Mutable world resources commonly edited together by gameplay systems.
#[derive(SystemParam)]
pub struct WorldState<'w> {
    pub grid: ResMut<'w, crate::world::WorldGrid>,
    pub world_data: ResMut<'w, crate::world::WorldData>,
    pub town_grids: ResMut<'w, crate::world::TownGrids>,
    pub building_occupancy: ResMut<'w, crate::world::BuildingOccupancy>,
    pub farm_states: ResMut<'w, GrowthStates>,
    pub dirty: ResMut<'w, DirtyFlags>,
    pub slot_alloc: ResMut<'w, SlotAllocator>,
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
        commands: &mut Commands,
    ) -> Result<(), &'static str> {
        crate::world::place_building(
            &mut self.grid, &self.world_data, &mut self.farm_states,
            food_storage, &mut self.slot_alloc, &mut self.building_slots,
            &mut self.dirty, kind, town_data_idx, world_pos, cost,
            &self.town_grids, commands,
        )
    }

    pub fn destroy_building(
        &mut self,
        combat_log: &mut CombatLog,
        game_time: &GameTime,
        row: i32, col: i32,
        town_center: Vec2,
        reason: &str,
        commands: &mut Commands,
    ) -> Result<(), &'static str> {
        crate::world::destroy_building(
            &mut self.grid, &self.world_data, &mut self.farm_states,
            &mut self.building_slots, combat_log, game_time,
            row, col, town_center, reason, commands,
        )
    }
}

/// Mutable economy resources shared by gameplay systems.
#[derive(SystemParam)]
pub struct EconomyState<'w> {
    pub food_storage: ResMut<'w, FoodStorage>,
    pub gold_storage: ResMut<'w, GoldStorage>,
    pub food_events: ResMut<'w, FoodEvents>,
    pub pop_stats: ResMut<'w, PopulationStats>,
}
