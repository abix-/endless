//! UI module — main menu, game startup, in-game HUD, and gameplay panels.

pub mod main_menu;
pub mod game_hud;
pub mod build_menu;
pub mod left_panel;

use std::collections::VecDeque;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiPrimaryContextPass, egui};
use rand::Rng;

use crate::AppState;
use crate::components::*;
use crate::messages::SpawnNpcMsg;
use crate::resources::*;
use crate::systems::{AiPlayerState, AiKind, AiPlayer, AiPersonality};
use crate::world::{self, WorldGenConfig};

/// Register all UI systems.
pub fn register_ui(app: &mut App) {
    // Global overlays (all states)
    app.add_systems(EguiPrimaryContextPass, game_hud::fps_display_system);

    // Main menu (egui)
    app.add_systems(EguiPrimaryContextPass,
        main_menu::main_menu_system.run_if(in_state(AppState::MainMenu)));

    // Game startup (world gen + NPC spawn)
    app.add_systems(OnEnter(AppState::Playing), game_startup_system);

    // Egui panels — ordered so top bar claims height first, then side panels, then bottom.
    // Top bar → left panel → bottom panel (inspector+log) + overlay → windows → pause overlay.
    app.add_systems(EguiPrimaryContextPass, (
        game_hud::top_bar_system,
        left_panel::left_panel_system,
        (game_hud::bottom_panel_system, game_hud::combat_log_system, game_hud::target_overlay_system, game_hud::squad_overlay_system),
        build_menu::build_menu_system,
        pause_menu_system,
    ).chain().run_if(in_state(AppState::Playing)));

    // Panel toggle keyboard shortcuts + ESC
    app.add_systems(Update, (
        ui_toggle_system,
        game_escape_system,
    ).run_if(in_state(AppState::Playing)));

    // Building slot click detection + visual indicators
    app.add_systems(Update, (
        slot_right_click_system,
        slot_double_click_system,
        draw_slot_indicators,
    ).run_if(in_state(AppState::Playing)));

    // Cleanup when leaving Playing
    app.add_systems(OnExit(AppState::Playing), game_cleanup_system);
}

/// Keyboard shortcuts for toggling UI panels.
fn ui_toggle_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut ui_state: ResMut<UiState>,
    mut follow: ResMut<FollowSelected>,
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
    miner_target: ResMut<'w, MinerTarget>,
    bgrid: ResMut<'w, world::BuildingSpatialGrid>,
}

/// Initialize the world and spawn NPCs when entering Playing state.
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
    info!("Game startup: generating world...");

    // Generate world (populates grid + world_data + farm_states + houses/barracks + town_grids)
    town_grids.grids.clear();
    world::generate_world(&config, &mut grid, &mut world_data, &mut farm_states, &mut extra.mine_states, &mut town_grids);

    // Build spatial grid for startup find calls
    extra.bgrid.rebuild(&world_data, grid.width as f32 * grid.cell_size);

    // Load saved policies for player's town
    let saved = crate::settings::load_settings();
    let town_idx = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);
    if town_idx < extra.policies.policies.len() {
        extra.policies.policies[town_idx] = saved.policy;
    }

    // Init NPC tracking per town
    let num_towns = world_data.towns.len();
    extra.npcs_by_town.0.resize(num_towns, Vec::new());
    food_storage.init(num_towns);
    extra.gold_storage.init(num_towns);
    extra.miner_target.targets = vec![0; num_towns];
    faction_stats.init(num_towns); // one per settlement (player + AI + camps)
    camp_state.init(num_towns, 10);

    // Sync GameConfig from WorldGenConfig
    game_config.farmers_per_town = config.farmers_per_town as i32;
    game_config.guards_per_town = config.guards_per_town as i32;
    game_config.raiders_per_camp = config.raiders_per_camp as i32;

    // Reset game time
    *game_time = GameTime::default();

    // Build SpawnerState from world gen Houses + Barracks + Tents
    spawner_state.0.clear();
    for house in world_data.houses.iter() {
        spawner_state.0.push(SpawnerEntry {
            building_kind: 0,
            town_idx: house.town_idx as i32,
            position: house.position,
            npc_slot: -1,
            respawn_timer: -1.0,
        });
    }
    for barracks in world_data.barracks.iter() {
        spawner_state.0.push(SpawnerEntry {
            building_kind: 1,
            town_idx: barracks.town_idx as i32,
            position: barracks.position,
            npc_slot: -1,
            respawn_timer: -1.0,
        });
    }
    for tent in world_data.tents.iter() {
        spawner_state.0.push(SpawnerEntry {
            building_kind: 2,
            town_idx: tent.town_idx as i32,
            position: tent.position,
            npc_slot: -1,
            respawn_timer: -1.0,
        });
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

        let town_faction = world_data.towns.get(entry.town_idx as usize)
            .map(|t| t.faction).unwrap_or(0);

        let (job, faction, work_x, work_y, starting_post, attack_type) = match entry.building_kind {
            0 => {
                // House -> Farmer: find nearest FREE farm in own town
                let farm = world::find_nearest_free(
                    entry.position, &extra.bgrid, world::BuildingKind::Farm, &startup_claimed, Some(entry.town_idx as u32),
                ).unwrap_or(entry.position);
                // Mark in local tracker so next farmer picks a different farm
                startup_claimed.claim(farm);
                (0, town_faction, farm.x, farm.y, -1, 0)
            }
            1 => {
                // Barracks -> Guard: find nearest guard post
                let post_idx = world::find_location_within_radius(
                    entry.position, &extra.bgrid, world::LocationKind::GuardPost, f32::MAX,
                ).map(|(idx, _)| idx as i32).unwrap_or(-1);
                (1, town_faction, -1.0, -1.0, post_idx, 1)
            }
            _ => {
                // Tent -> Raider: home = camp center
                let camp_faction = world_data.towns.get(town_data_idx)
                    .map(|t| t.faction).unwrap_or(1);
                (2, camp_faction, -1.0, -1.0, -1, 0)
            }
        };

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
) {
    if keys.just_pressed(KeyCode::Escape) {
        // Cancel squad target placement first
        if squad_state.placing_target {
            squad_state.placing_target = false;
            return;
        }
        if ui_state.build_menu_open {
            ui_state.build_menu_open = false;
        } else {
            ui_state.pause_menu_open = !ui_state.pause_menu_open;
            // Auto-pause when opening, unpause when closing
            game_time.paused = ui_state.pause_menu_open;
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
                    ui.add(egui::Slider::new(&mut settings.scroll_speed, 100.0..=2000.0)
                        .text("Scroll Speed"));

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
                    ui.checkbox(&mut settings.debug_coordinates, "NPC Coordinates");
                    ui.checkbox(&mut settings.debug_all_npcs, "All NPCs in Roster");
                    ui.checkbox(&mut settings.debug_readback, "GPU Readback");
                    ui.checkbox(&mut settings.debug_combat, "Combat Logging");
                    ui.checkbox(&mut settings.debug_spawns, "Spawn Logging");
                    ui.checkbox(&mut settings.debug_behavior, "Behavior Logging");
                    ui.checkbox(&mut settings.debug_profiler, "System Profiler");
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

/// Right-click on a town grid slot opens the build menu with appropriate options.
fn slot_right_click_system(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_query: Query<(&Transform, &Projection), With<crate::render::MainCamera>>,
    mut egui_contexts: bevy_egui::EguiContexts,
    world_data: Res<world::WorldData>,
    town_grids: Res<world::TownGrids>,
    grid: Res<world::WorldGrid>,
    mut build_ctx: ResMut<BuildMenuContext>,
    mut ui_state: ResMut<UiState>,
) {
    if !mouse.just_pressed(MouseButton::Right) { return; }

    // Don't steal clicks from egui — but only block on left panel, not the whole screen
    if let Ok(ctx) = egui_contexts.ctx_mut() {
        if ctx.wants_pointer_input() { return; }
    }

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((transform, projection)) = camera_query.single() else { return };

    let world_pos = screen_to_world(cursor_pos, transform, projection, window);

    // Find which town slot was clicked
    let Some(info) = world::find_town_slot(world_pos, &world_data.towns, &town_grids) else {
        return;
    };

    let slot_world_pos = world::town_grid_to_world(
        world_data.towns[info.town_data_idx].center,
        info.row, info.col,
    );

    // Check if slot has a building
    let (gc, gr) = grid.world_to_grid(slot_world_pos);
    let has_building = grid.cell(gc, gr)
        .map(|c| c.building.is_some())
        .unwrap_or(false);
    let is_fountain = grid.cell(gc, gr)
        .and_then(|c| c.building.as_ref())
        .map(|b| matches!(b, world::Building::Fountain { .. }))
        .unwrap_or(false);

    // Populate context and open build menu
    *build_ctx = BuildMenuContext {
        grid_idx: Some(info.grid_idx),
        town_data_idx: Some(info.town_data_idx),
        slot: Some((info.row, info.col)),
        slot_world_pos,
        screen_pos: [cursor_pos.x, cursor_pos.y],
        is_locked: info.slot_state == world::SlotState::Locked,
        has_building,
        is_fountain,
    };
    ui_state.build_menu_open = true;
}

/// Double-click on a locked adjacent slot to instantly unlock it.
/// TODO: Bevy lacks native double-click — needs Local<f64> timer. Using right-click menu for now.
fn slot_double_click_system() {}

/// Marker component for slot indicator sprite entities.
#[derive(Component)]
struct SlotIndicator;

/// Spawn/rebuild slot indicator sprites when the town grid or world grid changes.
/// Uses actual Sprite entities at z=-0.3 so they render between buildings and NPCs.
fn draw_slot_indicators(
    mut commands: Commands,
    existing: Query<Entity, With<SlotIndicator>>,
    world_data: Res<world::WorldData>,
    town_grids: Res<world::TownGrids>,
    grid: Res<world::WorldGrid>,
) {
    // Only rebuild when grid state changes
    if !town_grids.is_changed() && !grid.is_changed() { return; }

    // Despawn old indicators
    for entity in existing.iter() {
        commands.entity(entity).despawn();
    }

    // Only show indicators for the player's villager town (first grid)
    let Some(town_grid) = town_grids.grids.first() else { return };
    let town_data_idx = town_grid.town_data_idx;
    let Some(town) = world_data.towns.get(town_data_idx) else { return };
    let center = town.center;

    let green = Color::srgba(0.3, 0.7, 0.3, 0.5);
    let locked_color = Color::srgba(0.5, 0.5, 0.5, 0.3);
    let indicator_z = -0.3;
    let line_w = 2.0;
    let line_len = 10.0;
    let bracket_len = 5.0;
    let half_slot = crate::constants::TOWN_GRID_SPACING * 0.4;

    // Green "+" on empty unlocked slots
    for &(row, col) in &town_grid.unlocked {
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

    // Dim bracket corners on adjacent locked slots
    let adjacent = world::get_adjacent_locked_slots(town_grid);
    for (row, col) in adjacent {
        let raw = world::town_grid_to_world(center, row, col);
        let (gc, gr) = grid.world_to_grid(raw);
        let sp = grid.grid_to_world(gc, gr);

        // Each corner: one horizontal + one vertical bar
        let corners = [
            (sp.x - half_slot, sp.y + half_slot),  // top-left
            (sp.x + half_slot, sp.y + half_slot),  // top-right
            (sp.x - half_slot, sp.y - half_slot),  // bottom-left
            (sp.x + half_slot, sp.y - half_slot),  // bottom-right
        ];
        let h_dirs = [1.0, -1.0, 1.0, -1.0]; // horizontal bar direction from corner
        let v_dirs = [-1.0, -1.0, 1.0, 1.0];  // vertical bar direction from corner

        for i in 0..4 {
            let (cx, cy) = corners[i];
            // Horizontal bracket segment
            commands.spawn((
                Sprite { color: locked_color, custom_size: Some(Vec2::new(bracket_len, line_w)), ..default() },
                Transform::from_xyz(cx + h_dirs[i] * bracket_len * 0.5, cy, indicator_z),
                SlotIndicator,
            ));
            // Vertical bracket segment
            commands.spawn((
                Sprite { color: locked_color, custom_size: Some(Vec2::new(line_w, bracket_len)), ..default() },
                Transform::from_xyz(cx, cy + v_dirs[i] * bracket_len * 0.5, indicator_z),
                SlotIndicator,
            ));
        }
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
    miner_target: ResMut<'w, MinerTarget>,
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
    tilemap_query: Query<Entity, With<bevy::sprite_render::TilemapChunk>>,
    mut world: CleanupWorld,
    mut debug: CleanupDebug,
    mut combat_log: ResMut<CombatLog>,
    mut ui_state: ResMut<UiState>,
    mut squad_state: ResMut<SquadState>,
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
    *world.miner_target = Default::default();

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
