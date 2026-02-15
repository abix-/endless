//! Build palette and placement HUD.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::constants::*;
use crate::resources::*;
use crate::world;

struct BuildOption {
    kind: BuildKind,
    label: &'static str,
    cost: i32,
    help_key: &'static str,
}

const PLAYER_BUILD_OPTIONS: &[BuildOption] = &[
    BuildOption { kind: BuildKind::Farm, label: "Farm", cost: FARM_BUILD_COST, help_key: "build_farm" },
    BuildOption { kind: BuildKind::House, label: "House", cost: HOUSE_BUILD_COST, help_key: "build_house" },
    BuildOption { kind: BuildKind::Barracks, label: "Barracks", cost: BARRACKS_BUILD_COST, help_key: "build_barracks" },
    BuildOption { kind: BuildKind::GuardPost, label: "Guard Post", cost: GUARD_POST_BUILD_COST, help_key: "build_guard_post" },
];

const CAMP_BUILD_OPTIONS: &[BuildOption] = &[
    BuildOption { kind: BuildKind::Tent, label: "Tent", cost: TENT_BUILD_COST, help_key: "build_tent" },
];

fn build_kind_name(kind: BuildKind) -> &'static str {
    match kind {
        BuildKind::Farm => "Farm",
        BuildKind::GuardPost => "Guard Post",
        BuildKind::House => "House",
        BuildKind::Barracks => "Barracks",
        BuildKind::Tent => "Tent",
    }
}

pub fn build_menu_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut build_ctx: ResMut<BuildMenuContext>,
    world_data: Res<world::WorldData>,
    food_storage: Res<FoodStorage>,
    catalog: Res<HelpCatalog>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    if build_ctx.town_data_idx.is_none() {
        build_ctx.town_data_idx = world_data.towns.iter().position(|t| t.faction == 0);
    }

    let Some(town_data_idx) = build_ctx.town_data_idx else {
        return Ok(());
    };
    let Some(town) = world_data.towns.get(town_data_idx) else {
        return Ok(());
    };
    let is_camp = town.faction > 0;
    let food = food_storage.food.get(town_data_idx).copied().unwrap_or(0);
    let options = if is_camp { CAMP_BUILD_OPTIONS } else { PLAYER_BUILD_OPTIONS };

    if ui_state.build_menu_open {
        let mut open = true;
        egui::Window::new("Build")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(220.0)
            .show(ctx, |ui| {
                ui.label(format!("{} - Food: {}", town.name, food));
                ui.separator();

                for option in options {
                    let can_afford = food >= option.cost;
                    let selected = build_ctx.selected_build == Some(option.kind);
                    let label = if selected {
                        format!("{} (selected)", option.label)
                    } else {
                        format!("{} ({} food)", option.label, option.cost)
                    };
                    if ui.add_enabled(can_afford, egui::Button::new(label))
                        .on_hover_text(catalog.0.get(option.help_key).copied().unwrap_or(""))
                        .clicked() {
                        build_ctx.selected_build = Some(option.kind);
                    }
                }

                ui.separator();
                if ui.button("Cancel Placement").clicked() {
                    build_ctx.selected_build = None;
                }
                ui.small("Left click: place  |  Right click: cancel");
            });
        if !open {
            ui_state.build_menu_open = false;
        }
    }

    if let Some(selected) = build_ctx.selected_build {
        if let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) {
            egui::Area::new(egui::Id::new("build_cursor_hint"))
                .fixed_pos(pos + egui::vec2(16.0, 16.0))
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgba_unmultiplied(20, 20, 20, 220))
                        .inner_margin(egui::Margin::same(4))
                        .show(ui, |ui| {
                            ui.label(format!("Placing: {}", build_kind_name(selected)));
                        });
                });
        }
    }

    Ok(())
}
