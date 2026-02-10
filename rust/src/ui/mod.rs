//! UI module — main menu, game startup, in-game HUD, and gameplay panels.

pub mod main_menu;
pub mod game_hud;
pub mod roster_panel;
pub mod combat_log;
pub mod build_menu;
pub mod upgrade_menu;
pub mod policies_panel;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;

use crate::AppState;
use crate::components::*;
use crate::messages::SpawnNpcMsg;
use crate::resources::*;
use crate::world::{self, WorldGenConfig};

/// Register all UI systems.
pub fn register_ui(app: &mut App) {
    // Main menu (egui)
    app.add_systems(EguiPrimaryContextPass,
        main_menu::main_menu_system.run_if(in_state(AppState::MainMenu)));

    // Game startup (world gen + NPC spawn)
    app.add_systems(OnEnter(AppState::Playing), game_startup_system);

    // In-game HUD
    app.add_systems(EguiPrimaryContextPass,
        game_hud::game_hud_system.run_if(in_state(AppState::Playing)));

    // Gameplay panels (egui, gated on Playing)
    app.add_systems(EguiPrimaryContextPass, (
        roster_panel::roster_panel_system,
        combat_log::combat_log_system,
        build_menu::build_menu_system,
        upgrade_menu::upgrade_menu_system,
        policies_panel::policies_panel_system,
    ).run_if(in_state(AppState::Playing)));

    // Panel toggle keyboard shortcuts
    app.add_systems(Update,
        ui_toggle_system.run_if(in_state(AppState::Playing)));

    // ESC to leave game
    app.add_systems(Update,
        game_escape_system.run_if(in_state(AppState::Playing)));

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
) {
    if keys.just_pressed(KeyCode::KeyR) {
        ui_state.roster_open = !ui_state.roster_open;
    }
    if keys.just_pressed(KeyCode::KeyL) {
        ui_state.combat_log_open = !ui_state.combat_log_open;
    }
    if keys.just_pressed(KeyCode::KeyB) {
        ui_state.build_menu_open = !ui_state.build_menu_open;
    }
    if keys.just_pressed(KeyCode::KeyU) {
        ui_state.upgrade_menu_open = !ui_state.upgrade_menu_open;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        ui_state.policies_open = !ui_state.policies_open;
    }
}

// ============================================================================
// GAME STARTUP
// ============================================================================

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
) {
    info!("Game startup: generating world...");

    // Generate world (populates grid + world_data + farm_states)
    world::generate_world(&config, &mut grid, &mut world_data, &mut farm_states);

    // Init town building grids (one per villager town)
    town_grids.grids.clear();
    for _ in 0..config.num_towns {
        town_grids.grids.push(world::TownGrid::new_base());
    }

    // Init economy resources
    let num_towns = world_data.towns.len();
    food_storage.init(num_towns);
    faction_stats.init(1 + config.num_towns); // faction 0 = villagers, 1+ = raider camps
    camp_state.init(config.num_towns, 10);

    // Sync GameConfig from WorldGenConfig
    game_config.farmers_per_town = config.farmers_per_town as i32;
    game_config.guards_per_town = config.guards_per_town as i32;
    game_config.raiders_per_camp = config.raiders_per_camp as i32;

    // Reset game time
    *game_time = GameTime::default();

    // Spawn NPCs per town (mirrors main.gd._spawn_npcs)
    let mut total = 0;
    for town_idx in 0..config.num_towns {
        let _villager_town = &world_data.towns[town_idx * 2]; // even indices = villager towns
        let raider_town = &world_data.towns[town_idx * 2 + 1]; // odd indices = raider camps

        // Collect beds and farms for this town
        let beds: Vec<_> = world_data.beds.iter()
            .filter(|b| b.town_idx == town_idx as u32)
            .map(|b| b.position)
            .collect();
        let farms: Vec<_> = world_data.farms.iter()
            .filter(|f| f.town_idx == town_idx as u32)
            .map(|f| f.position)
            .collect();
        let posts: Vec<_> = world_data.guard_posts.iter()
            .filter(|g| g.town_idx == town_idx as u32)
            .collect();

        if beds.is_empty() || farms.is_empty() {
            warn!("Town {} has no beds or farms, skipping NPC spawn", town_idx);
            continue;
        }

        // Farmers
        for i in 0..config.farmers_per_town {
            let Some(slot) = slots.alloc() else { break };
            let bed = beds[i % beds.len()];
            let farm = farms[i % farms.len()];
            spawn_writer.write(SpawnNpcMsg {
                slot_idx: slot,
                x: bed.x + (i as f32 * 3.0 % 30.0) - 15.0,
                y: bed.y + (i as f32 * 7.0 % 30.0) - 15.0,
                job: 0,
                faction: 0,
                town_idx: (town_idx * 2) as i32,
                home_x: bed.x,
                home_y: bed.y,
                work_x: farm.x,
                work_y: farm.y,
                starting_post: -1,
                attack_type: 0,
            });
            total += 1;
        }

        // Guards
        let post_count = posts.len().max(1);
        for i in 0..config.guards_per_town {
            let Some(slot) = slots.alloc() else { break };
            let bed = beds[i % beds.len()];
            spawn_writer.write(SpawnNpcMsg {
                slot_idx: slot,
                x: bed.x + (i as f32 * 5.0 % 30.0) - 15.0,
                y: bed.y + (i as f32 * 11.0 % 30.0) - 15.0,
                job: 1,
                faction: 0,
                town_idx: (town_idx * 2) as i32,
                home_x: bed.x,
                home_y: bed.y,
                work_x: -1.0,
                work_y: -1.0,
                starting_post: (i % post_count) as i32,
                attack_type: 1,
            });
            total += 1;
        }

        // Raiders
        let camp_pos = raider_town.center;
        let raider_town_idx = (town_idx * 2 + 1) as i32;
        for i in 0..config.raiders_per_camp {
            let Some(slot) = slots.alloc() else { break };
            spawn_writer.write(SpawnNpcMsg {
                slot_idx: slot,
                x: camp_pos.x + (i as f32 * 13.0 % 160.0) - 80.0,
                y: camp_pos.y + (i as f32 * 17.0 % 160.0) - 80.0,
                job: 2,
                faction: (town_idx + 1) as i32,
                town_idx: raider_town_idx,
                home_x: camp_pos.x,
                home_y: camp_pos.y,
                work_x: -1.0,
                work_y: -1.0,
                starting_post: -1,
                attack_type: 1,
            });
            total += 1;
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

/// ESC returns to main menu. Space/+/- control time.
fn game_escape_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<AppState>>,
    mut game_time: ResMut<GameTime>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        next_state.set(AppState::MainMenu);
    }
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

    // Don't steal clicks from egui
    if let Ok(ctx) = egui_contexts.ctx_mut() {
        if ctx.wants_pointer_input() || ctx.is_pointer_over_area() { return; }
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
        is_locked: info.slot_state == world::SlotState::Locked,
        has_building,
        is_fountain,
    };
    ui_state.build_menu_open = true;
}

/// Double-click on a locked adjacent slot to instantly unlock it.
/// TODO: Bevy lacks native double-click — needs Local<f64> timer. Using right-click menu for now.
fn slot_double_click_system() {}

/// Draw visual indicators on town grid slots using Bevy gizmos.
/// Green "+" on empty unlocked slots, dim brackets on adjacent locked slots,
/// gold ring around town expanding to farthest unlocked slot.
fn draw_slot_indicators(
    mut gizmos: Gizmos,
    world_data: Res<world::WorldData>,
    town_grids: Res<world::TownGrids>,
    grid: Res<world::WorldGrid>,
) {
    let player_town = 0usize; // First villager town is player-controlled

    let Some(town_grid) = town_grids.grids.get(player_town) else { return };
    let town_data_idx = player_town * 2;
    let Some(town) = world_data.towns.get(town_data_idx) else { return };
    let center = town.center;

    let green = Color::srgba(0.5, 0.8, 0.5, 0.6);
    let locked_color = Color::srgba(0.6, 0.6, 0.6, 0.4);
    let gold = Color::srgba(1.0, 0.85, 0.3, 0.8);
    let size = 6.0;
    let half_slot = crate::constants::TOWN_GRID_SPACING * 0.4;
    let corner = 4.0;

    // Track farthest unlocked slot for gold ring
    let mut max_dist: f32 = 60.0;

    // Draw green "+" on empty unlocked slots
    for &(row, col) in &town_grid.unlocked {
        // Skip fountain
        if row == 0 && col == 0 { continue; }

        let slot_pos = world::town_grid_to_world(center, row, col);
        let dist = center.distance(slot_pos) + crate::constants::TOWN_GRID_SPACING;
        if dist > max_dist { max_dist = dist; }

        // Check if slot has a building
        let (gc, gr) = grid.world_to_grid(slot_pos);
        let has_building = grid.cell(gc, gr)
            .map(|c| c.building.is_some())
            .unwrap_or(false);

        if !has_building {
            // Green "+"
            let p = Vec3::new(slot_pos.x, slot_pos.y, 0.5);
            gizmos.line_2d(
                Vec2::new(p.x - size, p.y),
                Vec2::new(p.x + size, p.y),
                green,
            );
            gizmos.line_2d(
                Vec2::new(p.x, p.y - size),
                Vec2::new(p.x, p.y + size),
                green,
            );
        }
    }

    // Draw dim brackets on adjacent locked slots
    let adjacent = world::get_adjacent_locked_slots(town_grid);
    for (row, col) in adjacent {
        let sp = world::town_grid_to_world(center, row, col);

        // Top-left bracket
        gizmos.line_2d(Vec2::new(sp.x - half_slot, sp.y - half_slot), Vec2::new(sp.x - half_slot + corner, sp.y - half_slot), locked_color);
        gizmos.line_2d(Vec2::new(sp.x - half_slot, sp.y - half_slot), Vec2::new(sp.x - half_slot, sp.y - half_slot + corner), locked_color);
        // Top-right bracket
        gizmos.line_2d(Vec2::new(sp.x + half_slot, sp.y - half_slot), Vec2::new(sp.x + half_slot - corner, sp.y - half_slot), locked_color);
        gizmos.line_2d(Vec2::new(sp.x + half_slot, sp.y - half_slot), Vec2::new(sp.x + half_slot, sp.y - half_slot + corner), locked_color);
        // Bottom-left bracket
        gizmos.line_2d(Vec2::new(sp.x - half_slot, sp.y + half_slot), Vec2::new(sp.x - half_slot + corner, sp.y + half_slot), locked_color);
        gizmos.line_2d(Vec2::new(sp.x - half_slot, sp.y + half_slot), Vec2::new(sp.x - half_slot, sp.y + half_slot - corner), locked_color);
        // Bottom-right bracket
        gizmos.line_2d(Vec2::new(sp.x + half_slot, sp.y + half_slot), Vec2::new(sp.x + half_slot - corner, sp.y + half_slot), locked_color);
        gizmos.line_2d(Vec2::new(sp.x + half_slot, sp.y + half_slot), Vec2::new(sp.x + half_slot, sp.y + half_slot - corner), locked_color);
    }

    // Gold ring around town
    let fountain_pos = world::town_grid_to_world(center, 0, 0);
    gizmos.circle_2d(Isometry2d::from_translation(fountain_pos), max_dist, gold);
}

// ============================================================================
// GAME CLEANUP
// ============================================================================

// SystemParam bundles to keep cleanup under 16-param limit
#[derive(SystemParam)]
struct CleanupWorld<'w> {
    npc_count: ResMut<'w, NpcCount>,
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
}

#[derive(SystemParam)]
struct CleanupDebug<'w> {
    combat_debug: ResMut<'w, CombatDebug>,
    health_debug: ResMut<'w, HealthDebug>,
    kill_stats: ResMut<'w, KillStats>,
    bed_occ: ResMut<'w, world::BedOccupancy>,
    farm_occ: ResMut<'w, world::FarmOccupancy>,
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
    tilemap_query: Query<Entity, With<bevy::sprite_render::TilemapChunk>>,
    mut world: CleanupWorld,
    mut debug: CleanupDebug,
    mut combat_log: ResMut<CombatLog>,
    mut ui_state: ResMut<UiState>,
) {
    // Despawn all entities
    for entity in npc_query.iter() {
        commands.entity(entity).despawn();
    }
    for entity in marker_query.iter() {
        commands.entity(entity).despawn();
    }
    for entity in tilemap_query.iter() {
        commands.entity(entity).despawn();
    }

    // Reset world resources
    *world.npc_count = Default::default();
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

    // Reset debug/tracking resources
    *debug.combat_debug = Default::default();
    *debug.health_debug = Default::default();
    *debug.kill_stats = Default::default();
    *debug.bed_occ = Default::default();
    *debug.farm_occ = Default::default();
    *debug.camp_state = Default::default();
    *debug.raid_queue = Default::default();
    *debug.npc_entity_map = Default::default();
    *debug.pop_stats = Default::default();

    // Reset UI state
    *combat_log = Default::default();
    *ui_state = Default::default();

    info!("Game cleanup complete");
}
