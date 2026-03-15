//! AI Building Test — observe AI town building in isolation.
//! Setup: 1 AI town, 100K food+gold, 1s decision interval.
//! Phase 1: egui personality picker (Economic default).
//! Phase 2: auto-passes, scene runs indefinitely for observation.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::resources::*;
use crate::systems::{AiPersonality, AiPlayerState};
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
    _spawn_writer: MessageWriter<crate::messages::SpawnNpcMsg>,
    mut state: TestScenarioSetup,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut town_index: ResMut<crate::resources::TownIndex>,
) {
    config.gen_style = WorldGenStyle::Continents;
    config.num_towns = 0;
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

    // Give AI town massive resources via ECS town entities
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

    if let Some(town) = world_data.towns.first() {
        if let Ok(mut cam) = camera_query.single_mut() {
            cam.translation.x = town.center.x;
            cam.translation.y = town.center.y;
        }
    }
    state.test_state.phase_name = "Pick personality...".into();
    info!(
        "ai-building: setup — {} towns, 1 AI, 100K food+gold, 1s interval",
        world_data.towns.len()
    );
}

/// Egui UI — runs in EguiPrimaryContextPass so buttons actually receive clicks.
/// Only shows personality picker (phase 1). Use Factions tab (I key) for AI stats.
pub fn ui(mut contexts: EguiContexts, mut test: ResMut<TestState>) -> Result {
    if test.phase != 1 {
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;
    egui::Window::new("AI Personality")
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label("Choose AI personality:");
            ui.add_space(8.0);
            if ui.button("Economic (default)").clicked() {
                test.counters.insert("personality".into(), 0);
            }
            if ui.button("Balanced").clicked() {
                test.counters.insert("personality".into(), 1);
            }
            if ui.button("Aggressive").clicked() {
                test.counters.insert("personality".into(), 2);
            }
        });
    Ok(())
}

/// Tick — non-UI logic: phase transitions and AI personality assignment.
pub fn tick(mut ai_state: ResMut<AiPlayerState>, time: Res<Time>, mut test: ResMut<TestState>) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    match test.phase {
        1 => {
            if test.counters.contains_key("personality") {
                let p = match test.count("personality") {
                    1 => AiPersonality::Balanced,
                    2 => AiPersonality::Aggressive,
                    _ => AiPersonality::Economic,
                };
                for player in &mut ai_state.players {
                    player.personality = p;
                }
                test.pass_phase(elapsed, format!("personality: {}", p.name()));
            }
        }

        2 => {
            let ai_count = ai_state.players.iter().filter(|p| p.active).count();
            if ai_count > 0 {
                test.pass_phase(
                    elapsed,
                    format!("{} active AI player(s), observing...", ai_count),
                );
                test.complete(elapsed);
            } else if elapsed > 3.0 {
                test.fail_phase(elapsed, "no active AI players");
            }
        }

        _ => {}
    }
}
