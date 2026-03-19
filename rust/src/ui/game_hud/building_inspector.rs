//! Building inspector panel.

use super::inspector::{BuildingInspectorData, InspectorAction, building_link, npc_link};
use crate::components::*;
use crate::constants::{
    ResourceKind, UpgradeStatKind, WALL_TIER_HP, WALL_TIER_NAMES, WALL_UPGRADE_COSTS, building_def,
    npc_def,
};
use crate::resources::*;
use crate::settings::UserSettings;
use crate::systems::stats::{
    UPGRADES, level_from_xp, resolve_tower_instance_stats, resolve_town_tower_stats,
};
use crate::world::{BuildingKind, WorldData, WorldGrid};
use bevy::prelude::*;
use bevy_egui::egui;

// ============================================================================
// BUILDING INSPECTOR
// ============================================================================

pub(crate) fn selected_building_info(
    selected: &SelectedBuilding,
    grid: &WorldGrid,
    entity_map: &EntityMap,
) -> Option<(BuildingKind, u32, Vec2, usize, usize)> {
    if !selected.active {
        return None;
    }

    if let (Some(kind), Some(slot)) = (selected.kind, selected.slot) {
        if let Some(inst) = entity_map.get_instance(slot) {
            let (col, row) = grid.world_to_grid(inst.position);
            return Some((kind, inst.town_idx, inst.position, col, row));
        }
    }

    let col = selected.col;
    let row = selected.row;
    let inst = entity_map.get_at_grid(col as i32, row as i32)?;
    let pos = grid.grid_to_world(col, row);
    Some((inst.kind, inst.town_idx, pos, col, row))
}

/// Mine assignment UI: show assigned mine, "Set Mine" / "Clear" buttons.
/// Shared by building inspector (MinerHome) and NPC inspector (Miner).
pub(crate) fn mine_assignment_ui(
    ui: &mut egui::Ui,
    _world_data: &mut WorldData,
    entity_map: &EntityMap,
    mh_slot: usize,
    ref_pos: Vec2,
    dirty_writers: &mut crate::messages::DirtyWriters,
    ui_state: &mut UiState,
    miner_cfg_q: &mut Query<&mut MinerHomeConfig>,
) -> Option<InspectorAction> {
    let mh_entity = entity_map.entities.get(&mh_slot).copied();
    let (assigned, manual) = mh_entity
        .and_then(|e| {
            miner_cfg_q
                .get(e)
                .ok()
                .map(|mc| (mc.assigned_mine, mc.manual_mine))
        })
        .unwrap_or((None, false));
    let mut action = None;
    if let Some(mine_pos) = assigned {
        let dist = mine_pos.distance(ref_pos);
        let mine_slot = entity_map.slot_at_position(mine_pos);
        let label = if let Some(mine_idx) = entity_map.gold_mine_index(mine_pos) {
            format!(
                "Mine: {} - {:.0}px",
                crate::ui::gold_mine_name(mine_idx),
                dist
            )
        } else {
            format!(
                "Mine: ({:.0}, {:.0}) - {:.0}px",
                mine_pos.x, mine_pos.y, dist
            )
        };
        if let Some(slot) = mine_slot {
            action = building_link(ui, &label, slot);
        } else {
            ui.label(label);
        }
    } else {
        ui.label("Mine: Auto (nearest)");
    }
    ui.small(if manual {
        "Mode: Manual"
    } else {
        "Mode: Auto-policy"
    });
    ui.horizontal(|ui| {
        if ui.button("Set Mine").clicked() {
            if let Some(e) = mh_entity {
                if let Ok(mut mc) = miner_cfg_q.get_mut(e) {
                    mc.manual_mine = true;
                }
            }
            dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
            ui_state.assigning_mine = Some(mh_slot);
        }
        if assigned.is_some() || manual {
            if ui.button("Clear").clicked() {
                if let Some(e) = mh_entity {
                    if let Ok(mut mc) = miner_cfg_q.get_mut(e) {
                        mc.manual_mine = false;
                        mc.assigned_mine = None;
                    }
                }
                dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
            }
        }
    });
    action
}

/// Render building inspector content when a building cell is selected.
pub(crate) fn building_inspector_content(
    ui: &mut egui::Ui,
    bld: &mut BuildingInspectorData,
    world_data: &mut WorldData,
    mining_policy: &mut MiningPolicy,
    dirty_writers: &mut crate::messages::DirtyWriters,
    npc_stats_q: &mut Query<&mut NpcStats>,
    ui_state: &mut UiState,
    settings: &UserSettings,
    gpu_state: &GpuReadState,
    copy_text: &mut Option<String>,
    faction_select: &mut MessageWriter<crate::messages::SelectFactionMsg>,
) -> Option<InspectorAction> {
    let (kind, bld_town_idx, world_pos, col, row) =
        selected_building_info(&bld.selected_building, &bld.grid, &bld.entity_map)?;

    let def = building_def(kind);
    let town_idx = bld_town_idx as usize;

    // Header
    ui.strong(def.label);

    // Town + faction
    if let Some(town) = world_data.towns.get(town_idx) {
        ui.label(format!("Town: {}", town.name));
        if ui
            .link(format!("Faction: {} (F{})", town.name, town.faction))
            .clicked()
        {
            ui_state.left_panel_open = true;
            ui_state.left_panel_tab = LeftPanelTab::Factions;
            faction_select.write(crate::messages::SelectFactionMsg(town.faction));
        }
    } else if kind == BuildingKind::GoldMine {
        ui.label("Faction: Unowned");
    }

    // Construction status from ECS ConstructionProgress
    let bld_slot = bld.entity_map.slot_at_position(world_pos);
    let bld_entity = bld_slot.and_then(|s| bld.entity_map.entities.get(&s).copied());
    let construction_remaining = bld_entity
        .and_then(|e| bld.construction_q.get(e).ok())
        .map(|cp| cp.0)
        .unwrap_or(0.0);
    let is_constructing = construction_remaining > 0.0;
    if is_constructing {
        let total = crate::constants::BUILDING_CONSTRUCT_SECS;
        let progress = ((total - construction_remaining) / total).clamp(0.0, 1.0);
        ui.colored_label(egui::Color32::from_rgb(200, 200, 40), "Under Construction");
        ui.horizontal(|ui| {
            ui.label("Progress:");
            ui.add(
                egui::ProgressBar::new(progress)
                    .text(format!(
                        "{:.0}% ({:.1}s)",
                        progress * 100.0,
                        construction_remaining
                    ))
                    .fill(egui::Color32::from_rgb(200, 160, 40)),
            );
        });
    }

    // Per-type details (hidden during construction)
    if !is_constructing {
        match kind {
            BuildingKind::Farm => {
                // Farm mode toggle
                let current_mode = bld_entity
                    .and_then(|e| bld.farm_mode_q.get(e).ok())
                    .map_or(FarmMode::Crops, |m| m.0);
                ui.horizontal(|ui| {
                    ui.label("Mode:");
                    let mut is_cows = current_mode == FarmMode::Cows;
                    if ui.selectable_label(!is_cows, "Crops").clicked() && is_cows {
                        is_cows = false;
                    }
                    if ui.selectable_label(is_cows, "Cows").clicked() && !is_cows {
                        is_cows = true;
                    }
                    let new_mode = if is_cows {
                        FarmMode::Cows
                    } else {
                        FarmMode::Crops
                    };
                    if new_mode != current_mode {
                        if let Some(e) = bld_entity {
                            if let Ok(mut fm) = bld.farm_mode_q.get_mut(e) {
                                fm.0 = new_mode;
                            }
                        }
                    }
                });

                if let Some(ps) = bld_entity.and_then(|e| bld.production_q.get(e).ok()) {
                    let state_name = if ps.ready {
                        "Ready to harvest"
                    } else {
                        "Growing"
                    };
                    ui.label(format!("Status: {}", state_name));

                    let color = if ps.ready {
                        egui::Color32::from_rgb(200, 200, 60)
                    } else {
                        egui::Color32::from_rgb(80, 180, 80)
                    };
                    ui.horizontal(|ui| {
                        ui.label("Growth:");
                        ui.add(
                            egui::ProgressBar::new(ps.progress)
                                .text(format!("{:.0}%", ps.progress * 100.0))
                                .fill(color),
                        );
                    });

                    if current_mode == FarmMode::Crops {
                        let occupants = bld_slot
                            .map(|s| bld.entity_map.occupant_count(s))
                            .unwrap_or(0);
                        ui.label(format!("Farmers: {}", occupants));
                    } else {
                        ui.label("Autonomous (no farmer needed)");
                    }
                }
            }

            BuildingKind::Waypoint => {
                let order = bld_entity
                    .and_then(|e| bld.waypoint_order_q.get(e).ok())
                    .map(|w| w.0)
                    .unwrap_or(0);
                ui.label(format!("Patrol order: {}", order));
            }

            BuildingKind::Fountain => {
                // Healing + tower info
                let base_radius = bld.combat_config.heal_radius;
                let levels = bld.town_access.upgrade_levels(town_idx as i32);
                let upgrade_bonus =
                    UPGRADES.stat_level(&levels, "Town", UpgradeStatKind::FountainRange) as f32
                        * 24.0;
                let tower = resolve_town_tower_stats(&levels);
                ui.label(format!("Heal radius: {:.0}px", base_radius + upgrade_bonus));
                ui.label(format!("Heal rate: {:.0}/s", bld.combat_config.heal_rate));
                ui.separator();
                ui.label(format!("Tower range: {:.0}px", tower.range));
                ui.label(format!("Tower damage: {:.1}", tower.damage));
                ui.label(format!("Tower cooldown: {:.2}s", tower.cooldown));
                ui.label(format!(
                    "Tower projectile life: {:.2}s",
                    tower.proj_lifetime
                ));

                // Kills / XP / Level from TowerBuildingState
                if let Some(tbs) = bld_entity.and_then(|e| bld.tower_bld_q.get(e).ok()) {
                    let level = level_from_xp(tbs.xp);
                    let xp_next = (level + 1) * (level + 1) * 100;
                    ui.label(format!(
                        "Kills: {}  Lv.{}  XP: {}/{}",
                        tbs.kills, level, tbs.xp, xp_next
                    ));
                }

                let food = bld.town_access.food(town_idx as i32);
                ui.label(format!("Food: {}", food));
            }

            BuildingKind::Bed => {
                ui.label("Rest point");
            }

            BuildingKind::GoldMine => {
                if let Some(mine_inst) = bld.entity_map.find_by_position(world_pos) {
                    let mine_label = if let Some(idx) = bld.entity_map.gold_mine_index(world_pos) {
                        crate::ui::gold_mine_name(idx)
                    } else {
                        format!("Gold Mine (slot {})", mine_inst.slot)
                    };
                    ui.label(format!("Name: {}", mine_label));
                    let enabled = *mining_policy
                        .mine_enabled
                        .get(&mine_inst.slot)
                        .unwrap_or(&true);
                    let label = if enabled {
                        "Auto-mining: ON"
                    } else {
                        "Auto-mining: OFF"
                    };
                    if ui.button(label).clicked() {
                        mining_policy.mine_enabled.insert(mine_inst.slot, !enabled);
                        dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
                    }
                }
                if let Some(ps) = bld_entity.and_then(|e| bld.production_q.get(e).ok()) {
                    let label = if ps.ready {
                        "Ready to harvest".to_string()
                    } else {
                        format!("Growing: {:.0}%", ps.progress * 100.0)
                    };
                    ui.label(&label);
                    let color = if ps.ready {
                        egui::Color32::from_rgb(200, 180, 40)
                    } else if ps.progress > 0.0 {
                        egui::Color32::from_rgb(160, 140, 40)
                    } else {
                        egui::Color32::from_rgb(100, 100, 100)
                    };
                    ui.add(
                        egui::ProgressBar::new(ps.progress)
                            .text(format!("{:.0}%", ps.progress * 100.0))
                            .fill(color),
                    );
                    let occupants = bld_slot
                        .map(|s| bld.entity_map.occupant_count(s))
                        .unwrap_or(0);
                    if occupants > 0 {
                        let mult = crate::constants::mine_productivity_mult(occupants);
                        ui.label(format!(
                            "Miners: {} ({:.0}% speed)",
                            occupants,
                            mult * 100.0
                        ));
                    }
                }
            }

            BuildingKind::Wall | BuildingKind::Gate => {
                // Wall/gate tier info + upgrade button
                if let Some(_wall_inst) = bld.entity_map.find_by_position(world_pos) {
                    let wall_lv = bld_entity
                        .and_then(|e| bld.wall_level_q.get(e).ok())
                        .map(|w| w.0)
                        .unwrap_or(1);
                    let level = wall_lv.max(1) as usize;
                    let tier_name = WALL_TIER_NAMES.get(level - 1).unwrap_or(&"Wall");
                    let tier_hp = WALL_TIER_HP.get(level - 1).copied().unwrap_or(80.0);
                    ui.label(format!("Tier: {} (Lv.{})", tier_name, level));
                    ui.label(format!("Max HP: {:.0}", tier_hp));

                    // Show current HP from building entity
                    {
                        let hp = bld_entity
                            .and_then(|e| bld.building_health.get(e).ok())
                            .map(|h| h.0);
                        if let Some(hp) = hp {
                            let color = if hp > tier_hp * 0.5 {
                                egui::Color32::from_rgb(80, 200, 80)
                            } else {
                                egui::Color32::from_rgb(200, 80, 80)
                            };
                            ui.horizontal(|ui| {
                                ui.label("HP:");
                                ui.add(
                                    egui::ProgressBar::new(hp / tier_hp)
                                        .text(format!("{:.0}/{:.0}", hp, tier_hp))
                                        .fill(color),
                                );
                            });
                        }
                    }

                    // Upgrade button (if not max tier)
                    if level < 3 {
                        let costs = WALL_UPGRADE_COSTS[level - 1];
                        let cost_str: Vec<String> = costs
                            .iter()
                            .map(|(r, amt)| match r {
                                ResourceKind::Food => format!("{} food", amt),
                                ResourceKind::Gold => format!("{} gold", amt),
                                ResourceKind::Wood => format!("{} wood", amt),
                                ResourceKind::Stone => format!("{} stone", amt),
                            })
                            .collect();
                        let next_name = WALL_TIER_NAMES[level];
                        let can_afford = costs.iter().all(|(r, amt)| match r {
                            ResourceKind::Food => bld.town_access.food(town_idx as i32) >= *amt,
                            ResourceKind::Gold => bld.town_access.gold(town_idx as i32) >= *amt,
                            ResourceKind::Wood | ResourceKind::Stone => false,
                        });

                        ui.separator();
                        let btn_text =
                            format!("Upgrade to {} ({})", next_name, cost_str.join(", "));
                        let btn = ui.add_enabled(
                            can_afford,
                            egui::Button::new(egui::RichText::new(btn_text).color(if can_afford {
                                egui::Color32::from_rgb(80, 200, 200)
                            } else {
                                egui::Color32::from_rgb(120, 120, 120)
                            })),
                        );
                        if btn.clicked() && can_afford {
                            // Deduct costs
                            for (r, amt) in costs {
                                match r {
                                    ResourceKind::Food => {
                                        if let Some(mut f) =
                                            bld.town_access.food_mut(town_idx as i32)
                                        {
                                            f.0 -= amt;
                                        }
                                    }
                                    ResourceKind::Gold => {
                                        if let Some(mut g) =
                                            bld.town_access.gold_mut(town_idx as i32)
                                        {
                                            g.0 -= amt;
                                        }
                                    }
                                    ResourceKind::Wood | ResourceKind::Stone => {}
                                }
                            }
                            // Upgrade wall level + HP via ECS
                            let new_level = (level + 1) as u8;
                            let new_hp = WALL_TIER_HP[level]; // level is 0-indexed for next tier
                            if let Some(e) = bld_entity {
                                if let Ok(mut wl) = bld.wall_level_q.get_mut(e) {
                                    wl.0 = new_level;
                                }
                                if let Ok(mut health) = bld.building_health.get_mut(e) {
                                    health.0 = new_hp;
                                }
                            }
                            dirty_writers
                                .building_grid
                                .write(crate::messages::BuildingGridDirtyMsg);
                        }
                    } else {
                        ui.colored_label(egui::Color32::from_rgb(200, 180, 40), "Max tier reached");
                    }
                }
            }

            kind if crate::constants::building_def(kind).is_tower
                && kind != BuildingKind::Fountain =>
            {
                // Resolve per-instance stats from ECS TowerBuildingState
                let slot = bld_slot.unwrap_or(usize::MAX);
                let (level, upgrade_levels_clone) = bld_entity
                    .and_then(|e| bld.tower_bld_q.get(e).ok())
                    .map(|tbs| (level_from_xp(tbs.xp), tbs.upgrade_levels.clone()))
                    .unwrap_or((0, Vec::new()));
                // tower building kinds always have tower_stats in BUILDING_REGISTRY
                let base = crate::constants::building_def(kind).tower_stats?;
                let stats = resolve_tower_instance_stats(&base, level, &upgrade_levels_clone);

                // Tower combat stats (resolved)
                ui.label(format!("Range: {:.0}px", stats.range));
                ui.label(format!("Damage: {:.1}", stats.damage));
                ui.label(format!("Cooldown: {:.2}s", stats.cooldown));
                if stats.hp_regen > 0.0 {
                    ui.label(format!("HP Regen: {:.1}/s", stats.hp_regen));
                }

                // HP bar
                if let Some(&entity) = bld.entity_map.entities.get(&slot) {
                    if let Ok(health) = bld.building_health.get(entity) {
                        let max_hp = stats.max_hp;
                        let pct = health.0 / max_hp;
                        let color = if pct > 0.5 {
                            egui::Color32::from_rgb(80, 200, 80)
                        } else {
                            egui::Color32::from_rgb(200, 80, 80)
                        };
                        ui.horizontal(|ui| {
                            ui.label("HP:");
                            ui.add(
                                egui::ProgressBar::new(pct)
                                    .text(format!("{:.0}/{:.0}", health.0, max_hp))
                                    .fill(color),
                            );
                        });
                    }
                }

                // Kills / XP / Level
                if let Some(tbs) = bld_entity.and_then(|e| bld.tower_bld_q.get(e).ok()) {
                    let xp_next = (level + 1) * (level + 1) * 100;
                    ui.label(format!(
                        "Kills: {}  Lv.{}  XP: {}/{}",
                        tbs.kills, level, tbs.xp, xp_next
                    ));
                }

                // Upgrade button — opens popup window
                if ui
                    .button(egui::RichText::new("Upgrades").strong())
                    .clicked()
                {
                    ui_state.tower_upgrade_slot = Some(slot);
                }
            }

            BuildingKind::Merchant => {
                let tidx = town_idx;
                let stock = bld.merchant_inv.stocks.get(tidx);
                let stock_count = stock.map(|s| s.items.len()).unwrap_or(0);
                let timer = stock.map(|s| s.refresh_timer).unwrap_or(0.0);
                ui.label(format!(
                    "Stock ({} items) — refresh in {:.1}h",
                    stock_count, timer
                ));
                ui.separator();

                // List stock items with Buy buttons
                let mut buy_id: Option<u64> = None;
                if let Some(stock) = bld.merchant_inv.stocks.get(tidx) {
                    for item in &stock.items {
                        let (r, g, b) = item.rarity.color();
                        let cost = item.rarity.gold_cost();
                        let gold = bld.town_access.gold(tidx as i32);
                        let can_afford = gold >= cost;
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(&item.name)
                                    .color(egui::Color32::from_rgb(r, g, b)),
                            );
                            ui.label(format!("{:?} +{:.0}%", item.kind, item.stat_bonus * 100.0));
                            let btn = ui.add_enabled(
                                can_afford,
                                egui::Button::new(format!("Buy {}g", cost)),
                            );
                            if btn.clicked() && can_afford {
                                buy_id = Some(item.id);
                            }
                        });
                    }
                }
                // Process buy
                if let Some(id) = buy_id {
                    if let Some(item) = bld.merchant_inv.remove(tidx, id) {
                        let cost = item.rarity.gold_cost();
                        if let Some(mut g) = bld.town_access.gold_mut(tidx as i32) {
                            g.0 -= cost;
                        }
                        if let Some(mut eq) = bld.town_access.equipment_mut(tidx as i32) {
                            eq.0.push(item);
                        }
                    }
                }

                // Sell section — items from town equipment
                ui.separator();
                ui.label("Sell from inventory:");
                let mut sell_id: Option<u64> = None;
                let inv_items = bld.town_access.equipment(tidx as i32).unwrap_or_default();
                for item in &inv_items {
                    let (r, g, b) = item.rarity.color();
                    let sell_price = item.rarity.gold_cost() / 2;
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&item.name).color(egui::Color32::from_rgb(r, g, b)),
                        );
                        ui.label(format!("{:?}", item.kind));
                        if ui.button(format!("Sell {}g", sell_price)).clicked() {
                            sell_id = Some(item.id);
                        }
                    });
                }
                drop(inv_items);
                // Process sell
                if let Some(id) = sell_id {
                    let sold_rarity =
                        if let Some(mut eq) = bld.town_access.equipment_mut(tidx as i32) {
                            eq.0.iter().position(|it| it.id == id).map(|pos| {
                                let item = eq.0.swap_remove(pos);
                                item.rarity
                            })
                        } else {
                            None
                        };
                    if let Some(rarity) = sold_rarity {
                        let sell_price = rarity.gold_cost() / 2;
                        if let Some(mut g) = bld.town_access.gold_mut(tidx as i32) {
                            g.0 += sell_price;
                        }
                    }
                }

                // Reroll button
                ui.separator();
                let reroll_cost = 50;
                let gold = bld.town_access.gold(tidx as i32);
                let can_reroll = gold >= reroll_cost;
                let btn = ui.add_enabled(
                    can_reroll,
                    egui::Button::new(format!("Reroll Stock ({}g)", reroll_cost)),
                );
                if btn.clicked() && can_reroll {
                    if let Some(mut g) = bld.town_access.gold_mut(tidx as i32) {
                        g.0 -= reroll_cost;
                    }
                    bld.merchant_inv.refresh(tidx, &mut bld.next_loot_id);
                }
            }

            BuildingKind::Casino => {
                if ui
                    .button(egui::RichText::new("Open Casino").size(16.0).strong())
                    .clicked()
                {
                    ui_state.casino_open = true;
                }
            }

            _ => {
                if let Some(spawner) = def.spawner {
                    let spawns_label = npc_def(Job::from_i32(spawner.job)).label;
                    if let Some(inst) = bld
                        .entity_map
                        .find_by_position(world_pos)
                        .filter(|i| crate::constants::building_def(i.kind).spawner.is_some())
                    {
                        ui.label(format!("Spawns: {}", spawns_label));
                        let spawner_state = bld
                            .entity_map
                            .entities
                            .get(&inst.slot)
                            .and_then(|&e| bld.spawner_q.get(e).ok());
                        let npc_slot_opt = spawner_state.and_then(|s| s.npc_slot);
                        let respawn_timer = spawner_state.map(|s| s.respawn_timer).unwrap_or(0.0);
                        if let Some(slot) = npc_slot_opt {
                            if bld.entity_map.get_npc(slot).is_some() {
                                if let Some(action) =
                                    npc_link(ui, npc_stats_q, &bld.entity_map, slot)
                                {
                                    return Some(action);
                                }
                                ui.colored_label(egui::Color32::from_rgb(80, 200, 80), "Alive");
                                if let Some(npc) = bld.entity_map.get_npc(slot) {
                                    let mut parts: Vec<&str> = Vec::new();
                                    let combat_name = bld
                                        .combat_state_q
                                        .get(npc.entity)
                                        .map(|cs| cs.name())
                                        .unwrap_or("");
                                    if !combat_name.is_empty() {
                                        parts.push(combat_name);
                                    }
                                    parts.push(
                                        bld.activity_q
                                            .get(npc.entity)
                                            .map(|a| a.name())
                                            .unwrap_or("Unknown"),
                                    );
                                    ui.label(format!("State: {}", parts.join(", ")));
                                    if let Some(sq) =
                                        bld.squad_id_q.get(npc.entity).ok().map(|s| s.0)
                                    {
                                        ui.label(format!("Squad: {}", sq + 1));
                                    }
                                    let has_patrol = bld
                                        .patrol_route_q
                                        .get(npc.entity)
                                        .is_ok_and(|r| !r.posts.is_empty());
                                    ui.label(format!(
                                        "Patrol route: {}",
                                        if has_patrol { "yes" } else { "none" }
                                    ));
                                    if slot * 2 + 1 < gpu_state.positions.len() {
                                        let px = gpu_state.positions[slot * 2];
                                        let py = gpu_state.positions[slot * 2 + 1];
                                        if px > -9000.0 {
                                            ui.label(format!("GPU pos: ({:.0}, {:.0})", px, py));
                                        }
                                    }
                                    ui.label(format!(
                                        "Home: ({:.0}, {:.0})",
                                        bld.home_q.get(npc.entity).map(|h| h.0.x).unwrap_or(0.0),
                                        bld.home_q.get(npc.entity).map(|h| h.0.y).unwrap_or(0.0)
                                    ));
                                }
                            }
                        } else if respawn_timer > 0.0 {
                            ui.colored_label(
                                egui::Color32::from_rgb(200, 200, 40),
                                format!("Respawning in {:.0}h", respawn_timer),
                            );
                        } else {
                            ui.colored_label(egui::Color32::from_rgb(200, 200, 40), "Spawning...");
                        }
                    }
                    if def.kind == BuildingKind::MinerHome {
                        ui.separator();
                        let mh_slot = bld
                            .entity_map
                            .find_by_position(world_pos)
                            .filter(|i| i.kind == BuildingKind::MinerHome)
                            .map(|i| i.slot);
                        if let Some(mh_slot) = mh_slot {
                            if let Some(action) = mine_assignment_ui(
                                ui,
                                world_data,
                                &bld.entity_map,
                                mh_slot,
                                world_pos,
                                dirty_writers,
                                ui_state,
                                &mut bld.miner_cfg_q,
                            ) {
                                return Some(action);
                            }
                        }
                    }
                }
            }
        }
    } // end if !is_constructing + match

    // Debug IDs: show slot + UID + world coords for BRP queries, plus copy button
    if settings.debug_ids {
        ui.separator();
        let selected_slot = bld.selected_building.slot.or_else(|| {
            bld.entity_map
                .find_by_position(world_pos)
                .map(|inst| inst.slot)
        });
        if let Some(slot) = selected_slot {
            let entity = bld.entity_map.entities.get(&slot);
            let entity_str = entity.map_or("?".to_string(), |e| format!("{:?}", e));
            ui.label(format!(
                "Slot: {}  Entity: {}  Pos: ({:.0}, {:.0})",
                slot, entity_str, world_pos.x, world_pos.y
            ));

            if ui.button("Copy Debug Info").clicked() {
                let max_hp = crate::constants::building_def(kind).hp;
                let hp = bld
                    .entity_map
                    .entities
                    .get(&slot)
                    .and_then(|&e| bld.building_health.get(e).ok())
                    .map(|h| h.0)
                    .unwrap_or(0.0);
                let town_name = world_data
                    .towns
                    .get(town_idx)
                    .map(|t| t.name.as_str())
                    .unwrap_or("?");
                let faction_text = world_data
                    .towns
                    .get(town_idx)
                    .map(|t| format!("{} (F{})", t.name, t.faction))
                    .unwrap_or_else(|| "?".to_string());
                let mut info = format!(
                    "{name} [{kind:?}]\n\
                     Slot: {slot}  Entity: {entity}\n\
                     Town: {town}  Faction: {faction}\n\
                     Pos: ({px:.0}, {py:.0})  Grid: ({col}, {row})\n\
                     HP: {hp:.0}/{max:.0}\n",
                    name = def.label,
                    kind = kind,
                    slot = slot,
                    entity = entity_str,
                    town = town_name,
                    faction = faction_text,
                    px = world_pos.x,
                    py = world_pos.y,
                    col = col,
                    row = row,
                    hp = hp,
                    max = max_hp,
                );
                // Spawner NPC state
                if let Some(spawner) = def.spawner {
                    let spawns_label = npc_def(Job::from_i32(spawner.job)).label;
                    info.push_str(&format!("Spawns: {}\n", spawns_label));
                    if let Some(ss) = bld_entity.and_then(|e| bld.spawner_q.get(e).ok()) {
                        if let Some(npc_slot) = ss.npc_slot {
                            if let Some(npc) = bld.entity_map.get_npc(npc_slot) {
                                let npc_entity = Some(npc.entity);
                                let (name, level) = npc_stats_q
                                    .get(npc.entity)
                                    .map(|s| {
                                        (
                                            s.name.as_str(),
                                            crate::systems::stats::level_from_xp(s.xp),
                                        )
                                    })
                                    .unwrap_or(("?", 0));
                                info.push_str(&format!(
                                    "NPC: {} (Lv.{}) slot={} entity={:?}\n",
                                    name, level, npc_slot, npc_entity
                                ));
                            }
                        } else if ss.respawn_timer > 0.0 {
                            info.push_str(&format!("Respawning in {:.0}h\n", ss.respawn_timer));
                        }
                    }
                }
                *copy_text = Some(info);
            }
        }
    }
    None
}
