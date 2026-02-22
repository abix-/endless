//! UI module — main menu, game startup, in-game HUD, and gameplay panels.

pub mod main_menu;
pub mod game_hud;
pub mod build_menu;
pub mod left_panel;
pub mod tutorial;

use bevy::audio::Volume;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContextSettings, EguiPrimaryContextPass, egui};

use crate::AppState;
use crate::constants::TOWN_GRID_SPACING;
use crate::components::*;
use crate::messages::SpawnNpcMsg;
use crate::resources::*;
use crate::systemparams::WorldState;
use crate::systems::{AiPlayerState, TownUpgrades, UpgradeQueue};
use crate::world::{self, BuildingKind, WorldGenConfig, allocate_all_building_slots};

/// Render a small "?" button (frameless) that shows help text on hover.
pub fn help_tip(ui: &mut egui::Ui, catalog: &HelpCatalog, key: &str) {
    if let Some(text) = catalog.0.get(key) {
        ui.add(egui::Button::new(
            egui::RichText::new("?").color(egui::Color32::from_rgb(120, 120, 180)).small()
        ).frame(false))
        .on_hover_text(*text);
    }
}
/// Render a label that shows a tooltip on hover (frameless button trick).
pub fn tipped(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>, tip: &str) -> egui::Response {
    ui.add(egui::Button::new(text).frame(false)).on_hover_text(tip)
}

/// Stable display name for a gold mine index used across all inspectors/policies.
pub fn gold_mine_name(mine_idx: usize) -> String {
    format!("Gold Mine {}", mine_idx + 1)
}

/// Apply user's UI scale to all egui contexts via EguiContextSettings.
fn apply_ui_scale(
    settings: Res<crate::settings::UserSettings>,
    mut egui_settings: Query<&mut EguiContextSettings>,
) {
    if !settings.is_changed() { return; }
    for mut s in egui_settings.iter_mut() {
        s.scale_factor = settings.ui_scale;
    }
}

/// Register all UI systems.
pub fn register_ui(app: &mut App) {
    // Global: UI scale + overlays (all states)
    app.add_systems(Update, apply_ui_scale);
    app.add_systems(EguiPrimaryContextPass, game_hud::jukebox_ui_system);

    // Main menu (egui)
    app.add_systems(EguiPrimaryContextPass,
        main_menu::main_menu_system.run_if(in_state(AppState::MainMenu)));

    // Game startup: load from save (if requested) then world gen (if not loaded) then tutorial init
    app.add_systems(OnEnter(AppState::Playing), (game_load_system, game_startup_system, tutorial_init_system).chain());

    // Egui panels — ordered so top bar claims height first, then side panels, then bottom.
    // Top bar → left panel → bottom panel (inspector+log) + overlay → windows → pause overlay.
    app.add_systems(EguiPrimaryContextPass, (
        game_hud::top_bar_system,
        left_panel::left_panel_system,
        (
            game_hud::bottom_panel_system,
            game_hud::combat_log_system,
            game_hud::selection_overlay_system,
            game_hud::target_overlay_system,
            game_hud::squad_overlay_system,
            game_hud::faction_squad_overlay_system,
        ),
        build_menu::build_menu_system,
        pause_menu_system,
        game_hud::save_toast_system,
        tutorial::tutorial_ui_system,
    ).chain().run_if(in_state(AppState::Playing)));

    // Panel toggle keyboard shortcuts + ESC
    app.add_systems(Update, (
        ui_toggle_system,
        game_escape_system,
    ).run_if(in_state(AppState::Playing)));

    // Escape + settings + inspector in test scenes
    app.add_systems(Update, game_escape_system.run_if(in_state(AppState::Running)));
    app.add_systems(EguiPrimaryContextPass, (
        game_hud::bottom_panel_system,
        game_hud::selection_overlay_system,
        pause_menu_system,
    ).run_if(in_state(AppState::Running)));

    // Building slot click detection + visual indicators + ghost
    app.add_systems(Update, (
        build_place_click_system,
        slot_right_click_system,
        build_ghost_system,
        draw_slot_indicators,
        process_destroy_system,
    ).run_if(in_state(AppState::Playing)));

    // Cleanup when leaving Playing
    app.add_systems(OnExit(AppState::Playing), game_cleanup_system);
}

/// Keyboard shortcuts for toggling UI panels.
pub fn ui_toggle_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut ui_state: ResMut<UiState>,
    mut follow: ResMut<FollowSelected>,
    mut squad_state: ResMut<SquadState>,
    mut build_ctx: ResMut<BuildMenuContext>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        ui_state.toggle_left_tab(LeftPanelTab::Roster);
    }
    if keys.just_pressed(KeyCode::KeyB) {
        ui_state.build_menu_open = !ui_state.build_menu_open;
    }
    if keys.just_pressed(KeyCode::KeyU) {
        ui_state.toggle_left_tab(LeftPanelTab::Upgrades);
    }
    if keys.just_pressed(KeyCode::KeyP) {
        ui_state.toggle_left_tab(LeftPanelTab::Policies);
    }
    if keys.just_pressed(KeyCode::KeyT) {
        ui_state.toggle_left_tab(LeftPanelTab::Patrols);
    }
    if keys.just_pressed(KeyCode::KeyQ) {
        ui_state.toggle_left_tab(LeftPanelTab::Squads);
    }
    if keys.just_pressed(KeyCode::KeyI) {
        ui_state.toggle_left_tab(LeftPanelTab::Factions);
    }
    if keys.just_pressed(KeyCode::KeyH) {
        ui_state.toggle_left_tab(LeftPanelTab::Help);
    }
    if keys.just_pressed(KeyCode::KeyL) {
        ui_state.combat_log_visible = !ui_state.combat_log_visible;
    }
    if keys.just_pressed(KeyCode::KeyF) {
        follow.0 = !follow.0;
    }
    // Squad target hotkeys: 1-9,0 => squads 1-10 and enter set-target mode.
    let squad_hotkey = if keys.just_pressed(KeyCode::Digit1) { Some(0) }
        else if keys.just_pressed(KeyCode::Digit2) { Some(1) }
        else if keys.just_pressed(KeyCode::Digit3) { Some(2) }
        else if keys.just_pressed(KeyCode::Digit4) { Some(3) }
        else if keys.just_pressed(KeyCode::Digit5) { Some(4) }
        else if keys.just_pressed(KeyCode::Digit6) { Some(5) }
        else if keys.just_pressed(KeyCode::Digit7) { Some(6) }
        else if keys.just_pressed(KeyCode::Digit8) { Some(7) }
        else if keys.just_pressed(KeyCode::Digit9) { Some(8) }
        else if keys.just_pressed(KeyCode::Digit0) { Some(9) }
        else { None };
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
    // WASD cancels follow — user wants manual control
    if follow.0 && (keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::KeyA)
        || keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::KeyD)) {
        follow.0 = false;
    }
}

// ============================================================================
// GAME STARTUP
// ============================================================================

// SystemParam bundle for startup to stay under 16-param limit
#[derive(SystemParam)]
struct StartupExtra<'w> {
    policies: ResMut<'w, TownPolicies>,
    npcs_by_town: ResMut<'w, NpcsByTownCache>,
    ai_state: ResMut<'w, AiPlayerState>,
    combat_log: ResMut<'w, CombatLog>,
    gold_storage: ResMut<'w, GoldStorage>,
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
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
    combat_config: Res<crate::systems::stats::CombatConfig>,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut mining_policy: ResMut<MiningPolicy>,
) {
    if !save_request.load_on_enter { return; }
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

    let town_count = save.building_data.get("towns")
        .and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    info!("Loading save from menu: {} NPCs, {} towns", save.npcs.len(), town_count);

    // Apply save data to all game resources
    crate::save::apply_save(
        &save,
        &mut ws.grid, &mut ws.world_data, &mut ws.town_grids, &mut ws.game_time,
        &mut ws.food_storage, &mut ws.gold_storage, &mut ws.farm_states,
        &mut ws.spawner_state, &mut ws.building_hp, &mut ws.upgrades, &mut ws.policies,
        &mut ws.auto_upgrade, &mut ws.squad_state, &mut fs.raider_state,
        &mut fs.faction_stats, &mut fs.kill_stats, &mut fs.ai_state,
        &mut fs.migration_state, &mut fs.endless,
        &mut tracking.npcs_by_town, &mut tracking.slots,
    );

    // Rebuild spatial grid
    tracking.bgrid.rebuild(&ws.world_data, ws.grid.width as f32 * ws.grid.cell_size);
    *tracking.dirty = DirtyFlags::default();
    *mining_policy = MiningPolicy::default();

    // Allocate GPU slots for buildings (collision via GPU compute)
    allocate_all_building_slots(&ws.world_data, &mut tracking.slots, &mut tracking.building_slots);

    // Spawn NPC entities from save data
    crate::save::spawn_npcs_from_save(
        &save, &mut commands,
        &mut tracking.npc_map, &mut tracking.pop_stats, &mut tracking.npc_meta,
        &mut tracking.npcs_by_town, &mut gpu_updates,
        &ws.world_data, &combat_config, &ws.upgrades,
    );

    // Re-attach Migrating component to migration group members
    if let Some(mg) = &fs.migration_state.active {
        for &slot in &mg.member_slots {
            if let Some(&entity) = tracking.npc_map.0.get(&slot) {
                commands.entity(entity).insert(crate::components::Migrating);
            }
        }
    }

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
    config: Res<WorldGenConfig>,
    mut world_state: WorldState,
    mut food_storage: ResMut<FoodStorage>,
    mut faction_stats: ResMut<FactionStats>,
    mut raider_state: ResMut<RaiderState>,
    mut game_config: ResMut<GameConfig>,
    mut spawn_writer: MessageWriter<SpawnNpcMsg>,
    mut game_time: ResMut<GameTime>,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut extra: StartupExtra,
) {
    // If game_load_system already populated the world, skip world gen.
    // The flag was cleared by game_load_system, but we can detect load happened
    // by checking if the world grid is already populated.
    if world_state.grid.cells.len() > 0 {
        info!("Game startup: skipping world gen (loaded from save)");
        return;
    }

    info!("Game startup: generating world...");

    // Full world setup: terrain, towns, resources, buildings, spawners, NPCs, AI players
    let (npc_msgs, ai_players) = world::setup_world(
        &config,
        &mut world_state.grid, &mut world_state.world_data,
        &mut world_state.farm_states, &mut world_state.town_grids,
        &mut world_state.spawner_state, &mut world_state.building_hp,
        &mut world_state.slot_alloc, &mut world_state.building_slots,
        &mut food_storage, &mut extra.gold_storage,
        &mut faction_stats, &mut raider_state,
    );
    let total = npc_msgs.len();
    for msg in npc_msgs { spawn_writer.write(msg); }

    // Game-specific post-setup: settings, policies, combat log
    *extra.mining_policy = MiningPolicy::default();
    let num_towns = world_state.world_data.towns.len();
    extra.npcs_by_town.0.resize(num_towns, Vec::new());
    game_config.npc_counts = config.npc_counts.iter().map(|(&job, &count)| (job, count as i32)).collect();
    *game_time = GameTime::default();
    world_state.building_occupancy.clear();

    // Load saved policies + auto-upgrade flags for player's town
    let saved = crate::settings::load_settings();
    let town_idx = world_state.world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);
    if town_idx < extra.policies.policies.len() {
        extra.policies.policies[town_idx] = saved.policy;
    }
    if !saved.auto_upgrades.is_empty() && town_idx < extra.auto_upgrade.flags.len() {
        let flags = &mut extra.auto_upgrade.flags[town_idx];
        *flags = crate::systems::stats::decode_auto_upgrade_flags(&saved.auto_upgrades);
    }

    // Apply personality-based policies + log AI players joining
    for player in &ai_players {
        if let Some(policy) = extra.policies.policies.get_mut(player.town_data_idx) {
            *policy = player.personality.default_policies();
        }
        if let Some(town) = world_state.world_data.towns.get(player.town_data_idx) {
            extra.combat_log.push(CombatEventKind::Ai, -1, 1, 6, 0,
                format!("{} [{}] joined the game", town.name, player.personality.name()));
        }
    }
    extra.ai_state.players = ai_players;

    // Center camera on first town
    if let Some(first_town) = world_state.world_data.towns.first() {
        if let Ok(mut transform) = camera_query.single_mut() {
            transform.translation.x = first_town.center.x;
            transform.translation.y = first_town.center.y;
        }
    }

    *world_state.dirty = DirtyFlags::default();

    info!("Game startup complete: {} NPCs spawned across {} towns",
        total, config.num_towns);
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

    let player_town = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);

    // Snapshot initial building counts for completion checks
    tutorial.initial_farms = world_data.farms().iter().filter(|f| f.town_idx as usize == player_town).count();
    tutorial.initial_farmer_homes = world_data.get(BuildingKind::FarmerHome).iter().filter(|h| h.town_idx as usize == player_town).count();
    tutorial.initial_waypoints = world_data.waypoints().iter().filter(|g| g.town_idx as usize == player_town).count();
    tutorial.initial_archer_homes = world_data.get(BuildingKind::ArcherHome).iter().filter(|a| a.town_idx as usize == player_town).count();
    tutorial.initial_miner_homes = world_data.miner_homes().iter().filter(|m| m.town_idx as usize == player_town).count();

    // Snapshot camera start position
    if let Ok(transform) = camera_query.single() {
        tutorial.camera_start = Vec2::new(transform.translation.x, transform.translation.y);
    }

    tutorial.start_time = time.elapsed_secs_f64();
    tutorial.step = 1;
    info!("Tutorial started (farms={}, farmer_homes={}, waypoints={}, archer_homes={}, miner_homes={})",
        tutorial.initial_farms, tutorial.initial_farmer_homes,
        tutorial.initial_waypoints, tutorial.initial_archer_homes, tutorial.initial_miner_homes);
}

// ============================================================================
// GAME EXIT
// ============================================================================

/// ESC toggles pause menu. Space/+/- control time (only when menu closed).
fn game_escape_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut ui_state: ResMut<UiState>,
    mut game_time: ResMut<GameTime>,
    mut squad_state: ResMut<SquadState>,
    mut build_ctx: ResMut<BuildMenuContext>,
    settings: Res<crate::settings::UserSettings>,
) {
    if keys.just_pressed(KeyCode::Escape) {
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
    // Time controls only when pause menu is closed
    if !ui_state.pause_menu_open {
        if keys.just_pressed(KeyCode::Space) {
            game_time.paused = !game_time.paused;
        }
        if keys.just_pressed(KeyCode::Equal) {
            game_time.time_scale = (game_time.time_scale * 2.0).min(128.0);
        }
        if keys.just_pressed(KeyCode::Minus) {
            game_time.time_scale = (game_time.time_scale / 2.0).max(0.25);
        }
    }
}

/// Pause menu overlay — Resume, Settings, Exit to Main Menu.
fn pause_menu_system(
    mut contexts: bevy_egui::EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut game_time: ResMut<GameTime>,
    mut next_state: ResMut<NextState<AppState>>,
    mut settings: ResMut<crate::settings::UserSettings>,
    mut winit_settings: ResMut<bevy::winit::WinitSettings>,
    mut audio: ResMut<crate::resources::GameAudio>,
    mut music_sinks: Query<&mut AudioSink, With<crate::resources::MusicTrack>>,
) -> Result {
    if !ui_state.pause_menu_open { return Ok(()); }

    let ctx = contexts.ctx_mut()?;

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
    egui::Window::new("Paused")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .min_width(280.0)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                if ui.button("Resume").clicked() {
                    ui_state.pause_menu_open = false;
                    game_time.paused = false;
                    crate::settings::save_settings(&settings);
                }
                ui.add_space(4.0);
            });

            ui.separator();

            // Settings section
            egui::CollapsingHeader::new("Settings")
                .default_open(true)
                .show(ui, |ui| {
                    ui.add(egui::Slider::new(&mut settings.ui_scale, 0.8..=2.5)
                        .text("UI Scale"));
                    ui.add(egui::Slider::new(&mut settings.scroll_speed, 100.0..=2000.0)
                        .text("Scroll Speed"));
                    ui.add(egui::Slider::new(&mut settings.help_text_size, 8.0..=24.0)
                        .text("Help Text Size"));
                    ui.add(egui::Slider::new(&mut settings.build_menu_text_scale, 0.7..=2.0)
                        .text("Build Menu Text Scale"));

                    let prev_bg_fps = settings.background_fps;
                    ui.checkbox(&mut settings.background_fps, "Full FPS in Background");
                    if settings.background_fps != prev_bg_fps {
                        winit_settings.unfocused_mode = if settings.background_fps {
                            bevy::winit::UpdateMode::Continuous
                        } else {
                            bevy::winit::UpdateMode::reactive_low_power(
                                std::time::Duration::from_secs_f64(1.0 / 60.0),
                            )
                        };
                    }

                    ui.add_space(4.0);
                    ui.label("Audio:");
                    let prev_music = settings.music_volume;
                    ui.add(egui::Slider::new(&mut settings.music_volume, 0.0..=1.0)
                        .text("Music"));
                    if settings.music_volume != prev_music {
                        audio.music_volume = settings.music_volume;
                        for mut sink in &mut music_sinks {
                            sink.set_volume(Volume::Linear(settings.music_volume));
                        }
                    }
                    ui.add(egui::Slider::new(&mut settings.sfx_volume, 0.0..=1.0)
                        .text("SFX"));
                    audio.sfx_volume = settings.sfx_volume;

                    ui.add_space(4.0);
                    ui.label("Combat Log Filters:");
                    ui.checkbox(&mut settings.log_kills, "Kills");
                    ui.checkbox(&mut settings.log_spawns, "Spawns");
                    ui.checkbox(&mut settings.log_raids, "Raids");
                    ui.checkbox(&mut settings.log_harvests, "Harvests");
                    ui.checkbox(&mut settings.log_levelups, "Level Ups");
                    ui.checkbox(&mut settings.log_npc_activity, "NPC Activity");
                    ui.checkbox(&mut settings.log_ai, "AI Actions");

                    ui.add_space(4.0);
                    ui.label("Debug:");
                    let prev_debug = (settings.debug_coordinates, settings.debug_all_npcs,
                        settings.debug_readback, settings.debug_combat,
                        settings.debug_spawns, settings.debug_behavior, settings.debug_profiler,
                        settings.show_terrain_sprites, settings.show_all_faction_squad_lines,
                        settings.debug_ai_decisions);
                    ui.checkbox(&mut settings.debug_coordinates, "NPC Coordinates");
                    ui.checkbox(&mut settings.debug_all_npcs, "All NPCs in Roster");
                    ui.checkbox(&mut settings.debug_readback, "GPU Readback");
                    ui.checkbox(&mut settings.debug_combat, "Combat Logging");
                    ui.checkbox(&mut settings.debug_spawns, "Spawn Logging");
                    ui.checkbox(&mut settings.debug_behavior, "Behavior Logging");
                    ui.checkbox(&mut settings.debug_profiler, "System Profiler");
                    ui.checkbox(&mut settings.debug_ai_decisions, "AI Decision Logging");
                    ui.checkbox(&mut settings.show_terrain_sprites, "Show Terrain Sprites");
                    ui.checkbox(&mut settings.show_all_faction_squad_lines, "Show All Faction Squad Lines");
                    let now_debug = (settings.debug_coordinates, settings.debug_all_npcs,
                        settings.debug_readback, settings.debug_combat,
                        settings.debug_spawns, settings.debug_behavior, settings.debug_profiler,
                        settings.show_terrain_sprites, settings.show_all_faction_squad_lines,
                        settings.debug_ai_decisions);
                    if prev_debug != now_debug {
                        crate::settings::save_settings(&settings);
                    }
                });

            ui.separator();
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                if ui.button("Exit to Main Menu").clicked() {
                    ui_state.pause_menu_open = false;
                    crate::settings::save_settings(&settings);
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
fn slots_on_line(start: (i32, i32), end: (i32, i32)) -> Vec<(i32, i32)> {
    let (mut r0, mut c0) = start;
    let (r1, c1) = end;
    let dr = (r1 - r0).abs();
    let dc = (c1 - c0).abs();
    let sr = if r0 < r1 { 1 } else if r0 > r1 { -1 } else { 0 };
    let sc = if c0 < c1 { 1 } else if c0 > c1 { -1 } else { 0 };
    let mut err = dr - dc;

    let mut out = Vec::new();
    loop {
        out.push((r0, c0));
        if r0 == r1 && c0 == c1 { break; }
        let e2 = 2 * err;
        if e2 > -dc {
            err -= dc;
            r0 += sr;
        }
        if e2 < dr {
            err += dr;
            c0 += sc;
        }
    }
    out
}

/// Right-click cancels active build placement.
fn slot_right_click_system(
    mouse: Res<ButtonInput<MouseButton>>,
    mut build_ctx: ResMut<BuildMenuContext>,
) {
    if !mouse.just_pressed(MouseButton::Right) { return; }
    build_ctx.selected_build = None;
    build_ctx.destroy_mode = false;
    build_ctx.clear_drag();
}

/// Left-click places the currently selected building into any valid slot in buildable area.
fn build_place_click_system(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_query: Query<(&Transform, &Projection), With<crate::render::MainCamera>>,
    mut egui_contexts: bevy_egui::EguiContexts,
    mut build_ctx: ResMut<BuildMenuContext>,
    mut world_state: WorldState,
    mut food_storage: ResMut<FoodStorage>,
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    _difficulty: Res<Difficulty>,
) {
    if build_ctx.selected_build.is_none() && !build_ctx.destroy_mode { return; }
    let just_pressed = mouse.just_pressed(MouseButton::Left);
    let pressed = mouse.pressed(MouseButton::Left);
    let just_released = mouse.just_released(MouseButton::Left);
    if !just_pressed && !pressed && !just_released { return; }

    if let Ok(ctx) = egui_contexts.ctx_mut() {
        if ctx.wants_pointer_input() || ctx.is_pointer_over_area() {
            if just_released { build_ctx.clear_drag(); }
            return;
        }
    }

    let Some(town_data_idx) = build_ctx.town_data_idx else { return };
    let Some(town) = world_state.world_data.towns.get(town_data_idx) else { return };
    let center = town.center;
    let town_name = town.name.clone();
    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((transform, projection)) = camera_query.single() else { return };
    let world_pos = screen_to_world(cursor_pos, transform, projection, window);
    let (row, col) = world::world_to_town_grid(center, world_pos);
    let slot_pos = world::town_grid_to_world(center, row, col);
    build_ctx.hover_world_pos = slot_pos;

    let (gc, gr) = world_state.grid.world_to_grid(slot_pos);

    // Destroy mode: remove building at clicked cell (player-owned only)
    if build_ctx.destroy_mode {
        if !just_pressed { return; }
        build_ctx.clear_drag();
        let cell_building = world_state.grid.cell(gc, gr).and_then(|c| c.building);
        let is_destructible = cell_building
            .map(|(k, ti)| {
                !matches!(k, world::BuildingKind::Fountain | world::BuildingKind::GoldMine)
                && world_state.world_data.towns.get(ti as usize).map_or(false, |t| t.faction == 0)
            })
            .unwrap_or(false);
        if !is_destructible { return; }
        let bld_kind = cell_building.map(|(k, _)| k);

        let _ = world::destroy_building(
            &mut world_state.grid, &mut world_state.world_data, &mut world_state.farm_states,
            &mut world_state.spawner_state, &mut world_state.building_hp,
            &mut world_state.slot_alloc, &mut world_state.building_slots,
            &mut combat_log, &game_time,
            row, col, center,
            &format!("Destroyed building at ({},{}) in {}", row, col, town_name),
        );
        if let Some(bk) = bld_kind {
            world_state.dirty.mark_building_changed(bk);
        }
        return;
    }

    let kind = build_ctx.selected_build.unwrap();

    // Waypoint: single-click wilderness placement
    if kind == BuildingKind::Waypoint {
        if !just_pressed { return; }
        build_ctx.clear_drag();
        let cost = crate::constants::building_cost(kind);
        if world::place_wilderness_building(
            kind,
            &mut world_state.grid, &mut world_state.world_data,
            &mut world_state.building_hp, &mut food_storage,
            &mut world_state.slot_alloc, &mut world_state.building_slots,
            town_data_idx, world_pos, cost, &world_state.town_grids,
        ).is_ok() {
            world_state.dirty.mark_building_changed(kind);
            let label = crate::constants::building_def(kind).label;
            combat_log.push(
                CombatEventKind::Harvest, 0,
                game_time.day(), game_time.hour(), game_time.minute(),
                format!("Built {} in {}", label.to_lowercase(), town_name),
            );
        }
        return;
    }

    // Road: drag-line placement on world grid (reuses drag_start_slot/slots_on_line)
    if kind == BuildingKind::Road {
        let (gc, gr) = world_state.grid.world_to_grid(world_pos);
        if just_pressed {
            build_ctx.drag_start_slot = Some((gr as i32, gc as i32));
            build_ctx.drag_current_slot = Some((gr as i32, gc as i32));
        } else if pressed && build_ctx.drag_start_slot.is_some() {
            build_ctx.drag_current_slot = Some((gr as i32, gc as i32));
        }
        if !just_released { return; }

        let start = build_ctx.drag_start_slot.take().unwrap_or((gr as i32, gc as i32));
        let end = build_ctx.drag_current_slot.take().unwrap_or((gr as i32, gc as i32));
        let cost = crate::constants::building_cost(kind);
        let mut placed = 0usize;
        for (sr, sc) in slots_on_line(start, end) {
            let cell_pos = world_state.grid.grid_to_world(sc as usize, sr as usize);
            if world::place_wilderness_building(
                kind, &mut world_state.grid, &mut world_state.world_data,
                &mut world_state.building_hp, &mut food_storage,
                &mut world_state.slot_alloc, &mut world_state.building_slots,
                town_data_idx, cell_pos, cost, &world_state.town_grids,
            ).is_ok() { placed += 1; }
        }
        if placed > 0 {
            world_state.dirty.mark_building_changed(kind);
            let label = crate::constants::building_def(kind).label;
            let msg = if placed == 1 {
                format!("Built {} in {}", label.to_lowercase(), town_name)
            } else {
                format!("Built {} {}s in {}", placed, label.to_lowercase(), town_name)
            };
            combat_log.push(CombatEventKind::Harvest, 0,
                game_time.day(), game_time.hour(), game_time.minute(), msg);
        }
        return;
    }

    // Town-grid build mode: supports single-click and click-drag line placement.
    let label = crate::constants::building_def(kind).label;

    let mut try_place_at_slot = |slot_row: i32, slot_col: i32| -> bool {
        let Some(town_grid) = world_state.town_grids.grids.iter().find(|tg| tg.town_data_idx == town_data_idx) else { return false };
        if !world::is_slot_buildable(town_grid, slot_row, slot_col) { return false; }
        if slot_row == 0 && slot_col == 0 { return false; }
        let snapped = world::town_grid_to_world(center, slot_row, slot_col);
        let (sgc, sgr) = world_state.grid.world_to_grid(snapped);
        if world_state.grid.cell(sgc, sgr).map(|c| c.building.is_some()) != Some(false) { return false; }

        let cost = crate::constants::building_cost(kind);
        let food = food_storage.food.get(town_data_idx).copied().unwrap_or(0);
        if food < cost { return false; }

        world::build_and_pay(
            &mut world_state.grid, &mut world_state.world_data, &mut world_state.farm_states,
            &mut food_storage, &mut world_state.spawner_state, &mut world_state.building_hp,
            &mut world_state.slot_alloc, &mut world_state.building_slots, &mut world_state.dirty,
            kind, town_data_idx,
            slot_row, slot_col, center, cost,
        )
    };

    if just_pressed {
        build_ctx.drag_start_slot = Some((row, col));
        build_ctx.drag_current_slot = Some((row, col));
    } else if pressed && build_ctx.drag_start_slot.is_some() {
        build_ctx.drag_current_slot = Some((row, col));
    }

    if !just_released { return; }

    let start = build_ctx.drag_start_slot.take().unwrap_or((row, col));
    let end = build_ctx.drag_current_slot.take().unwrap_or((row, col));
    let mut placed = 0usize;
    let mut first_placed: Option<(i32, i32)> = None;
    for (sr, sc) in slots_on_line(start, end) {
        if try_place_at_slot(sr, sc) {
            if first_placed.is_none() { first_placed = Some((sr, sc)); }
            placed += 1;
        }
    }
    if placed == 0 { return; }

    if placed == 1 {
        let (pr, pc) = first_placed.unwrap_or((row, col));
        combat_log.push(
            CombatEventKind::Harvest, 0,
            game_time.day(), game_time.hour(), game_time.minute(),
            format!("Built {} at ({},{}) in {}", label, pr, pc, town_name),
        );
    } else {
        combat_log.push(
            CombatEventKind::Harvest, 0,
            game_time.day(), game_time.hour(), game_time.minute(),
            format!("Built {} {}s in {} (drag line)", placed, label, town_name),
        );
    }
}

/// Marker component for slot indicator sprite entities.
#[derive(Component)]
struct SlotIndicator;

/// Marker for the build ghost preview sprite.
#[derive(Component)]
struct BuildGhost;

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
    town_grids: Res<world::TownGrids>,
    food_storage: Res<FoodStorage>,
    mut ghost_query: Query<(Entity, &mut Transform, &mut Sprite), (With<BuildGhost>, Without<BuildGhostTrail>, Without<crate::render::MainCamera>)>,
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
    let Some(cursor_pos) = window.cursor_position() else { return };

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

    let Ok((cam_transform, projection)) = camera_query.single() else { return };
    let world_pos = screen_to_world(cursor_pos, cam_transform, projection, window);

    // Destroy mode: snap to town grid, show red ghost over destructible buildings
    if build_ctx.destroy_mode {
        for entity in trail_query.iter() {
            commands.entity(entity).despawn();
        }
        let Some(town_data_idx) = build_ctx.town_data_idx else { return };
        let Some(town) = world_data.towns.get(town_data_idx) else { return };
        let center = town.center;
        let (row, col) = world::world_to_town_grid(center, world_pos);
        let slot_pos = world::town_grid_to_world(center, row, col);
        build_ctx.hover_world_pos = slot_pos;
        let (gc, gr) = grid.world_to_grid(slot_pos);
        let cell = grid.cell(gc, gr);
        let has_building = cell.map(|c| c.building.is_some()).unwrap_or(false);
        let is_fountain = cell
            .and_then(|c| c.building)
            .map(|(k, _)| matches!(k, world::BuildingKind::Fountain | world::BuildingKind::GoldMine))
            .unwrap_or(false);
        let valid = has_building && !is_fountain;
        build_ctx.show_cursor_hint = true;
        let color = if valid { Color::srgba(0.8, 0.2, 0.2, 0.6) } else { Color::NONE };
        let snapped = grid.grid_to_world(gc, gr);
        let ghost_z = 0.5;
        if let Some((_, mut transform, mut sprite)) = ghost_query.iter_mut().next() {
            transform.translation = Vec3::new(snapped.x, snapped.y, ghost_z);
            sprite.color = color;
            sprite.image = Handle::default();
        } else {
            commands.spawn((
                Sprite { color, image: Handle::default(), custom_size: Some(Vec2::splat(TOWN_GRID_SPACING)), ..default() },
                Transform::from_xyz(snapped.x, snapped.y, ghost_z),
                BuildGhost,
            ));
        }
        return;
    }

    let kind = build_ctx.selected_build.unwrap();

    // Road: world-grid ghost with drag trail preview (mirrors town-grid trail pattern)
    if kind == BuildingKind::Road {
        let (gc, gr) = grid.world_to_grid(world_pos);
        let snapped = grid.grid_to_world(gc, gr);
        build_ctx.hover_world_pos = snapped;

        let path = match build_ctx.drag_start_slot {
            Some(start) => {
                build_ctx.drag_current_slot = Some((gr as i32, gc as i32));
                slots_on_line(start, (gr as i32, gc as i32))
            }
            None => vec![(gr as i32, gc as i32)],
        };

        let cost = crate::constants::building_cost(kind);
        let town_idx = build_ctx.town_data_idx.unwrap_or(0);
        let mut budget = food_storage.food.get(town_idx).copied().unwrap_or(0);
        let ghost_image = build_ctx.ghost_sprites.get(&kind).cloned().unwrap_or_default();
        let ghost_z = 0.5;

        // Despawn old trail, rebuild (same pattern as town-grid lines 1118-1142)
        for entity in trail_query.iter() { commands.entity(entity).despawn(); }

        let mut cursor_valid = false;
        for (idx, &(sr, sc)) in path.iter().enumerate() {
            let cell_world = grid.grid_to_world(sc as usize, sr as usize);
            let (cgc, cgr) = grid.world_to_grid(cell_world);
            let cell = grid.cell(cgc, cgr);
            let empty = cell.map(|c| c.building.is_none()).unwrap_or(false);
            let not_water = cell.map(|c| c.terrain != world::Biome::Water).unwrap_or(false);
            let valid = empty && not_water && budget >= cost;
            if valid { budget -= cost; }

            if idx == path.len() - 1 {
                cursor_valid = valid;
            } else {
                let color = if valid {
                    Color::srgba(1.0, 1.0, 1.0, 0.45)
                } else {
                    Color::srgba(0.8, 0.2, 0.2, 0.35)
                };
                commands.spawn((
                    Sprite { color, image: ghost_image.clone(),
                        custom_size: Some(Vec2::splat(TOWN_GRID_SPACING)), ..default() },
                    Transform::from_xyz(cell_world.x, cell_world.y, ghost_z),
                    BuildGhost, BuildGhostTrail,
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
                Sprite { color, image: ghost_image,
                    custom_size: Some(Vec2::splat(TOWN_GRID_SPACING)), ..default() },
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
        let empty = cell.map(|c| c.building.is_none()).unwrap_or(false);
        let not_water = cell.map(|c| c.terrain != world::Biome::Water).unwrap_or(false);
        let valid = empty && not_water;
        build_ctx.show_cursor_hint = !valid;

        let color = if valid {
            Color::srgba(1.0, 1.0, 1.0, 0.7)
        } else {
            Color::srgba(0.8, 0.2, 0.2, 0.5)
        };
        let ghost_image = build_ctx.ghost_sprites.get(&kind).cloned().unwrap_or_default();
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
    let Some(town_data_idx) = build_ctx.town_data_idx else { return };
    let Some(town) = world_data.towns.get(town_data_idx) else { return };
    let center = town.center;
    let (row, col) = world::world_to_town_grid(center, world_pos);
    let slot_pos = world::town_grid_to_world(center, row, col);
    build_ctx.hover_world_pos = slot_pos;

    // Determine validity
    let (gc, gr) = grid.world_to_grid(slot_pos);
    let cell = grid.cell(gc, gr);
    let has_building = cell.map(|c| c.building.is_some()).unwrap_or(false);
    let town_grid = town_grids.grids.iter().find(|tg| tg.town_data_idx == town_data_idx);
    let in_bounds = town_grid
        .map(|tg| world::is_slot_buildable(tg, row, col))
        .unwrap_or(false);
    let is_center = row == 0 && col == 0;

    let mut drag_preview: Vec<(i32, i32, bool, bool)> = Vec::new();
    {
        let path = match (build_ctx.drag_start_slot, build_ctx.drag_current_slot) {
            (Some(start), Some(end)) => slots_on_line(start, end),
            _ => vec![(row, col)],
        };
        let cost = crate::constants::building_cost(kind);
        let mut budget = food_storage.food.get(town_data_idx).copied().unwrap_or(0);

        for (slot_row, slot_col) in path {
            let visible_slot = town_grid
                .map(|tg| world::is_slot_buildable(tg, slot_row, slot_col))
                .unwrap_or(false)
                && !(slot_row == 0 && slot_col == 0);
            if !visible_slot {
                drag_preview.push((slot_row, slot_col, false, false));
                continue;
            }

            let slot_world = world::town_grid_to_world(center, slot_row, slot_col);
            let (sgc, sgr) = grid.world_to_grid(slot_world);
            let slot_empty = grid.cell(sgc, sgr).map(|c| c.building.is_none()).unwrap_or(false);
            let can_pay = budget >= cost;
            let slot_valid = slot_empty && can_pay;
            if slot_valid {
                budget -= cost;
            }
            drag_preview.push((slot_row, slot_col, slot_valid, true));
        }
    }

    // Build: valid on empty buildable non-center slots, including drag budget preview.
    let (valid, visible) = {
        let current = drag_preview
            .iter()
            .find(|(sr, sc, _, _)| *sr == row && *sc == col)
            .copied();
        if let Some((_, _, v, vis)) = current {
            (v, vis)
        } else {
            (in_bounds && !is_center && !has_building, in_bounds && !is_center)
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
    let ghost_image = build_ctx.ghost_sprites.get(&kind).cloned().unwrap_or_default();

    // Rebuild drag trail each frame (all slots except the cursor slot).
    for entity in trail_query.iter() {
        commands.entity(entity).despawn();
    }
    if drag_preview.len() > 1 {
        for (slot_row, slot_col, slot_valid, slot_visible) in drag_preview.iter().copied() {
            if (slot_row == row && slot_col == col) || !slot_visible {
                continue;
            }
            let slot_world = world::town_grid_to_world(center, slot_row, slot_col);
            let (sgc, sgr) = grid.world_to_grid(slot_world);
            let snapped_slot = grid.grid_to_world(sgc, sgr);
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
/// Uses actual Sprite entities at z=-0.3 so they render between buildings and NPCs.
fn draw_slot_indicators(
    mut commands: Commands,
    existing: Query<Entity, With<SlotIndicator>>,
    world_data: Res<world::WorldData>,
    town_grids: Res<world::TownGrids>,
    grid: Res<world::WorldGrid>,
    build_ctx: Res<BuildMenuContext>,
) {
    // Only rebuild when grid state changes or build selection changes
    if !town_grids.is_changed() && !grid.is_changed() && !build_ctx.is_changed() { return; }

    // Despawn old indicators
    for entity in existing.iter() {
        commands.entity(entity).despawn();
    }

    // Only show indicators when a build type is selected (not destroy mode)
    if build_ctx.selected_build.is_none() || build_ctx.destroy_mode { return; }

    // Only show indicators for the player's villager town (first grid)
    let Some(town_grid) = town_grids.grids.first() else { return };
    let town_data_idx = town_grid.town_data_idx;
    let Some(town) = world_data.towns.get(town_data_idx) else { return };
    let center = town.center;

    let green = Color::srgba(0.3, 0.7, 0.3, 0.5);
    let indicator_z = -0.3;
    let line_w = 2.0;
    let line_len = 10.0;

    // Green "+" on empty unlocked slots
    let (min_row, max_row, min_col, max_col) = world::build_bounds(town_grid);
    for row in min_row..=max_row {
        for col in min_col..=max_col {
            if row == 0 && col == 0 { continue; }

            let raw_pos = world::town_grid_to_world(center, row, col);
            let (gc, gr) = grid.world_to_grid(raw_pos);
            let slot_pos = grid.grid_to_world(gc, gr);

            let has_building = grid.cell(gc, gr)
                .map(|c| c.building.is_some())
                .unwrap_or(false);

            if !has_building {
                // Horizontal bar
                commands.spawn((
                    Sprite { color: green, custom_size: Some(Vec2::new(line_len, line_w)), ..default() },
                    Transform::from_xyz(slot_pos.x, slot_pos.y, indicator_z),
                    SlotIndicator,
                ));
                // Vertical bar
                commands.spawn((
                    Sprite { color: green, custom_size: Some(Vec2::new(line_w, line_len)), ..default() },
                    Transform::from_xyz(slot_pos.x, slot_pos.y, indicator_z),
                    SlotIndicator,
                ));
            }
        }
    }
}

/// Process destroy requests from the building inspector.
fn process_destroy_system(
    mut request: ResMut<DestroyRequest>,
    mut world_state: WorldState,
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    mut selected_building: ResMut<SelectedBuilding>,
) {
    let Some((col, row)) = request.0.take() else { return };

    let cell = world_state.grid.cell(col, row);
    let cell_building = cell.and_then(|c| c.building);
    let is_destructible = cell_building
        .map(|(k, ti)| {
            !matches!(k, world::BuildingKind::Fountain | world::BuildingKind::GoldMine)
            && world_state.world_data.towns.get(ti as usize).map_or(false, |t| t.faction == 0)
        })
        .unwrap_or(false);
    if !is_destructible { return; }
    let bld_kind = cell_building.map(|(k, _)| k);

    // Find which town this building belongs to, derive town center
    let town_idx = cell_building
        .map(|(_, ti)| ti as usize)
        .unwrap_or(0);
    let center = world_state.world_data.towns.get(town_idx)
        .map(|t| t.center)
        .unwrap_or_default();
    let town_name = world_state.world_data.towns.get(town_idx)
        .map(|t| t.name.clone())
        .unwrap_or_default();

    let world_pos = world_state.grid.grid_to_world(col, row);
    let (trow, tcol) = world::world_to_town_grid(center, world_pos);

    if world::destroy_building(
        &mut world_state.grid, &mut world_state.world_data, &mut world_state.farm_states,
        &mut world_state.spawner_state, &mut world_state.building_hp,
        &mut world_state.slot_alloc, &mut world_state.building_slots,
        &mut combat_log, &game_time,
        trow, tcol, center,
        &format!("Destroyed building in {}", town_name),
    ).is_ok() {
        selected_building.active = false;
        if let Some(bk) = bld_kind {
            world_state.dirty.mark_building_changed(bk);
        }
    }
}

// ============================================================================
// GAME CLEANUP
// ============================================================================

// SystemParam bundles to keep cleanup under 16-param limit
#[derive(SystemParam)]
struct CleanupWorld<'w> {
    world_state: WorldState<'w>,
    food_storage: ResMut<'w, FoodStorage>,
    faction_stats: ResMut<'w, FactionStats>,
    gpu_state: ResMut<'w, GpuReadState>,
    render_config: ResMut<'w, crate::gpu::RenderFrameConfig>,
    npc_gpu_state: ResMut<'w, crate::gpu::NpcGpuState>,
    npc_visual_upload: ResMut<'w, crate::gpu::NpcVisualUpload>,
    proj_buffer_writes: ResMut<'w, crate::gpu::ProjBufferWrites>,
    game_time: ResMut<'w, GameTime>,
    tilemap_spawned: ResMut<'w, crate::render::TilemapSpawned>,
    build_menu_ctx: ResMut<'w, BuildMenuContext>,
    ai_state: ResMut<'w, AiPlayerState>,
    gold_storage: ResMut<'w, GoldStorage>,
}

#[derive(SystemParam)]
struct CleanupDebug<'w> {
    combat_debug: ResMut<'w, CombatDebug>,
    health_debug: ResMut<'w, HealthDebug>,
    kill_stats: ResMut<'w, KillStats>,
    raider_state: ResMut<'w, RaiderState>,
    npc_entity_map: ResMut<'w, NpcEntityMap>,
    pop_stats: ResMut<'w, PopulationStats>,
}

#[derive(SystemParam)]
struct CleanupGameplay<'w> {
    upgrades: ResMut<'w, TownUpgrades>,
    upgrade_queue: ResMut<'w, UpgradeQueue>,
    policies: ResMut<'w, TownPolicies>,
    auto_upgrade: ResMut<'w, AutoUpgrade>,
    npc_logs: ResMut<'w, NpcLogCache>,
    npc_meta: ResMut<'w, NpcMetaCache>,
    npcs_by_town: ResMut<'w, NpcsByTownCache>,
    migration: ResMut<'w, MigrationState>,
    tower_state: ResMut<'w, TowerState>,
    selected_npc: ResMut<'w, SelectedNpc>,
    selected_building: ResMut<'w, SelectedBuilding>,
    follow: ResMut<'w, FollowSelected>,
    food_events: ResMut<'w, FoodEvents>,
    proj_slots: ResMut<'w, ProjSlotAllocator>,
}

/// Clean up world when leaving Playing state.
fn game_cleanup_system(
    mut commands: Commands,
    npc_query: Query<Entity, With<NpcIndex>>,
    marker_query: Query<Entity, With<FarmReadyMarker>>,
    indicator_query: Query<Entity, With<SlotIndicator>>,
    ghost_query: Query<Entity, With<BuildGhost>>,
    tilemap_query: Query<Entity, With<bevy::sprite_render::TilemapChunk>>,
    mut world: CleanupWorld,
    mut debug: CleanupDebug,
    mut combat_log: ResMut<CombatLog>,
    mut ui_state: ResMut<UiState>,
    mut squad_state: ResMut<SquadState>,
    mut building_hp_render: ResMut<crate::resources::BuildingHpRender>,
    mut healing_cache: ResMut<HealingZoneCache>,
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

    // Reset world resources
    world.world_state.slot_alloc.reset();
    *world.world_state.world_data = Default::default();
    *world.food_storage = Default::default();
    *world.world_state.farm_states = Default::default();
    *world.faction_stats = Default::default();
    *world.gpu_state = Default::default();
    *world.game_time = Default::default();
    *world.world_state.grid = Default::default();
    world.tilemap_spawned.0 = false;
    *world.world_state.town_grids = Default::default();
    *world.build_menu_ctx = Default::default();
    *world.world_state.spawner_state = Default::default();
    *world.ai_state = Default::default();
    *world.gold_storage = Default::default();
    *world.world_state.building_hp = Default::default();
    *building_hp_render = Default::default();
    world.render_config.npc = Default::default();
    world.render_config.proj = Default::default();
    *world.npc_gpu_state = Default::default();
    *world.npc_visual_upload = Default::default();
    *world.proj_buffer_writes = Default::default();

    // Reset debug/tracking resources
    *debug.combat_debug = Default::default();
    *debug.health_debug = Default::default();
    *debug.kill_stats = Default::default();
    *world.world_state.building_occupancy = Default::default();
    *debug.raider_state = Default::default();
    *debug.npc_entity_map = Default::default();
    *debug.pop_stats = Default::default();

    // Reset UI state
    *combat_log = Default::default();
    *ui_state = Default::default();
    *squad_state = Default::default();
    *world.world_state.dirty = DirtyFlags::default();
    healing_cache.by_faction.clear();

    // Reset gameplay resources missed by original cleanup
    *gameplay.upgrades = Default::default();
    *gameplay.upgrade_queue = Default::default();
    *gameplay.policies = Default::default();
    *gameplay.auto_upgrade = Default::default();
    *gameplay.npc_logs = Default::default();
    *gameplay.npc_meta = Default::default();
    *gameplay.npcs_by_town = Default::default();
    *gameplay.migration = Default::default();
    *gameplay.tower_state = Default::default();
    *gameplay.selected_npc = Default::default();
    *gameplay.selected_building = Default::default();
    *gameplay.follow = Default::default();
    *gameplay.food_events = Default::default();
    *gameplay.proj_slots = Default::default();
    world.world_state.building_slots.clear();

    info!("Game cleanup complete");
}

