//! UI module — main menu, game startup, in-game HUD.

pub mod main_menu;
pub mod game_hud;

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

    // ESC to leave game
    app.add_systems(Update,
        game_escape_system.run_if(in_state(AppState::Playing)));

    // Cleanup when leaving Playing
    app.add_systems(OnExit(AppState::Playing), game_cleanup_system);
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
) {
    info!("Game startup: generating world...");

    // Generate world (populates grid + world_data + farm_states)
    world::generate_world(&config, &mut grid, &mut world_data, &mut farm_states);

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

    info!("Game cleanup complete");
}
