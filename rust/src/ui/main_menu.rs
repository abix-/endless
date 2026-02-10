//! Main menu — world config sliders + Play / Debug Tests buttons.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::AppState;
use crate::world::WorldGenConfig;

/// Slider state persisted across frames via Local.
#[derive(Default)]
pub struct MenuState {
    pub world_size: f32,
    pub towns: f32,
    pub farmers: f32,
    pub guards: f32,
    pub raiders: f32,
    pub initialized: bool,
}

fn size_name(size: f32) -> &'static str {
    match size as i32 {
        4000 => "Tiny",
        8000 => "Small",
        12000 => "Medium",
        16000 => "Large",
        20000 => "Huge",
        24000 => "Massive",
        28000 => "Epic",
        32000 => "Endless",
        _ => "Custom",
    }
}

pub fn main_menu_system(
    mut contexts: EguiContexts,
    mut next_state: ResMut<NextState<AppState>>,
    mut wg_config: ResMut<WorldGenConfig>,
    mut state: Local<MenuState>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    // Init slider defaults from WorldGenConfig
    if !state.initialized {
        state.world_size = wg_config.world_width;
        state.towns = wg_config.num_towns as f32;
        state.farmers = wg_config.farmers_per_town as f32;
        state.guards = wg_config.guards_per_town as f32;
        state.raiders = wg_config.raiders_per_camp as f32;
        state.initialized = true;
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading(egui::RichText::new("Endless").size(32.0));
            ui.add_space(10.0);
            ui.label("Colony simulation");
            ui.add_space(30.0);
        });

        // Config sliders in a centered frame
        let panel_width = 400.0;
        ui.vertical_centered(|ui| {
            ui.set_max_width(panel_width);

            // World size
            ui.horizontal(|ui| {
                ui.label("World size:");
                ui.add(egui::Slider::new(&mut state.world_size, 4000.0..=32000.0)
                    .step_by(500.0)
                    .show_value(false));
                let tiles = state.world_size as i32 / 32;
                ui.label(format!("{} ({}x{})", size_name(state.world_size), tiles, tiles));
            });

            ui.add_space(4.0);

            // Towns
            ui.horizontal(|ui| {
                ui.label("Towns:");
                ui.add(egui::Slider::new(&mut state.towns, 1.0..=7.0)
                    .step_by(1.0)
                    .show_value(false));
                let t = state.towns as i32;
                ui.label(format!("{} town{}", t, if t > 1 { "s" } else { "" }));
            });

            ui.add_space(4.0);

            // Farmers per town
            ui.horizontal(|ui| {
                ui.label("Farmers:");
                ui.add(egui::Slider::new(&mut state.farmers, 0.0..=50.0)
                    .step_by(1.0)
                    .show_value(false));
                ui.label(format!("{} per town", state.farmers as i32));
            });

            ui.add_space(4.0);

            // Guards per town
            ui.horizontal(|ui| {
                ui.label("Guards:");
                ui.add(egui::Slider::new(&mut state.guards, 0.0..=50.0)
                    .step_by(1.0)
                    .show_value(false));
                ui.label(format!("{} per town", state.guards as i32));
            });

            ui.add_space(4.0);

            // Raiders per camp
            ui.horizontal(|ui| {
                ui.label("Raiders:");
                ui.add(egui::Slider::new(&mut state.raiders, 0.0..=50.0)
                    .step_by(1.0)
                    .show_value(false));
                ui.label(format!("{} per camp", state.raiders as i32));
            });

            ui.add_space(8.0);

            // NPC total
            let towns = state.towns as i32;
            let villagers = towns * (state.farmers as i32 + state.guards as i32);
            let raiders = towns * state.raiders as i32;
            let total = villagers + raiders;
            ui.label(format!("~{} NPCs total", total));

            ui.add_space(20.0);

            // Buttons
            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new("  Play  ").size(18.0)).clicked() {
                    // Write config
                    wg_config.world_width = state.world_size;
                    wg_config.world_height = state.world_size;
                    wg_config.num_towns = state.towns as usize;
                    wg_config.farmers_per_town = state.farmers as usize;
                    wg_config.guards_per_town = state.guards as usize;
                    wg_config.raiders_per_camp = state.raiders as usize;

                    next_state.set(AppState::Playing);
                }

                ui.add_space(20.0);

                if ui.button(egui::RichText::new("Debug Tests").size(14.0)).clicked() {
                    next_state.set(AppState::TestMenu);
                }
            });
        });
    });

    Ok(())
}
