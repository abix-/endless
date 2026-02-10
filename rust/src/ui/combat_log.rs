//! Combat log panel — scrollable event feed (mirrors combat_log.gd).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::resources::*;
use crate::settings::{self, UserSettings};

#[derive(Default)]
pub struct CombatLogState {
    pub show_kills: bool,
    pub show_spawns: bool,
    pub show_raids: bool,
    pub show_harvests: bool,
    pub show_levelups: bool,
    pub initialized: bool,
}

pub fn combat_log_system(
    mut contexts: EguiContexts,
    ui_state: Res<UiState>,
    combat_log: Res<CombatLog>,
    mut state: Local<CombatLogState>,
    user_settings: Res<UserSettings>,
) -> Result {
    if !ui_state.combat_log_open {
        return Ok(());
    }

    // Load filters from saved settings
    if !state.initialized {
        state.show_kills = user_settings.log_kills;
        state.show_spawns = user_settings.log_spawns;
        state.show_raids = user_settings.log_raids;
        state.show_harvests = user_settings.log_harvests;
        state.show_levelups = user_settings.log_levelups;
        state.initialized = true;
    }

    let ctx = contexts.ctx_mut()?;

    // Snapshot filter state before UI to detect changes
    let prev = (state.show_kills, state.show_spawns, state.show_raids, state.show_harvests, state.show_levelups);

    egui::TopBottomPanel::bottom("combat_log").default_height(120.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Combat Log");
            ui.separator();
            ui.checkbox(&mut state.show_kills, "Deaths");
            ui.checkbox(&mut state.show_spawns, "Spawns");
            ui.checkbox(&mut state.show_raids, "Raids");
            ui.checkbox(&mut state.show_harvests, "Harvests");
            ui.checkbox(&mut state.show_levelups, "Levels");
        });

        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for entry in &combat_log.entries {
                    // Filter
                    let show = match entry.kind {
                        CombatEventKind::Kill => state.show_kills,
                        CombatEventKind::Spawn => state.show_spawns,
                        CombatEventKind::Raid => state.show_raids,
                        CombatEventKind::Harvest => state.show_harvests,
                        CombatEventKind::LevelUp => state.show_levelups,
                    };
                    if !show {
                        continue;
                    }

                    let color = match entry.kind {
                        CombatEventKind::Kill => egui::Color32::from_rgb(220, 80, 80),
                        CombatEventKind::Spawn => egui::Color32::from_rgb(80, 200, 80),
                        CombatEventKind::Raid => egui::Color32::from_rgb(220, 160, 40),
                        CombatEventKind::Harvest => egui::Color32::from_rgb(200, 200, 60),
                        CombatEventKind::LevelUp => egui::Color32::from_rgb(80, 180, 255),
                    };

                    let timestamp = format!("[D{} {:02}:{:02}]", entry.day, entry.hour, entry.minute);
                    ui.horizontal(|ui| {
                        ui.small(&timestamp);
                        ui.colored_label(color, &entry.message);
                    });
                }
            });
    });

    // Persist on change
    let curr = (state.show_kills, state.show_spawns, state.show_raids, state.show_harvests, state.show_levelups);
    if curr != prev {
        let mut s = user_settings.clone();
        s.log_kills = state.show_kills;
        s.log_spawns = state.show_spawns;
        s.log_raids = state.show_raids;
        s.log_harvests = state.show_harvests;
        s.log_levelups = state.show_levelups;
        settings::save_settings(&s);
    }

    Ok(())
}
