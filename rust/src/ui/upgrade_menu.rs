//! Upgrade menu — town upgrade tree scaffold (mirrors upgrade_menu.gd).
//! Controls disabled until Stage 8 upgrade system is implemented.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::resources::*;
use crate::world::WorldData;

/// Upgrade definition for display.
struct UpgradeDef {
    label: &'static str,
    tooltip: &'static str,
    category: &'static str,
}

const UPGRADES: &[UpgradeDef] = &[
    UpgradeDef { label: "Guard Health",      tooltip: "+10% guard HP per level",                    category: "Guard" },
    UpgradeDef { label: "Guard Attack",      tooltip: "+10% guard damage per level",                category: "Guard" },
    UpgradeDef { label: "Guard Range",       tooltip: "+5% guard attack range per level",           category: "Guard" },
    UpgradeDef { label: "Guard Size",        tooltip: "+5% guard size per level",                   category: "Guard" },
    UpgradeDef { label: "Attack Speed",      tooltip: "-8% attack cooldown per level",              category: "Guard" },
    UpgradeDef { label: "Move Speed",        tooltip: "+5% guard movement speed per level",         category: "Guard" },
    UpgradeDef { label: "Alert Radius",      tooltip: "+10% alert radius per level",                category: "Guard" },
    UpgradeDef { label: "Farm Yield",        tooltip: "+15% food production per level",             category: "Farm" },
    UpgradeDef { label: "Farmer HP",         tooltip: "+20% farmer HP per level",                   category: "Farm" },
    UpgradeDef { label: "Farmer Cap",        tooltip: "+2 max farmers per level",                   category: "Farm" },
    UpgradeDef { label: "Guard Cap",         tooltip: "+10 max guards per level",                   category: "Guard" },
    UpgradeDef { label: "Healing Rate",      tooltip: "+20% HP regen at fountain per level",        category: "Town" },
    UpgradeDef { label: "Food Efficiency",   tooltip: "10% chance per level to not consume food",   category: "Town" },
    UpgradeDef { label: "Fountain Radius",   tooltip: "+24px fountain healing range per level",     category: "Town" },
];

pub fn upgrade_menu_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    food_storage: Res<FoodStorage>,
    faction_stats: Res<FactionStats>,
    world_data: Res<WorldData>,
) -> Result {
    if !ui_state.upgrade_menu_open {
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;
    let mut open = true;

    egui::Window::new("Upgrades")
        .open(&mut open)
        .collapsible(true)
        .resizable(false)
        .default_width(380.0)
        .show(ctx, |ui| {
            // Header stats
            let num_villager_towns = world_data.towns.len() / 2;
            let town_food: i32 = food_storage.food.iter().take(num_villager_towns).sum();
            let villager_stats = faction_stats.stats.first();
            let alive = villager_stats.map(|s| s.alive).unwrap_or(0);

            ui.horizontal(|ui| {
                ui.label(format!("Food: {}", town_food));
                ui.separator();
                ui.label(format!("Villagers: {}", alive));
            });
            ui.separator();

            // Upgrade rows by category
            let mut last_category = "";
            for upgrade in UPGRADES {
                if upgrade.category != last_category {
                    if !last_category.is_empty() {
                        ui.add_space(4.0);
                    }
                    ui.label(egui::RichText::new(upgrade.category).strong());
                    last_category = upgrade.category;
                }

                ui.horizontal(|ui| {
                    ui.label(upgrade.label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Auto-upgrade checkbox (disabled)
                        let mut auto = false;
                        ui.add_enabled(false, egui::Checkbox::new(&mut auto, "Auto"));

                        // Cost button (disabled)
                        ui.add_enabled(false, egui::Button::new("10"))
                            .on_disabled_hover_text(upgrade.tooltip);

                        ui.label("Lv0");
                    });
                });
            }

            ui.separator();
            ui.small("Upgrade system coming in Stage 8");
        });

    if !open {
        ui_state.upgrade_menu_open = false;
    }

    Ok(())
}
