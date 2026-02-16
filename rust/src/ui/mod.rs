//! UI module — main menu, game startup, in-game HUD, and gameplay panels.

pub mod main_menu;
pub mod game_hud;
pub mod build_menu;
pub mod left_panel;

use std::collections::VecDeque;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContextSettings, EguiPrimaryContextPass, egui};
use rand::Rng;

use crate::AppState;
use crate::constants::TOWN_GRID_SPACING;
use crate::components::*;
use crate::messages::SpawnNpcMsg;
use crate::resources::*;
use crate::systems::{AiPlayerState, AiKind, AiPlayer, AiPersonality};
use crate::world::{self, WorldGenConfig};

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
    app.add_systems(EguiPrimaryContextPass, game_hud::fps_display_system);

    // Main menu (egui)
    app.add_systems(EguiPrimaryContextPass,
        main_menu::main_menu_system.run_if(in_state(AppState::MainMenu)));

    // Game startup: load from save (if requested) then world gen (if not loaded)
    app.add_systems(OnEnter(AppState::Playing), (game_load_system, game_startup_system).chain());

    // Egui panels — ordered so top bar claims height first, then side panels, then bottom.
    // Top bar → left panel → bottom panel (inspector+log) + overlay → windows → pause overlay.
    app.add_systems(EguiPrimaryContextPass, (
        game_hud::top_bar_system,
        left_panel::left_panel_system,
        (game_hud::bottom_panel_system, game_hud::combat_log_system, game_hud::selection_overlay_system, game_hud::target_overlay_system, game_hud::squad_overlay_system),
        build_menu::build_menu_system,
        pause_menu_system,
        game_hud::save_toast_system,
    ).chain().run_if(in_state(AppState::Playing)));

    // Panel toggle keyboard shortcuts + ESC
    app.add_systems(Update, (
        ui_toggle_system,
        game_escape_system,
    ).run_if(in_state(AppState::Playing)));

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
fn ui_toggle_system(
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
        ui_state.toggle_left_tab(LeftPanelTab::Intel);
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
    farm_occupancy: ResMut<'w, world::BuildingOccupancy>,
    npcs_by_town: ResMut<'w, NpcsByTownCache>,
    ai_state: ResMut<'w, AiPlayerState>,
    combat_log: ResMut<'w, CombatLog>,
    mine_states: ResMut<'w, MineStates>,
    gold_storage: ResMut<'w, GoldStorage>,
    bgrid: ResMut<'w, world::BuildingSpatialGrid>,
    auto_upgrade: ResMut<'w, AutoUpgrade>,
    building_hp: ResMut<'w, BuildingHpState>,
    patrols_dirty: ResMut<'w, PatrolsDirty>,
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
) {
    if !save_request.load_on_enter { return; }
    save_request.load_on_enter = false;

    let save = match crate::save::read_save() {
        Ok(data) => data,
        Err(e) => {
            error!("Load from menu failed: {e}");
            toast.message = format!("Load failed: {e}");
            toast.timer = 3.0;
            return;
        }
    };

    info!("Loading save from menu: {} NPCs, {} towns", save.npcs.len(), save.towns.len());

    // Apply save data to all game resources
    crate::save::apply_save(
        &save,
        &mut ws.grid, &mut ws.world_data, &mut ws.town_grids, &mut ws.game_time,
        &mut ws.food_storage, &mut ws.gold_storage, &mut ws.farm_states, &mut ws.mine_states,
        &mut ws.spawner_state, &mut ws.building_hp, &mut ws.upgrades, &mut ws.policies,
        &mut ws.auto_upgrade, &mut ws.squad_state, &mut ws.guard_post_state, &mut fs.camp_state,
        &mut fs.faction_stats, &mut fs.kill_stats, &mut fs.ai_state,
        &mut tracking.npcs_by_town, &mut tracking.slots,
    );

    // Rebuild spatial grid
    tracking.bgrid.rebuild(&ws.world_data, ws.grid.width as f32 * ws.grid.cell_size);
    tracking.patrols_dirty.dirty = true;

    // Spawn NPC entities from save data
    crate::save::spawn_npcs_from_save(
        &save, &mut commands,
        &mut tracking.npc_map, &mut tracking.pop_stats, &mut tracking.npc_meta,
        &mut tracking.npcs_by_town, &mut gpu_updates,
        &ws.world_data, &combat_config, &ws.upgrades,
    );

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
    mut grid: ResMut<world::WorldGrid>,
    mut world_data: ResMut<world::WorldData>,
    mut farm_states: ResMut<FarmStates>,
    mut food_storage: ResMut<FoodStorage>,
    mut faction_stats: ResMut<FactionStats>,
    mut camp_state: ResMut<CampState>,
    mut game_config: ResMut<GameConfig>,
    mut slots: ResMut<SlotAllocator>,
    mut spawn_writer: MessageWriter<SpawnNpcMsg>,
    mut game_time: ResMut<GameTime>,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut town_grids: ResMut<world::TownGrids>,
    mut spawner_state: ResMut<SpawnerState>,
    mut extra: StartupExtra,
) {
    // If game_load_system already populated the world, skip world gen.
    // The flag was cleared by game_load_system, but we can detect load happened
    // by checking if the world grid is already populated.
    if grid.cells.len() > 0 {
        info!("Game startup: skipping world gen (loaded from save)");
        return;
    }

    info!("Game startup: generating world...");

    // Generate world (populates grid + world_data + farm_states + houses/barracks + town_grids)
    town_grids.grids.clear();
    world::generate_world(&config, &mut grid, &mut world_data, &mut farm_states, &mut extra.mine_states, &mut town_grids);

    // Build spatial grid for startup find calls
    extra.bgrid.rebuild(&world_data, grid.width as f32 * grid.cell_size);

    // Load saved policies + auto-upgrade flags for player's town
    let saved = crate::settings::load_settings();
    let town_idx = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);
    if town_idx < extra.policies.policies.len() {
        extra.policies.policies[town_idx] = saved.policy;
    }
    if !saved.auto_upgrades.is_empty() && town_idx < extra.auto_upgrade.flags.len() {
        let flags = &mut extra.auto_upgrade.flags[town_idx];
        for (i, &val) in saved.auto_upgrades.iter().enumerate().take(flags.len()) {
            flags[i] = val;
        }
    }

    // Init NPC tracking per town
    let num_towns = world_data.towns.len();
    extra.npcs_by_town.0.resize(num_towns, Vec::new());
    food_storage.init(num_towns);
    extra.gold_storage.init(num_towns);
    faction_stats.init(num_towns); // one per settlement (player + AI + camps)
    camp_state.init(num_towns, 10);

    // Sync GameConfig from WorldGenConfig
    game_config.farmers_per_town = config.farmers_per_town as i32;
    game_config.archers_per_town = config.archers_per_town as i32;
    game_config.raiders_per_camp = config.raiders_per_camp as i32;

    // Reset game time
    *game_time = GameTime::default();

    // Build SpawnerState from world gen Houses + Barracks + Tents
    spawner_state.0.clear();
    for house in world_data.farmer_homes.iter() {
        world::register_spawner(&mut spawner_state, world::Building::FarmerHome { town_idx: 0 },
            house.town_idx as i32, house.position, -1.0);
    }
    for barracks in world_data.archer_homes.iter() {
        world::register_spawner(&mut spawner_state, world::Building::ArcherHome { town_idx: 0 },
            barracks.town_idx as i32, barracks.position, -1.0);
    }
    for tent in world_data.tents.iter() {
        world::register_spawner(&mut spawner_state, world::Building::Tent { town_idx: 0 },
            tent.town_idx as i32, tent.position, -1.0);
    }
    for ms in world_data.miner_homes.iter() {
        world::register_spawner(&mut spawner_state, world::Building::MinerHome { town_idx: 0 },
            ms.town_idx as i32, ms.position, -1.0);
    }

    // Initialize building HP for all world-gen buildings
    {
        use crate::constants::*;
        let hp = &mut extra.building_hp;
        **hp = BuildingHpState::default();
        for _ in &world_data.guard_posts { hp.guard_posts.push(GUARD_POST_HP); }
        for _ in &world_data.farmer_homes { hp.farmer_homes.push(FARMER_HOME_HP); }
        for _ in &world_data.archer_homes { hp.archer_homes.push(ARCHER_HOME_HP); }
        for _ in &world_data.tents { hp.tents.push(TENT_HP); }
        for _ in &world_data.miner_homes { hp.miner_homes.push(MINER_HOME_HP); }
        for _ in &world_data.farms { hp.farms.push(FARM_HP); }
        for _ in &world_data.towns { hp.towns.push(TOWN_HP); }
        for _ in &world_data.beds { hp.beds.push(BED_HP); }
        for _ in &world_data.gold_mines { hp.gold_mines.push(GOLD_MINE_HP); }
    }

    // Reset farm occupancy for fresh game
    extra.farm_occupancy.clear();

    // Local tracker to prevent two farmers picking the same farm at startup.
    // NOT written to BuildingOccupancy — the arrival handler will populate that when farmers arrive.
    let mut startup_claimed = world::BuildingOccupancy::default();

    // Spawn 1 NPC per building spawner (instant, no timer)
    let mut total = 0;
    for entry in spawner_state.0.iter_mut() {
        let Some(slot) = slots.alloc() else { break };
        let town_data_idx = entry.town_idx as usize;

        let (job, faction, work_x, work_y, starting_post, attack_type, _, _) =
            world::resolve_spawner_npc(entry, &world_data.towns, &extra.bgrid, &startup_claimed);
        // Mark farm as claimed so next farmer picks a different one
        if work_x > 0.0 { startup_claimed.claim(Vec2::new(work_x, work_y)); }

        // Home = spawner building position (house/barracks/tent)
        let (home_x, home_y) = (entry.position.x, entry.position.y);

        spawn_writer.write(SpawnNpcMsg {
            slot_idx: slot,
            x: entry.position.x,
            y: entry.position.y,
            job,
            faction,
            town_idx: town_data_idx as i32,
            home_x,
            home_y,
            work_x,
            work_y,
            starting_post,
            attack_type,
        });
        entry.npc_slot = slot as i32;
        total += 1;
    }

    // Populate AI players (non-player factions) with random personalities
    extra.ai_state.players.clear();
    let personalities = [AiPersonality::Aggressive, AiPersonality::Balanced, AiPersonality::Economic];
    let mut rng = rand::rng();
    for (grid_idx, town_grid) in town_grids.grids.iter().enumerate() {
        let tdi = town_grid.town_data_idx;
        if let Some(town) = world_data.towns.get(tdi) {
            if town.faction > 0 {
                let kind = if town.sprite_type == 1 { AiKind::Raider } else { AiKind::Builder };
                let personality = personalities[rng.random_range(0..personalities.len())];
                // Set town policies based on personality
                if let Some(policy) = extra.policies.policies.get_mut(tdi) {
                    *policy = personality.default_policies();
                }
                extra.ai_state.players.push(AiPlayer { town_data_idx: tdi, grid_idx, kind, personality, last_actions: VecDeque::new() });
                // Log AI player joining
                extra.combat_log.push(CombatEventKind::Ai, 1, 6, 0,
                    format!("{} [{}] joined the game", town.name, personality.name()));
            }
        }
    }

    // Center camera on first town
    if let Some(first_town) = world_data.towns.first() {
        if let Ok(mut transform) = camera_query.single_mut() {
            transform.translation.x = first_town.center.x;
            transform.translation.y = first_town.center.y;
        }
    }

    extra.patrols_dirty.dirty = true;

    info!("Game startup complete: {} NPCs spawned across {} towns",
        total, config.num_towns);
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
        // Cancel squad target placement first
        if squad_state.placing_target {
            squad_state.placing_target = false;
            return;
        }
        if build_ctx.selected_build.is_some() {
            build_ctx.selected_build = None;
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
) -> Result {
    if !ui_state.pause_menu_open { return Ok(()); }

    let ctx = contexts.ctx_mut()?;

    // Dim background
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("pause_dim"))
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (response, painter) = ui.allocate_painter(screen.size(), egui::Sense::click());
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
                        settings.debug_spawns, settings.debug_behavior, settings.debug_profiler);
                    ui.checkbox(&mut settings.debug_coordinates, "NPC Coordinates");
                    ui.checkbox(&mut settings.debug_all_npcs, "All NPCs in Roster");
                    ui.checkbox(&mut settings.debug_readback, "GPU Readback");
                    ui.checkbox(&mut settings.debug_combat, "Combat Logging");
                    ui.checkbox(&mut settings.debug_spawns, "Spawn Logging");
                    ui.checkbox(&mut settings.debug_behavior, "Behavior Logging");
                    ui.checkbox(&mut settings.debug_profiler, "System Profiler");
                    let now_debug = (settings.debug_coordinates, settings.debug_all_npcs,
                        settings.debug_readback, settings.debug_combat,
                        settings.debug_spawns, settings.debug_behavior, settings.debug_profiler);
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

/// Right-click cancels active build placement.
fn slot_right_click_system(
    mouse: Res<ButtonInput<MouseButton>>,
    mut build_ctx: ResMut<BuildMenuContext>,
) {
    if !mouse.just_pressed(MouseButton::Right) { return; }
    build_ctx.selected_build = None;
}

/// Left-click places the currently selected building into any valid slot in buildable area.
fn build_place_click_system(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_query: Query<(&Transform, &Projection), With<crate::render::MainCamera>>,
    mut egui_contexts: bevy_egui::EguiContexts,
    mut build_ctx: ResMut<BuildMenuContext>,
    mut grid: ResMut<world::WorldGrid>,
    mut world_data: ResMut<world::WorldData>,
    mut farm_states: ResMut<FarmStates>,
    mut food_storage: ResMut<FoodStorage>,
    town_grids: Res<world::TownGrids>,
    mut spawner_state: ResMut<SpawnerState>,
    mut building_hp: ResMut<BuildingHpState>,
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    mut patrols_dirty: ResMut<PatrolsDirty>,
    difficulty: Res<Difficulty>,
) {
    let Some(kind) = build_ctx.selected_build else { return };
    if !mouse.just_pressed(MouseButton::Left) { return; }

    if let Ok(ctx) = egui_contexts.ctx_mut() {
        if ctx.wants_pointer_input() || ctx.is_pointer_over_area() {
            return;
        }
    }

    let Some(town_data_idx) = build_ctx.town_data_idx else { return };
    let Some(town) = world_data.towns.get(town_data_idx) else { return };
    let center = town.center;
    let town_name = town.name.clone();
    let town_idx = town_data_idx as u32;

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((transform, projection)) = camera_query.single() else { return };
    let world_pos = screen_to_world(cursor_pos, transform, projection, window);
    let (row, col) = world::world_to_town_grid(center, world_pos);
    let slot_pos = world::town_grid_to_world(center, row, col);
    build_ctx.hover_world_pos = slot_pos;

    let (gc, gr) = grid.world_to_grid(slot_pos);

    // Destroy mode: remove building at clicked cell
    if kind == BuildKind::Destroy {
        let cell_building = grid.cell(gc, gr).and_then(|c| c.building);
        let is_destructible = cell_building
            .as_ref()
            .map(|b| !matches!(b, world::Building::Fountain { .. } | world::Building::Camp { .. }))
            .unwrap_or(false);
        if !is_destructible { return; }
        let is_guard_post = matches!(cell_building, Some(world::Building::GuardPost { .. }));

        let _ = world::destroy_building(
            &mut grid, &mut world_data, &mut farm_states,
            &mut spawner_state, &mut building_hp,
            &mut combat_log, &game_time,
            row, col, center,
            &format!("Destroyed building at ({},{}) in {}", row, col, town_name),
        );
        if is_guard_post { patrols_dirty.dirty = true; }
        return;
    }

    // Build mode: place building on empty slot
    let Some(town_grid) = town_grids.grids.iter().find(|tg| tg.town_data_idx == town_data_idx) else { return };
    if !world::is_slot_buildable(town_grid, row, col) { return; }
    if row == 0 && col == 0 { return; }
    if grid.cell(gc, gr).map(|c| c.building.is_some()) != Some(false) { return; }

    let cost = crate::constants::building_cost(kind, *difficulty);
    let (building, label) = match kind {
        BuildKind::Farm => (world::Building::Farm { town_idx }, "farm"),
        BuildKind::GuardPost => {
            let existing_posts = world_data.guard_posts.iter()
                .filter(|g| g.town_idx == town_idx && g.position.x > -9000.0)
                .count() as u32;
            (world::Building::GuardPost { town_idx, patrol_order: existing_posts }, "guard post")
        }
        BuildKind::FarmerHome => (world::Building::FarmerHome { town_idx }, "house"),
        BuildKind::ArcherHome => (world::Building::ArcherHome { town_idx }, "barracks"),
        BuildKind::Tent => (world::Building::Tent { town_idx }, "tent"),
        BuildKind::MinerHome => (world::Building::MinerHome { town_idx }, "mine shaft"),
        BuildKind::Destroy => unreachable!(),
    };

    let food = food_storage.food.get(town_data_idx).copied().unwrap_or(0);
    if food < cost { return; }

    if !world::build_and_pay(
        &mut grid, &mut world_data, &mut farm_states,
        &mut food_storage, &mut spawner_state, &mut building_hp,
        building, town_data_idx,
        row, col, center, cost,
    ) { return; }

    if kind == BuildKind::GuardPost { patrols_dirty.dirty = true; }

    combat_log.push(
        CombatEventKind::Harvest,
        game_time.day(), game_time.hour(), game_time.minute(),
        format!("Built {} at ({},{}) in {}", label, row, col, town_name),
    );
}

/// Marker component for slot indicator sprite entities.
#[derive(Component)]
struct SlotIndicator;

/// Marker for the build ghost preview sprite.
#[derive(Component)]
struct BuildGhost;

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
    mut ghost_query: Query<(Entity, &mut Transform, &mut Sprite), (With<BuildGhost>, Without<crate::render::MainCamera>)>,
) {
    let has_selection = build_ctx.selected_build.is_some();

    // Despawn ghost if no selection
    if !has_selection {
        build_ctx.show_cursor_hint = true;
        for (entity, _, _) in ghost_query.iter() {
            commands.entity(entity).despawn();
        }
        return;
    }

    let kind = build_ctx.selected_build.unwrap();

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
            return;
        }
    }

    let Ok((cam_transform, projection)) = camera_query.single() else { return };
    let world_pos = screen_to_world(cursor_pos, cam_transform, projection, window);

    // Snap to town grid
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
    let is_fountain = cell
        .and_then(|c| c.building.as_ref())
        .map(|b| matches!(b, world::Building::Fountain { .. } | world::Building::Camp { .. }))
        .unwrap_or(false);

    let town_grid = town_grids.grids.iter().find(|tg| tg.town_data_idx == town_data_idx);
    let in_bounds = town_grid
        .map(|tg| world::is_slot_buildable(tg, row, col))
        .unwrap_or(false);
    let is_center = row == 0 && col == 0;

    let (valid, visible) = if kind == BuildKind::Destroy {
        // Destroy: valid over destructible buildings, invisible over empty/fountain
        (has_building && !is_fountain, has_building && !is_fountain)
    } else {
        // Build: valid on empty buildable non-center slots
        let v = in_bounds && !is_center && !has_building;
        (v, in_bounds && !is_center)
    };
    // Hide mouse-follow sprite when we're snapped to a valid build slot.
    build_ctx.show_cursor_hint = kind == BuildKind::Destroy || !valid;

    let is_destroy = kind == BuildKind::Destroy;
    let color = if !visible {
        Color::NONE
    } else if is_destroy {
        if valid { Color::srgba(0.8, 0.2, 0.2, 0.6) } else { Color::NONE }
    } else if valid {
        Color::srgba(1.0, 1.0, 1.0, 0.7)
    } else {
        Color::srgba(0.8, 0.2, 0.2, 0.5)
    };

    let snapped = grid.grid_to_world(gc, gr);
    let ghost_z = 0.5;
    let ghost_image = if is_destroy {
        Handle::default()
    } else {
        build_ctx.ghost_sprites.get(&kind).cloned().unwrap_or_default()
    };

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

    // Only show indicators when a build type is selected (not Destroy)
    let show = build_ctx.selected_build
        .map(|k| k != BuildKind::Destroy)
        .unwrap_or(false);
    if !show { return; }

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
    mut grid: ResMut<world::WorldGrid>,
    mut world_data: ResMut<world::WorldData>,
    mut farm_states: ResMut<FarmStates>,
    mut spawner_state: ResMut<SpawnerState>,
    mut building_hp: ResMut<BuildingHpState>,
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    mut selected_building: ResMut<SelectedBuilding>,
    mut patrols_dirty: ResMut<PatrolsDirty>,
) {
    let Some((col, row)) = request.0.take() else { return };

    let cell = grid.cell(col, row);
    let cell_building = cell.and_then(|c| c.building);
    let is_destructible = cell_building
        .as_ref()
        .map(|b| !matches!(b, world::Building::Fountain { .. } | world::Building::Camp { .. }))
        .unwrap_or(false);
    if !is_destructible { return; }
    let is_guard_post = matches!(cell_building, Some(world::Building::GuardPost { .. }));

    // Find which town this building belongs to, derive town center
    let town_idx = cell_building
        .as_ref()
        .map(|b| crate::ui::game_hud::building_town_idx(b) as usize)
        .unwrap_or(0);
    let center = world_data.towns.get(town_idx)
        .map(|t| t.center)
        .unwrap_or_default();
    let town_name = world_data.towns.get(town_idx)
        .map(|t| t.name.clone())
        .unwrap_or_default();

    let world_pos = grid.grid_to_world(col, row);
    let (trow, tcol) = world::world_to_town_grid(center, world_pos);

    if world::destroy_building(
        &mut grid, &mut world_data, &mut farm_states,
        &mut spawner_state, &mut building_hp,
        &mut combat_log, &game_time,
        trow, tcol, center,
        &format!("Destroyed building in {}", town_name),
    ).is_ok() {
        selected_building.active = false;
        if is_guard_post { patrols_dirty.dirty = true; }
    }
}

// ============================================================================
// GAME CLEANUP
// ============================================================================

// SystemParam bundles to keep cleanup under 16-param limit
#[derive(SystemParam)]
struct CleanupWorld<'w> {
    slot_alloc: ResMut<'w, SlotAllocator>,
    world_data: ResMut<'w, world::WorldData>,
    food_storage: ResMut<'w, FoodStorage>,
    farm_states: ResMut<'w, FarmStates>,
    faction_stats: ResMut<'w, FactionStats>,
    gpu_state: ResMut<'w, GpuReadState>,
    game_time: ResMut<'w, GameTime>,
    grid: ResMut<'w, world::WorldGrid>,
    tilemap_spawned: ResMut<'w, crate::render::TilemapSpawned>,
    town_grids: ResMut<'w, world::TownGrids>,
    build_menu_ctx: ResMut<'w, BuildMenuContext>,
    spawner_state: ResMut<'w, SpawnerState>,
    ai_state: ResMut<'w, AiPlayerState>,
    mine_states: ResMut<'w, MineStates>,
    gold_storage: ResMut<'w, GoldStorage>,
    building_hp: ResMut<'w, BuildingHpState>,
}

#[derive(SystemParam)]
struct CleanupDebug<'w> {
    combat_debug: ResMut<'w, CombatDebug>,
    health_debug: ResMut<'w, HealthDebug>,
    kill_stats: ResMut<'w, KillStats>,
    farm_occ: ResMut<'w, world::BuildingOccupancy>,
    camp_state: ResMut<'w, CampState>,
    raid_queue: ResMut<'w, RaidQueue>,
    npc_entity_map: ResMut<'w, NpcEntityMap>,
    pop_stats: ResMut<'w, PopulationStats>,
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
    world.slot_alloc.reset();
    *world.world_data = Default::default();
    *world.food_storage = Default::default();
    *world.farm_states = Default::default();
    *world.faction_stats = Default::default();
    *world.gpu_state = Default::default();
    *world.game_time = Default::default();
    *world.grid = Default::default();
    world.tilemap_spawned.0 = false;
    *world.town_grids = Default::default();
    *world.build_menu_ctx = Default::default();
    *world.spawner_state = Default::default();
    *world.ai_state = Default::default();
    *world.mine_states = Default::default();
    *world.gold_storage = Default::default();
    *world.building_hp = Default::default();
    *building_hp_render = Default::default();

    // Reset debug/tracking resources
    *debug.combat_debug = Default::default();
    *debug.health_debug = Default::default();
    *debug.kill_stats = Default::default();
    *debug.farm_occ = Default::default();
    *debug.camp_state = Default::default();
    *debug.raid_queue = Default::default();
    *debug.npc_entity_map = Default::default();
    *debug.pop_stats = Default::default();

    // Reset UI state
    *combat_log = Default::default();
    *ui_state = Default::default();
    *squad_state = Default::default();

    info!("Game cleanup complete");
}
