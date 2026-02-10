//! Upgrade menu — spend food to upgrade town stats.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::resources::*;
use crate::systems::stats::{TownUpgrades, UpgradeQueue, UPGRADE_COUNT, upgrade_cost};
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
    UpgradeDef { label: "Move Speed",        tooltip: "+5% movement speed per level",               category: "Guard" },
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
    upgrades: Res<TownUpgrades>,
    mut queue: ResMut<UpgradeQueue>,
) -> Result {
    if !ui_state.upgrade_menu_open {
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;
    let mut open = true;

    // Find first villager town (faction 0)
    let town_idx = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);

    egui::Window::new("Upgrades")
        .open(&mut open)
        .collapsible(true)
        .resizable(false)
        .default_width(380.0)
        .show(ctx, |ui| {
            // Header stats
            let food = food_storage.food.get(town_idx).copied().unwrap_or(0);
            let villager_stats = faction_stats.stats.first();
            let alive = villager_stats.map(|s| s.alive).unwrap_or(0);

            ui.horizontal(|ui| {
                ui.label(format!("Food: {}", food));
                ui.separator();
                ui.label(format!("Villagers: {}", alive));
            });
            if let Some(town) = world_data.towns.get(town_idx) {
                ui.small(format!("Town: {}", town.name));
            }
            ui.separator();

            // Upgrade rows by category
            let levels = upgrades.levels.get(town_idx).copied().unwrap_or([0; UPGRADE_COUNT]);

            let mut last_category = "";
            for (i, upgrade) in UPGRADES.iter().enumerate() {
                if upgrade.category != last_category {
                    if !last_category.is_empty() {
                        ui.add_space(4.0);
                    }
                    ui.label(egui::RichText::new(upgrade.category).strong());
                    last_category = upgrade.category;
                }

                let level = levels[i];
                let cost = upgrade_cost(level);
                let can_afford = food >= cost;

                ui.horizontal(|ui| {
                    ui.label(upgrade.label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Buy button
                        let btn = egui::Button::new(format!("{}", cost));
                        if ui.add_enabled(can_afford, btn).on_hover_text(upgrade.tooltip).clicked() {
                            queue.0.push((town_idx, i));
                        }

                        ui.label(format!("Lv{}", level));
                    });
                });
            }
        });

    if !open {
        ui_state.upgrade_menu_open = false;
    }

    Ok(())
}
