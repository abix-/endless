//! Build menu — building placement UI scaffold (mirrors build_menu.gd).
//! Controls disabled until Stage 7 building system is implemented.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::resources::*;

pub fn build_menu_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
) -> Result {
    if !ui_state.build_menu_open {
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;
    let mut open = true;

    egui::Window::new("Build")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(250.0)
        .show(ctx, |ui| {
            ui.label("Place buildings in town grid slots");
            ui.separator();

            // Building options (all disabled — no backend yet)
            ui.add_enabled(false, egui::Button::new("Farm (50 food)"))
                .on_disabled_hover_text("Farms produce food when tended by farmers");
            ui.add_enabled(false, egui::Button::new("Bed (10 food)"))
                .on_disabled_hover_text("Beds let NPCs rest and recover energy");
            ui.add_enabled(false, egui::Button::new("Guard Post (25 food)"))
                .on_disabled_hover_text("Guard posts provide patrol waypoints");

            ui.separator();

            ui.add_enabled(false, egui::Button::new("Destroy Building"))
                .on_disabled_hover_text("Remove a building from a slot");
            ui.add_enabled(false, egui::Button::new("Unlock Slot"))
                .on_disabled_hover_text("Unlock a new grid slot for building");

            ui.separator();
            ui.small("Building system coming in Stage 7");
            ui.small("Click-to-place on the world grid");
        });

    if !open {
        ui_state.build_menu_open = false;
    }

    Ok(())
}
