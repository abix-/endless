//! Policies panel — per-town behavior configuration.
//! Controls flee thresholds, work schedules, off-duty behavior, healing priority.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::resources::*;
use crate::world::WorldData;

const SCHEDULE_OPTIONS: &[&str] = &["Both Shifts", "Day Only", "Night Only"];
const OFF_DUTY_OPTIONS: &[&str] = &["Go to Bed", "Stay at Fountain", "Wander Town"];

pub fn policies_panel_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut policies: ResMut<TownPolicies>,
    world_data: Res<WorldData>,
) -> Result {
    if !ui_state.policies_open {
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;
    let mut open = true;

    // Find first villager town (faction 0)
    let town_idx = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);

    // Ensure policies vec is big enough
    if town_idx >= policies.policies.len() {
        policies.policies.resize(town_idx + 1, PolicySet::default());
    }
    let policy = &mut policies.policies[town_idx];

    egui::Window::new("Policies")
        .open(&mut open)
        .collapsible(true)
        .resizable(false)
        .default_width(320.0)
        .show(ctx, |ui| {
            if let Some(town) = world_data.towns.get(town_idx) {
                ui.small(format!("Town: {}", town.name));
                ui.separator();
            }

            // Checkboxes
            ui.label(egui::RichText::new("General").strong());
            ui.checkbox(&mut policy.eat_food, "Eat Food")
                .on_hover_text("NPCs consume food to restore HP and energy");
            ui.checkbox(&mut policy.prioritize_healing, "Prioritize Healing")
                .on_hover_text("Wounded NPCs go to fountain before resuming work");

            ui.add_space(4.0);
            ui.label(egui::RichText::new("Guard Behavior").strong());
            ui.checkbox(&mut policy.guard_aggressive, "Aggressive")
                .on_hover_text("Guards never flee combat");
            ui.checkbox(&mut policy.guard_leash, "Leash")
                .on_hover_text("Guards return home if too far from post");

            ui.add_space(4.0);
            ui.label(egui::RichText::new("Farmer Behavior").strong());
            ui.checkbox(&mut policy.farmer_fight_back, "Fight Back")
                .on_hover_text("Farmers attack enemies instead of fleeing");

            // Sliders
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Thresholds").strong());

            // Convert 0.0-1.0 to 0-100 for display
            let mut farmer_flee_pct = policy.farmer_flee_hp * 100.0;
            let mut guard_flee_pct = policy.guard_flee_hp * 100.0;
            let mut recovery_pct = policy.recovery_hp * 100.0;

            ui.horizontal(|ui| {
                ui.label("Farmer flee HP:");
                ui.add(egui::Slider::new(&mut farmer_flee_pct, 0.0..=100.0).suffix("%"));
            });
            ui.horizontal(|ui| {
                ui.label("Guard flee HP:");
                ui.add(egui::Slider::new(&mut guard_flee_pct, 0.0..=100.0).suffix("%"));
            });
            ui.horizontal(|ui| {
                ui.label("Recovery HP:");
                ui.add(egui::Slider::new(&mut recovery_pct, 0.0..=100.0).suffix("%"));
            });

            policy.farmer_flee_hp = farmer_flee_pct / 100.0;
            policy.guard_flee_hp = guard_flee_pct / 100.0;
            policy.recovery_hp = recovery_pct / 100.0;

            // Dropdowns
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Schedules").strong());

            let mut schedule_idx = policy.work_schedule as usize;
            let mut farmer_off_idx = policy.farmer_off_duty as usize;
            let mut guard_off_idx = policy.guard_off_duty as usize;

            ui.horizontal(|ui| {
                ui.label("Work schedule:");
                egui::ComboBox::from_id_salt("work_schedule")
                    .selected_text(SCHEDULE_OPTIONS[schedule_idx])
                    .show_index(ui, &mut schedule_idx, SCHEDULE_OPTIONS.len(), |i| SCHEDULE_OPTIONS[i]);
            });

            ui.horizontal(|ui| {
                ui.label("Farmer off-duty:");
                egui::ComboBox::from_id_salt("farmer_off_duty")
                    .selected_text(OFF_DUTY_OPTIONS[farmer_off_idx])
                    .show_index(ui, &mut farmer_off_idx, OFF_DUTY_OPTIONS.len(), |i| OFF_DUTY_OPTIONS[i]);
            });

            ui.horizontal(|ui| {
                ui.label("Guard off-duty:");
                egui::ComboBox::from_id_salt("guard_off_duty")
                    .selected_text(OFF_DUTY_OPTIONS[guard_off_idx])
                    .show_index(ui, &mut guard_off_idx, OFF_DUTY_OPTIONS.len(), |i| OFF_DUTY_OPTIONS[i]);
            });

            policy.work_schedule = match schedule_idx {
                1 => WorkSchedule::DayOnly,
                2 => WorkSchedule::NightOnly,
                _ => WorkSchedule::Both,
            };
            policy.farmer_off_duty = match farmer_off_idx {
                1 => OffDutyBehavior::StayAtFountain,
                2 => OffDutyBehavior::WanderTown,
                _ => OffDutyBehavior::GoToBed,
            };
            policy.guard_off_duty = match guard_off_idx {
                1 => OffDutyBehavior::StayAtFountain,
                2 => OffDutyBehavior::WanderTown,
                _ => OffDutyBehavior::GoToBed,
            };
        });

    if !open {
        ui_state.policies_open = false;
    }

    Ok(())
}
