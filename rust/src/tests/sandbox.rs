//! Sandbox Test — human player testing scene.
//! 1 player town + 1 AI builder town, 100K food + 100K gold, no raiders.
//! Auto-completes after setup so the scene runs indefinitely.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::resources::*;
use crate::systems::{AiPlayerConfig, AiPlayerState};
use crate::world::{self, WorldGenStyle};

use super::{BuildingInitParams, TestState};

#[derive(SystemParam)]
pub(super) struct SandboxState<'w> {
    raider_state: ResMut<'w, RaiderState>,
    test_state: ResMut<'w, TestState>,
    game_time: ResMut<'w, GameTime>,
    endless: ResMut<'w, EndlessMode>,
    ai_state: ResMut<'w, AiPlayerState>,
    ai_config: ResMut<'w, AiPlayerConfig>,
}

pub(super) fn setup(
    mut commands: Commands,
    mut world_data: ResMut<world::WorldData>,
    mut world_grid: ResMut<world::WorldGrid>,
    mut config: ResMut<world::WorldGenConfig>,
    mut food_storage: ResMut<FoodStorage>,
    mut gold_storage: ResMut<GoldStorage>,
    mut faction_stats: ResMut<FactionStats>,

    mut slot_alloc: ResMut<GpuSlotPool>,
    mut bld: BuildingInitParams,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
    mut state: SandboxState,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut uid_alloc: ResMut<crate::resources::NextEntityUid>,
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

        &mut slot_alloc,
        &mut bld.entity_map,
        &mut food_storage,
        &mut gold_storage,
        &mut faction_stats,
        &mut crate::resources::Reputation::default(),
        &mut state.raider_state,
        &mut uid_alloc,
        &mut commands,
        &mut gpu_updates,
    );
    state.ai_state.players = ai_players;

    // 100K food + gold for player town (index 0)
    if let Some(f) = food_storage.food.get_mut(0) {
        *f = 100_000;
    }
    if let Some(g) = gold_storage.gold.get_mut(0) {
        *g = 100_000;
    }

    // Give AI town resources too
    for player in &state.ai_state.players {
        let ti = player.town_data_idx;
        if let Some(f) = food_storage.food.get_mut(ti) {
            *f = 100_000;
        }
        if let Some(g) = gold_storage.gold.get_mut(ti) {
            *g = 100_000;
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

    match test.phase {
        1 => {
            test.pass_phase(elapsed, "sandbox active — play freely");
            test.complete(elapsed);
        }
        _ => {}
    }
}
