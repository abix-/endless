//! Combat log panel — scrollable event feed (mirrors combat_log.gd).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::resources::*;

#[derive(Default)]
pub struct CombatLogState {
    pub show_kills: bool,
    pub show_spawns: bool,
    pub show_raids: bool,
    pub show_harvests: bool,
    pub initialized: bool,
}

pub fn combat_log_system(
    mut contexts: EguiContexts,
    ui_state: Res<UiState>,
    combat_log: Res<CombatLog>,
    mut state: Local<CombatLogState>,
) -> Result {
    if !ui_state.combat_log_open {
        return Ok(());
    }

    // Default all filters on
    if !state.initialized {
        state.show_kills = true;
        state.show_spawns = true;
        state.show_raids = true;
        state.show_harvests = true;
        state.initialized = true;
    }

    let ctx = contexts.ctx_mut()?;

    egui::TopBottomPanel::bottom("combat_log").default_height(120.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Combat Log");
            ui.separator();
            ui.checkbox(&mut state.show_kills, "Deaths");
            ui.checkbox(&mut state.show_spawns, "Spawns");
            ui.checkbox(&mut state.show_raids, "Raids");
            ui.checkbox(&mut state.show_harvests, "Harvests");
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
                    };
                    if !show {
                        continue;
                    }

                    let color = match entry.kind {
                        CombatEventKind::Kill => egui::Color32::from_rgb(220, 80, 80),
                        CombatEventKind::Spawn => egui::Color32::from_rgb(80, 200, 80),
                        CombatEventKind::Raid => egui::Color32::from_rgb(220, 160, 40),
                        CombatEventKind::Harvest => egui::Color32::from_rgb(200, 200, 60),
                    };

                    let timestamp = format!("[D{} {:02}:{:02}]", entry.day, entry.hour, entry.minute);
                    ui.horizontal(|ui| {
                        ui.small(&timestamp);
                        ui.colored_label(color, &entry.message);
                    });
                }
            });
    });

    Ok(())
}
