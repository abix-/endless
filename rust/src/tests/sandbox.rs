//! Sandbox Test — human player testing scene.
//! 1 player town + 1 AI builder town, 100K food + 100K gold, no raiders.
//! Auto-completes after setup so the scene runs indefinitely.

use bevy::prelude::*;

use crate::resources::*;
use crate::world::{self, WorldGenStyle};

use super::{BuildingInitParams, TestScenarioSetup, TestState};

pub(super) fn setup(
    mut commands: Commands,
    mut world_data: ResMut<world::WorldData>,
    mut world_grid: ResMut<world::WorldGrid>,
    mut config: ResMut<world::WorldGenConfig>,
    mut faction_stats: ResMut<FactionStats>,

    mut slot_alloc: ResMut<GpuSlotPool>,
    mut bld: BuildingInitParams,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
    mut state: TestScenarioSetup,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut town_index: ResMut<crate::resources::TownIndex>,
) {
    config.gen_style = WorldGenStyle::Continents;
    config.num_towns = 1;
    config.ai_towns = 1;
    config.raider_towns = 0;
    config.world_width = 10000.0;
    config.world_height = 10000.0;
    config.world_margin = 300.0;
    config.min_town_distance = 500.0;

    let ai_players = world::setup_world(
        &config,
        &mut world_grid,
        &mut world_data,
        &mut crate::resources::FactionList::default(),
        &mut slot_alloc,
        &mut bld.entity_map,
        &mut faction_stats,
        &mut crate::resources::Reputation::default(),
        &mut state.raider_state,
        &mut town_index,
        &mut commands,
        &mut gpu_updates,
    );
    state.ai_state.players = ai_players;

    // 100K food + gold for all towns via ECS
    if let Some(&e) = town_index.0.get(&0) {
        commands
            .entity(e)
            .insert(crate::components::FoodStore(100_000));
        commands
            .entity(e)
            .insert(crate::components::GoldStore(100_000));
    }
    for player in &state.ai_state.players {
        let ti = player.town_data_idx as i32;
        if let Some(&e) = town_index.0.get(&ti) {
            commands
                .entity(e)
                .insert(crate::components::FoodStore(100_000));
            commands
                .entity(e)
                .insert(crate::components::GoldStore(100_000));
        }
    }

    state.ai_config.decision_interval = 1.0;
    state.endless.enabled = true;
    state.game_time.time_scale = 1.0;

    // Focus camera on player town
    if let Some(town) = world_data.towns.first() {
        if let Ok(mut cam) = camera_query.single_mut() {
            cam.translation.x = town.center.x;
            cam.translation.y = town.center.y;
        }
    }
    state.test_state.phase_name = "Sandbox ready".into();
    info!(
        "sandbox: 1 player + 1 AI town, 100K food+gold, {} towns total",
        world_data.towns.len()
    );
}

pub(super) fn tick(time: Res<Time>, mut test: ResMut<TestState>) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    if test.phase == 1 {
        test.pass_phase(elapsed, "sandbox active — play freely");
        test.complete(elapsed);
    }
}
