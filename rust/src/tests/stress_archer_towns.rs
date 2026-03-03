//! Stress test scene:
//! - 20 AI builder towns
//! - Configurable archer homes per town (default 1,000)
//! Verifies counts on phase 1, then leaves scene running for observation.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::components::Job;
use crate::resources::*;
use crate::systems::{AiPlayerConfig, AiPlayerState};
use crate::world::{self, BuildingKind, WorldGenStyle};

use super::{BuildingInitParams, TestState};

const STRESS_AI_TOWNS: usize = 20;
const DEFAULT_ARCHER_HOMES: usize = 1_000;

#[derive(Resource)]
pub struct StressArcherConfig {
    pub archer_homes: usize,
    /// Text buffer for the egui input field.
    input_buf: String,
}

impl Default for StressArcherConfig {
    fn default() -> Self {
        Self {
            archer_homes: DEFAULT_ARCHER_HOMES,
            input_buf: DEFAULT_ARCHER_HOMES.to_string(),
        }
    }
}

#[derive(SystemParam)]
pub(super) struct StressArcherTownsState<'w> {
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
    mut town_grids: ResMut<world::TownGrids>,
    mut slot_alloc: ResMut<GpuSlotPool>,
    mut bld: BuildingInitParams,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
    mut state: StressArcherTownsState,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut uid_alloc: ResMut<crate::resources::NextEntityUid>,
    stress_config: Res<StressArcherConfig>,
) {
    let archer_homes = stress_config.archer_homes;

    config.gen_style = WorldGenStyle::Classic;
    config.num_towns = 0;
    config.ai_towns = STRESS_AI_TOWNS;
    config.raider_towns = 0;
    config.world_width = 64_000.0;
    config.world_height = 64_000.0;
    config.world_margin = 4_000.0;
    config.min_town_distance = 7_000.0;
    config.farms_per_town = 0;
    config.gold_mines_per_town = 0;
    config.npc_counts.clear();
    for def in crate::constants::NPC_REGISTRY {
        config.npc_counts.insert(def.job, 0);
    }
    config.npc_counts.insert(Job::Archer, archer_homes);

    let ai_players = world::setup_world(
        &config,
        &mut world_grid,
        &mut world_data,
        &mut town_grids,
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

    if let Some(town) = world_data.towns.first() {
        if let Ok(mut cam) = camera_query.single_mut() {
            cam.translation.x = town.center.x;
            cam.translation.y = town.center.y;
        }
    }

    state.test_state.phase_name = "Validating stress setup...".into();
    info!(
        "stress-archer-towns: setup target={} AI towns, {} archer homes each",
        STRESS_AI_TOWNS, archer_homes
    );
}

pub(super) fn tick(
    time: Res<Time>,
    ai_state: Res<AiPlayerState>,
    entity_map: Res<EntityMap>,
    mut test: ResMut<TestState>,
    stress_config: Res<StressArcherConfig>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };
    let archer_homes = stress_config.archer_homes;

    match test.phase {
        1 => {
            let ai_towns = ai_state.players.len();
            let homes_ok = ai_state.players.iter().all(|player| {
                entity_map.count_for_town(
                    BuildingKind::ArcherHome,
                    player.town_data_idx as u32,
                ) == archer_homes
            });

            if ai_towns == STRESS_AI_TOWNS && homes_ok {
                test.pass_phase(
                    elapsed,
                    format!(
                        "spawned {} AI towns with {} archer homes each",
                        STRESS_AI_TOWNS, archer_homes
                    ),
                );
                test.complete(elapsed);
            } else {
                let mismatched = ai_state
                    .players
                    .iter()
                    .filter(|player| {
                        entity_map.count_for_town(
                            BuildingKind::ArcherHome,
                            player.town_data_idx as u32,
                        ) != archer_homes
                    })
                    .count();
                test.fail_phase(
                    elapsed,
                    format!(
                        "expected {} AI towns with {} archer homes each; got towns={} mismatched_towns={}",
                        STRESS_AI_TOWNS,
                        archer_homes,
                        ai_towns,
                        mismatched
                    ),
                );
            }
        }
        _ => {}
    }
}

/// Stress test config panel — shows below the main test HUD.
pub(super) fn ui(
    mut contexts: EguiContexts,
    mut stress_config: ResMut<StressArcherConfig>,
    mut test_state: ResMut<TestState>,
    mut next_state: ResMut<NextState<crate::AppState>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    egui::Window::new("Stress Config")
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 300.0])
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui: &mut egui::Ui| {
            ui.horizontal(|ui: &mut egui::Ui| {
                ui.label("Archer homes/town:");
                ui.add(egui::TextEdit::singleline(&mut stress_config.input_buf).desired_width(80.0));
            });
            // Parse input, show current effective value
            if let Ok(val) = stress_config.input_buf.trim().parse::<usize>() {
                if val > 0 {
                    stress_config.archer_homes = val;
                }
            }
            ui.label(format!(
                "Total: {} towns × {} = {} homes",
                STRESS_AI_TOWNS,
                stress_config.archer_homes,
                STRESS_AI_TOWNS * stress_config.archer_homes
            ));
            ui.add_space(4.0);
            if ui.button("Restart with this value").clicked() {
                test_state.pending_relaunch = Some("stress-archer-towns".into());
                next_state.set(crate::AppState::TestMenu);
            }
        });
    Ok(())
}
