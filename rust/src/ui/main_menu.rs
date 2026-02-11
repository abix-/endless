//! Main menu — world config sliders + Play / Debug Tests buttons.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::AppState;
use crate::settings;
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

    // Init slider defaults from saved settings (or WorldGenConfig defaults)
    if !state.initialized {
        let saved = settings::load_settings();
        state.world_size = saved.world_size;
        state.towns = saved.towns as f32;
        state.farmers = saved.farmers as f32;
        state.guards = saved.guards as f32;
        state.raiders = saved.raiders as f32;
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
                let mut ws = state.world_size as i32;
                if ui.add(egui::DragValue::new(&mut ws).range(4000..=32000).speed(500)).changed() {
                    state.world_size = ws as f32;
                }
                let tiles = state.world_size as i32 / 32;
                ui.label(format!("{} ({}x{})", size_name(state.world_size), tiles, tiles));
            });

            ui.add_space(4.0);

            // Towns
            ui.horizontal(|ui| {
                ui.label("Towns:");
                ui.add(egui::Slider::new(&mut state.towns, 1.0..=50.0)
                    .step_by(1.0)
                    .show_value(false));
                let mut t = state.towns as i32;
                if ui.add(egui::DragValue::new(&mut t).range(1..=50)).changed() {
                    state.towns = t as f32;
                }
            });

            ui.add_space(4.0);

            // Huts per town (each supports 1 farmer)
            ui.horizontal(|ui| {
                ui.label("Huts:");
                ui.add(egui::Slider::new(&mut state.farmers, 0.0..=50.0)
                    .step_by(1.0)
                    .show_value(false));
                let mut f = state.farmers as i32;
                if ui.add(egui::DragValue::new(&mut f).range(0..=50).suffix(" /town")).changed() {
                    state.farmers = f as f32;
                }
            });

            ui.add_space(4.0);

            // Barracks per town (each supports 1 guard)
            ui.horizontal(|ui| {
                ui.label("Barracks:");
                ui.add(egui::Slider::new(&mut state.guards, 0.0..=1000.0)
                    .step_by(1.0)
                    .show_value(false));
                let mut g = state.guards as i32;
                if ui.add(egui::DragValue::new(&mut g).range(0..=1000).suffix(" /town")).changed() {
                    state.guards = g as f32;
                }
            });

            ui.add_space(4.0);

            // Tents per camp (1 raider per tent)
            ui.horizontal(|ui| {
                ui.label("Tents:");
                ui.add(egui::Slider::new(&mut state.raiders, 0.0..=1000.0)
                    .step_by(1.0)
                    .show_value(false));
                let mut r = state.raiders as i32;
                if ui.add(egui::DragValue::new(&mut r).range(0..=1000).suffix(" /camp")).changed() {
                    state.raiders = r as f32;
                }
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

                    // Persist settings (merge into existing to preserve log filters)
                    let mut saved = settings::load_settings();
                    saved.world_size = state.world_size;
                    saved.towns = state.towns as usize;
                    saved.farmers = state.farmers as usize;
                    saved.guards = state.guards as usize;
                    saved.raiders = state.raiders as usize;
                    settings::save_settings(&saved);

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
