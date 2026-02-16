//! Main menu — world config sliders + Play / Debug Tests buttons.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::AppState;
use crate::settings;
use crate::world::{WorldGenConfig, WorldGenStyle};
use crate::systems::AiPlayerConfig;

/// Slider state persisted across frames via Local.
#[derive(Default)]
pub struct MenuState {
    pub world_size: f32,
    pub towns: f32,
    pub farms: f32,
    pub farmers: f32,
    pub archers: f32,
    pub raiders: f32,
    pub ai_towns: f32,
    pub raider_camps: f32,
    pub ai_interval: f32,
    pub npc_interval: f32,
    pub gen_style: i32,
    pub gold_mines: f32,
    pub raider_passive_forage: bool,
    pub difficulty: crate::resources::Difficulty,
    pub autosave_hours: i32,
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
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut next_state: ResMut<NextState<AppState>>,
    mut wg_config: ResMut<WorldGenConfig>,
    mut ai_config: ResMut<AiPlayerConfig>,
    mut npc_config: ResMut<crate::resources::NpcDecisionConfig>,
    mut user_settings: ResMut<settings::UserSettings>,
    mut save_request: ResMut<crate::save::SaveLoadRequest>,
    mut state: Local<MenuState>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    // Init slider defaults from saved settings (or WorldGenConfig defaults)
    if !state.initialized {
        let saved = settings::load_settings();
        state.world_size = saved.world_size;
        state.towns = saved.towns as f32;
        state.farms = saved.farms as f32;
        state.farmers = saved.farmers as f32;
        state.archers = saved.archers as f32;
        state.raiders = saved.raiders as f32;
        state.ai_towns = saved.ai_towns as f32;
        state.raider_camps = saved.raider_camps as f32;
        state.ai_interval = saved.ai_interval;
        state.npc_interval = saved.npc_interval;
        state.gen_style = saved.gen_style as i32;
        state.gold_mines = saved.gold_mines_per_town as f32;
        state.raider_passive_forage = saved.raider_passive_forage;
        state.difficulty = saved.difficulty;
        state.autosave_hours = saved.autosave_hours;
        state.initialized = true;
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading(egui::RichText::new("Endless").size(32.0));
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

            // World gen style
            ui.horizontal(|ui| {
                ui.label("World gen:");
                egui::ComboBox::from_id_salt("gen_style")
                    .selected_text(match state.gen_style {
                        1 => "Continents",
                        _ => "Classic",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut state.gen_style, 0, "Classic");
                        ui.selectable_value(&mut state.gen_style, 1, "Continents");
                    });
            });

            ui.add_space(4.0);

            // Player Towns
            ui.horizontal(|ui| {
                ui.label("Your Towns:");
                ui.add(egui::Slider::new(&mut state.towns, 1.0..=50.0)
                    .step_by(1.0)
                    .show_value(false));
                let mut t = state.towns as i32;
                if ui.add(egui::DragValue::new(&mut t).range(1..=50)).changed() {
                    state.towns = t as f32;
                }
            });

            ui.add_space(4.0);

            // AI Towns
            ui.horizontal(|ui| {
                ui.label("AI Towns:");
                ui.add(egui::Slider::new(&mut state.ai_towns, 0.0..=10.0)
                    .step_by(1.0)
                    .show_value(false));
                let mut at = state.ai_towns as i32;
                if ui.add(egui::DragValue::new(&mut at).range(0..=10)).changed() {
                    state.ai_towns = at as f32;
                }
            });

            ui.add_space(4.0);

            // Raider Camps
            ui.horizontal(|ui| {
                ui.label("Raider Camps:");
                ui.add(egui::Slider::new(&mut state.raider_camps, 0.0..=10.0)
                    .step_by(1.0)
                    .show_value(false));
                let mut rc = state.raider_camps as i32;
                if ui.add(egui::DragValue::new(&mut rc).range(0..=10)).changed() {
                    state.raider_camps = rc as f32;
                }
            });

            ui.add_space(4.0);

            // Gold Mines
            ui.horizontal(|ui| {
                ui.label("Gold Mines:");
                ui.add(egui::Slider::new(&mut state.gold_mines, 0.0..=10.0)
                    .step_by(1.0)
                    .show_value(false));
                let mut gm = state.gold_mines as i32;
                if ui.add(egui::DragValue::new(&mut gm).range(0..=10).suffix(" /town")).changed() {
                    state.gold_mines = gm as f32;
                }
            });

            ui.add_space(4.0);

            // Difficulty
            ui.horizontal(|ui| {
                ui.label("Difficulty:");
                egui::ComboBox::from_id_salt("difficulty")
                    .selected_text(state.difficulty.label())
                    .show_ui(ui, |ui| {
                        for d in crate::resources::Difficulty::ALL {
                            ui.selectable_value(&mut state.difficulty, d, d.label());
                        }
                    });
            });

            ui.add_space(4.0);

            // Autosave
            ui.horizontal(|ui| {
                ui.label("Autosave:");
                ui.add(egui::Slider::new(&mut state.autosave_hours, 0..=48)
                    .step_by(1.0)
                    .show_value(false));
                let label = if state.autosave_hours == 0 { "Off".to_string() } else { format!("{}h", state.autosave_hours) };
                ui.label(label);
            });

            ui.add_space(20.0);

            // Play button
            if ui.button(egui::RichText::new("  Play  ").size(18.0)).clicked() {
                wg_config.gen_style = if state.gen_style == 1 { WorldGenStyle::Continents } else { WorldGenStyle::Classic };
                wg_config.world_width = state.world_size;
                wg_config.world_height = state.world_size;
                wg_config.num_towns = state.towns as usize;
                wg_config.farms_per_town = state.farms as usize;
                wg_config.farmers_per_town = state.farmers as usize;
                wg_config.archers_per_town = state.archers as usize;
                wg_config.raiders_per_camp = state.raiders as usize;
                wg_config.ai_towns = state.ai_towns as usize;
                wg_config.raider_camps = state.raider_camps as usize;
                wg_config.gold_mines_per_town = state.gold_mines as usize;
                ai_config.decision_interval = state.ai_interval;
                npc_config.interval = state.npc_interval;

                let mut saved = settings::load_settings();
                saved.world_size = state.world_size;
                saved.towns = state.towns as usize;
                saved.farms = state.farms as usize;
                saved.farmers = state.farmers as usize;
                saved.archers = state.archers as usize;
                saved.raiders = state.raiders as usize;
                saved.ai_towns = state.ai_towns as usize;
                saved.raider_camps = state.raider_camps as usize;
                saved.ai_interval = state.ai_interval;
                saved.npc_interval = state.npc_interval;
                saved.gen_style = state.gen_style as u8;
                saved.gold_mines_per_town = state.gold_mines as usize;
                saved.raider_passive_forage = state.raider_passive_forage;
                saved.difficulty = state.difficulty;
                saved.autosave_hours = state.autosave_hours;
                settings::save_settings(&saved);
                user_settings.raider_passive_forage = state.raider_passive_forage;

                commands.insert_resource(state.difficulty);
                save_request.autosave_hours = state.autosave_hours;
                next_state.set(AppState::Playing);
            }

            ui.add_space(8.0);

            // Load Game — collapsible save file picker
            let saves = crate::save::list_saves();
            if saves.is_empty() {
                ui.add_enabled_ui(false, |ui| {
                    let _ = ui.button(egui::RichText::new("Load Game").size(18.0));
                });
                ui.label(egui::RichText::new("No save files found").size(12.0).weak());
            } else {
                egui::CollapsingHeader::new(egui::RichText::new("Load Game").size(18.0))
                    .default_open(false)
                    .show(ui, |ui| {
                        for save_info in &saves {
                            ui.horizontal(|ui| {
                                let elapsed = save_info.modified.elapsed().unwrap_or_default();
                                let age = if elapsed.as_secs() < 60 {
                                    "just now".to_string()
                                } else if elapsed.as_secs() < 3600 {
                                    format!("{}m ago", elapsed.as_secs() / 60)
                                } else if elapsed.as_secs() < 86400 {
                                    format!("{}h ago", elapsed.as_secs() / 3600)
                                } else {
                                    format!("{}d ago", elapsed.as_secs() / 86400)
                                };
                                let name = save_info.filename.trim_end_matches(".json");
                                if ui.button(name).clicked() {
                                    save_request.load_on_enter = true;
                                    save_request.load_path = Some(save_info.path.clone());
                                    save_request.autosave_hours = state.autosave_hours;
                                    next_state.set(AppState::Playing);
                                }
                                ui.label(egui::RichText::new(age).size(12.0).weak());
                            });
                        }
                    });
            }

            ui.add_space(20.0);

            // Debug options — collapsed by default
            egui::CollapsingHeader::new("Debug Options")
                .default_open(true)
                .show(ui, |ui| {
                    ui.add_space(4.0);

                    // AI Think interval
                    ui.horizontal(|ui| {
                        ui.label("AI Think:");
                        ui.add(egui::Slider::new(&mut state.ai_interval, 1.0..=30.0)
                            .step_by(0.5)
                            .suffix("s")
                            .show_value(true));
                    });

                    ui.add_space(4.0);

                    // NPC Think interval
                    ui.horizontal(|ui| {
                        ui.label("NPC Think:");
                        ui.add(egui::Slider::new(&mut state.npc_interval, 0.5..=10.0)
                            .step_by(0.5)
                            .suffix("s")
                            .show_value(true));
                    });

                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        ui.label("Raider Passive Forage:");
                        ui.checkbox(&mut state.raider_passive_forage, "Enabled");
                    });

                    ui.add_space(4.0);

                    // Farms per town
                    ui.horizontal(|ui| {
                        ui.label("Farms:");
                        ui.add(egui::Slider::new(&mut state.farms, 0.0..=50.0)
                            .step_by(1.0)
                            .show_value(false));
                        let mut fm = state.farms as i32;
                        if ui.add(egui::DragValue::new(&mut fm).range(0..=50).suffix(" /town")).changed() {
                            state.farms = fm as f32;
                        }
                    });

                    ui.add_space(4.0);

                    // Farmer homes per town (each supports 1 farmer)
                    ui.horizontal(|ui| {
                        ui.label("Farmer Homes:");
                        ui.add(egui::Slider::new(&mut state.farmers, 0.0..=50.0)
                            .step_by(1.0)
                            .show_value(false));
                        let mut f = state.farmers as i32;
                        if ui.add(egui::DragValue::new(&mut f).range(0..=50).suffix(" /town")).changed() {
                            state.farmers = f as f32;
                        }
                    });

                    ui.add_space(4.0);

                    // Archer homes per town (each supports 1 archer)
                    ui.horizontal(|ui| {
                        ui.label("Archer Homes:");
                        ui.add(egui::Slider::new(&mut state.archers, 0.0..=5000.0)
                            .step_by(1.0)
                            .show_value(false));
                        let mut g = state.archers as i32;
                        if ui.add(egui::DragValue::new(&mut g).range(0..=5000).suffix(" /town")).changed() {
                            state.archers = g as f32;
                        }
                    });

                    ui.add_space(4.0);

                    // Tents per camp (1 raider per tent)
                    ui.horizontal(|ui| {
                        ui.label("Tents:");
                        ui.add(egui::Slider::new(&mut state.raiders, 0.0..=5000.0)
                            .step_by(1.0)
                            .show_value(false));
                        let mut r = state.raiders as i32;
                        if ui.add(egui::DragValue::new(&mut r).range(0..=5000).suffix(" /camp")).changed() {
                            state.raiders = r as f32;
                        }
                    });

                    ui.add_space(8.0);

                    // NPC total
                    let player_towns = state.towns as i32;
                    let ai_towns = state.ai_towns as i32;
                    let camps = state.raider_camps as i32;
                    let per_town = state.farmers as i32 + state.archers as i32;
                    let villagers = (player_towns + ai_towns) * per_town;
                    let raiders = camps * state.raiders as i32;
                    let total = villagers + raiders;
                    ui.label(format!("~{} NPCs total", total));

                    ui.add_space(12.0);

                    ui.horizontal(|ui| {
                        if ui.button(egui::RichText::new("Debug Tests").size(14.0)).clicked() {
                            next_state.set(AppState::TestMenu);
                        }

                        if ui.button(egui::RichText::new("Reset Defaults").size(14.0)).clicked() {
                            let defaults = settings::UserSettings::default();
                            state.world_size = defaults.world_size;
                            state.towns = defaults.towns as f32;
                            state.farms = defaults.farms as f32;
                            state.farmers = defaults.farmers as f32;
                            state.archers = defaults.archers as f32;
                            state.raiders = defaults.raiders as f32;
                            state.ai_towns = defaults.ai_towns as f32;
                            state.raider_camps = defaults.raider_camps as f32;
                            state.ai_interval = defaults.ai_interval;
                            state.npc_interval = defaults.npc_interval;
                            state.gen_style = defaults.gen_style as i32;
                            state.gold_mines = defaults.gold_mines_per_town as f32;
                            state.raider_passive_forage = defaults.raider_passive_forage;
                            state.difficulty = defaults.difficulty;
                            state.autosave_hours = defaults.autosave_hours;
                            settings::save_settings(&defaults);
                        }
                    });
                });
        });
    });

    Ok(())
}
