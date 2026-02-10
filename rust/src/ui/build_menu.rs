//! Build menu — context menu for building/destroying/unlocking town grid slots.
//! Opened by right-clicking a town grid slot (slot_right_click_system in ui/mod.rs).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::constants::*;
use crate::resources::*;
use crate::world::{self, Building};

pub fn build_menu_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut build_ctx: ResMut<BuildMenuContext>,
    mut grid: ResMut<world::WorldGrid>,
    mut world_data: ResMut<world::WorldData>,
    mut farm_states: ResMut<FarmStates>,
    mut food_storage: ResMut<FoodStorage>,
    mut town_grids: ResMut<world::TownGrids>,
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    mut gp_state: ResMut<GuardPostState>,
) -> Result {
    if !ui_state.build_menu_open {
        return Ok(());
    }

    // Need valid context to show anything
    let Some(grid_idx) = build_ctx.grid_idx else {
        ui_state.build_menu_open = false;
        return Ok(());
    };
    let Some(town_data_idx) = build_ctx.town_data_idx else {
        ui_state.build_menu_open = false;
        return Ok(());
    };
    let Some((row, col)) = build_ctx.slot else {
        ui_state.build_menu_open = false;
        return Ok(());
    };

    let ctx = contexts.ctx_mut()?;
    let mut open = true;
    let mut action_taken = false;

    // Get food for this town
    let food = food_storage.food.get(town_data_idx).copied().unwrap_or(0);
    let town_name = world_data.towns.get(town_data_idx)
        .map(|t| t.name.clone())
        .unwrap_or_default();
    let town_center = world_data.towns.get(town_data_idx)
        .map(|t| t.center)
        .unwrap_or_default();

    let title = if build_ctx.is_locked {
        format!("Unlock ({},{})", row, col)
    } else {
        format!("Build ({},{})", row, col)
    };

    egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(220.0)
        .show(ctx, |ui| {
            ui.label(format!("{} — Food: {}", town_name, food));
            ui.separator();

            if build_ctx.is_locked {
                // Locked slot: show unlock button
                let can_unlock = food >= SLOT_UNLOCK_COST;
                if ui.add_enabled(can_unlock, egui::Button::new(
                    format!("Unlock ({} food)", SLOT_UNLOCK_COST)
                )).clicked() {
                    // Unlock the slot
                    if let Some(town_grid) = town_grids.grids.get_mut(grid_idx) {
                        town_grid.unlocked.insert((row, col));
                    }
                    if let Some(f) = food_storage.food.get_mut(town_data_idx) {
                        *f -= SLOT_UNLOCK_COST;
                    }
                    combat_log.push(
                        CombatEventKind::Harvest,
                        game_time.day(), game_time.hour(), game_time.minute(),
                        format!("Unlocked slot ({},{}) in {}", row, col, town_name),
                    );
                    action_taken = true;
                }
                if !can_unlock {
                    ui.small("Not enough food");
                }
            } else if build_ctx.is_fountain {
                ui.label("Fountain (indestructible)");
            } else if build_ctx.has_building {
                // Guard post turret toggle
                let (gc, gr) = grid.world_to_grid(build_ctx.slot_world_pos);
                let is_guard_post = grid.cell(gc, gr)
                    .and_then(|c| c.building.as_ref())
                    .map(|b| matches!(b, Building::GuardPost { .. }))
                    .unwrap_or(false);

                if is_guard_post {
                    // Find this guard post's index by position
                    let snapped = grid.grid_to_world(gc, gr);
                    if let Some(gp_idx) = world_data.guard_posts.iter().position(|g| {
                        (g.position - snapped).length() < 1.0
                    }) {
                        // Sync state length
                        while gp_state.attack_enabled.len() <= gp_idx {
                            gp_state.timers.push(0.0);
                            gp_state.attack_enabled.push(true);
                        }

                        let enabled = gp_state.attack_enabled[gp_idx];
                        let label = if enabled { "Disable Turret" } else { "Enable Turret" };
                        if ui.button(label).clicked() {
                            gp_state.attack_enabled[gp_idx] = !enabled;
                        }
                    }
                    ui.separator();
                }

                // Occupied slot: show destroy button
                if ui.button("Destroy Building").clicked() {
                    match world::remove_building(
                        &mut grid, &mut world_data, &mut farm_states,
                        row, col, town_center,
                    ) {
                        Ok(()) => {
                            combat_log.push(
                                CombatEventKind::Harvest,
                                game_time.day(), game_time.hour(), game_time.minute(),
                                format!("Destroyed building at ({},{}) in {}", row, col, town_name),
                            );
                        }
                        Err(e) => {
                            warn!("Failed to destroy building: {}", e);
                        }
                    }
                    action_taken = true;
                }
            } else {
                // Empty unlocked slot: show build options
                let town_idx = grid_idx as u32; // villager town index for Building

                // Farm
                let can_farm = food >= FARM_BUILD_COST;
                if ui.add_enabled(can_farm, egui::Button::new(
                    format!("Farm ({} food)", FARM_BUILD_COST)
                )).on_hover_text("Produces food when tended by farmers")
                .clicked() {
                    let building = Building::Farm { town_idx };
                    if let Ok(()) = world::place_building(
                        &mut grid, &mut world_data, &mut farm_states,
                        building, row, col, town_center,
                    ) {
                        if let Some(f) = food_storage.food.get_mut(town_data_idx) {
                            *f -= FARM_BUILD_COST;
                        }
                        combat_log.push(
                            CombatEventKind::Harvest,
                            game_time.day(), game_time.hour(), game_time.minute(),
                            format!("Built farm at ({},{}) in {}", row, col, town_name),
                        );
                    }
                    action_taken = true;
                }

                // Bed
                let can_bed = food >= BED_BUILD_COST;
                if ui.add_enabled(can_bed, egui::Button::new(
                    format!("Bed ({} food)", BED_BUILD_COST)
                )).on_hover_text("NPCs rest and recover energy here")
                .clicked() {
                    let building = Building::Bed { town_idx };
                    if let Ok(()) = world::place_building(
                        &mut grid, &mut world_data, &mut farm_states,
                        building, row, col, town_center,
                    ) {
                        if let Some(f) = food_storage.food.get_mut(town_data_idx) {
                            *f -= BED_BUILD_COST;
                        }
                        combat_log.push(
                            CombatEventKind::Harvest,
                            game_time.day(), game_time.hour(), game_time.minute(),
                            format!("Built bed at ({},{}) in {}", row, col, town_name),
                        );
                    }
                    action_taken = true;
                }

                // Guard Post
                let can_post = food >= GUARD_POST_BUILD_COST;
                if ui.add_enabled(can_post, egui::Button::new(
                    format!("Guard Post ({} food)", GUARD_POST_BUILD_COST)
                )).on_hover_text("Guards patrol between posts")
                .clicked() {
                    // patrol_order = count of existing posts for this town
                    let existing_posts = world_data.guard_posts.iter()
                        .filter(|g| g.town_idx == town_idx && g.position.x > -9000.0)
                        .count() as u32;
                    let building = Building::GuardPost { town_idx, patrol_order: existing_posts };
                    if let Ok(()) = world::place_building(
                        &mut grid, &mut world_data, &mut farm_states,
                        building, row, col, town_center,
                    ) {
                        if let Some(f) = food_storage.food.get_mut(town_data_idx) {
                            *f -= GUARD_POST_BUILD_COST;
                        }
                        combat_log.push(
                            CombatEventKind::Harvest,
                            game_time.day(), game_time.hour(), game_time.minute(),
                            format!("Built guard post at ({},{}) in {}", row, col, town_name),
                        );
                    }
                    action_taken = true;
                }

                if food < BED_BUILD_COST {
                    ui.small("Not enough food");
                }
            }
        });

    if !open || action_taken {
        ui_state.build_menu_open = false;
        *build_ctx = Default::default();
    }

    Ok(())
}
