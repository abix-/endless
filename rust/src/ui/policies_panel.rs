//! Policies panel — faction behavior config scaffold (mirrors policies_panel.gd).
//! Controls disabled until Stage 8 policy system is implemented.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::resources::*;

/// Local state for policy controls (visual only until backend exists).
#[derive(Default)]
pub struct PolicyState {
    pub eat_food: bool,
    pub guard_aggressive: bool,
    pub guard_leash: bool,
    pub farmer_fight_back: bool,
    pub prioritize_healing: bool,
    pub farmer_flee_hp: f32,
    pub guard_flee_hp: f32,
    pub recovery_hp: f32,
    pub work_schedule: usize,      // 0=Both, 1=Day, 2=Night
    pub farmer_off_duty: usize,    // 0=Bed, 1=Fountain, 2=Wander
    pub guard_off_duty: usize,     // 0=Bed, 1=Fountain, 2=Wander
    pub initialized: bool,
}

const SCHEDULE_OPTIONS: &[&str] = &["Both Shifts", "Day Only", "Night Only"];
const OFF_DUTY_OPTIONS: &[&str] = &["Go to Bed", "Stay at Fountain", "Wander Town"];

pub fn policies_panel_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut state: Local<PolicyState>,
) -> Result {
    if !ui_state.policies_open {
        return Ok(());
    }

    // Set defaults
    if !state.initialized {
        state.eat_food = true;
        state.guard_aggressive = false;
        state.guard_leash = true;
        state.farmer_fight_back = false;
        state.prioritize_healing = true;
        state.farmer_flee_hp = 30.0;
        state.guard_flee_hp = 15.0;
        state.recovery_hp = 80.0;
        state.initialized = true;
    }

    let ctx = contexts.ctx_mut()?;
    let mut open = true;

    egui::Window::new("Policies")
        .open(&mut open)
        .collapsible(true)
        .resizable(false)
        .default_width(320.0)
        .show(ctx, |ui| {
            // All controls disabled — policy backend not yet implemented
            ui.disable();

            // Checkboxes
            ui.label(egui::RichText::new("General").strong());
            ui.checkbox(&mut state.eat_food, "Eat Food")
                .on_disabled_hover_text("NPCs consume food to restore HP and energy");
            ui.checkbox(&mut state.prioritize_healing, "Prioritize Healing")
                .on_disabled_hover_text("Wounded NPCs go to fountain before resuming work");

            ui.add_space(4.0);
            ui.label(egui::RichText::new("Guard Behavior").strong());
            ui.checkbox(&mut state.guard_aggressive, "Aggressive")
                .on_disabled_hover_text("Guards chase enemies beyond patrol range");
            ui.checkbox(&mut state.guard_leash, "Leash")
                .on_disabled_hover_text("Guards return home if too far from post");

            ui.add_space(4.0);
            ui.label(egui::RichText::new("Farmer Behavior").strong());
            ui.checkbox(&mut state.farmer_fight_back, "Fight Back")
                .on_disabled_hover_text("Farmers attack enemies instead of fleeing");

            // Sliders
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Thresholds").strong());

            ui.horizontal(|ui| {
                ui.label("Farmer flee HP:");
                ui.add(egui::Slider::new(&mut state.farmer_flee_hp, 0.0..=100.0).suffix("%"));
            });
            ui.horizontal(|ui| {
                ui.label("Guard flee HP:");
                ui.add(egui::Slider::new(&mut state.guard_flee_hp, 0.0..=100.0).suffix("%"));
            });
            ui.horizontal(|ui| {
                ui.label("Recovery HP:");
                ui.add(egui::Slider::new(&mut state.recovery_hp, 0.0..=100.0).suffix("%"));
            });

            // Dropdowns
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Schedules").strong());

            ui.horizontal(|ui| {
                ui.label("Work schedule:");
                egui::ComboBox::from_id_salt("work_schedule")
                    .selected_text(SCHEDULE_OPTIONS[state.work_schedule])
                    .show_index(ui, &mut state.work_schedule, SCHEDULE_OPTIONS.len(), |i| SCHEDULE_OPTIONS[i]);
            });

            ui.horizontal(|ui| {
                ui.label("Farmer off-duty:");
                egui::ComboBox::from_id_salt("farmer_off_duty")
                    .selected_text(OFF_DUTY_OPTIONS[state.farmer_off_duty])
                    .show_index(ui, &mut state.farmer_off_duty, OFF_DUTY_OPTIONS.len(), |i| OFF_DUTY_OPTIONS[i]);
            });

            ui.horizontal(|ui| {
                ui.label("Guard off-duty:");
                egui::ComboBox::from_id_salt("guard_off_duty")
                    .selected_text(OFF_DUTY_OPTIONS[state.guard_off_duty])
                    .show_index(ui, &mut state.guard_off_duty, OFF_DUTY_OPTIONS.len(), |i| OFF_DUTY_OPTIONS[i]);
            });

            ui.separator();
            ui.small("Policy system coming in Stage 8");
        });

    if !open {
        ui_state.policies_open = false;
    }

    Ok(())
}
