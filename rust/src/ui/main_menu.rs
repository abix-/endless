//! Main menu — world config sliders + Play / Debug Tests buttons.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use std::collections::BTreeMap;

use bevy::audio::Volume;

use crate::AppState;
use crate::components::Job;
use crate::constants::NPC_REGISTRY;
use crate::resources::{GameAudio, MusicTrack, PauseSettingsTab};
use crate::settings;
use crate::systems::AiPlayerConfig;
use crate::world::{WorldGenConfig, WorldGenStyle};

/// Per-AI-player slot kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AiSlotKind {
    Builder,
    Raider,
}

impl AiSlotKind {
    pub fn label(self) -> &'static str {
        match self {
            AiSlotKind::Builder => "Builder",
            AiSlotKind::Raider => "Raider",
        }
    }
}

/// Per-AI-player slot config for WC3-style lobby.
#[derive(Clone)]
pub struct AiSlotConfig {
    pub kind: AiSlotKind,
    pub llm: bool,
}

/// Slider state persisted across frames via Local.
#[derive(Default)]
pub struct MenuState {
    pub world_size: f32,
    pub towns: f32,
    pub farms: f32,
    /// Per-job NPC home counts, driven by NPC_REGISTRY.
    pub npc_counts: BTreeMap<Job, f32>,
    pub ai_slots: Vec<AiSlotConfig>,
    pub ai_interval: f32,
    pub npc_interval: f32,
    pub gold_mines: f32,
    pub raider_forage_hours: f32,
    pub difficulty: crate::resources::Difficulty,
    pub prev_difficulty: crate::resources::Difficulty,
    pub autosave_hours: i32,
    pub show_load_menu: bool,
    pub show_settings: bool,
    pub settings_tab: PauseSettingsTab,
    pub rebinding_action: Option<settings::ControlAction>,
    pub initialized: bool,
    pub endless_mode: bool,
    pub endless_strength: f32,
}

fn is_player_home_job(job: Job) -> bool {
    matches!(job, Job::Farmer | Job::Archer)
}

fn strip_disabled_home_jobs(npc_counts: &mut BTreeMap<Job, f32>) {
    npc_counts.insert(Job::Miner, 0.0);
    npc_counts.insert(Job::Fighter, 0.0);
    npc_counts.insert(Job::Crossbow, 0.0);
}

fn clamp_player_menu_caps(state: &mut MenuState) {
    state.farms = state.farms.clamp(0.0, 10.0);
    state.gold_mines = state.gold_mines.clamp(0.0, 10.0);
    for job in [Job::Farmer, Job::Archer] {
        let v = state.npc_counts.entry(job).or_insert(0.0);
        *v = v.clamp(0.0, 10.0);
    }
}

fn size_name(size: f32) -> &'static str {
    match size as i32 {
        8000 => "Tiny",
        16000 => "Small",
        24000 => "Medium",
        32000 => "Large",
        40000 => "Huge",
        48000 => "Massive",
        56000 => "Epic",
        64000 => "Endless",
        _ => "Custom",
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct MenuVideoParams<'w> {
    winit_settings: ResMut<'w, bevy::winit::WinitSettings>,
    framepace: ResMut<'w, bevy_framepace::FramepaceSettings>,
}

pub fn main_menu_system(
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut next_state: ResMut<NextState<AppState>>,
    mut wg_config: ResMut<WorldGenConfig>,
    mut ai_config: ResMut<AiPlayerConfig>,
    mut npc_config: ResMut<crate::resources::NpcDecisionConfig>,
    mut pathfind_config: ResMut<crate::resources::PathfindConfig>,
    mut user_settings: ResMut<settings::UserSettings>,
    mut save_request: ResMut<crate::save::SaveLoadRequest>,
    mut windows: Query<&mut Window>,
    keys: Res<ButtonInput<KeyCode>>,
    mut audio: ResMut<GameAudio>,
    mut music_sinks: Query<&mut AudioSink, With<MusicTrack>>,
    mut video: MenuVideoParams,
    mut state: Local<MenuState>,
    mut exit: MessageWriter<AppExit>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    pathfind_config.max_per_frame = user_settings.pathfind_max_per_frame.max(1);

    // Init slider defaults from saved settings (or WorldGenConfig defaults)
    if !state.initialized {
        let saved = settings::load_settings();
        state.world_size = saved.world_size;
        state.towns = saved.towns as f32;
        state.farms = saved.farms as f32;
        // Load npc_counts from settings (String keys → Job)
        for def in NPC_REGISTRY {
            let key = format!("{:?}", def.job);
            let count = saved
                .npc_counts
                .get(&key)
                .copied()
                .unwrap_or(def.default_count);
            state.npc_counts.insert(def.job, count as f32);
        }
        state.ai_slots = saved
            .ai_slots
            .iter()
            .map(|s| AiSlotConfig {
                kind: if s.kind == 1 {
                    AiSlotKind::Raider
                } else {
                    AiSlotKind::Builder
                },
                llm: s.llm,
            })
            .collect();
        state.ai_interval = saved.ai_interval;
        state.npc_interval = saved.npc_interval;
        state.gold_mines = saved.gold_mines_per_town as f32;
        state.raider_forage_hours = saved.raider_forage_hours;
        state.difficulty = saved.difficulty;
        state.prev_difficulty = saved.difficulty;
        state.autosave_hours = saved.autosave_hours;
        state.endless_mode = saved.endless_mode;
        state.endless_strength = saved.endless_strength;
        strip_disabled_home_jobs(&mut state.npc_counts);
        clamp_player_menu_caps(&mut state);
        state.initialized = true;
    }

    // Apply difficulty presets when difficulty changes
    if state.difficulty != state.prev_difficulty {
        let preset = state.difficulty.presets();
        state.farms = preset.farms as f32;
        // Rebuild ai_slots from preset counts, preserving LLM flags where possible
        let mut new_slots = Vec::new();
        for i in 0..preset.ai_towns {
            let llm = state
                .ai_slots
                .get(i)
                .is_some_and(|s| s.llm && s.kind == AiSlotKind::Builder);
            new_slots.push(AiSlotConfig {
                kind: AiSlotKind::Builder,
                llm,
            });
        }
        let old_raider_start = state
            .ai_slots
            .iter()
            .position(|s| s.kind == AiSlotKind::Raider)
            .unwrap_or(state.ai_slots.len());
        for i in 0..preset.raider_towns {
            let llm = state
                .ai_slots
                .get(old_raider_start + i)
                .is_some_and(|s| s.llm && s.kind == AiSlotKind::Raider);
            new_slots.push(AiSlotConfig {
                kind: AiSlotKind::Raider,
                llm,
            });
        }
        state.ai_slots = new_slots;
        state.gold_mines = preset.gold_mines as f32;
        state.endless_mode = preset.endless_mode;
        state.endless_strength = preset.endless_strength;
        state.raider_forage_hours = preset.raider_forage_hours;
        for (&job, &count) in &preset.npc_counts {
            state.npc_counts.insert(job, count as f32);
        }
        strip_disabled_home_jobs(&mut state.npc_counts);
        clamp_player_menu_caps(&mut state);
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
                ui.add(egui::Slider::new(&mut state.world_size, 8000.0..=64000.0)
                    .step_by(1000.0)
                    .show_value(false));
                let mut ws = state.world_size as i32;
                if ui.add(egui::DragValue::new(&mut ws).range(8000..=64000).speed(1000)).changed() {
                    state.world_size = ws as f32;
                }
                let tiles = state.world_size as i32 / 64;
                ui.label(format!("{} ({}x{})", size_name(state.world_size), tiles, tiles));
            });

            ui.add_space(4.0);

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
                    ui.add(egui::Slider::new(&mut state.farms, 0.0..=10.0)
                        .step_by(1.0)
                        .show_value(false));
                    let mut fm = state.farms as i32;
                    if ui.add(egui::DragValue::new(&mut fm).range(0..=10).suffix(" /town")).changed() {
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
                for def in NPC_REGISTRY.iter().filter(|d| !d.is_raider_unit && is_player_home_job(d.job)) {
                    ui.horizontal(|ui| {
                        let label = format!("{} Homes:", def.label);
                        let tip = format!("{} homes per town (player & AI). Each home spawns one {}.", def.label, def.label.to_lowercase());
                        ui.label(label).on_hover_text(tip);
                        let val = state.npc_counts.entry(def.job).or_insert(0.0);
                        ui.add(egui::Slider::new(val, 0.0..=10.0)
                            .step_by(1.0)
                            .show_value(false));
                        let mut iv = *val as i32;
                        if ui.add(egui::DragValue::new(&mut iv).range(0..=10).suffix(" /town")).changed() {
                            *val = iv as f32;
                        }
                    });
                }
            });

            ui.add_space(4.0);

            // AI Players (WC3-style per-slot config)
            ui.label(egui::RichText::new("AI Players").weak());
            let mut remove_idx = None;
            for (i, slot) in state.ai_slots.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("{}.", i + 1));
                    egui::ComboBox::from_id_salt(format!("ai_kind_{i}"))
                        .selected_text(slot.kind.label())
                        .width(70.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut slot.kind, AiSlotKind::Builder, "Builder");
                            ui.selectable_value(&mut slot.kind, AiSlotKind::Raider, "Raider");
                        });
                    ui.checkbox(&mut slot.llm, "LLM").on_hover_text("Allow an AI model to control this town via BRP endpoints.");
                    if ui.small_button("x").on_hover_text("Remove this AI player").clicked() {
                        remove_idx = Some(i);
                    }
                });
            }
            if let Some(idx) = remove_idx {
                state.ai_slots.remove(idx);
            }
            ui.horizontal(|ui| {
                if state.ai_slots.len() < 20 {
                    if ui.small_button("+ Builder").clicked() {
                        state.ai_slots.push(AiSlotConfig { kind: AiSlotKind::Builder, llm: false });
                    }
                    if ui.small_button("+ Raider").clicked() {
                        state.ai_slots.push(AiSlotConfig { kind: AiSlotKind::Raider, llm: false });
                    }
                }
            });

            // Raider settings (applied globally to all raider slots)
            let has_raiders = state.ai_slots.iter().any(|s| s.kind == AiSlotKind::Raider);
            if has_raiders {
                ui.indent("raider_settings", |ui| {
                    ui.label(egui::RichText::new("Raider Settings").weak());
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
                    ui.horizontal(|ui| {
                        ui.label("Forage:");
                        ui.add(egui::Slider::new(&mut state.raider_forage_hours, 0.0..=24.0)
                            .step_by(1.0)
                            .show_value(false));
                        let label = if state.raider_forage_hours == 0.0 { "Off".to_string() } else { format!("{}h/food", state.raider_forage_hours as i32) };
                        ui.label(label);
                    }).response.on_hover_text("Hours for each raider town to passively forage 1 food. 0 = disabled.");
                });
            }

            ui.add_space(20.0);

            // Play button
            if ui.button(egui::RichText::new("  Play  ").size(18.0)).clicked() {
                strip_disabled_home_jobs(&mut state.npc_counts);
                clamp_player_menu_caps(&mut state);
                wg_config.gen_style = WorldGenStyle::Continents;
                wg_config.world_width = state.world_size;
                wg_config.world_height = state.world_size;
                wg_config.num_towns = 1;
                wg_config.farms_per_town = state.farms as usize;
                wg_config.npc_counts = state.npc_counts.iter().map(|(&job, &v)| (job, v as usize)).collect();
                let ai_builder_count = state.ai_slots.iter().filter(|s| s.kind == AiSlotKind::Builder).count();
                let ai_raider_count = state.ai_slots.iter().filter(|s| s.kind == AiSlotKind::Raider).count();
                wg_config.ai_towns = ai_builder_count;
                wg_config.raider_towns = ai_raider_count;
                wg_config.gold_mines_per_town = state.gold_mines as usize;
                ai_config.decision_interval = state.ai_interval;
                npc_config.interval = state.npc_interval;
                pathfind_config.max_per_frame = user_settings.pathfind_max_per_frame.max(1);

                let mut saved = settings::load_settings();
                saved.world_size = state.world_size;
                saved.towns = 1;
                saved.farms = state.farms as usize;
                saved.npc_counts = state.npc_counts.iter()
                    .map(|(&job, &v)| (format!("{:?}", job), v as usize))
                    .collect();
                saved.ai_towns = ai_builder_count;
                saved.raider_towns = ai_raider_count;
                saved.ai_slots = state.ai_slots.iter().map(|s| settings::AiSlotSave {
                    kind: if s.kind == AiSlotKind::Raider { 1 } else { 0 },
                    llm: s.llm,
                }).collect();
                saved.ai_interval = state.ai_interval;
                saved.npc_interval = state.npc_interval;
                saved.gen_style = 1;
                saved.gold_mines_per_town = state.gold_mines as usize;
                saved.raider_forage_hours = state.raider_forage_hours;
                saved.difficulty = state.difficulty;
                saved.autosave_hours = state.autosave_hours;
                saved.endless_mode = state.endless_mode;
                saved.endless_strength = state.endless_strength;
                settings::save_settings(&saved);
                user_settings.raider_forage_hours = state.raider_forage_hours;

                commands.insert_resource(state.difficulty);
                commands.insert_resource(crate::resources::EndlessMode {
                    enabled: true,
                    strength_fraction: state.endless_strength,
                    pending_spawns: Vec::new(),
                });
                // Populate LLM-allowed towns from slot config
                // Town ordering: player(0), builders(1..=N), raiders(N+1..)
                let num_player_towns = wg_config.num_towns; // typically 1
                let mut llm_towns = Vec::new();
                let mut builder_idx = 0usize;
                let mut raider_idx = 0usize;
                for slot in &state.ai_slots {
                    match slot.kind {
                        AiSlotKind::Builder => {
                            if slot.llm {
                                llm_towns.push(num_player_towns + builder_idx);
                            }
                            builder_idx += 1;
                        }
                        AiSlotKind::Raider => {
                            if slot.llm {
                                llm_towns.push(num_player_towns + ai_builder_count + raider_idx);
                            }
                            raider_idx += 1;
                        }
                    }
                }
                // Insert LLM player state for first LLM town (built-in claude --print)
                if let Some(&first_llm) = llm_towns.first() {
                    commands.insert_resource(
                        crate::systems::llm_player::LlmPlayerState::new(first_llm),
                    );
                }
                commands.insert_resource(crate::resources::RemoteAllowedTowns { towns: llm_towns });

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

            if ui.button(egui::RichText::new("Settings").size(18.0)).clicked() {
                state.show_settings = !state.show_settings;
            }
            ui.add_space(8.0);
            if ui.button(egui::RichText::new("Debug Tests").size(18.0)).clicked() {
                next_state.set(AppState::TestMenu);
            }
            ui.add_space(8.0);
            if ui.button(egui::RichText::new("Exit").size(18.0)).clicked() {
                exit.write(AppExit::Success);
            }
        });
    });

    // Settings window — shared with pause menu (minus Save/Load)
    if state.show_settings {
        let prev_fullscreen = user_settings.fullscreen;
        let prev_vsync = user_settings.vsync;
        let prev_width = user_settings.window_width;
        let prev_height = user_settings.window_height;
        let prev_maximized = user_settings.window_maximized;
        let prev_bg_fps = user_settings.background_fps;
        let prev_fps_cap = user_settings.fps_cap;
        let prev_music_vol = user_settings.music_volume;

        // Handle key rebinding
        if let Some(action) = state.rebinding_action {
            if let Some(bound_key) = keys
                .get_just_pressed()
                .copied()
                .find(|key| settings::is_rebindable_key(*key))
            {
                user_settings.set_key_for_action(action, bound_key);
                state.rebinding_action = None;
                settings::save_settings(&user_settings);
            }
        }

        let mut open = true;
        let window_resp = egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .min_width(820.0)
            .min_height(520.0)
            .show(ctx, |ui| {
                let MenuState {
                    ref mut settings_tab,
                    ref mut rebinding_action,
                    ..
                } = *state;
                crate::ui::settings_panel_ui(
                    ui,
                    &mut user_settings,
                    settings_tab,
                    rebinding_action,
                    None, // no save
                    None, // no load
                    None, // no LLM player in main menu
                )
            });
        if !open {
            state.show_settings = false;
            state.rebinding_action = None;
        }

        if let Some(inner) = window_resp.and_then(|r| r.inner) {
            if inner.reset_requested {
                state.rebinding_action = None;
                *user_settings = settings::UserSettings::default();
                settings::apply_fps_cap(user_settings.fps_cap, &mut video.framepace);
            }
        }

        // Apply side effects
        if user_settings.fullscreen != prev_fullscreen
            || user_settings.vsync != prev_vsync
            || user_settings.window_width != prev_width
            || user_settings.window_height != prev_height
            || user_settings.window_maximized != prev_maximized
        {
            user_settings.clamp_video_settings();
            if let Ok(mut window) = windows.single_mut() {
                settings::apply_video_settings_to_window(&mut window, &user_settings);
            }
        }
        if user_settings.fps_cap != prev_fps_cap {
            settings::apply_fps_cap(user_settings.fps_cap, &mut video.framepace);
        }
        if user_settings.background_fps != prev_bg_fps {
            video.winit_settings.unfocused_mode = if user_settings.background_fps {
                bevy::winit::UpdateMode::Continuous
            } else {
                bevy::winit::UpdateMode::reactive_low_power(std::time::Duration::from_secs_f64(
                    1.0 / 60.0,
                ))
            };
        }
        if (user_settings.music_volume - prev_music_vol).abs() > f32::EPSILON {
            audio.music_volume = user_settings.music_volume;
            for mut sink in &mut music_sinks {
                sink.set_volume(Volume::Linear(user_settings.music_volume));
            }
        }
        audio.sfx_volume = user_settings.sfx_volume;

        settings::save_settings(&user_settings);
    }

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
