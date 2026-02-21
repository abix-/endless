//! Main menu — world config sliders + Play / Debug Tests buttons.

use std::collections::BTreeMap;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::AppState;
use crate::components::Job;
use crate::constants::NPC_REGISTRY;
use crate::settings;
use crate::world::{WorldGenConfig, WorldGenStyle};
use crate::systems::AiPlayerConfig;

/// Slider state persisted across frames via Local.
#[derive(Default)]
pub struct MenuState {
    pub world_size: f32,
    pub towns: f32,
    pub farms: f32,
    /// Per-job NPC home counts, driven by NPC_REGISTRY.
    pub npc_counts: BTreeMap<Job, f32>,
    pub ai_towns: f32,
    pub raider_towns: f32,
    pub ai_interval: f32,
    pub npc_interval: f32,
    pub gen_style: i32,
    pub gold_mines: f32,
    pub raider_passive_forage: bool,
    pub difficulty: crate::resources::Difficulty,
    pub prev_difficulty: crate::resources::Difficulty,
    pub autosave_hours: i32,
    pub show_load_menu: bool,
    pub initialized: bool,
    pub endless_mode: bool,
    pub endless_strength: f32,
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
        // Load npc_counts from settings (String keys → Job)
        for def in NPC_REGISTRY {
            let key = format!("{:?}", def.job);
            let count = saved.npc_counts.get(&key).copied().unwrap_or(def.default_count);
            state.npc_counts.insert(def.job, count as f32);
        }
        state.ai_towns = saved.ai_towns as f32;
        state.raider_towns = saved.raider_towns as f32;
        state.ai_interval = saved.ai_interval;
        state.npc_interval = saved.npc_interval;
        state.gen_style = saved.gen_style as i32;
        state.gold_mines = saved.gold_mines_per_town as f32;
        state.raider_passive_forage = saved.raider_passive_forage;
        state.difficulty = saved.difficulty;
        state.prev_difficulty = saved.difficulty;
        state.autosave_hours = saved.autosave_hours;
        state.endless_mode = saved.endless_mode;
        state.endless_strength = saved.endless_strength;
        state.initialized = true;
    }

    // Apply difficulty presets when difficulty changes
    if state.difficulty != state.prev_difficulty {
        let preset = state.difficulty.presets();
        state.farms = preset.farms as f32;
        state.ai_towns = preset.ai_towns as f32;
        state.raider_towns = preset.raider_towns as f32;
        state.gold_mines = preset.gold_mines as f32;
        state.endless_mode = preset.endless_mode;
        state.endless_strength = preset.endless_strength;
        for (&job, &count) in &preset.npc_counts {
            state.npc_counts.insert(job, count as f32);
        }
        state.prev_difficulty = state.difficulty;
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading(egui::RichText::new("Endless").size(32.0));
            ui.add_space(20.0);
        });

        // Config sliders in a centered frame
        let panel_width = 400.0;
        ui.vertical_centered(|ui| {
            ui.set_max_width(panel_width);

            // ── World ──────────────────────────────
            ui.separator();
            ui.label(egui::RichText::new("World").strong());
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("World Size:").on_hover_text("Total world size in pixels. Larger worlds take longer to generate.");
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

            ui.horizontal(|ui| {
                ui.label("World Gen:").on_hover_text("Classic: single landmass. Continents: multiple islands separated by ocean.");
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

            ui.add_space(8.0);

            // ── Difficulty ─────────────────────────
            ui.separator();
            ui.label(egui::RichText::new("Difficulty").strong());
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Preset:").on_hover_text("Adjusts farms, mines, and NPC counts. Change individual sliders for custom difficulty.");
                egui::ComboBox::from_id_salt("difficulty")
                    .selected_text(state.difficulty.label())
                    .show_ui(ui, |ui| {
                        for d in crate::resources::Difficulty::ALL {
                            ui.selectable_value(&mut state.difficulty, d, d.label());
                        }
                    });
            });

            ui.add_space(4.0);

            // Per-town settings (apply to player AND AI towns)
            ui.label(egui::RichText::new("Per Town (player & AI)").weak());
            ui.indent("per_town_settings", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Farms:").on_hover_text("Farms per town. Farms produce food for NPCs.");
                    ui.add(egui::Slider::new(&mut state.farms, 0.0..=100.0)
                        .step_by(1.0)
                        .show_value(false));
                    let mut fm = state.farms as i32;
                    if ui.add(egui::DragValue::new(&mut fm).range(0..=100).suffix(" /town")).changed() {
                        state.farms = fm as f32;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Gold Mines:").on_hover_text("Gold mines per town. Miners extract gold for upgrades and recruiting.");
                    ui.add(egui::Slider::new(&mut state.gold_mines, 0.0..=10.0)
                        .step_by(1.0)
                        .show_value(false));
                    let mut gm = state.gold_mines as i32;
                    if ui.add(egui::DragValue::new(&mut gm).range(0..=10).suffix(" /town")).changed() {
                        state.gold_mines = gm as f32;
                    }
                });
                for def in NPC_REGISTRY.iter().filter(|d| !d.is_raider_unit) {
                    ui.horizontal(|ui| {
                        let label = format!("{} Homes:", def.label);
                        let tip = format!("{} homes per town (player & AI). Each home spawns one {}.", def.label, def.label.to_lowercase());
                        ui.label(label).on_hover_text(tip);
                        let val = state.npc_counts.entry(def.job).or_insert(0.0);
                        ui.add(egui::Slider::new(val, 0.0..=1000.0)
                            .step_by(1.0)
                            .show_value(false));
                        let mut iv = *val as i32;
                        if ui.add(egui::DragValue::new(&mut iv).range(0..=1000).suffix(" /town")).changed() {
                            *val = iv as f32;
                        }
                    });
                }
            });

            ui.add_space(4.0);

            // Town counts
            ui.horizontal(|ui| {
                ui.label("AI Builder Towns:").on_hover_text("Number of AI-controlled friendly builder towns on the map.");
                ui.add(egui::Slider::new(&mut state.ai_towns, 0.0..=20.0)
                    .step_by(1.0)
                    .show_value(false));
                let mut at = state.ai_towns as i32;
                if ui.add(egui::DragValue::new(&mut at).range(0..=20)).changed() {
                    state.ai_towns = at as f32;
                }
            });
            ui.horizontal(|ui| {
                ui.label("AI Raider Towns:").on_hover_text("Number of AI-controlled hostile raider towns on the map.");
                ui.add(egui::Slider::new(&mut state.raider_towns, 0.0..=20.0)
                    .step_by(1.0)
                    .show_value(false));
                let mut rc = state.raider_towns as i32;
                if ui.add(egui::DragValue::new(&mut rc).range(0..=20)).changed() {
                    state.raider_towns = rc as f32;
                }
            });
            ui.indent("raider_town_children", |ui| {
                for def in NPC_REGISTRY.iter().filter(|d| d.is_raider_unit) {
                    ui.horizontal(|ui| {
                        let label = format!("{}s:", def.label);
                        let tip = format!("Raider tents per raider town. Each tent spawns one {}.", def.label.to_lowercase());
                        ui.label(label).on_hover_text(tip);
                        let val = state.npc_counts.entry(def.job).or_insert(0.0);
                        ui.add(egui::Slider::new(val, 0.0..=1000.0)
                            .step_by(1.0)
                            .show_value(false));
                        let mut iv = *val as i32;
                        if ui.add(egui::DragValue::new(&mut iv).range(0..=1000).suffix(" /town")).changed() {
                            *val = iv as f32;
                        }
                    });
                }
            });

            ui.add_space(4.0);

            // Endless mode (part of difficulty)
            ui.checkbox(&mut state.endless_mode, "Endless Mode")
                .on_hover_text("Destroyed AI towns (builder and raider) are replaced by new, stronger ones.");
            if state.endless_mode {
                ui.horizontal(|ui| {
                    ui.label("Replacement Strength:").on_hover_text("Strength of replacement towns relative to the player. Higher = harder.");
                    ui.add(egui::Slider::new(&mut state.endless_strength, 0.25..=1.5)
                        .step_by(0.05)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                });
            }

            ui.add_space(8.0);

            // ── Options ────────────────────────────
            ui.separator();
            ui.label(egui::RichText::new("Options").strong());
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Autosave:").on_hover_text("Auto-save interval in game hours. 0 = disabled.");
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
                wg_config.num_towns = 1;
                wg_config.farms_per_town = state.farms as usize;
                wg_config.npc_counts = state.npc_counts.iter().map(|(&job, &v)| (job, v as usize)).collect();
                wg_config.ai_towns = state.ai_towns as usize;
                wg_config.raider_towns = state.raider_towns as usize;
                wg_config.gold_mines_per_town = state.gold_mines as usize;
                ai_config.decision_interval = state.ai_interval;
                npc_config.interval = state.npc_interval;

                let mut saved = settings::load_settings();
                saved.world_size = state.world_size;
                saved.towns = 1;
                saved.farms = state.farms as usize;
                saved.npc_counts = state.npc_counts.iter()
                    .map(|(&job, &v)| (format!("{:?}", job), v as usize))
                    .collect();
                saved.ai_towns = state.ai_towns as usize;
                saved.raider_towns = state.raider_towns as usize;
                saved.ai_interval = state.ai_interval;
                saved.npc_interval = state.npc_interval;
                saved.gen_style = state.gen_style as u8;
                saved.gold_mines_per_town = state.gold_mines as usize;
                saved.raider_passive_forage = state.raider_passive_forage;
                saved.difficulty = state.difficulty;
                saved.autosave_hours = state.autosave_hours;
                saved.endless_mode = state.endless_mode;
                saved.endless_strength = state.endless_strength;
                settings::save_settings(&saved);
                user_settings.raider_passive_forage = state.raider_passive_forage;

                commands.insert_resource(state.difficulty);
                commands.insert_resource(crate::resources::EndlessMode {
                    enabled: state.endless_mode,
                    strength_fraction: state.endless_strength,
                    pending_spawns: Vec::new(),
                });
                save_request.autosave_hours = state.autosave_hours;
                next_state.set(AppState::Playing);
            }

            ui.add_space(8.0);

            // Load Game button — opens a save picker window
            let saves = crate::save::list_saves();
            if saves.is_empty() {
                ui.add_enabled_ui(false, |ui| {
                    let _ = ui.button(egui::RichText::new("Load Game").size(18.0));
                });
            } else if ui.button(egui::RichText::new("Load Game").size(18.0)).clicked() {
                state.show_load_menu = !state.show_load_menu;
            }

            ui.add_space(8.0);

            if user_settings.tutorial_completed {
                if ui.button(egui::RichText::new("Restart Tutorial").size(14.0)).clicked() {
                    user_settings.tutorial_completed = false;
                    settings::save_settings(&user_settings);
                }
            }

            ui.add_space(20.0);

            // Debug options — collapsed by default
            egui::CollapsingHeader::new("Debug Options")
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        ui.label("AI Think:").on_hover_text("How often AI towns make decisions (build, recruit, attack). Lower = more responsive.");
                        ui.add(egui::Slider::new(&mut state.ai_interval, 1.0..=30.0)
                            .step_by(0.5)
                            .suffix("s")
                            .show_value(true));
                    });

                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        ui.label("NPC Think:").on_hover_text("How often NPCs re-evaluate their behavior. Lower = more responsive but heavier on CPU.");
                        ui.add(egui::Slider::new(&mut state.npc_interval, 0.5..=10.0)
                            .step_by(0.5)
                            .suffix("s")
                            .show_value(true));
                    });

                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        ui.label("Raider Passive Forage:").on_hover_text("Raiders passively gather food even when not actively raiding.");
                        ui.checkbox(&mut state.raider_passive_forage, "Enabled");
                    });

                    ui.add_space(8.0);

                    // NPC total — computed from registry grouping
                    let towns = state.towns as i32 + state.ai_towns as i32;
                    let raiders = state.raider_towns as i32;
                    let village_per_town: i32 = NPC_REGISTRY.iter()
                        .filter(|d| !d.is_raider_unit)
                        .map(|d| *state.npc_counts.get(&d.job).unwrap_or(&0.0) as i32)
                        .sum();
                    let raider_per_town: i32 = NPC_REGISTRY.iter()
                        .filter(|d| d.is_raider_unit)
                        .map(|d| *state.npc_counts.get(&d.job).unwrap_or(&0.0) as i32)
                        .sum();
                    let total = towns * village_per_town + raiders * raider_per_town;
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
                            for def in NPC_REGISTRY {
                                let key = format!("{:?}", def.job);
                                let count = defaults.npc_counts.get(&key).copied().unwrap_or(def.default_count);
                                state.npc_counts.insert(def.job, count as f32);
                            }
                            state.ai_towns = defaults.ai_towns as f32;
                            state.raider_towns = defaults.raider_towns as f32;
                            state.ai_interval = defaults.ai_interval;
                            state.npc_interval = defaults.npc_interval;
                            state.gen_style = defaults.gen_style as i32;
                            state.gold_mines = defaults.gold_mines_per_town as f32;
                            state.raider_passive_forage = defaults.raider_passive_forage;
                            state.difficulty = defaults.difficulty;
                            state.autosave_hours = defaults.autosave_hours;
                            state.endless_mode = defaults.endless_mode;
                            state.endless_strength = defaults.endless_strength;
                            settings::save_settings(&defaults);
                        }
                    });
                });
        });
    });

    // Load Game window — shown when show_load_menu is true
    if state.show_load_menu {
        let saves = crate::save::list_saves();
        let mut open = true;
        egui::Window::new("Load Game")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(400.0)
            .show(ctx, |ui| {
                if saves.is_empty() {
                    ui.label("No save files found");
                } else {
                    for save_info in &saves {
                        ui.horizontal(|ui| {
                            let name = save_info.filename.trim_end_matches(".json");
                            if ui.button(egui::RichText::new(name).size(14.0)).clicked() {
                                save_request.load_on_enter = true;
                                save_request.load_path = Some(save_info.path.clone());
                                save_request.autosave_hours = state.autosave_hours;
                                next_state.set(AppState::Playing);
                            }
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
                            ui.label(egui::RichText::new(age).size(12.0).weak());
                        });
                    }
                }
            });
        if !open {
            state.show_load_menu = false;
        }
    }

    Ok(())
}
