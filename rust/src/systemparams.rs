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
    pub building_hp: ResMut<'w, BuildingHpState>,
    pub spawner_state: ResMut<'w, SpawnerState>,
    pub farm_states: ResMut<'w, GrowthStates>,
    pub dirty: ResMut<'w, DirtyFlags>,
}

/// Mutable economy resources shared by gameplay systems.
#[derive(SystemParam)]
pub struct EconomyState<'w> {
    pub food_storage: ResMut<'w, FoodStorage>,
    pub gold_storage: ResMut<'w, GoldStorage>,
    pub food_events: ResMut<'w, FoodEvents>,
    pub pop_stats: ResMut<'w, PopulationStats>,
}
