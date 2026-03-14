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
}

impl WorldState<'_> {
    pub fn place_building(
        &mut self,
        food: &mut i32,
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
            commands,
            gpu_updates,
            kind,
            world_pos,
            town_data_idx as u32,
            faction,
            &crate::world::BuildingOverrides {
                patrol_order,
                wall_level,
                hp: None,
            },
            Some(crate::world::BuildContext {
                grid: &mut self.grid,
                world_data: &self.world_data,
                food,
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
        food: &mut i32,
        new_kind: crate::world::BuildingKind,
        town_data_idx: usize,
        world_pos: Vec2,
        gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
        commands: &mut Commands,
    ) -> Result<(), &'static str> {
        let (gc, gr) = self.grid.world_to_grid(world_pos);
        let snapped = self.grid.grid_to_world(gc, gr);
        let inst = self
            .entity_map
            .get_at_grid(gc as i32, gr as i32)
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
        let upgrade_cost =
            crate::constants::building_cost(new_kind) - crate::constants::building_cost(old_kind);
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
        let faction = self
            .world_data
            .towns
            .get(town_data_idx)
            .map(|t| t.faction)
            .unwrap_or(0);
        crate::world::place_building(
            &mut self.entity_slots,
            &mut self.entity_map,
            commands,
            gpu_updates,
            new_kind,
            snapped,
            town_data_idx as u32,
            faction,
            &Default::default(),
            None, // no BuildContext — skip validation (already done)
            Some(&mut self.dirty_writers),
        )
        .map(|_| ())
    }

    pub fn destroy_building(
        &mut self,
        combat_log: &mut MessageWriter<crate::messages::CombatLogMsg>,
        game_time: &GameTime,
        gc: usize,
        gr: usize,
        reason: &str,
        gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
    ) -> Result<(), &'static str> {
        crate::world::destroy_building(
            &mut self.grid,
            &self.world_data,
            &mut self.entity_map,
            combat_log,
            game_time,
            gc,
            gr,
            reason,
            gpu_updates,
        )
    }
}

/// Mutable economy resources shared by gameplay systems.
#[derive(SystemParam)]
pub struct EconomyState<'w, 's> {
    pub pop_stats: ResMut<'w, PopulationStats>,
    pub towns: TownAccess<'w, 's>,
}

/// ECS-backed town data access. Each town is an entity with FoodStore, GoldStore, etc.
#[derive(SystemParam)]
pub struct TownAccess<'w, 's> {
    index: ResMut<'w, TownIndex>,
    food: Query<
        'w,
        's,
        &'static mut crate::components::FoodStore,
        With<crate::components::TownMarker>,
    >,
    gold: Query<
        'w,
        's,
        &'static mut crate::components::GoldStore,
        With<crate::components::TownMarker>,
    >,
    wood: Query<
        'w,
        's,
        &'static mut crate::components::WoodStore,
        With<crate::components::TownMarker>,
    >,
    stone: Query<
        'w,
        's,
        &'static mut crate::components::StoneStore,
        With<crate::components::TownMarker>,
    >,
    policy: Query<
        'w,
        's,
        &'static mut crate::components::TownPolicy,
        With<crate::components::TownMarker>,
    >,
    upgrades: Query<
        'w,
        's,
        &'static mut crate::components::TownUpgradeLevel,
        With<crate::components::TownMarker>,
    >,
    equipment: Query<
        'w,
        's,
        &'static mut crate::components::TownEquipment,
        With<crate::components::TownMarker>,
    >,
    area: Query<
        'w,
        's,
        &'static mut crate::components::TownAreaLevel,
        With<crate::components::TownMarker>,
    >,
}

impl TownAccess<'_, '_> {
    pub fn entity(&self, town_idx: i32) -> Option<Entity> {
        self.index.0.get(&town_idx).copied()
    }

    pub fn town_index_mut(&mut self) -> &mut TownIndex {
        &mut self.index
    }

    pub fn food(&self, town_idx: i32) -> i32 {
        self.entity(town_idx)
            .and_then(|e| self.food.get(e).ok())
            .map(|f| f.0)
            .unwrap_or(0)
    }

    pub fn food_mut(&mut self, town_idx: i32) -> Option<Mut<'_, crate::components::FoodStore>> {
        let e = self.entity(town_idx)?;
        self.food.get_mut(e).ok()
    }

    pub fn gold(&self, town_idx: i32) -> i32 {
        self.entity(town_idx)
            .and_then(|e| self.gold.get(e).ok())
            .map(|g| g.0)
            .unwrap_or(0)
    }

    pub fn gold_mut(&mut self, town_idx: i32) -> Option<Mut<'_, crate::components::GoldStore>> {
        let e = self.entity(town_idx)?;
        self.gold.get_mut(e).ok()
    }

    pub fn wood(&self, town_idx: i32) -> i32 {
        self.entity(town_idx)
            .and_then(|e| self.wood.get(e).ok())
            .map(|w| w.0)
            .unwrap_or(0)
    }

    pub fn wood_mut(&mut self, town_idx: i32) -> Option<Mut<'_, crate::components::WoodStore>> {
        let e = self.entity(town_idx)?;
        self.wood.get_mut(e).ok()
    }

    pub fn stone(&self, town_idx: i32) -> i32 {
        self.entity(town_idx)
            .and_then(|e| self.stone.get(e).ok())
            .map(|s| s.0)
            .unwrap_or(0)
    }

    pub fn stone_mut(&mut self, town_idx: i32) -> Option<Mut<'_, crate::components::StoneStore>> {
        let e = self.entity(town_idx)?;
        self.stone.get_mut(e).ok()
    }

    pub fn policy(&self, town_idx: i32) -> Option<crate::resources::PolicySet> {
        let e = self.entity(town_idx)?;
        self.policy.get(e).ok().map(|p| p.0.clone())
    }

    pub fn policy_mut(&mut self, town_idx: i32) -> Option<Mut<'_, crate::components::TownPolicy>> {
        let e = self.entity(town_idx)?;
        self.policy.get_mut(e).ok()
    }

    pub fn upgrade_levels(&self, town_idx: i32) -> Vec<u8> {
        self.entity(town_idx)
            .and_then(|e| self.upgrades.get(e).ok())
            .map(|u| u.0.clone())
            .unwrap_or_default()
    }

    pub fn upgrade_level(&self, town_idx: i32, upgrade_idx: usize) -> u8 {
        self.entity(town_idx)
            .and_then(|e| self.upgrades.get(e).ok())
            .and_then(|u| u.0.get(upgrade_idx).copied())
            .unwrap_or(0)
    }

    pub fn upgrades_mut(
        &mut self,
        town_idx: i32,
    ) -> Option<Mut<'_, crate::components::TownUpgradeLevel>> {
        let e = self.entity(town_idx)?;
        self.upgrades.get_mut(e).ok()
    }

    pub fn equipment(&self, town_idx: i32) -> Option<Vec<crate::constants::LootItem>> {
        let e = self.entity(town_idx)?;
        self.equipment.get(e).ok().map(|eq| eq.0.clone())
    }

    pub fn equipment_mut(
        &mut self,
        town_idx: i32,
    ) -> Option<Mut<'_, crate::components::TownEquipment>> {
        let e = self.entity(town_idx)?;
        self.equipment.get_mut(e).ok()
    }

    pub fn area_level(&self, town_idx: i32) -> i32 {
        self.entity(town_idx)
            .and_then(|e| self.area.get(e).ok())
            .map(|a| a.0)
            .unwrap_or(0)
    }

    pub fn set_area_level(&mut self, town_idx: i32, val: i32) {
        if let Some(e) = self.entity(town_idx) {
            if let Ok(mut a) = self.area.get_mut(e) {
                a.0 = val;
            }
        }
    }
}
