use super::*;

#[derive(SystemParam)]
pub struct UpgradeParams<'w> {
    pub(crate) faction_stats: Res<'w, FactionStats>,
    pub(crate) queue: MessageWriter<'w, UpgradeMsg>,
    pub(crate) auto: ResMut<'w, AutoUpgrade>,
}

// ============================================================================
// UPGRADE CONTENT
// ============================================================================

pub(crate) fn upgrade_content(
    ui: &mut egui::Ui,
    upgrade: &mut UpgradeParams,
    town_access: &crate::systemparams::TownAccess<'_, '_>,
    world_data: &WorldData,
    settings: &mut UserSettings,
) {
    let town_idx = world_data
        .towns
        .iter()
        .position(|t| t.faction == crate::constants::FACTION_PLAYER)
        .unwrap_or(0);
    let food = town_access.food(town_idx as i32);
    let gold = town_access.gold(town_idx as i32);
    let player_faction = world_data
        .towns
        .get(town_idx)
        .map(|t| t.faction as usize)
        .unwrap_or(0);
    let villager_stats = upgrade.faction_stats.stats.get(player_faction);
    let alive = villager_stats.map(|s| s.alive).unwrap_or(0);
    let levels = town_access.upgrade_levels(town_idx as i32);

    // Header: resources + town name
    ui.horizontal(|ui| {
        ui.label(format!("Food: {}", food));
        ui.separator();
        ui.label(format!("Gold: {}", gold));
        ui.separator();
        ui.label(format!("Villagers: {}", alive));
    });
    if let Some(town) = world_data.towns.get(town_idx) {
        ui.small(format!("Town: {}", town.name));
    }

    // Branch totals + overall total
    let reg = &*UPGRADES;
    let total: u32 = levels.iter().map(|&l| l as u32).sum();
    ui.horizontal(|ui| {
        for branch in &reg.branches {
            let bt = branch_total(&levels, branch.label);
            ui.label(egui::RichText::new(format!("{}: {}", branch.label, bt)).small());
        }
        ui.label(
            egui::RichText::new(format!("Total: {}", total))
                .small()
                .strong(),
        );
    });
    ui.separator();

    // Tree-ordered upgrade list grouped by section (driven by dynamic registry)
    for section_name in ["Economy", "Military"] {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(section_name).strong().size(16.0));
        ui.separator();

        for branch in reg.branches.iter().filter(|b| b.section == section_name) {
            let bt = branch_total(&levels, branch.label);
            let is_expanded = settings.upgrade_expanded.iter().any(|s| s == branch.label);
            let id = ui.make_persistent_id(format!("upg_{}", branch.label));
            let state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                is_expanded,
            );
            let header_res = state.show_header(ui, |ui| {
                ui.label(egui::RichText::new(format!("{} ({})", branch.label, bt)).strong());
            });
            header_res.body(|ui| {
                for &(i, depth) in &branch.entries {
                    let upg = &reg.nodes[i];
                    let unlocked = upgrade_unlocked(&levels, i);
                    let lv_i = levels.get(i).copied().unwrap_or(0);
                    let available = upgrade_available(&levels, i, food, gold);
                    let indent = depth as f32 * 16.0;

                    ui.horizontal(|ui| {
                        ui.add_space(indent);

                        // Auto-upgrade checkbox
                        upgrade.auto.ensure_towns(town_idx + 1);
                        let count = upgrade_count();
                        upgrade.auto.flags[town_idx].resize(count, false);
                        let auto_flag = &mut upgrade.auto.flags[town_idx][i];
                        let prev_auto = *auto_flag;
                        ui.add_enabled(unlocked, egui::Checkbox::new(auto_flag, ""))
                            .on_hover_text("Auto-buy each game hour");
                        if *auto_flag != prev_auto {
                            settings.auto_upgrades = upgrade.auto.flags[town_idx].clone();
                        }

                        // Label (dimmed when locked)
                        let label_text = egui::RichText::new(upg.label);
                        ui.label(if unlocked {
                            label_text
                        } else {
                            label_text.weak()
                        });

                        // Effect summary (now/next)
                        let (now, next) = upgrade_effect_summary(i, lv_i);
                        ui.label(
                            egui::RichText::new(format!("{} -> {}", now, next))
                                .small()
                                .weak(),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let cost_text = format_upgrade_cost(i, lv_i);
                            let response = ui.add_enabled(available, egui::Button::new(&cost_text));

                            let response = if !unlocked {
                                if let Some(msg) = missing_prereqs(&levels, i) {
                                    response.on_hover_text(msg)
                                } else {
                                    response
                                }
                            } else {
                                response.on_hover_text(upg.tooltip)
                            };
                            if response.clicked() {
                                upgrade.queue.write(UpgradeMsg {
                                    town_idx,
                                    upgrade_idx: i,
                                });
                            }

                            ui.label(format!("Lv{}", lv_i));
                        });
                    });
                }
            });
            // Persist expand/collapse changes after body renders (borrow on ui released)
            let now_open = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                false,
            )
            .is_open();
            if now_open != is_expanded {
                if now_open {
                    settings.upgrade_expanded.push(branch.label.to_string());
                } else {
                    settings.upgrade_expanded.retain(|s| s != branch.label);
                }
            }
        }
    }
}
