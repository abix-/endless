//! Guided tutorial — condition-driven hints that auto-advance as the player plays.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::render::MainCamera;
use crate::resources::*;
use crate::settings::{self, UserSettings};
use crate::world::WorldData;

const STEP_COUNT: u8 = 20;

const STEPS: [&str; STEP_COUNT as usize] = [
    "Camera: right-drag, WASD, or screen edge. Scroll = zoom",
    "B = build menu",
    "Build a Farm",
    "Build a Farmer Home - spawns 1 farmer who works the nearest farm",
    "Each building = 1 NPC. NPC dies -> respawns after 12 hours\nBuilding destroyed -> NPC lives on but won't respawn",
    "Click an NPC to select them - see their stats in the bottom panel",
    "F = follow selected NPC. Press F again or WASD to stop",
    "Food incoming - top bar shows your stockpile",
    "NPCs eat 1 food when low on energy\nNo food -> starvation (half HP, half speed)",
    "Build a Guard Post - archers patrol between them in order",
    "Build an Archer Home - spawns 1 archer",
    "U = upgrades - spend food and gold to buff your NPCs",
    "Upgrades cost food and gold. Miners extract gold from mines",
    "Expansion upgrade = +1 build range per level (starts 8x8)",
    "Build a Miner Home - spawns 1 miner who works the nearest gold mine",
    "P = policies - control NPC behavior (schedules, flee HP, aggression)",
    "T = patrol order - reorder guard posts to set the archer patrol route",
    "Q = squads - group archers into squads",
    "1-9 = select squad + click map to send them. 0 = squad 10",
    "R = roster, I = factions, L = combat log, H = help",
];

/// Check if the current step's completion condition is met.
fn step_complete(
    step: u8,
    tutorial: &TutorialState,
    ui_state: &UiState,
    world_data: &WorldData,
    food_storage: &FoodStorage,
    camera_pos: Vec2,
    selected_npc: &SelectedNpc,
    follow: &FollowSelected,
) -> bool {
    let pt = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);
    match step {
        1 => (camera_pos - tutorial.camera_start).length() > 50.0,
        2 => ui_state.build_menu_open,
        3 => {
            let farms = world_data.farms.iter().filter(|f| f.town_idx as usize == pt).count();
            farms > tutorial.initial_farms
        }
        4 => {
            let homes = world_data.farmer_homes.iter().filter(|h| h.town_idx as usize == pt).count();
            homes > tutorial.initial_farmer_homes
        }
        5 => false, // info-only — user clicks Next
        6 => selected_npc.0 >= 0,
        7 => follow.0,
        8 => {
            pt < food_storage.food.len() && food_storage.food[pt] > 0
        }
        9 => false, // info-only — user clicks Next
        10 => {
            let posts = world_data.guard_posts.iter().filter(|g| g.town_idx as usize == pt).count();
            posts > tutorial.initial_guard_posts
        }
        11 => {
            let homes = world_data.archer_homes.iter().filter(|a| a.town_idx as usize == pt).count();
            homes > tutorial.initial_archer_homes
        }
        12 => ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Upgrades,
        13 => false, // info-only — user clicks Next
        14 => false, // info-only — user clicks Next
        15 => {
            let homes = world_data.miner_homes.iter().filter(|m| m.town_idx as usize == pt).count();
            homes > tutorial.initial_miner_homes
        }
        16 => ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Policies,
        17 => ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Patrols,
        18 => ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Squads,
        19 => false, // info-only — user clicks Next
        20 => false, // info-only — user clicks Next
        _ => true,
    }
}

pub fn tutorial_ui_system(
    mut contexts: EguiContexts,
    mut tutorial: ResMut<TutorialState>,
    mut settings: ResMut<UserSettings>,
    ui_state: Res<UiState>,
    world_data: Res<WorldData>,
    food_storage: Res<FoodStorage>,
    camera_query: Query<&Transform, With<MainCamera>>,
    selected_npc: Res<SelectedNpc>,
    follow: Res<FollowSelected>,
) -> Result {
    // Not active
    if tutorial.step == 0 || tutorial.step == 255 { return Ok(()); }

    let ctx = contexts.ctx_mut()?;

    let camera_pos = camera_query.single().map(|t| Vec2::new(t.translation.x, t.translation.y)).unwrap_or(Vec2::ZERO);

    // Check completion
    if step_complete(tutorial.step, &tutorial, &ui_state, &world_data, &food_storage, camera_pos, &selected_npc, &follow) {
        tutorial.step += 1;
        if tutorial.step > STEP_COUNT {
            tutorial.step = 255;
            settings.tutorial_completed = true;
            settings::save_settings(&settings);
            return Ok(());
        }
    }

    // Render current step
    let step_idx = (tutorial.step - 1) as usize;
    if step_idx >= STEPS.len() { return Ok(()); }
    let text = STEPS[step_idx];

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
                        ui.label(egui::RichText::new(text)
                            .size(15.0)
                            .color(egui::Color32::from_rgb(230, 230, 210)));
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{}/{}", tutorial.step, STEP_COUNT))
                                .size(11.0)
                                .color(egui::Color32::from_rgb(100, 100, 120)));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Skip Tutorial").clicked() {
                                    skip_all = true;
                                }
                                if ui.small_button("Next").clicked() {
                                    skip_step = true;
                                }
                            });
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
