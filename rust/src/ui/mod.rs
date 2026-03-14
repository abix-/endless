//! UI module — main menu, game startup, in-game HUD, and gameplay panels.

pub mod blackjack;
pub mod build_menu;
pub mod game_hud;
pub mod left_panel;
pub mod main_menu;
pub mod tutorial;

use bevy::audio::Volume;
use bevy::ecs::system::SystemParam;
use bevy::math::primitives::Rectangle;
use bevy::mesh::Mesh2d;
use bevy::prelude::*;
use bevy::sprite_render::{ColorMaterial, MeshMaterial2d};
use bevy_egui::{EguiContextSettings, EguiPrimaryContextPass, egui};

use crate::AppState;
use crate::components::*;
use crate::constants::TOWN_GRID_SPACING;
use crate::messages::SpawnNpcMsg;
use crate::resources::*;
use crate::settings::{self, ControlAction, ControlGroup, UserSettings};
use crate::systemparams::WorldState;
use crate::systems::{AiPersonality, AiPlayerState};
use crate::systems::ai_player::RoadStyle;
use crate::world::{self, BuildingKind, WorldGenConfig};

/// Render a small "?" button (frameless) that shows help text on hover.
pub fn help_tip(ui: &mut egui::Ui, catalog: &HelpCatalog, key: &str) {
    if let Some(text) = catalog.0.get(key) {
        ui.add(
            egui::Button::new(
                egui::RichText::new("?")
                    .color(egui::Color32::from_rgb(120, 120, 180))
                    .small(),
            )
            .frame(false),
        )
        .on_hover_text(*text);
    }
}
/// Render a label that shows a tooltip on hover (frameless button trick).
pub fn tipped(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>, tip: &str) -> egui::Response {
    ui.add(egui::Button::new(text).frame(false))
        .on_hover_text(tip)
}

/// Stable display name for a gold mine index used across all inspectors/policies.
pub fn gold_mine_name(mine_idx: usize) -> String {
    format!("Gold Mine {}", mine_idx + 1)
}

// ============================================================================
// SHARED SETTINGS PANEL
// ============================================================================

/// Response from the shared settings panel UI.
pub struct SettingsResponse {
    pub reset_requested: bool,
    pub save_requested: bool,
    pub load_requested: bool,
    /// If save was "Save As", this is the named path.
    pub save_path: Option<std::path::PathBuf>,
    /// If load was by name or from list, this is the path.
    pub load_path: Option<std::path::PathBuf>,
}

/// Render the full settings panel (tab sidebar + content area).
/// Called from both pause menu and main menu.
pub fn settings_panel_ui(
    ui: &mut egui::Ui,
    settings: &mut UserSettings,
    tab: &mut PauseSettingsTab,
    rebinding_action: &mut Option<ControlAction>,
    // Save/Load tab state — None hides those tabs
    manual_save_name: Option<&mut String>,
    manual_load_name: Option<&mut String>,
    // LLM player inspector — (command, payload, response), None if no LLM player active
    llm_preview: Option<(&str, &str, &str)>,
) -> SettingsResponse {
    let mut resp = SettingsResponse {
        reset_requested: false,
        save_requested: false,
        load_requested: false,
        save_path: None,
        load_path: None,
    };
    let show_save_load = manual_save_name.is_some();

    ui.horizontal(|ui| {
        ui.set_min_height(390.0);
        ui.set_max_height(390.0);

        // Tab sidebar
        ui.vertical(|ui| {
            ui.set_min_width(180.0);
            ui.add_space(8.0);
            for &tab_val in &[
                PauseSettingsTab::Interface,
                PauseSettingsTab::Video,
                PauseSettingsTab::Camera,
                PauseSettingsTab::Controls,
                PauseSettingsTab::Audio,
                PauseSettingsTab::Logs,
                PauseSettingsTab::Debug,
                PauseSettingsTab::LlmPlayer,
            ] {
                ui.selectable_value(
                    tab,
                    tab_val,
                    egui::RichText::new(tab_val.label()).size(18.0),
                );
            }
            if show_save_load {
                ui.selectable_value(
                    tab,
                    PauseSettingsTab::SaveGame,
                    egui::RichText::new("Save Game").size(18.0),
                );
                ui.selectable_value(
                    tab,
                    PauseSettingsTab::LoadGame,
                    egui::RichText::new("Load Game").size(18.0),
                );
            }
        });

        ui.separator();

        // Content area
        ui.vertical(|ui| {
            ui.set_min_width(580.0);
            let (title, subtitle) = tab.title_subtitle();
            ui.heading(title);
            ui.small(subtitle);
            ui.separator();

            egui::ScrollArea::vertical()
                .max_height(340.0)
                .show(ui, |ui| {
                    match *tab {
                        PauseSettingsTab::Interface => {
                            ui.add(egui::Slider::new(&mut settings.ui_scale, 0.8..=2.5).text("UI Scale"))
                                .on_hover_text("Scales all UI windows and controls.");
                            ui.small("Higher values make every panel larger.");
                            ui.add_space(6.0);

                            ui.add(egui::Slider::new(&mut settings.interface_text_size, 10.0..=28.0).text("Interface Text Size"))
                                .on_hover_text("Base font size for menus, buttons, and panel text.");
                            ui.small("Increase this to make settings and interface text easier to read.");
                            ui.add_space(6.0);

                            ui.add(egui::Slider::new(&mut settings.help_text_size, 8.0..=24.0).text("Help Text Size"))
                                .on_hover_text("Font size for inline tips and help text.");
                            ui.small("Increase for better readability.");
                            ui.add_space(6.0);

                            ui.add(egui::Slider::new(&mut settings.build_menu_text_scale, 0.7..=2.0).text("Build Menu Text Scale"))
                                .on_hover_text("Extra scaling for build-menu labels.");
                            ui.small("Useful when build entries feel cramped.");
                            ui.add_space(6.0);

                            ui.checkbox(&mut settings.background_fps, "Full FPS in Background")
                                .on_hover_text("Keep full update/render speed when the game window is unfocused.");
                            ui.small("Disable to reduce CPU/GPU usage while tabbed out.");
                            ui.add_space(6.0);

                            ui.horizontal(|ui| {
                                ui.label("Autosave:");
                                ui.add(egui::Slider::new(&mut settings.autosave_hours, 0..=48)
                                    .step_by(1.0)
                                    .show_value(false));
                                let label = if settings.autosave_hours == 0 { "Off".to_string() } else { format!("{}h", settings.autosave_hours) };
                                ui.label(label);
                            });
                            ui.small("Auto-save interval in game hours. 0 = disabled.");
                            ui.add_space(6.0);

                            if settings.tutorial_completed {
                                if ui.button("Restart Tutorial").clicked() {
                                    settings.tutorial_completed = false;
                                }
                            }
                        }
                        PauseSettingsTab::Video => {
                            const RESOLUTIONS: &[(u32, u32)] = &[
                                (1280, 720),
                                (1600, 900),
                                (1920, 1080),
                                (2560, 1440),
                                (3840, 2160),
                            ];

                            ui.checkbox(&mut settings.fullscreen, "Fullscreen")
                                .on_hover_text("Borderless fullscreen on the current monitor.");
                            ui.add_space(6.0);

                            ui.add_enabled_ui(!settings.fullscreen, |ui| {
                                ui.label("Resolution");
                                let current_label = format!(
                                    "{} x {}",
                                    settings.window_width, settings.window_height
                                );
                                egui::ComboBox::from_id_salt("settings_resolution")
                                    .selected_text(&current_label)
                                    .show_ui(ui, |ui| {
                                        for &(w, h) in RESOLUTIONS {
                                            let label = format!("{w} x {h}");
                                            if ui.selectable_label(
                                                settings.window_width == w
                                                    && settings.window_height == h,
                                                &label,
                                            ).clicked() {
                                                settings.window_width = w;
                                                settings.window_height = h;
                                            }
                                        }
                                    });
                                ui.add_space(6.0);

                                ui.checkbox(&mut settings.window_maximized, "Start Maximized")
                                    .on_hover_text("Open in maximized mode on launch.");
                            });
                            ui.add_space(6.0);

                            ui.checkbox(&mut settings.vsync, "VSync")
                                .on_hover_text("Reduces tearing by syncing frame presentation to refresh rate.");
                            ui.add_space(6.0);

                            ui.label("FPS Cap");
                            const FPS_OPTIONS: &[(u32, &str)] = &[
                                (0, "Uncapped"),
                                (30, "30"),
                                (60, "60"),
                                (120, "120"),
                                (144, "144"),
                                (240, "240"),
                            ];
                            let current_label = FPS_OPTIONS
                                .iter()
                                .find(|(v, _)| *v == settings.fps_cap)
                                .map(|(_, l)| *l)
                                .unwrap_or("Custom");
                            egui::ComboBox::from_id_salt("settings_fps_cap")
                                .selected_text(if settings.fps_cap > 0 && current_label == "Custom" {
                                    format!("{}", settings.fps_cap)
                                } else {
                                    current_label.to_string()
                                })
                                .show_ui(ui, |ui| {
                                    for &(val, label) in FPS_OPTIONS {
                                        if ui.selectable_label(settings.fps_cap == val, label).clicked() {
                                            settings.fps_cap = val;
                                        }
                                    }
                                });
                            ui.small("Limit render frame rate. 0 = unlimited.");
                            ui.add_space(6.0);

                            ui.label("Graphics Backend");
                            const BACKEND_OPTIONS: &[(u8, &str)] = &[
                                (0, "Auto"),
                                (1, "Vulkan"),
                                (2, "DX12"),
                            ];
                            let backend_label = BACKEND_OPTIONS
                                .iter()
                                .find(|(v, _)| *v == settings.gpu_backend)
                                .map(|(_, l)| *l)
                                .unwrap_or("Auto");
                            egui::ComboBox::from_id_salt("settings_gpu_backend")
                                .selected_text(backend_label)
                                .show_ui(ui, |ui| {
                                    for &(val, label) in BACKEND_OPTIONS {
                                        if ui.selectable_label(settings.gpu_backend == val, label).clicked() {
                                            settings.gpu_backend = val;
                                        }
                                    }
                                });
                            ui.small("Requires restart to take effect.");
                        }
                        PauseSettingsTab::Camera => {
                            ui.add(egui::Slider::new(&mut settings.scroll_speed, 100.0..=2000.0).text("Scroll Speed"))
                                .on_hover_text("Camera pan speed for keyboard and edge scrolling.");
                            ui.small("Higher values move the camera faster.");
                            ui.add_space(6.0);

                            ui.add(egui::Slider::new(&mut settings.zoom_speed, 0.02..=0.5).text("Zoom Speed"))
                                .on_hover_text("How quickly mouse-wheel zoom changes.");
                            ui.small("Lower values are smoother; higher values are snappier.");
                            ui.add_space(6.0);

                            ui.add(egui::Slider::new(&mut settings.zoom_min, 0.01..=0.5).text("Min Zoom"))
                                .on_hover_text("Closest allowed camera zoom.");
                            ui.small("Prevents zooming in too far.");
                            ui.add_space(6.0);

                            ui.add(egui::Slider::new(&mut settings.zoom_max, 1.0..=10.0).text("Max Zoom"))
                                .on_hover_text("Farthest allowed camera zoom.");
                            ui.small("Increase to see more of the world at once.");
                            ui.add_space(6.0);

                            if settings.zoom_min > settings.zoom_max {
                                std::mem::swap(&mut settings.zoom_min, &mut settings.zoom_max);
                            }

                            ui.add(egui::Slider::new(&mut settings.lod_transition, 0.1..=2.0).text("LOD Transition"))
                                .on_hover_text("Below this zoom level, sprites render as flat rectangles.");
                            ui.small("Lower values keep detailed sprites visible longer.");
                        }
                        PauseSettingsTab::Controls => {
                            if let Some(action) = *rebinding_action {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(format!("Press a key for {}.", action.label()));
                                    if ui.button("Cancel").clicked() {
                                        *rebinding_action = None;
                                    }
                                });
                            } else {
                                ui.small("Click any key button below to rebind that action.");
                            }
                            ui.add_space(8.0);

                            for group in ControlGroup::ALL {
                                ui.strong(group.label());
                                ui.add_space(4.0);

                                for action in crate::settings::control_actions_for_group(group) {
                                    ui.horizontal(|ui| {
                                        ui.label(action.label());
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            let waiting = *rebinding_action == Some(*action);
                                            let key_label = if waiting {
                                                "Press key...".to_string()
                                            } else {
                                                settings.key_label_for_action(*action)
                                            };
                                            if ui.add_sized([140.0, 24.0], egui::Button::new(key_label)).clicked() {
                                                *rebinding_action = Some(*action);
                                            }
                                        });
                                    });
                                    ui.small(action.help_text());
                                    ui.add_space(4.0);
                                }
                                ui.add_space(8.0);
                            }

                            if ui.button("Reset Controls to Defaults").clicked() {
                                settings.reset_key_bindings();
                                *rebinding_action = None;
                                crate::settings::save_settings(settings);
                            }
                        }
                        PauseSettingsTab::Audio => {
                            ui.add(egui::Slider::new(&mut settings.music_volume, 0.0..=1.0).text("Music Volume"))
                                .on_hover_text("Master volume for background music.");
                            ui.small("Applied immediately to currently playing tracks.");
                            ui.add_space(6.0);

                            ui.add(egui::Slider::new(&mut settings.sfx_volume, 0.0..=1.0).text("SFX Volume"))
                                .on_hover_text("Master volume for UI and gameplay sound effects.");
                            ui.small("Affects new sound effects as they play.");
                            ui.add_space(6.0);
                            ui.checkbox(&mut settings.sfx_shoot_enabled, "Arrow Shoot SFX")
                                .on_hover_text("Play arrow/projectile firing sounds.");
                        }
                        PauseSettingsTab::Logs => {
                            ui.checkbox(&mut settings.log_kills, "Log Kills");
                            ui.small("Include NPC and building kills in the combat log.");
                            ui.checkbox(&mut settings.log_spawns, "Log Spawns");
                            ui.small("Show spawn events as units enter the world.");
                            ui.checkbox(&mut settings.log_raids, "Log Raids");
                            ui.small("Report raid starts and major raid activity.");
                            ui.checkbox(&mut settings.log_harvests, "Log Harvests");
                            ui.small("Show farming and resource-harvest events.");
                            ui.checkbox(&mut settings.log_levelups, "Log Level Ups");
                            ui.small("Show experience level gains.");
                            ui.checkbox(&mut settings.log_npc_activity, "Log NPC Activity");
                            ui.small("Enable task/activity messages generated by NPC behavior.");
                            ui.checkbox(&mut settings.log_ai, "Log AI Actions");
                            ui.small("Show AI-player planning and action decisions.");
                            ui.add_space(10.0);

                            ui.label("NPC Activity Scope");
                            let mode = &mut settings.npc_log_mode;
                            ui.horizontal(|ui| {
                                use crate::settings::NpcLogMode;
                                if ui.selectable_label(*mode == NpcLogMode::SelectedOnly, "Selected Only").clicked() { *mode = NpcLogMode::SelectedOnly; }
                                if ui.selectable_label(*mode == NpcLogMode::Faction, "My Faction").clicked() { *mode = NpcLogMode::Faction; }
                                if ui.selectable_label(*mode == NpcLogMode::All, "All NPCs").clicked() { *mode = NpcLogMode::All; }
                            });
                            match settings.npc_log_mode {
                                crate::settings::NpcLogMode::SelectedOnly => { ui.small("Only logs the selected NPC. Best performance."); }
                                crate::settings::NpcLogMode::Faction => { ui.small("Logs your faction's NPCs only."); }
                                crate::settings::NpcLogMode::All => { ui.small("Logs all NPCs. Highest memory use."); }
                            }
                        }
                        PauseSettingsTab::Debug => {
                            ui.checkbox(&mut settings.debug_ids, "Debug IDs");
                            ui.small("Show slot/UID for selected NPCs and buildings (for BRP queries).");
                            ui.checkbox(&mut settings.debug_all_npcs, "All NPCs in Roster");
                            ui.small("Force all NPCs visible in roster/debug lists.");
                            ui.checkbox(&mut settings.debug_readback, "GPU Readback");
                            ui.small("Enable render readback diagnostics.");
                            ui.checkbox(&mut settings.debug_combat, "Combat Logging");
                            ui.small("Verbose combat internals in the log.");
                            ui.checkbox(&mut settings.debug_spawns, "Spawn Logging");
                            ui.small("Verbose spawn diagnostics.");
                            ui.checkbox(&mut settings.debug_behavior, "Behavior Logging");
                            ui.small("Verbose behavior-tree/task diagnostics.");
                            ui.checkbox(&mut settings.debug_profiler, "System Profiler");
                            ui.small("Enable per-system timing overlays/logging.");
                            ui.checkbox(&mut settings.debug_ai_decisions, "AI Decision Logging");
                            ui.small("Log AI player action selection details.");
                            ui.checkbox(&mut settings.show_terrain_sprites, "Show Terrain Sprites");
                            ui.small("Toggle sprite-vs-plain rendering for terrain.");
                            ui.checkbox(&mut settings.show_all_faction_squad_lines, "Show All Faction Squad Lines");
                            ui.small("Draw squad path lines for all factions.");
                            ui.separator();
                            ui.horizontal(|ui| {
                                ui.label("AI Think:");
                                ui.add(egui::Slider::new(&mut settings.ai_interval, 1.0..=30.0)
                                    .step_by(0.5)
                                    .suffix("s"));
                            });
                            ui.small("How often AI towns make decisions.");
                            ui.horizontal(|ui| {
                                ui.label("NPC Think:");
                                ui.add(egui::Slider::new(&mut settings.npc_interval, 0.5..=10.0)
                                    .step_by(0.5)
                                    .suffix("s"));
                            });
                            ui.small("How often NPCs re-evaluate behavior.");
                            ui.horizontal(|ui| {
                                ui.label("Pathfind/Tick:");
                                ui.add(egui::Slider::new(&mut settings.pathfind_max_per_frame, 10..=2000)
                                    .step_by(10.0));
                            });
                            ui.small("Max path requests processed per tick. Higher reduces queueing but costs more CPU.");
                        }
                        PauseSettingsTab::LlmPlayer => {
                            ui.horizontal(|ui| {
                                ui.label("Cycle Interval:");
                                ui.add(egui::Slider::new(&mut settings.llm_interval, 5.0..=120.0)
                                    .step_by(5.0)
                                    .suffix("s"));
                            });
                            ui.small("How often the LLM player runs claude --print.");
                            ui.add_space(8.0);
                            if let Some((cmd, payload, response)) = llm_preview {
                                ui.separator();
                                egui::CollapsingHeader::new("Last Command").default_open(true).show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(cmd).monospace().size(12.0));
                                        if ui.small_button("Copy").clicked() {
                                            ui.output_mut(|o| o.commands.push(egui::OutputCommand::CopyText(cmd.to_string())));
                                        }
                                    });
                                });
                                ui.add_space(4.0);
                                egui::CollapsingHeader::new("Last Payload").default_open(false).show(ui, |ui| {
                                    if ui.small_button("Copy").clicked() {
                                        ui.output_mut(|o| o.commands.push(egui::OutputCommand::CopyText(payload.to_string())));
                                    }
                                    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                                        ui.label(egui::RichText::new(payload).monospace().size(11.0));
                                    });
                                });
                                ui.add_space(4.0);
                                egui::CollapsingHeader::new("Last Response").default_open(false).show(ui, |ui| {
                                    if response.is_empty() {
                                        ui.label(egui::RichText::new("No response yet.").weak());
                                    } else {
                                        if ui.small_button("Copy").clicked() {
                                            ui.output_mut(|o| o.commands.push(egui::OutputCommand::CopyText(response.to_string())));
                                        }
                                        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                                            ui.label(egui::RichText::new(response).monospace().size(11.0));
                                        });
                                    }
                                });
                            } else {
                                ui.label(egui::RichText::new("No LLM player active.").weak());
                            }
                        }
                        PauseSettingsTab::SaveGame => {
                            if let Some(save_name) = manual_save_name {
                                ui.label("Quick save");
                                ui.small("Writes to quicksave.json.");
                                ui.add_space(10.0);
                                if ui.button("Save Game (Quicksave)").clicked() {
                                    resp.save_requested = true;
                                }

                                ui.add_space(12.0);
                                ui.separator();
                                ui.add_space(8.0);
                                ui.label("Manual save");
                                ui.small("Creates Documents/Endless/saves/<name>.json");
                                ui.horizontal(|ui| {
                                    ui.label("Name:");
                                    ui.text_edit_singleline(save_name);
                                });
                                if ui.button("Save Game As...").clicked() {
                                    resp.save_path = crate::save::named_save_path(save_name.as_str());
                                    resp.save_requested = true;
                                }
                            }
                        }
                        PauseSettingsTab::LoadGame => {
                            if let Some(load_name) = manual_load_name {
                                ui.label("Quick load");
                                let has_quicksave = crate::save::has_quicksave();
                                if ui.add_enabled(has_quicksave, egui::Button::new("Load Game (Quicksave)")).clicked() {
                                    resp.load_requested = true;
                                }
                                if !has_quicksave {
                                    ui.small("No quicksave found yet.");
                                }

                                ui.add_space(12.0);
                                ui.separator();
                                ui.add_space(8.0);
                                ui.label("Manual load");
                                ui.small("Loads Documents/Endless/saves/<name>.json");
                                ui.horizontal(|ui| {
                                    ui.label("Name:");
                                    ui.text_edit_singleline(load_name);
                                });
                                if ui.button("Load Game By Name").clicked() {
                                    resp.load_path = crate::save::named_save_path(load_name.as_str());
                                    resp.load_requested = true;
                                }

                                ui.add_space(10.0);
                                ui.label("Existing saves");
                                for save_info in crate::save::list_saves() {
                                    let label = save_info.filename.trim_end_matches(".json").to_string();
                                    if ui.button(label).clicked() {
                                        resp.load_path = Some(save_info.path);
                                        resp.load_requested = true;
                                    }
                                }
                            }
                        }
                    }
                });
        });
    });

    ui.separator();
    ui.vertical_centered(|ui| {
        ui.add_space(4.0);
        if ui.button("Reset Defaults").clicked() {
            resp.reset_requested = true;
        }
        ui.add_space(8.0);
    });

    resp
}

/// Apply user's UI scale to all egui contexts via EguiContextSettings.
fn apply_ui_scale(
    settings: Res<crate::settings::UserSettings>,
    mut egui_settings: Query<&mut EguiContextSettings>,
) {
    if !settings.is_changed() {
        return;
    }
    for mut s in egui_settings.iter_mut() {
        s.scale_factor = settings.ui_scale;
    }
}

/// Apply global egui text sizes from user settings.
fn apply_interface_text_size(
    settings: Res<crate::settings::UserSettings>,
    mut contexts: bevy_egui::EguiContexts,
    mut initialized: Local<bool>,
    mut last_size: Local<f32>,
) -> Result {
    let size = settings.interface_text_size.clamp(10.0, 32.0);
    if *initialized && !settings.is_changed() && (*last_size - size).abs() <= f32::EPSILON {
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;
    let mut style = (*ctx.style()).clone();
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::proportional(size + 4.0),
    );
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(size));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(size));
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::monospace((size - 1.0).max(9.0)),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::proportional((size - 2.0).max(8.0)),
    );
    ctx.set_style(style);

    *initialized = true;
    *last_size = size;
    Ok(())
}

/// Register all UI systems.
pub fn register_ui(app: &mut App) {
    // Global: UI scale + overlays (all states)
    app.add_systems(Update, apply_ui_scale);
    app.add_systems(EguiPrimaryContextPass, apply_interface_text_size);
    app.add_systems(EguiPrimaryContextPass, game_hud::jukebox_ui_system);

    // Main menu (egui)
    app.add_systems(
        EguiPrimaryContextPass,
        main_menu::main_menu_system.run_if(in_state(AppState::MainMenu)),
    );

    // Game startup: load from save (if requested) then world gen (if not loaded) then tutorial init
    app.add_systems(
        OnEnter(AppState::Playing),
        (game_load_system, game_startup_system, tutorial_init_system).chain(),
    );

    // Egui panels — ordered so top bar claims height first, then side panels, then bottom.
    // Top bar → left panel → bottom panel (inspector+log) + overlay → windows → pause overlay.
    app.add_systems(
        EguiPrimaryContextPass,
        (
            game_hud::top_bar_system,
            left_panel::left_panel_system,
            left_panel::tech_tree::tech_tree_system,
            (
                game_hud::bottom_panel_system,
                game_hud::combat_log_system,
                game_hud::target_overlay_system,
                game_hud::squad_overlay_system,
                game_hud::faction_squad_overlay_system,
            ),
            build_menu::build_menu_system,
            blackjack::blackjack_window_system,
            pause_menu_system,
            game_over_system,
            game_hud::save_toast_system,
            tutorial::tutorial_ui_system,
        )
            .chain()
            .run_if(in_state(AppState::Playing)),
    );

    // Panel toggle keyboard shortcuts + ESC
    app.add_systems(
        Update,
        (ui_toggle_system, game_escape_system).run_if(in_state(AppState::Playing)),
    );

    // Escape + settings + keyboard toggles in test scenes
    app.add_systems(
        Update,
        (game_escape_system, ui_toggle_system).run_if(in_state(AppState::Running)),
    );
    // Test scene UI: bottom panel + overlays + build menu + pause
    // (top_bar, left_panel, combat_log already registered in tests/mod.rs)
    app.add_systems(
        EguiPrimaryContextPass,
        (
            game_hud::bottom_panel_system,
            game_hud::target_overlay_system,
            game_hud::squad_overlay_system,
            game_hud::faction_squad_overlay_system,
            build_menu::build_menu_system,
            blackjack::blackjack_window_system,
            pause_menu_system,
            game_hud::save_toast_system,
        )
            .run_if(in_state(AppState::Running)),
    );
    // Building placement + ghost preview in test scenes
    app.add_systems(
        Update,
        (
            build_place_click_system,
            slot_right_click_system,
            build_ghost_system,
            draw_slot_indicators,
            process_destroy_system,
        )
            .run_if(in_state(AppState::Running)),
    );

    // Building slot click detection + visual indicators + ghost
    app.add_systems(
        Update,
        (
            build_place_click_system,
            slot_right_click_system,
            build_ghost_system,
            draw_slot_indicators,
            process_destroy_system,
        )
            .run_if(in_state(AppState::Playing)),
    );

    // Cleanup when leaving Playing
    app.add_systems(OnExit(AppState::Playing), game_cleanup_system);
}

/// Keyboard shortcuts for toggling UI panels.
pub fn ui_toggle_system(
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<UserSettings>,
    mut ui_state: ResMut<UiState>,
    mut follow: ResMut<FollowSelected>,
    mut squad_state: ResMut<SquadState>,
    mut build_ctx: ResMut<BuildMenuContext>,
    mut contexts: bevy_egui::EguiContexts,
) {
    if ui_state.pause_menu_open {
        return;
    }
    // Suppress hotkeys when an egui text field (chat, etc.) has keyboard focus
    if contexts.ctx_mut().is_ok_and(|ctx| ctx.wants_keyboard_input()) {
        return;
    }

    if keys.just_pressed(settings.key_for_action(ControlAction::ToggleRoster)) {
        ui_state.toggle_left_tab(LeftPanelTab::Roster);
    }
    if keys.just_pressed(settings.key_for_action(ControlAction::ToggleBuildMenu)) {
        ui_state.build_menu_open = !ui_state.build_menu_open;
    }
    if keys.just_pressed(settings.key_for_action(ControlAction::ToggleUpgrades)) {
        ui_state.tech_tree_open = !ui_state.tech_tree_open;
    }
    if keys.just_pressed(settings.key_for_action(ControlAction::TogglePolicies)) {
        ui_state.toggle_left_tab(LeftPanelTab::Policies);
    }
    if keys.just_pressed(settings.key_for_action(ControlAction::TogglePatrols)) {
        ui_state.toggle_left_tab(LeftPanelTab::Patrols);
    }
    if keys.just_pressed(settings.key_for_action(ControlAction::ToggleSquads)) {
        ui_state.toggle_left_tab(LeftPanelTab::Squads);
    }
    if keys.just_pressed(settings.key_for_action(ControlAction::ToggleInventory)) {
        ui_state.toggle_left_tab(LeftPanelTab::Inventory);
    }
    if keys.just_pressed(settings.key_for_action(ControlAction::ToggleFactions)) {
        ui_state.toggle_left_tab(LeftPanelTab::Factions);
    }
    if keys.just_pressed(settings.key_for_action(ControlAction::ToggleBlackjack)) {
        ui_state.casino_open = !ui_state.casino_open;
    }
    if keys.just_pressed(settings.key_for_action(ControlAction::ToggleHelp)) {
        ui_state.toggle_left_tab(LeftPanelTab::Help);
    }
    if keys.just_pressed(settings.key_for_action(ControlAction::ToggleCombatLog)) {
        ui_state.combat_log_visible = !ui_state.combat_log_visible;
    }
    if keys.just_pressed(settings.key_for_action(ControlAction::ToggleFollow)) {
        follow.0 = !follow.0;
    }
    // Squad target hotkeys: defaults are 1-9,0 => squads 1-10.
    let squad_hotkey =
        settings::SQUAD_TARGET_ACTIONS
            .iter()
            .enumerate()
            .find_map(|(idx, action)| {
                keys.just_pressed(settings.key_for_action(*action))
                    .then_some(idx)
            });
    if let Some(si) = squad_hotkey {
        if si < squad_state.squads.len() {
            build_ctx.selected_build = None;
            build_ctx.clear_drag();
            ui_state.left_panel_open = true;
            ui_state.left_panel_tab = LeftPanelTab::Squads;
            squad_state.selected = si as i32;
            squad_state.placing_target = true;
        }
    }
    // Manual pan keys cancel follow mode.
    let pan_up = settings.key_for_action(ControlAction::PanUp);
    let pan_down = settings.key_for_action(ControlAction::PanDown);
    let pan_left = settings.key_for_action(ControlAction::PanLeft);
    let pan_right = settings.key_for_action(ControlAction::PanRight);
    if follow.0
        && (keys.pressed(pan_up)
            || keys.pressed(pan_left)
            || keys.pressed(pan_down)
            || keys.pressed(pan_right))
    {
        follow.0 = false;
    }
}

// ============================================================================
// GAME STARTUP
// ============================================================================

// SystemParam bundle for startup to stay under 16-param limit
#[derive(SystemParam)]
struct StartupExtra<'w> {
    ai_state: ResMut<'w, AiPlayerState>,
    combat_log: MessageWriter<'w, crate::messages::CombatLogMsg>,
    reputation: ResMut<'w, Reputation>,
    auto_upgrade: ResMut<'w, AutoUpgrade>,
    mining_policy: ResMut<'w, MiningPolicy>,
}

/// Load a saved game when entering Playing state (if load_on_enter is set).
/// Runs before game_startup_system — if it loads, startup skips world gen.
fn game_load_system(
    mut commands: Commands,
    mut save_request: ResMut<crate::save::SaveLoadRequest>,
    mut toast: ResMut<crate::save::SaveToast>,
    mut ws: crate::save::SaveWorldState,
    mut fs: crate::save::SaveFactionState,
    mut tracking: crate::save::LoadNpcTracking,
    mut entity_map: ResMut<EntityMap>,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
    combat_config: Res<crate::systems::stats::CombatConfig>,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut mining_policy: ResMut<MiningPolicy>,
) {
    if !save_request.load_on_enter {
        return;
    }
    save_request.load_on_enter = false;

    let save = match if let Some(path) = save_request.load_path.take() {
        crate::save::read_save_from(&path)
    } else {
        crate::save::read_save()
    } {
        Ok(data) => data,
        Err(e) => {
            error!("Load from menu failed: {e}");
            toast.message = format!("Load failed: {e}");
            toast.timer = 3.0;
            return;
        }
    };

    let town_count = save
        .building_data
        .get("towns")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    info!(
        "Loading save from menu: {} NPCs, {} towns",
        save.npcs.len(),
        town_count
    );

    crate::save::restore_world_from_save(
        &save,
        &mut commands,
        &mut ws,
        &mut fs,
        &mut tracking,
        &mut entity_map,
        &mut gpu_updates,
        &combat_config,
    );
    *mining_policy = MiningPolicy::default();

    // Center camera on first town
    if let Some(first_town) = ws.world_data.towns.first() {
        if let Ok(mut transform) = camera_query.single_mut() {
            transform.translation.x = first_town.center.x;
            transform.translation.y = first_town.center.y;
        }
    }

    toast.message = format!("Game Loaded ({} NPCs)", save.npcs.len());
    toast.timer = 2.0;
    info!("Menu load complete: {} NPCs restored", save.npcs.len());
}

/// Initialize the world and spawn NPCs when entering Playing state.
/// Skips world gen if load_on_enter was handled by game_load_system.
fn game_startup_system(
    mut commands: Commands,
    config: Res<WorldGenConfig>,
    mut world_state: WorldState,
    mut faction_list: ResMut<crate::resources::FactionList>,
    mut faction_stats: ResMut<FactionStats>,
    mut raider_state: ResMut<RaiderState>,
    mut game_config: ResMut<GameConfig>,
    _spawn_writer: MessageWriter<SpawnNpcMsg>,
    mut game_time: ResMut<GameTime>,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut extra: StartupExtra,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
    mut town_index: ResMut<crate::resources::TownIndex>,
) {
    // If game_load_system already populated the world, skip world gen.
    // The flag was cleared by game_load_system, but we can detect load happened
    // by checking if the world grid is already populated.
    if !world_state.grid.cells.is_empty() {
        info!("Game startup: skipping world gen (loaded from save)");
        return;
    }

    info!("Game startup: generating world...");

    // Full world setup: terrain, towns, resources, buildings, spawners, NPCs, AI players
    let ai_players = world::setup_world(
        &config,
        &mut world_state.grid,
        &mut world_state.world_data,
        &mut faction_list,
        &mut world_state.entity_slots,
        &mut world_state.entity_map,
        &mut faction_stats,
        &mut extra.reputation,
        &mut raider_state,
        &mut town_index,
        &mut commands,
        &mut gpu_updates,
    );
    // Game-specific post-setup: settings, policies, combat log
    *extra.mining_policy = MiningPolicy::default();
    let num_towns = world_state.world_data.towns.len();
    game_config.npc_counts = config
        .npc_counts
        .iter()
        .map(|(&job, &count)| (job, count as i32))
        .collect();
    *game_time = GameTime::default();
    // Load saved policies + auto-upgrade flags for player's town
    let saved = crate::settings::load_settings();
    let town_idx = world_state
        .world_data
        .towns
        .iter()
        .position(|t| t.faction == crate::constants::FACTION_PLAYER)
        .unwrap_or(0);
    if let Some(&e) = town_index.0.get(&(town_idx as i32)) {
        commands.entity(e).insert(crate::components::TownPolicy(saved.policy));
    }
    if !saved.auto_upgrades.is_empty() && town_idx < extra.auto_upgrade.flags.len() {
        let flags = &mut extra.auto_upgrade.flags[town_idx];
        *flags = crate::systems::stats::decode_auto_upgrade_flags(&saved.auto_upgrades);
    }

    // Apply personality-based policies + log AI players joining
    for player in &ai_players {
        let mut policy = player.personality.default_policies();
        if let Some(town) = world_state.world_data.towns.get(player.town_data_idx) {
            policy.mining_radius = crate::systems::ai_player::initial_mining_radius(
                &world_state.entity_map,
                town.center,
            );
        }
        if let Some(&e) = town_index.0.get(&(player.town_data_idx as i32)) {
            commands.entity(e).insert(crate::components::TownPolicy(policy));
        }
        if let Some(town) = world_state.world_data.towns.get(player.town_data_idx) {
            extra.combat_log.write(crate::messages::CombatLogMsg {
                kind: CombatEventKind::Ai,
                faction: -1,
                day: 1,
                hour: 6,
                minute: 0,
                message: format!(
                    "{} [{}] joined the game",
                    town.name,
                    player.personality.name()
                ),
                location: None,
            });
        }
    }
    extra.ai_state.players = ai_players;

    // Restore AI manager settings for player town
    if let Some(player) = extra.ai_state.players.iter_mut().find(|p| p.town_data_idx == town_idx) {
        player.active = saved.ai_manager_active;
        player.build_enabled = saved.ai_manager_build;
        player.upgrade_enabled = saved.ai_manager_upgrade;
        player.personality = match saved.ai_manager_personality {
            0 => AiPersonality::Aggressive,
            2 => AiPersonality::Economic,
            _ => AiPersonality::Balanced,
        };
        player.road_style = match saved.ai_manager_road_style {
            0 => RoadStyle::None,
            1 => RoadStyle::Cardinal,
            3 => RoadStyle::Grid5,
            _ => RoadStyle::Grid4,
        };
    }

    // Center camera on first town
    if let Some(first_town) = world_state.world_data.towns.first() {
        if let Ok(mut transform) = camera_query.single_mut() {
            transform.translation.x = first_town.center.x;
            transform.translation.y = first_town.center.y;
        }
    }

    world_state.dirty_writers.emit_all();

    info!(
        "Game startup complete: {} towns",
        num_towns,
    );
}

// ============================================================================
// TUTORIAL INIT
// ============================================================================

/// Initialize tutorial state after world gen. Runs as third step in OnEnter(Playing) chain.
/// Skips if tutorial already completed or if loading a save.
fn tutorial_init_system(
    mut tutorial: ResMut<TutorialState>,
    settings: Res<crate::settings::UserSettings>,
    world_data: Res<world::WorldData>,
    entity_map: Res<EntityMap>,
    camera_query: Query<&Transform, With<crate::render::MainCamera>>,
    game_time: Res<GameTime>,
    time: Res<Time<Real>>,
) {
    // Reset tutorial state regardless (clean slate)
    *tutorial = TutorialState::default();

    // Skip if already completed or loading a save (loaded saves have non-zero game time)
    if settings.tutorial_completed || game_time.total_seconds > 0.0 {
        tutorial.step = 255;
        return;
    }

    let player_town = world_data
        .towns
        .iter()
        .position(|t| t.faction == crate::constants::FACTION_PLAYER)
        .unwrap_or(0);

    // Snapshot initial building counts for completion checks
    let pt = player_town as u32;
    tutorial.initial_farms = entity_map.count_for_town(BuildingKind::Farm, pt);
    tutorial.initial_farmer_homes = entity_map.count_for_town(BuildingKind::FarmerHome, pt);
    tutorial.initial_waypoints = entity_map.count_for_town(BuildingKind::Waypoint, pt);
    tutorial.initial_archer_homes = entity_map.count_for_town(BuildingKind::ArcherHome, pt);
    tutorial.initial_miner_homes = entity_map.count_for_town(BuildingKind::MinerHome, pt);

    // Snapshot camera start position
    if let Ok(transform) = camera_query.single() {
        tutorial.camera_start = Vec2::new(transform.translation.x, transform.translation.y);
    }

    tutorial.start_time = time.elapsed_secs_f64();
    tutorial.step = 1;
    info!(
        "Tutorial started (farms={}, farmer_homes={}, waypoints={}, archer_homes={}, miner_homes={})",
        tutorial.initial_farms,
        tutorial.initial_farmer_homes,
        tutorial.initial_waypoints,
        tutorial.initial_archer_homes,
        tutorial.initial_miner_homes
    );
}

// ============================================================================
// GAME EXIT
// ============================================================================

/// Pause key toggles pause menu. Pause/speed controls only run when menu is closed.
fn game_escape_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut ui_state: ResMut<UiState>,
    mut game_time: ResMut<GameTime>,
    mut squad_state: ResMut<SquadState>,
    mut build_ctx: ResMut<BuildMenuContext>,
    settings: Res<UserSettings>,
    mut contexts: bevy_egui::EguiContexts,
) {
    let egui_wants_kb = contexts.ctx_mut().is_ok_and(|ctx| ctx.wants_keyboard_input());
    if keys.just_pressed(settings.key_for_action(ControlAction::PauseMenu)) {
        // Cancel box-select or squad target placement first
        if squad_state.box_selecting || squad_state.drag_start.is_some() {
            squad_state.box_selecting = false;
            squad_state.drag_start = None;
            return;
        }
        if squad_state.placing_target {
            squad_state.placing_target = false;
            return;
        }
        // Close floating windows before left panel / pause menu
        if ui_state.tech_tree_open {
            ui_state.tech_tree_open = false;
            return;
        }
        if ui_state.casino_open {
            ui_state.casino_open = false;
            return;
        }
        // Close left panel before opening/toggling pause menu.
        if ui_state.left_panel_open {
            ui_state.left_panel_open = false;
            return;
        }
        if build_ctx.selected_build.is_some() || build_ctx.destroy_mode {
            build_ctx.selected_build = None;
            build_ctx.destroy_mode = false;
            build_ctx.clear_drag();
            return;
        }
        if ui_state.build_menu_open {
            ui_state.build_menu_open = false;
        } else {
            let was_open = ui_state.pause_menu_open;
            ui_state.pause_menu_open = !ui_state.pause_menu_open;
            // Auto-pause when opening, unpause when closing
            game_time.paused = ui_state.pause_menu_open;
            if was_open && !ui_state.pause_menu_open {
                crate::settings::save_settings(&settings);
            }
        }
    }
    // Time controls only when pause menu is closed and no text field focused
    if !ui_state.pause_menu_open && !egui_wants_kb {
        if keys.just_pressed(settings.key_for_action(ControlAction::TogglePause)) {
            if game_time.is_paused() {
                if game_time.time_scale <= 0.0 {
                    game_time.time_scale = 1.0;
                }
                game_time.paused = false;
            } else {
                game_time.paused = true;
            }
        }
        if keys.just_pressed(settings.key_for_action(ControlAction::SpeedUp)) {
            if game_time.is_paused() {
                game_time.time_scale = 0.5;
                game_time.paused = false;
            } else if game_time.time_scale < 0.5 {
                game_time.time_scale = 0.5;
            } else {
                game_time.time_scale = (game_time.time_scale * 2.0).min(128.0);
            }
        }
        if keys.just_pressed(settings.key_for_action(ControlAction::SpeedDown)) {
            if game_time.time_scale <= 0.5 {
                game_time.time_scale = 0.0;
                game_time.paused = true;
            } else {
                game_time.time_scale = (game_time.time_scale / 2.0).max(0.5);
            }
        }
    }
}

/// Pause menu overlay — Resume, Settings, Exit to Main Menu.
/// Bundled locals for pause_menu_system (avoids exceeding Bevy's 16-param limit).
#[derive(Default)]
struct PauseMenuLocals {
    save_name: String,
    load_name: String,
    rebinding: Option<ControlAction>,
}

#[derive(SystemParam)]
struct PauseRuntimeConfigs<'w> {
    ai_config: ResMut<'w, crate::systems::ai_player::AiPlayerConfig>,
    npc_config: ResMut<'w, crate::resources::NpcDecisionConfig>,
    pathfind_config: ResMut<'w, crate::resources::PathfindConfig>,
    framepace: ResMut<'w, bevy_framepace::FramepaceSettings>,
}

fn pause_menu_system(
    mut contexts: bevy_egui::EguiContexts,
    keys: Res<ButtonInput<KeyCode>>,
    mut ui_state: ResMut<UiState>,
    mut game_time: ResMut<GameTime>,
    mut next_state: ResMut<NextState<AppState>>,
    mut settings: ResMut<UserSettings>,
    mut save_request: ResMut<crate::save::SaveLoadRequest>,
    mut save_game_msgs: MessageWriter<crate::save::SaveGameMsg>,
    mut load_game_msgs: MessageWriter<crate::save::LoadGameMsg>,
    mut winit_settings: ResMut<bevy::winit::WinitSettings>,
    mut windows: Query<&mut Window>,
    mut audio: ResMut<crate::resources::GameAudio>,
    mut music_sinks: Query<&mut AudioSink, With<crate::resources::MusicTrack>>,
    mut locals: Local<PauseMenuLocals>,
    mut runtime_configs: PauseRuntimeConfigs,
    llm_state: Option<Res<crate::systems::llm_player::LlmPlayerState>>,
) -> Result {
    if !ui_state.pause_menu_open || ui_state.game_over {
        locals.rebinding = None;
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;
    let mut reset_requested = false;
    if let Some(action) = locals.rebinding {
        if let Some(bound_key) = keys
            .get_just_pressed()
            .copied()
            .find(|key| settings::is_rebindable_key(*key))
        {
            settings.set_key_for_action(action, bound_key);
            locals.rebinding = None;
            crate::settings::save_settings(&settings);
        }
    }
    if locals.save_name.is_empty() {
        locals.save_name = "save1".to_string();
    }
    if locals.load_name.is_empty() {
        locals.load_name = "save1".to_string();
    }
    let prev_window_width = settings.window_width;
    let prev_window_height = settings.window_height;
    let prev_window_maximized = settings.window_maximized;
    let prev_vsync = settings.vsync;
    let prev_fullscreen = settings.fullscreen;
    let prev_bg_fps = settings.background_fps;
    let prev_fps_cap = settings.fps_cap;
    let prev_music_vol = settings.music_volume;

    // Dim background
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("pause_dim"))
        .order(egui::Order::Background)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (response, painter) = ui.allocate_painter(screen.size(), egui::Sense::hover());
            painter.rect_filled(response.rect, 0.0, egui::Color32::from_black_alpha(120));
        });

    // Centered window
    let window_resp = egui::Window::new("Paused")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .min_width(820.0)
        .min_height(520.0)
        .show(ctx, |ui| {
            let PauseMenuLocals { save_name, load_name, rebinding } = &mut *locals;
            let llm_preview = llm_state.as_ref().map(|s| (s.last_command.as_str(), s.last_payload.as_str(), s.last_response.as_str()));
            let resp = settings_panel_ui(
                ui,
                &mut settings,
                &mut ui_state.pause_settings_tab,
                rebinding,
                Some(save_name),
                Some(load_name),
                llm_preview,
            );

            if ui.button("Resume").clicked() {
                ui_state.pause_menu_open = false;
                game_time.paused = false;
                crate::settings::save_settings(&settings);
            }
            if ui.button("Exit to Main Menu").clicked() {
                ui_state.pause_menu_open = false;
                crate::settings::save_settings(&settings);
                next_state.set(AppState::MainMenu);
            }
            ui.add_space(8.0);

            resp
        });

    // Apply side effects from settings panel
    if let Some(inner) = window_resp.and_then(|r| r.inner) {
        if inner.save_requested {
            if let Some(path) = inner.save_path {
                save_request.save_path = Some(path);
            }
            save_game_msgs.write(crate::save::SaveGameMsg);
        }
        if inner.load_requested {
            if let Some(path) = inner.load_path {
                save_request.load_path = Some(path);
            }
            load_game_msgs.write(crate::save::LoadGameMsg);
        }
        reset_requested = inner.reset_requested;
    }
    if reset_requested {
        locals.rebinding = None;
        *settings = crate::settings::UserSettings::default();
        crate::settings::apply_fps_cap(settings.fps_cap, &mut runtime_configs.framepace);
        winit_settings.unfocused_mode = if settings.background_fps {
            bevy::winit::UpdateMode::Continuous
        } else {
            bevy::winit::UpdateMode::reactive_low_power(std::time::Duration::from_secs_f64(
                1.0 / 60.0,
            ))
        };
        audio.music_volume = settings.music_volume;
        audio.sfx_volume = settings.sfx_volume;
        audio.sfx_shoot_enabled = settings.sfx_shoot_enabled;
        for mut sink in &mut music_sinks {
            sink.set_volume(Volume::Linear(settings.music_volume));
        }
        crate::settings::save_settings(&settings);
    }
    // Apply video changes
    if settings.window_width != prev_window_width
        || settings.window_height != prev_window_height
        || settings.window_maximized != prev_window_maximized
        || settings.vsync != prev_vsync
        || settings.fullscreen != prev_fullscreen
    {
        settings.clamp_video_settings();
        if let Ok(mut window) = windows.single_mut() {
            crate::settings::apply_video_settings_to_window(&mut window, &settings);
        }
    }
    // Apply FPS cap change
    if settings.fps_cap != prev_fps_cap {
        crate::settings::apply_fps_cap(settings.fps_cap, &mut runtime_configs.framepace);
    }
    // Apply background FPS change
    if settings.background_fps != prev_bg_fps {
        winit_settings.unfocused_mode = if settings.background_fps {
            bevy::winit::UpdateMode::Continuous
        } else {
            bevy::winit::UpdateMode::reactive_low_power(std::time::Duration::from_secs_f64(
                1.0 / 60.0,
            ))
        };
    }
    // Apply audio volume changes
    if (settings.music_volume - prev_music_vol).abs() > f32::EPSILON {
        audio.music_volume = settings.music_volume;
        for mut sink in &mut music_sinks {
            sink.set_volume(Volume::Linear(settings.music_volume));
        }
    }
    audio.sfx_volume = settings.sfx_volume;
    audio.sfx_shoot_enabled = settings.sfx_shoot_enabled;
    // Sync think intervals + autosave to runtime configs
    runtime_configs.ai_config.decision_interval = settings.ai_interval;
    runtime_configs.npc_config.interval = settings.npc_interval;
    runtime_configs.pathfind_config.max_per_frame = settings.pathfind_max_per_frame.max(1);
    save_request.autosave_hours = settings.autosave_hours;

    Ok(())
}

// ============================================================================
// GAME OVER SCREEN
// ============================================================================

fn game_over_system(
    mut contexts: bevy_egui::EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut game_time: ResMut<GameTime>,
    mut next_state: ResMut<NextState<AppState>>,
    faction_stats: Res<FactionStats>,
    kill_stats: Res<crate::resources::KillStats>,
    town_access: crate::systemparams::TownAccess,
    world_data: Res<crate::world::WorldData>,
) -> Result {
    if !ui_state.game_over {
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;

    // Dimmed background
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("game_over_dim"))
        .order(egui::Order::Background)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (response, painter) = ui.allocate_painter(screen.size(), egui::Sense::hover());
            painter.rect_filled(response.rect, 0.0, egui::Color32::from_black_alpha(180));
        });

    // Centered window
    egui::Window::new("Game Over")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .min_width(400.0)
        .show(ctx, |ui| {
            ui.add_space(4.0);

            let player_town = world_data.towns.iter().position(|t| t.faction == crate::constants::FACTION_PLAYER).unwrap_or(0);
            let days_survived = game_time.day();
            let (alive, dead, kills) = faction_stats
                .stats
                .first()
                .map(|s| (s.alive, s.dead, s.kills))
                .unwrap_or((0, 0, 0));
            let food = town_access.food(player_town as i32);
            let gold = town_access.gold(player_town as i32);

            egui::Grid::new("game_over_stats").num_columns(2).spacing([20.0, 4.0]).show(ui, |ui| {
                ui.label("Survived:");
                ui.label(format!("{} days", days_survived));
                ui.end_row();

                ui.label("NPCs alive:");
                ui.label(format!("{}", alive));
                ui.end_row();

                ui.label("NPCs lost:");
                ui.label(format!("{}", dead));
                ui.end_row();

                ui.label("Kills:");
                ui.label(format!("{}", kills));
                ui.end_row();

                ui.label("Raiders killed:");
                ui.label(format!("{}", kill_stats.archer_kills));
                ui.end_row();

                ui.label("Villagers lost:");
                ui.label(format!("{}", kill_stats.villager_kills));
                ui.end_row();

                ui.label("Food:");
                ui.label(format!("{}", food));
                ui.end_row();

                ui.label("Gold:");
                ui.label(format!("{}", gold));
                ui.end_row();
            });

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            ui.vertical_centered(|ui| {
                if ui.button(egui::RichText::new("  Play Again  ").size(16.0)).clicked() {
                    ui_state.game_over = false;
                    next_state.set(AppState::Playing);
                }
                ui.add_space(4.0);
                if ui.button(egui::RichText::new("  Keep Watching  ").size(16.0)).clicked() {
                    ui_state.game_over = false;
                    game_time.paused = false;
                }
                ui.add_space(4.0);
                if ui.button(egui::RichText::new("  Exit to Main Menu  ").size(16.0)).clicked() {
                    ui_state.game_over = false;
                    next_state.set(AppState::MainMenu);
                }
                ui.add_space(8.0);
            });
        });

    Ok(())
}

// ============================================================================
// BUILDING SLOT CLICK SYSTEMS
// ============================================================================

/// Convert screen cursor position to world coordinates (same math as click_to_select_system).
fn screen_to_world(
    cursor_pos: Vec2,
    transform: &Transform,
    projection: &Projection,
    window: &Window,
) -> Vec2 {
    let zoom = match projection {
        Projection::Orthographic(ortho) => 1.0 / ortho.scale,
        _ => 1.0,
    };
    let position = transform.translation.truncate();
    let viewport = Vec2::new(window.width(), window.height());
    let screen_center = viewport / 2.0;
    let mouse_offset = Vec2::new(
        cursor_pos.x - screen_center.x,
        screen_center.y - cursor_pos.y,
    );
    position + mouse_offset / zoom
}

/// Bresenham-style integer line over town-grid slots, inclusive of start/end.
fn slots_on_line(start: (usize, usize), end: (usize, usize)) -> Vec<(usize, usize)> {
    let (mut c0, mut r0) = (start.0 as i32, start.1 as i32);
    let (c1, r1) = (end.0 as i32, end.1 as i32);
    let dc = (c1 - c0).abs();
    let dr = (r1 - r0).abs();
    let sc: i32 = if c0 < c1 { 1 } else if c0 > c1 { -1 } else { 0 };
    let sr: i32 = if r0 < r1 { 1 } else if r0 > r1 { -1 } else { 0 };
    let mut err = dc - dr;

    let mut out = Vec::new();
    loop {
        out.push((c0 as usize, r0 as usize));
        if c0 == c1 && r0 == r1 {
            break;
        }
        let e2 = 2 * err;
        if e2 > -dr {
            err -= dr;
            c0 += sc;
        }
        if e2 < dc {
            err += dc;
            r0 += sr;
        }
    }
    out
}

/// Right-click cancels active build placement.
fn slot_right_click_system(
    mouse: Res<ButtonInput<MouseButton>>,
    mut build_ctx: ResMut<BuildMenuContext>,
) {
    if !mouse.just_pressed(MouseButton::Right) {
        return;
    }
    build_ctx.selected_build = None;
    build_ctx.destroy_mode = false;
    build_ctx.clear_drag();
}

/// Left-click places the currently selected building into any valid slot in buildable area.
fn build_place_click_system(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_query: Query<(&Transform, &Projection), With<crate::render::MainCamera>>,
    mut egui_contexts: bevy_egui::EguiContexts,
    mut build_ctx: ResMut<BuildMenuContext>,
    mut world_state: WorldState,
    mut town_access: crate::systemparams::TownAccess,
    mut combat_log: MessageWriter<crate::messages::CombatLogMsg>,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
    mut damage_writer: MessageWriter<crate::messages::DamageMsg>,
    game_time: Res<GameTime>,
    _difficulty: Res<Difficulty>,
    mut toast: ResMut<crate::save::SaveToast>,
) {
    if build_ctx.selected_build.is_none() && !build_ctx.destroy_mode {
        return;
    }
    let just_pressed = mouse.just_pressed(MouseButton::Left);
    let pressed = mouse.pressed(MouseButton::Left);
    let just_released = mouse.just_released(MouseButton::Left);
    if !just_pressed && !pressed && !just_released {
        return;
    }

    if let Ok(ctx) = egui_contexts.ctx_mut() {
        if ctx.wants_pointer_input() || ctx.is_pointer_over_area() {
            if just_released {
                build_ctx.clear_drag();
            }
            return;
        }
    }

    let Some(town_data_idx) = build_ctx.town_data_idx else {
        return;
    };
    let Some(town) = world_state.world_data.towns.get(town_data_idx) else {
        return;
    };
    let center = town.center;
    let town_name = town.name.clone();
    // Track food locally; write back to ECS at function exits that modify it
    let mut food_val = town_access.food(town_data_idx as i32);
    // Macro to write back food changes to ECS
    macro_rules! sync_food {
        () => {
            if let Some(mut f) = town_access.food_mut(town_data_idx as i32) {
                f.0 = food_val;
            }
        };
    }
    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };
    let Ok((transform, projection)) = camera_query.single() else {
        return;
    };
    let world_pos = screen_to_world(cursor_pos, transform, projection, window);
    let (gc, gr) = world_state.grid.world_to_grid(world_pos);
    let slot_pos = world_state.grid.grid_to_world(gc, gr);
    build_ctx.hover_world_pos = slot_pos;

    // Destroy mode: remove building at clicked cell (player-owned only)
    if build_ctx.destroy_mode {
        if !just_pressed {
            return;
        }
        build_ctx.clear_drag();
        let (building_gpu_slot, bld_kind) = {
            let inst = match world_state.entity_map.get_at_grid(gc as i32, gr as i32) {
                Some(inst)
                    if !matches!(
                        inst.kind,
                        world::BuildingKind::Fountain | world::BuildingKind::GoldMine
                    ) && world_state
                        .world_data
                        .towns
                        .get(inst.town_idx as usize)
                        .is_some_and(|t| t.faction == crate::constants::FACTION_PLAYER) =>
                {
                    inst
                }
                _ => return,
            };
            (inst.slot, inst.kind)
        };

        // Send lethal damage so death_system handles despawn (single Dead writer)
        let Some(&entity) = world_state.entity_map.entities.get(&building_gpu_slot) else {
            return;
        };
        damage_writer.write(crate::messages::DamageMsg {
            target: entity,
            amount: f32::MAX,
            attacker: -1,
            attacker_faction: 0,
        });
        let _ = world_state.destroy_building(
            &mut combat_log,
            &game_time,
            gc,
            gr,
            &format!("Destroyed building at ({},{}) in {}", gc, gr, town_name),
            &mut gpu_updates,
        );
        world_state.dirty_writers.mark_building_changed(bld_kind);
        return;
    }

    let kind = build_ctx.selected_build.expect("checked above");

    // Waypoint: single-click placement
    if kind == BuildingKind::Waypoint {
        if !just_pressed {
            return;
        }
        build_ctx.clear_drag();
        let cost = crate::constants::building_cost(kind);
        match world_state.place_building(
            &mut food_val, kind, town_data_idx, world_pos, cost,
            &mut gpu_updates, &mut commands,
        ) {
            Ok(()) => {
                let label = crate::constants::building_def(kind).label;
                combat_log.write(crate::messages::CombatLogMsg {
                    kind: CombatEventKind::Harvest,
                    faction: 0,
                    day: game_time.day(),
                    hour: game_time.hour(),
                    minute: game_time.minute(),
                    message: format!("Built {} in {}", label.to_lowercase(), town_name),
                    location: None,
                });
            }
            Err(reason) => {
                toast.message = reason.to_string();
                toast.timer = 2.0;
            }
        }
        sync_food!();
        return;
    }

    // Road: drag-line placement on world grid (reuses drag_start_slot/slots_on_line)
    if kind.is_road() {
        if just_pressed {
            build_ctx.drag_start_slot = Some((gc, gr));
            build_ctx.drag_current_slot = Some((gc, gr));
        } else if pressed && build_ctx.drag_start_slot.is_some() {
            build_ctx.drag_current_slot = Some((gc, gr));
        }
        if !just_released {
            return;
        }

        let start = build_ctx
            .drag_start_slot
            .take()
            .unwrap_or((gc, gr));
        let end = build_ctx
            .drag_current_slot
            .take()
            .unwrap_or((gc, gr));
        let cost = crate::constants::building_cost(kind);
        let mut placed = 0usize;
        let mut upgraded = 0usize;
        let mut last_err: Option<&str> = None;
        for (sc, sr) in slots_on_line(start, end) {
            let cell_pos = world_state.grid.grid_to_world(sc, sr);
            match world_state.place_building(
                &mut food_val, kind, town_data_idx, cell_pos, cost,
                &mut gpu_updates, &mut commands,
            ) {
                Ok(()) => { placed += 1; }
                Err("cell already has a building") => {
                    // Try upgrading existing road
                    match world_state.upgrade_road(
                        &mut food_val, kind, town_data_idx, cell_pos,
                        &mut gpu_updates, &mut commands,
                    ) {
                        Ok(()) => { upgraded += 1; }
                        Err(e) => { last_err = Some(e); }
                    }
                }
                Err(e) => { last_err = Some(e); }
            }
        }
        if placed == 0 && upgraded == 0 {
            if let Some(reason) = last_err {
                toast.message = reason.to_string();
                toast.timer = 2.0;
            }
        }
        if placed > 0 || upgraded > 0 {
            let label = crate::constants::building_def(kind).label;
            let mut parts = Vec::new();
            if placed > 0 {
                parts.push(format!("built {placed}"));
            }
            if upgraded > 0 {
                parts.push(format!("upgraded {upgraded}"));
            }
            let msg = format!("{} {} in {}", parts.join(", "), label.to_lowercase(), town_name);
            combat_log.write(crate::messages::CombatLogMsg {
                kind: CombatEventKind::Harvest,
                faction: 0,
                day: game_time.day(),
                hour: game_time.hour(),
                minute: game_time.minute(),
                message: msg,
                location: None,
            });
        }
        sync_food!();
        return;
    }

    // World-grid build mode: supports single-click and click-drag line placement.
    let label = crate::constants::building_def(kind).label;
    let (cc, cr) = world_state.grid.world_to_grid(center);

    let mut last_place_err: Option<&str> = None;
    let mut try_place_at_slot = |slot_col: usize, slot_row: usize, err_out: &mut Option<&str>| -> bool {
        if !world_state.grid.can_town_build(slot_col, slot_row, town_data_idx as u16) {
            *err_out = Some("outside buildable area");
            return false;
        }
        if slot_col == cc && slot_row == cr {
            *err_out = Some("cannot build on town center");
            return false;
        }
        let pos = world_state.grid.grid_to_world(slot_col, slot_row);
        let cost = crate::constants::building_cost(kind);

        match world_state.place_building(
            &mut food_val, kind, town_data_idx, pos, cost,
            &mut gpu_updates, &mut commands,
        ) {
            Ok(()) => true,
            Err(e) => { *err_out = Some(e); false }
        }
    };

    if just_pressed {
        build_ctx.drag_start_slot = Some((gc, gr));
        build_ctx.drag_current_slot = Some((gc, gr));
    } else if pressed && build_ctx.drag_start_slot.is_some() {
        build_ctx.drag_current_slot = Some((gc, gr));
    }

    if !just_released {
        return;
    }

    let start = build_ctx.drag_start_slot.take().unwrap_or((gc, gr));
    let end = build_ctx.drag_current_slot.take().unwrap_or((gc, gr));
    let mut placed = 0usize;
    let mut first_placed: Option<(usize, usize)> = None;
    for (sc, sr) in slots_on_line(start, end) {
        if try_place_at_slot(sc, sr, &mut last_place_err) {
            if first_placed.is_none() {
                first_placed = Some((sc, sr));
            }
            placed += 1;
        }
    }
    if placed == 0 {
        if let Some(reason) = last_place_err {
            toast.message = reason.to_string();
            toast.timer = 2.0;
        }
        return;
    }

    if placed == 1 {
        let (pc, pr) = first_placed.unwrap_or((gc, gr));
        combat_log.write(crate::messages::CombatLogMsg {
            kind: CombatEventKind::Harvest,
            faction: 0,
            day: game_time.day(),
            hour: game_time.hour(),
            minute: game_time.minute(),
            message: format!("Built {} at ({},{}) in {}", label, pr, pc, town_name),
            location: None,
        });
    } else {
        combat_log.write(crate::messages::CombatLogMsg {
            kind: CombatEventKind::Harvest,
            faction: 0,
            day: game_time.day(),
            hour: game_time.hour(),
            minute: game_time.minute(),
            message: format!("Built {} {}s in {} (drag line)", placed, label, town_name),
            location: None,
        });
    }
    sync_food!();
}

/// Marker component for slot indicator sprite entities.
#[derive(Component)]
pub(crate) struct SlotIndicator;

/// Cached mesh/material handles for slot indicators (lazy-initialized).
#[derive(Default)]
struct SlotIndicatorCache {
    mesh: Option<Handle<Mesh>>,
    base_mat: Option<Handle<ColorMaterial>>,
    road_mat: Option<Handle<ColorMaterial>>,
    chain_mat: Option<Handle<ColorMaterial>>,
}

fn road_ui_cell_allowed(cell: Option<&world::WorldCell>) -> bool {
    cell.is_some_and(|c| {
        !matches!(
            c.terrain,
            world::Biome::Forest | world::Biome::Water | world::Biome::Rock
        )
    })
}

/// Marker for the build ghost preview sprite.
#[derive(Component)]
pub(crate) struct BuildGhost;

/// Marker for additional ghost sprites used to preview drag placement lines.
#[derive(Component)]
struct BuildGhostTrail;

/// Update or spawn/despawn the ghost sprite to preview building placement.
fn build_ghost_system(
    mut commands: Commands,
    windows: Query<&Window>,
    camera_query: Query<(&Transform, &Projection), With<crate::render::MainCamera>>,
    mut egui_contexts: bevy_egui::EguiContexts,
    mut build_ctx: ResMut<BuildMenuContext>,
    grid: Res<world::WorldGrid>,
    world_data: Res<world::WorldData>,
    town_access: crate::systemparams::TownAccess,
    entity_map: Res<EntityMap>,
    mut ghost_query: Query<
        (Entity, &mut Transform, &mut Sprite),
        (
            With<BuildGhost>,
            Without<BuildGhostTrail>,
            Without<crate::render::MainCamera>,
        ),
    >,
    trail_query: Query<Entity, With<BuildGhostTrail>>,
) {
    let has_selection = build_ctx.selected_build.is_some() || build_ctx.destroy_mode;

    // Despawn ghost if no selection
    if !has_selection {
        build_ctx.show_cursor_hint = true;
        for (entity, _, _) in ghost_query.iter() {
            commands.entity(entity).despawn();
        }
        for entity in trail_query.iter() {
            commands.entity(entity).despawn();
        }
        return;
    }

    // Get cursor world position
    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };

    // Don't show ghost when hovering UI
    if let Ok(ctx) = egui_contexts.ctx_mut() {
        if ctx.is_pointer_over_area() {
            build_ctx.show_cursor_hint = true;
            for (_, _, mut sprite) in ghost_query.iter_mut() {
                sprite.color = Color::NONE;
            }
            for entity in trail_query.iter() {
                commands.entity(entity).despawn();
            }
            return;
        }
    }

    let Ok((cam_transform, projection)) = camera_query.single() else {
        return;
    };
    let world_pos = screen_to_world(cursor_pos, cam_transform, projection, window);

    // Destroy mode: snap to town grid, show red ghost over destructible buildings
    if build_ctx.destroy_mode {
        for entity in trail_query.iter() {
            commands.entity(entity).despawn();
        }
        let Some(town_data_idx) = build_ctx.town_data_idx else {
            return;
        };
        if town_data_idx >= world_data.towns.len() {
            return;
        }
        let (gc, gr) = grid.world_to_grid(world_pos);
        let slot_pos = grid.grid_to_world(gc, gr);
        build_ctx.hover_world_pos = slot_pos;
        let grid_inst = entity_map.get_at_grid(gc as i32, gr as i32);
        let has_building = grid_inst.is_some();
        let is_fountain = grid_inst
            .map(|inst| {
                matches!(
                    inst.kind,
                    world::BuildingKind::Fountain | world::BuildingKind::GoldMine
                )
            })
            .unwrap_or(false);
        let valid = has_building && !is_fountain;
        build_ctx.show_cursor_hint = true;
        let color = if valid {
            Color::srgba(0.8, 0.2, 0.2, 0.6)
        } else {
            Color::NONE
        };
        let snapped = grid.grid_to_world(gc, gr);
        let ghost_z = 0.5;
        if let Some((_, mut transform, mut sprite)) = ghost_query.iter_mut().next() {
            transform.translation = Vec3::new(snapped.x, snapped.y, ghost_z);
            sprite.color = color;
            sprite.image = Handle::default();
        } else {
            commands.spawn((
                Sprite {
                    color,
                    image: Handle::default(),
                    custom_size: Some(Vec2::splat(TOWN_GRID_SPACING)),
                    ..default()
                },
                Transform::from_xyz(snapped.x, snapped.y, ghost_z),
                BuildGhost,
            ));
        }
        return;
    }

    let kind = build_ctx.selected_build.expect("checked above");

    // Road: world-grid ghost with drag trail preview (mirrors town-grid trail pattern)
    if kind.is_road() {
        let (gc, gr) = grid.world_to_grid(world_pos);
        let snapped = grid.grid_to_world(gc, gr);
        build_ctx.hover_world_pos = snapped;

        let path = match build_ctx.drag_start_slot {
            Some(start) => {
                build_ctx.drag_current_slot = Some((gc, gr));
                slots_on_line(start, (gc, gr))
            }
            None => vec![(gc, gr)],
        };

        let cost = crate::constants::building_cost(kind);
        let town_idx = build_ctx.town_data_idx.unwrap_or(0);
        let mut budget = town_access.food(town_idx as i32);
        let ghost_image = build_ctx
            .ghost_sprites
            .get(&kind)
            .cloned()
            .unwrap_or_default();
        let ghost_z = 0.5;

        // Despawn old trail, rebuild (same pattern as town-grid lines 1118-1142)
        for entity in trail_query.iter() {
            commands.entity(entity).despawn();
        }

        let mut cursor_valid = false;
        for (idx, &(sc, sr)) in path.iter().enumerate() {
            let cell_world = grid.grid_to_world(sc, sr);
            let cell = grid.cell(sc, sr);
            let empty = !entity_map.has_building_at(sc as i32, sr as i32);
            let buildable_terrain = road_ui_cell_allowed(cell);
            let valid = empty && buildable_terrain && budget >= cost;
            if valid {
                budget -= cost;
            }

            if idx == path.len() - 1 {
                cursor_valid = valid;
            } else {
                let color = if valid {
                    Color::srgba(1.0, 1.0, 1.0, 0.45)
                } else {
                    Color::srgba(0.8, 0.2, 0.2, 0.35)
                };
                commands.spawn((
                    Sprite {
                        color,
                        image: ghost_image.clone(),
                        custom_size: Some(Vec2::splat(TOWN_GRID_SPACING)),
                        ..default()
                    },
                    Transform::from_xyz(cell_world.x, cell_world.y, ghost_z),
                    BuildGhost,
                    BuildGhostTrail,
                ));
            }
        }

        build_ctx.show_cursor_hint = !cursor_valid;
        let color = if cursor_valid {
            Color::srgba(1.0, 1.0, 1.0, 0.7)
        } else {
            Color::srgba(0.8, 0.2, 0.2, 0.5)
        };
        if let Some((_, mut transform, mut sprite)) = ghost_query.iter_mut().next() {
            transform.translation = Vec3::new(snapped.x, snapped.y, ghost_z);
            sprite.color = color;
            sprite.image = ghost_image;
        } else {
            commands.spawn((
                Sprite {
                    color,
                    image: ghost_image,
                    custom_size: Some(Vec2::splat(TOWN_GRID_SPACING)),
                    ..default()
                },
                Transform::from_xyz(snapped.x, snapped.y, ghost_z),
                BuildGhost,
            ));
        }
        return;
    }

    // Waypoint: snap to world grid (wilderness placement, single ghost)
    if kind == BuildingKind::Waypoint {
        for entity in trail_query.iter() {
            commands.entity(entity).despawn();
        }
        let (gc, gr) = grid.world_to_grid(world_pos);
        let snapped = grid.grid_to_world(gc, gr);
        build_ctx.hover_world_pos = snapped;
        let cell = grid.cell(gc, gr);
        let empty = !entity_map.has_building_at(gc as i32, gr as i32);
        let buildable_terrain = cell
            .map(|c| !matches!(c.terrain, world::Biome::Water | world::Biome::Rock))
            .unwrap_or(false);
        let valid = empty && buildable_terrain;
        build_ctx.show_cursor_hint = !valid;

        let color = if valid {
            Color::srgba(1.0, 1.0, 1.0, 0.7)
        } else {
            Color::srgba(0.8, 0.2, 0.2, 0.5)
        };
        let ghost_image = build_ctx
            .ghost_sprites
            .get(&kind)
            .cloned()
            .unwrap_or_default();
        let ghost_z = 0.5;

        if let Some((_, mut transform, mut sprite)) = ghost_query.iter_mut().next() {
            transform.translation = Vec3::new(snapped.x, snapped.y, ghost_z);
            sprite.color = color;
            sprite.image = ghost_image;
        } else {
            commands.spawn((
                Sprite {
                    color,
                    image: ghost_image,
                    custom_size: Some(Vec2::splat(TOWN_GRID_SPACING)),
                    ..default()
                },
                Transform::from_xyz(snapped.x, snapped.y, ghost_z),
                BuildGhost,
            ));
        }
        return;
    }

    // Snap to town grid (non-waypoint buildings)
    let Some(town_data_idx) = build_ctx.town_data_idx else {
        return;
    };
    let Some(town) = world_data.towns.get(town_data_idx) else {
        return;
    };
    let center = town.center;
    let (gc, gr) = grid.world_to_grid(world_pos);
    let slot_pos = grid.grid_to_world(gc, gr);
    build_ctx.hover_world_pos = slot_pos;

    // Determine validity
    let has_building = entity_map.has_building_at(gc as i32, gr as i32);
    let in_bounds = grid.can_town_build(gc, gr, town_data_idx as u16);
    let (cc, cr) = grid.world_to_grid(center);
    let is_center = gc == cc && gr == cr;

    let mut drag_preview: Vec<(usize, usize, bool, bool)> = Vec::new();
    {
        let path = match (build_ctx.drag_start_slot, build_ctx.drag_current_slot) {
            (Some(start), Some(end)) => slots_on_line(start, end),
            _ => vec![(gc, gr)],
        };
        let cost = crate::constants::building_cost(kind);
        let mut budget = town_access.food(town_data_idx as i32);

        for (slot_col, slot_row) in path {
            let visible_slot = grid.can_town_build(slot_col, slot_row, town_data_idx as u16)
                && !(slot_col == cc && slot_row == cr);
            if !visible_slot {
                drag_preview.push((slot_col, slot_row, false, false));
                continue;
            }

            let slot_empty = !entity_map.has_building_at(slot_col as i32, slot_row as i32);
            let can_pay = budget >= cost;
            let slot_valid = slot_empty && can_pay;
            if slot_valid {
                budget -= cost;
            }
            drag_preview.push((slot_col, slot_row, slot_valid, true));
        }
    }

    // Build: valid on empty buildable non-center slots, including drag budget preview.
    let (valid, visible) = {
        let current = drag_preview
            .iter()
            .find(|(sc, sr, _, _)| *sc == gc && *sr == gr)
            .copied();
        if let Some((_, _, v, vis)) = current {
            (v, vis)
        } else {
            (
                in_bounds && !is_center && !has_building,
                in_bounds && !is_center,
            )
        }
    };
    // Hide mouse-follow sprite when we're snapped to a valid build slot.
    build_ctx.show_cursor_hint = !valid;

    let color = if !visible {
        Color::NONE
    } else if valid {
        Color::srgba(1.0, 1.0, 1.0, 0.7)
    } else {
        Color::srgba(0.8, 0.2, 0.2, 0.5)
    };

    let snapped = grid.grid_to_world(gc, gr);
    let ghost_z = 0.5;
    let ghost_image = build_ctx
        .ghost_sprites
        .get(&kind)
        .cloned()
        .unwrap_or_default();

    // Rebuild drag trail each frame (all slots except the cursor slot).
    for entity in trail_query.iter() {
        commands.entity(entity).despawn();
    }
    if drag_preview.len() > 1 {
        for (slot_col, slot_row, slot_valid, slot_visible) in drag_preview.iter().copied() {
            if (slot_col == gc && slot_row == gr) || !slot_visible {
                continue;
            }
            let snapped_slot = grid.grid_to_world(slot_col, slot_row);
            let slot_color = if slot_valid {
                Color::srgba(1.0, 1.0, 1.0, 0.45)
            } else {
                Color::srgba(0.8, 0.2, 0.2, 0.35)
            };
            commands.spawn((
                Sprite {
                    color: slot_color,
                    image: ghost_image.clone(),
                    custom_size: Some(Vec2::splat(TOWN_GRID_SPACING)),
                    ..default()
                },
                Transform::from_xyz(snapped_slot.x, snapped_slot.y, ghost_z),
                BuildGhost,
                BuildGhostTrail,
            ));
        }
    }

    if let Some((_, mut transform, mut sprite)) = ghost_query.iter_mut().next() {
        transform.translation = Vec3::new(snapped.x, snapped.y, ghost_z);
        sprite.color = color;
        sprite.image = ghost_image;
    } else {
        // Spawn ghost
        commands.spawn((
            Sprite {
                color,
                image: ghost_image,
                custom_size: Some(Vec2::splat(TOWN_GRID_SPACING)),
                ..default()
            },
            Transform::from_xyz(snapped.x, snapped.y, ghost_z),
            BuildGhost,
        ));
    }
}

/// Spawn/rebuild slot indicator sprites when the town grid or world grid changes.
/// Renders slot indicators using Mesh2d+ColorMaterial (same pipeline as TilemapChunk).
/// Color-coded: green = base area, blue = road-expanded, yellow = road chain (road selected only).
fn draw_slot_indicators(
    mut commands: Commands,
    existing: Query<Entity, With<SlotIndicator>>,
    world_data: Res<world::WorldData>,
    grid: Res<world::WorldGrid>,
    entity_map: Res<EntityMap>,
    build_ctx: Res<BuildMenuContext>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut color_materials: ResMut<Assets<ColorMaterial>>,
    mut cache: Local<SlotIndicatorCache>,
    town_access: crate::systemparams::TownAccess,
) {
    // Only rebuild when grid state changes or build selection changes
    if !grid.is_changed() && !build_ctx.is_changed() {
        return;
    }

    // Despawn old indicators
    for entity in existing.iter() {
        commands.entity(entity).despawn();
    }

    // Only show indicators when a build type is selected (not destroy mode)
    if build_ctx.selected_build.is_none() || build_ctx.destroy_mode {
        return;
    }

    // Player's town is always index 0
    let town_data_idx = 0usize;
    let Some(town) = world_data.towns.get(town_data_idx) else {
        return;
    };
    let center = town.center;
    let area_level = town_access.area_level(town_data_idx as i32);

    let cell = crate::constants::TOWN_GRID_SPACING;
    // Keep indicators just above terrain (-1.0) so they remain visible when terrain sprites are enabled.
    let indicator_z = -0.2;

    // Lazy-init cached mesh and material handles
    let mesh_h = cache.mesh.get_or_insert_with(|| {
        meshes.add(Rectangle::from_size(Vec2::splat(cell)))
    }).clone();
    let base_h = cache.base_mat.get_or_insert_with(|| {
        color_materials.add(ColorMaterial::from(Color::srgba(0.2, 0.9, 0.2, 0.32)))
    }).clone();
    // Keep all buildable indicators visually consistent: green for any valid placement zone.
    let road_h = cache.road_mat.get_or_insert_with(|| {
        color_materials.add(ColorMaterial::from(Color::srgba(0.2, 0.9, 0.2, 0.32)))
    }).clone();
    let chain_h = cache.chain_mat.get_or_insert_with(|| {
        color_materials.add(ColorMaterial::from(Color::srgba(0.2, 0.9, 0.2, 0.28)))
    }).clone();

    let is_road_selected = build_ctx
        .selected_build
        .is_some_and(|k| k.is_road());

    // Collect all empty buildable slots (base + road-expanded)
    let slots = world::empty_slots(
        town_data_idx,
        center,
        &grid,
        &entity_map,
    );

    let (base_min_c, base_max_c, base_min_r, base_max_r) = world::build_bounds(area_level, center, &grid);
    let mut seen = std::collections::HashSet::new();
    for &(col, row) in &slots {
        if is_road_selected && !road_ui_cell_allowed(grid.cell(col, row)) {
            continue;
        }
        seen.insert((col, row));
        let is_base = col >= base_min_c && col <= base_max_c && row >= base_min_r && row <= base_max_r;
        let mat = if is_base { &base_h } else { &road_h };

        let slot_pos = grid.grid_to_world(col, row);

        commands.spawn((
            Mesh2d(mesh_h.clone()),
            MeshMaterial2d(mat.clone()),
            Transform::from_xyz(slot_pos.x, slot_pos.y, indicator_z),
            SlotIndicator,
        ));
    }

    // Road chain preview: show where roads can extend (1 tile beyond current buildable area)
    if is_road_selected {
        // Collect chain candidates from all existing roads + base boundary
        let ti = town_data_idx as u32;
        let mut chain_candidates = Vec::new();

        // 1 tile around each existing road
        for kind in [world::BuildingKind::Road, world::BuildingKind::StoneRoad, world::BuildingKind::MetalRoad] {
            for inst in entity_map.iter_kind_for_town(kind, ti) {
                let (rc, rr) = grid.world_to_grid(inst.position);
                for dr in -1i32..=1 {
                    for dc in -1i32..=1 {
                        if dr == 0 && dc == 0 { continue; }
                        let nc = rc as i32 + dc;
                        let nr = rr as i32 + dr;
                        if nc >= 0 && nr >= 0 {
                            chain_candidates.push((nc as usize, nr as usize));
                        }
                    }
                }
            }
        }

        // 1 tile around base boundary
        for row in base_min_r.saturating_sub(1)..=(base_max_r + 1) {
            for col in base_min_c.saturating_sub(1)..=(base_max_c + 1) {
                if col >= base_min_c && col <= base_max_c && row >= base_min_r && row <= base_max_r {
                    continue; // skip interior
                }
                chain_candidates.push((col, row));
            }
        }

        let (cc, cr) = grid.world_to_grid(center);
        for (col, row) in chain_candidates {
            if col == cc && row == cr { continue; }
            if seen.contains(&(col, row)) { continue; }
            if !seen.insert((col, row)) { continue; }

            // Must be a valid world cell, empty, and not forbidden terrain for roads
            let cell = grid.cell(col, row);
            if cell.is_none() || entity_map.has_building_at(col as i32, row as i32) {
                continue;
            }
            if !road_ui_cell_allowed(cell) {
                continue;
            }

            let slot_pos = grid.grid_to_world(col, row);
            commands.spawn((
                Mesh2d(mesh_h.clone()),
                MeshMaterial2d(chain_h.clone()),
                Transform::from_xyz(slot_pos.x, slot_pos.y, indicator_z),
                SlotIndicator,
            ));
        }
    }
}

/// Process destroy requests from the building inspector.
fn process_destroy_system(
    mut request: MessageReader<crate::messages::DestroyBuildingMsg>,
    mut world_state: WorldState,
    mut combat_log: MessageWriter<crate::messages::CombatLogMsg>,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
    mut damage_writer: MessageWriter<crate::messages::DamageMsg>,
    game_time: Res<GameTime>,
    mut selected_building: ResMut<SelectedBuilding>,
) {
    for msg in request.read() {
        let (col, row) = (msg.0, msg.1);

        let (building_gpu_slot, bld_kind, town_idx) = {
            let inst = match world_state.entity_map.get_at_grid(col as i32, row as i32) {
                Some(inst)
                    if !matches!(
                        inst.kind,
                        world::BuildingKind::Fountain | world::BuildingKind::GoldMine
                    ) && world_state
                        .world_data
                        .towns
                        .get(inst.town_idx as usize)
                        .is_some_and(|t| t.faction == crate::constants::FACTION_PLAYER) =>
                {
                    inst
                }
                _ => continue,
            };
            (inst.slot, inst.kind, inst.town_idx as usize)
        };
        let town_name = world_state
            .world_data
            .towns
            .get(town_idx)
            .map(|t| t.name.clone())
            .unwrap_or_default();

        // Send lethal damage so death_system handles despawn (single Dead writer)
        let Some(&entity) = world_state.entity_map.entities.get(&building_gpu_slot) else {
            return;
        };
        damage_writer.write(crate::messages::DamageMsg {
            target: entity,
            amount: f32::MAX,
            attacker: -1,
            attacker_faction: 0,
        });

        if world_state
            .destroy_building(
                &mut combat_log,
                &game_time,
                col,
                row,
                &format!("Destroyed building in {}", town_name),
                &mut gpu_updates,
            )
            .is_ok()
        {
            selected_building.active = false;
            world_state.dirty_writers.mark_building_changed(bld_kind);
        }
    }
}

// ============================================================================
// GAME CLEANUP — single shared cleanup for both Playing and Running (test) states
// ============================================================================

// SystemParam bundles to keep cleanup under 16-param limit
#[derive(SystemParam)]
pub(crate) struct CleanupWorld<'w> {
    world_state: WorldState<'w>,
    faction_stats: ResMut<'w, FactionStats>,
    gpu_state: ResMut<'w, GpuReadState>,
    render_config: ResMut<'w, crate::gpu::RenderFrameConfig>,
    npc_gpu_state: ResMut<'w, crate::gpu::EntityGpuState>,
    npc_visual_upload: ResMut<'w, crate::gpu::NpcVisualUpload>,
    proj_buffer_writes: ResMut<'w, crate::gpu::ProjBufferWrites>,
    game_time: ResMut<'w, GameTime>,
    tilemap_spawned: ResMut<'w, crate::render::TilemapSpawned>,
    build_menu_ctx: ResMut<'w, BuildMenuContext>,
    ai_state: ResMut<'w, AiPlayerState>,
    town_index: ResMut<'w, crate::resources::TownIndex>,
}

#[derive(SystemParam)]
pub(crate) struct CleanupDebug<'w> {
    combat_debug: ResMut<'w, CombatDebug>,
    health_debug: ResMut<'w, HealthDebug>,
    kill_stats: ResMut<'w, KillStats>,
    raider_state: ResMut<'w, RaiderState>,
    pop_stats: ResMut<'w, PopulationStats>,
    debug_flags: ResMut<'w, DebugFlags>,
}

#[derive(SystemParam)]
pub(crate) struct CleanupGameplay<'w> {
    auto_upgrade: ResMut<'w, AutoUpgrade>,
    npc_logs: ResMut<'w, NpcLogCache>,
    migration: ResMut<'w, MigrationState>,
    tower_state: ResMut<'w, TowerState>,
    selected_npc: ResMut<'w, SelectedNpc>,
    selected_building: ResMut<'w, SelectedBuilding>,
    follow: ResMut<'w, FollowSelected>,
    proj_slots: ResMut<'w, ProjSlotAllocator>,
    mining_policy: ResMut<'w, MiningPolicy>,
}

#[derive(SystemParam)]
pub(crate) struct CleanupUi<'w> {
    combat_log: ResMut<'w, CombatLog>,
    ui_state: ResMut<'w, UiState>,
    squad_state: ResMut<'w, SquadState>,
    building_hp_render: ResMut<'w, BuildingHpRender>,
    healing_cache: ResMut<'w, HealingZoneCache>,
    active_healing: ResMut<'w, ActiveHealingSlots>,
    endless: ResMut<'w, EndlessMode>,
    merchant_inv: ResMut<'w, MerchantInventory>,
    next_loot_id: ResMut<'w, NextLootItemId>,
}

/// Clean up world when leaving Playing or Running (test) state.
pub(crate) fn game_cleanup_system(
    mut commands: Commands,
    npc_query: Query<Entity, With<GpuSlot>>,
    marker_query: Query<Entity, With<FarmReadyMarker>>,
    indicator_query: Query<Entity, With<SlotIndicator>>,
    ghost_query: Query<Entity, With<BuildGhost>>,
    tilemap_query: Query<Entity, With<bevy::sprite_render::TilemapChunk>>,
    terrain_query: Query<Entity, With<crate::render::TerrainChunk>>,
    mut world: CleanupWorld,
    mut debug: CleanupDebug,
    mut ui: CleanupUi,
    mut gameplay: CleanupGameplay,
) {
    // Despawn all entities
    for entity in npc_query.iter() {
        commands.entity(entity).despawn();
    }
    for entity in marker_query.iter() {
        commands.entity(entity).despawn();
    }
    for entity in indicator_query.iter() {
        commands.entity(entity).despawn();
    }
    for entity in ghost_query.iter() {
        commands.entity(entity).despawn();
    }
    for entity in tilemap_query.iter() {
        commands.entity(entity).despawn();
    }
    for entity in terrain_query.iter() {
        commands.entity(entity).despawn();
    }

    // Despawn town entities
    for &entity in world.town_index.0.values() {
        commands.entity(entity).despawn();
    }
    world.town_index.0.clear();

    // Reset world resources
    world.world_state.entity_slots.reset();
    *world.world_state.world_data = Default::default();
    *world.faction_stats = Default::default();
    *world.gpu_state = Default::default();
    *world.game_time = Default::default();
    *world.world_state.grid = Default::default();
    world.tilemap_spawned.0 = false;
    *world.build_menu_ctx = Default::default();
    *world.ai_state = Default::default();
    world.render_config.npc = Default::default();
    world.render_config.proj = Default::default();
    *world.npc_gpu_state = Default::default();
    *world.npc_visual_upload = Default::default();
    *world.proj_buffer_writes = Default::default();

    // Reset debug/tracking resources
    *debug.combat_debug = Default::default();
    *debug.health_debug = Default::default();
    *debug.kill_stats = Default::default();
    *debug.raider_state = Default::default();
    *debug.pop_stats = Default::default();
    *debug.debug_flags = Default::default();
    *world.world_state.entity_map = Default::default();

    // Reset UI state
    *ui.combat_log = Default::default();
    *ui.ui_state = Default::default();
    *ui.squad_state = Default::default();
    *ui.building_hp_render = Default::default();
    world.world_state.dirty_writers.emit_all();
    ui.healing_cache.by_faction.clear();
    *ui.active_healing = Default::default();
    *ui.endless = Default::default();
    *ui.merchant_inv = Default::default();
    *ui.next_loot_id = Default::default();

    // Reset gameplay resources
    *gameplay.auto_upgrade = Default::default();
    *gameplay.npc_logs = Default::default();
    *gameplay.migration = Default::default();
    *gameplay.tower_state = Default::default();
    *gameplay.selected_npc = Default::default();
    *gameplay.selected_building = Default::default();
    *gameplay.follow = Default::default();
    *gameplay.proj_slots = Default::default();
    *gameplay.mining_policy = Default::default();

    commands.remove_resource::<crate::systems::llm_player::LlmPlayerState>();

    info!("Game cleanup complete");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn road_ui_cell_allowed_rejects_tree_like_blockers() {
        let grass = world::WorldCell {
            terrain: world::Biome::Grass,
            original_terrain: world::Biome::Grass,
        };
        let dirt = world::WorldCell {
            terrain: world::Biome::Dirt,
            original_terrain: world::Biome::Dirt,
        };
        let forest = world::WorldCell {
            terrain: world::Biome::Forest,
            original_terrain: world::Biome::Forest,
        };
        let water = world::WorldCell {
            terrain: world::Biome::Water,
            original_terrain: world::Biome::Water,
        };
        let rock = world::WorldCell {
            terrain: world::Biome::Rock,
            original_terrain: world::Biome::Rock,
        };

        assert!(road_ui_cell_allowed(Some(&grass)));
        assert!(road_ui_cell_allowed(Some(&dirt)));
        assert!(!road_ui_cell_allowed(Some(&forest)));
        assert!(!road_ui_cell_allowed(Some(&water)));
        assert!(!road_ui_cell_allowed(Some(&rock)));
        assert!(!road_ui_cell_allowed(None));
    }
}
