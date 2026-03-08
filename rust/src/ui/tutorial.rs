//! Guided tutorial — condition-driven hints that auto-advance as the player plays.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::render::MainCamera;
use crate::resources::*;
use crate::settings::{self, ControlAction, UserSettings};
use crate::world::{BuildingKind, WorldData};

const STEP_COUNT: u8 = 24;

/// Build step text with the player's actual keybindings interpolated.
fn step_text(step: u8, s: &UserSettings) -> String {
    let k = |a: ControlAction| s.key_label_for_action(a);
    match step {
        1 => "Camera: right-drag, WASD, or screen edge. Scroll = zoom".into(),
        2 => format!("{} = build menu. Use Economy/Military tabs to find buildings", k(ControlAction::ToggleBuildMenu)),
        3 => "Build a Farm".into(),
        4 => "Build a Farmer Home - spawns 1 farmer who works the nearest farm".into(),
        5 => "Each building = 1 NPC. NPC dies -> respawns after 12 hours\nBuilding destroyed -> NPC lives on but won't respawn".into(),
        6 => "Click an NPC to select them - see their stats in the bottom panel".into(),
        7 => format!("{} = follow selected NPC. Press {} again or WASD to stop", k(ControlAction::ToggleFollow), k(ControlAction::ToggleFollow)),
        8 => "Food incoming - top bar shows your stockpile".into(),
        9 => "NPCs eat 1 food when low on energy\nNo food -> starvation (half HP, half speed)".into(),
        10 => "Build a Waypoint - archers patrol between them in order".into(),
        11 => "Build an Archer Home - spawns 1 archer".into(),
        12 => "Build Walls around your town to block enemies\nClick a wall to upgrade its tier".into(),
        13 => format!("{} = upgrades - spend food and gold to buff your NPCs", k(ControlAction::ToggleUpgrades)),
        14 => "Upgrades cost food and gold. Miners extract gold from mines".into(),
        15 => "Build Roads between buildings for a 1.5x NPC speed boost".into(),
        16 => "Expansion upgrade = +1 build range per level (starts 8x8)".into(),
        17 => "Build a Miner Home - spawns 1 miner who works the nearest gold mine".into(),
        18 => format!("{} = policies - control NPC behavior (schedules, flee HP, aggression)", k(ControlAction::TogglePolicies)),
        19 => format!("{} = patrol order - reorder waypoints to set the archer patrol route", k(ControlAction::TogglePatrols)),
        20 => format!("{} = squads - group archers into squads", k(ControlAction::ToggleSquads)),
        21 => "1-9 = select squad + click map to send them. 0 = squad 10".into(),
        22 => format!("{} = quicksave, {} = quickload\nESC > Save/Load for named saves", k(ControlAction::QuickSave), k(ControlAction::QuickLoad)),
        23 => "All keys are rebindable in ESC > Settings > Controls".into(),
        24 => format!("{} = roster, {} = factions, {} = combat log, {} = help",
            k(ControlAction::ToggleRoster), k(ControlAction::ToggleFactions),
            k(ControlAction::ToggleCombatLog), k(ControlAction::ToggleHelp)),
        _ => String::new(),
    }
}

/// Check if the current step's completion condition is met.
fn step_complete(
    step: u8,
    tutorial: &TutorialState,
    ui_state: &UiState,
    world_data: &WorldData,
    entity_map: &EntityMap,
    food_storage: &FoodStorage,
    camera_pos: Vec2,
    selected_npc: &SelectedNpc,
    follow: &FollowSelected,
) -> bool {
    let pt = world_data
        .towns
        .iter()
        .position(|t| t.faction == crate::constants::FACTION_PLAYER)
        .unwrap_or(0);
    match step {
        1 => (camera_pos - tutorial.camera_start).length() > 50.0,
        2 => ui_state.build_menu_open,
        3 => entity_map.count_for_town(BuildingKind::Farm, pt as u32) > tutorial.initial_farms,
        4 => {
            entity_map.count_for_town(BuildingKind::FarmerHome, pt as u32)
                > tutorial.initial_farmer_homes
        }
        5 => false,  // info-only
        6 => selected_npc.0 >= 0,
        7 => follow.0,
        8 => pt < food_storage.food.len() && food_storage.food[pt] > 0,
        9 => false,  // info-only
        10 => {
            entity_map.count_for_town(BuildingKind::Waypoint, pt as u32)
                > tutorial.initial_waypoints
        }
        11 => {
            entity_map.count_for_town(BuildingKind::ArcherHome, pt as u32)
                > tutorial.initial_archer_homes
        }
        12 => false, // info-only (walls)
        13 => ui_state.tech_tree_open,
        14 => false, // info-only
        15 => false, // info-only (roads)
        16 => false, // info-only
        17 => {
            entity_map.count_for_town(BuildingKind::MinerHome, pt as u32)
                > tutorial.initial_miner_homes
        }
        18 => ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Policies,
        19 => ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Patrols,
        20 => ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Squads,
        21 => false, // info-only
        22 => false, // info-only (save/load)
        23 => false, // info-only (controls)
        24 => false, // info-only (hotkeys)
        _ => true,
    }
}

const TUTORIAL_TIMEOUT_SECS: f64 = 600.0; // 10 minutes

pub fn tutorial_ui_system(
    mut contexts: EguiContexts,
    mut tutorial: ResMut<TutorialState>,
    mut settings: ResMut<UserSettings>,
    ui_state: Res<UiState>,
    world_data: Res<WorldData>,
    entity_map: Res<EntityMap>,
    food_storage: Res<FoodStorage>,
    camera_query: Query<&Transform, With<MainCamera>>,
    selected_npc: Res<SelectedNpc>,
    follow: Res<FollowSelected>,
    time: Res<Time<Real>>,
) -> Result {
    // Not active
    if tutorial.step == 0 || tutorial.step == 255 {
        return Ok(());
    }

    // Auto-end after 10 minutes
    if time.elapsed_secs_f64() - tutorial.start_time >= TUTORIAL_TIMEOUT_SECS {
        tutorial.step = 255;
        settings.tutorial_completed = true;
        settings::save_settings(&settings);
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;

    let camera_pos = camera_query
        .single()
        .map(|t| Vec2::new(t.translation.x, t.translation.y))
        .unwrap_or(Vec2::ZERO);

    // Check completion
    if step_complete(
        tutorial.step,
        &tutorial,
        &ui_state,
        &world_data,
        &entity_map,
        &food_storage,
        camera_pos,
        &selected_npc,
        &follow,
    ) {
        tutorial.step += 1;
        if tutorial.step > STEP_COUNT {
            tutorial.step = 255;
            settings.tutorial_completed = true;
            settings::save_settings(&settings);
            return Ok(());
        }
    }

    // Render current step
    if tutorial.step > STEP_COUNT {
        return Ok(());
    }
    let text = step_text(tutorial.step, &settings);
    if text.is_empty() {
        return Ok(());
    }

    let mut skip_all = false;
    let mut skip_step = false;

    egui::Area::new(egui::Id::new("tutorial_hint"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -160.0])
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 25, 220))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    ui.set_max_width(500.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(&text)
                                .size(15.0)
                                .color(egui::Color32::from_rgb(230, 230, 210)),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{}/{}", tutorial.step, STEP_COUNT))
                                    .size(11.0)
                                    .color(egui::Color32::from_rgb(100, 100, 120)),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("Skip Tutorial").clicked() {
                                        skip_all = true;
                                    }
                                    if ui.small_button("Next").clicked() {
                                        skip_step = true;
                                    }
                                },
                            );
                        });
                    });
                });
        });

    if skip_all {
        tutorial.step = 255;
        settings.tutorial_completed = true;
        settings::save_settings(&settings);
    } else if skip_step {
        tutorial.step += 1;
        if tutorial.step > STEP_COUNT {
            tutorial.step = 255;
            settings.tutorial_completed = true;
            settings::save_settings(&settings);
        }
    }

    Ok(())
}
