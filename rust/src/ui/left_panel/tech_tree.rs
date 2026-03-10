//! Visual tech tree window - top-down node graph with tabbed branches.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::constants::FACTION_PLAYER;
use crate::resources::*;
use crate::settings::UserSettings;
use crate::systems::stats::{
    UPGRADES, UpgradeMsg, branch_total, format_upgrade_cost,
    missing_prereqs, upgrade_available, upgrade_unlocked, upgrade_count, upgrade_effect_summary,
};
use crate::world::WorldData;

// Layout constants
const NODE_W: f32 = 176.0;
const NODE_H: f32 = 76.0;
const COL_SPACING: f32 = 192.0;
const ROW_SPACING: f32 = 92.0;

/// Node visual state.
#[derive(Clone, Copy)]
enum NodeState {
    Locked,
    Unlocked,   // prereqs met, can't afford
    Available,  // can buy
    Maxed,      // at max level
}

fn node_state(levels: &[u8], idx: usize, food: i32, gold: i32) -> NodeState {
    let lv = levels.get(idx).copied().unwrap_or(0);
    let node = &UPGRADES.nodes[idx];
    if let Some(max) = node.max_level {
        if lv >= max {
            return NodeState::Maxed;
        }
    }
    if !upgrade_unlocked(levels, idx) {
        return NodeState::Locked;
    }
    if upgrade_available(levels, idx, food, gold) {
        NodeState::Available
    } else {
        NodeState::Unlocked
    }
}

fn node_bg(state: NodeState) -> egui::Color32 {
    match state {
        NodeState::Locked => egui::Color32::from_rgb(26, 30, 38),
        NodeState::Unlocked => egui::Color32::from_rgb(38, 45, 58),
        NodeState::Available => egui::Color32::from_rgb(22, 74, 48),
        NodeState::Maxed => egui::Color32::from_rgb(40, 52, 88),
    }
}

fn node_border(state: NodeState) -> egui::Stroke {
    match state {
        NodeState::Locked => egui::Stroke::new(1.0, egui::Color32::from_rgb(66, 72, 84)),
        NodeState::Unlocked => egui::Stroke::new(1.0, egui::Color32::from_rgb(98, 110, 132)),
        NodeState::Available => egui::Stroke::new(2.0, egui::Color32::from_rgb(112, 221, 141)),
        NodeState::Maxed => egui::Stroke::new(1.5, egui::Color32::from_rgb(150, 174, 255)),
    }
}

fn node_text_color(state: NodeState) -> egui::Color32 {
    match state {
        NodeState::Locked => egui::Color32::from_gray(124),
        _ => egui::Color32::from_gray(220),
    }
}

fn line_color(unlocked: bool) -> egui::Color32 {
    if unlocked {
        egui::Color32::from_rgb(106, 126, 150)
    } else {
        egui::Color32::from_gray(46)
    }
}

/// Positioned node for drawing.
struct PlacedNode {
    idx: usize,
    rect: egui::Rect,
}

/// Compute top-down node positions for a branch.
/// Returns (placed_nodes, content_width, content_height).
fn layout_branch_topdown(
    branch: &crate::systems::stats::UpgradeBranch,
    origin: egui::Pos2,
) -> (Vec<PlacedNode>, f32, f32) {
    let reg = &*UPGRADES;
    let mut placed: Vec<PlacedNode> = Vec::new();

    if branch.entries.is_empty() {
        return (placed, 0.0, 0.0);
    }

    let max_depth = branch.entries.iter().map(|&(_, d)| d).max().unwrap_or(0);

    // Group entries by depth row
    let mut depth_rows: Vec<Vec<usize>> = vec![Vec::new(); (max_depth + 1) as usize];
    for &(idx, depth) in &branch.entries {
        depth_rows[depth as usize].push(idx);
    }

    // Phase 1: initial placement - spread each row evenly
    let mut node_pos: std::collections::HashMap<usize, (f32, f32)> =
        std::collections::HashMap::new();

    for (depth, row) in depth_rows.iter().enumerate() {
        let y = origin.y + depth as f32 * ROW_SPACING;
        let row_width = row.len() as f32 * COL_SPACING;
        let start_x = origin.x - row_width / 2.0 + COL_SPACING / 2.0;
        for (i, &idx) in row.iter().enumerate() {
            let x = start_x + i as f32 * COL_SPACING;
            node_pos.insert(idx, (x, y));
        }
    }

    // Phase 2: center parents above their children, then resolve collisions
    // Work bottom-up so grandparents center above already-centered parents
    for depth in (0..max_depth).rev() {
        for &parent_idx in &depth_rows[depth as usize] {
            let children: Vec<usize> = depth_rows
                .get((depth + 1) as usize)
                .map(|row| {
                    row.iter()
                        .filter(|&&ci| {
                            reg.nodes[ci]
                                .prereqs
                                .iter()
                                .any(|&(pi, _)| pi == parent_idx)
                        })
                        .copied()
                        .collect()
                })
                .unwrap_or_default();

            if !children.is_empty() {
                let child_xs: Vec<f32> =
                    children.iter().filter_map(|ci| node_pos.get(ci).map(|p| p.0)).collect();
                let center_x =
                    (child_xs.iter().copied().fold(f32::MAX, f32::min)
                        + child_xs.iter().copied().fold(f32::MIN, f32::max))
                        / 2.0;
                if let Some(pos) = node_pos.get_mut(&parent_idx) {
                    pos.0 = center_x;
                }
            }
        }

        // Resolve collisions: sort row by X, push apart where too close
        let row = &depth_rows[depth as usize];
        let mut sorted: Vec<usize> = row.clone();
        sorted.sort_by(|a, b| {
            let ax = node_pos[a].0;
            let bx = node_pos[b].0;
            ax.partial_cmp(&bx).unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in 1..sorted.len() {
            let prev_x = node_pos[&sorted[i - 1]].0;
            let curr_x = node_pos[&sorted[i]].0;
            if curr_x - prev_x < COL_SPACING {
                let shift = COL_SPACING - (curr_x - prev_x);
                node_pos.get_mut(&sorted[i]).unwrap().0 += shift;
            }
        }
    }

    // Also resolve collisions on the deepest row (max_depth) which isn't a parent row
    {
        let row = &depth_rows[max_depth as usize];
        let mut sorted: Vec<usize> = row.clone();
        sorted.sort_by(|a, b| {
            let ax = node_pos[a].0;
            let bx = node_pos[b].0;
            ax.partial_cmp(&bx).unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in 1..sorted.len() {
            let prev_x = node_pos[&sorted[i - 1]].0;
            let curr_x = node_pos[&sorted[i]].0;
            if curr_x - prev_x < COL_SPACING {
                let shift = COL_SPACING - (curr_x - prev_x);
                node_pos.get_mut(&sorted[i]).unwrap().0 += shift;
            }
        }
    }

    // Phase 3: normalize X positions so nothing goes negative
    let min_x = node_pos.values().map(|p| p.0).fold(f32::MAX, f32::min);
    let offset_x = if min_x < origin.x { origin.x - min_x + NODE_W / 2.0 } else { 0.0 };

    let mut max_x = 0.0_f32;
    let mut max_y = 0.0_f32;
    for &(idx, _depth) in &branch.entries {
        let Some(&(x, y)) = node_pos.get(&idx) else {
            continue;
        };
        let px = x + offset_x - NODE_W / 2.0;
        let py = y;
        placed.push(PlacedNode {
            idx,
            rect: egui::Rect::from_min_size(egui::pos2(px, py), egui::vec2(NODE_W, NODE_H)),
        });
        max_x = max_x.max(px + NODE_W);
        max_y = max_y.max(py + NODE_H);
    }

    placed.sort_by(|a, b| {
        a.rect
            .min
            .y
            .total_cmp(&b.rect.min.y)
            .then_with(|| a.rect.min.x.total_cmp(&b.rect.min.x))
            .then_with(|| a.idx.cmp(&b.idx))
    });

    let content_w = max_x - origin.x + 20.0;
    let content_h = max_y - origin.y + 20.0;
    (placed, content_w, content_h)
}

pub fn tech_tree_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    world_data: Res<WorldData>,
    mut upgrade: super::UpgradeParams,
    town_access: crate::systemparams::TownAccess,
    mut settings: ResMut<UserSettings>,
) -> Result {
    if !ui_state.tech_tree_open {
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;

    let town_idx = world_data
        .towns
        .iter()
        .position(|t| t.faction == FACTION_PLAYER)
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
    let reg = &*UPGRADES;

    // Use tech_tree_tab from UiState if available, otherwise default 0
    let active_tab = ui_state.tech_tree_tab.min(reg.branches.len().saturating_sub(1));

    let mut open = ui_state.tech_tree_open;
    egui::Window::new("Tech Tree")
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_size([900.0, 600.0])
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            // Header: resources
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("Food: {food}")).strong());
                ui.separator();
                ui.label(egui::RichText::new(format!("Gold: {gold}")).strong());
                ui.separator();
                ui.label(egui::RichText::new(format!("Villagers: {alive}")).strong());
            });
            ui.label(
                egui::RichText::new("Click a node to buy. Use Auto to queue hourly buys.")
                    .small()
                    .color(egui::Color32::from_gray(160)),
            );
            ui.add_space(4.0);

            // Tab bar
            ui.horizontal(|ui| {
                for (i, branch) in reg.branches.iter().enumerate() {
                    let bt = branch_total(&levels, branch.label);
                    let label = format!("{} ({})", branch.label, bt);
                    let is_active = i == active_tab;
                    let text = if is_active {
                        egui::RichText::new(&label).strong()
                    } else {
                        egui::RichText::new(&label)
                    };
                    let btn = ui.selectable_label(is_active, text);
                    if btn.clicked() {
                        ui_state.tech_tree_tab = i;
                    }
                }
            });
            ui.separator();

            // Draw active branch
            let current_tab = ui_state.tech_tree_tab.min(reg.branches.len().saturating_sub(1));
            if let Some(branch) = reg.branches.get(current_tab) {
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let origin = ui.cursor().min + egui::vec2(20.0, 10.0);
                        let (placed, content_w, content_h) = layout_branch_topdown(branch, origin);

                        // Draw connection lines first (behind nodes)
                        let painter = ui.painter();
                        for pn in &placed {
                            let node = &reg.nodes[pn.idx];
                            for &(prereq_idx, _min_lv) in &node.prereqs {
                                if let Some(parent) = placed.iter().find(|p| p.idx == prereq_idx) {
                                    let unlocked = upgrade_unlocked(&levels, pn.idx);
                                    let color = line_color(unlocked);
                                    let stroke = egui::Stroke::new(1.5, color);
                                    // Parent bottom-center -> child top-center
                                    let from = egui::pos2(
                                        parent.rect.center().x,
                                        parent.rect.bottom(),
                                    );
                                    let to = egui::pos2(pn.rect.center().x, pn.rect.top());
                                    let mid_y = (from.y + to.y) / 2.0;
                                    // Right-angle connector: down, across, down
                                    painter.line_segment(
                                        [from, egui::pos2(from.x, mid_y)],
                                        stroke,
                                    );
                                    painter.line_segment(
                                        [egui::pos2(from.x, mid_y), egui::pos2(to.x, mid_y)],
                                        stroke,
                                    );
                                    painter.line_segment(
                                        [egui::pos2(to.x, mid_y), to],
                                        stroke,
                                    );
                                }
                            }
                        }

                        // Draw node boxes with proper egui child layout
                        for pn in &placed {
                            let node = &reg.nodes[pn.idx];
                            let lv = levels.get(pn.idx).copied().unwrap_or(0);
                            let state = node_state(&levels, pn.idx, food, gold);
                            let node_idx = pn.idx;

                            // Node interaction region is registered before child widgets so
                            // checkbox controls stay clickable on top.
                            let node_resp = ui.interact(
                                pn.rect,
                                egui::Id::new(("tech_node", node_idx)),
                                egui::Sense::click() | egui::Sense::hover(),
                            );

                            // Background + border via painter (decorative only)
                            let painter = ui.painter();
                            let mut fill = node_bg(state);
                            if node_resp.hovered() {
                                fill = fill.gamma_multiply(1.12);
                            }
                            painter.rect_filled(
                                pn.rect.translate(egui::vec2(0.0, 2.0)),
                                8.0,
                                egui::Color32::from_black_alpha(50),
                            );
                            painter.rect_filled(pn.rect, 8.0, fill);
                            painter.rect_stroke(
                                pn.rect,
                                8.0,
                                node_border(state),
                                egui::StrokeKind::Outside,
                            );

                            // Child UI for real widgets; advance_cursor_after_rect
                            // tells the parent ScrollArea this space is used.
                            let inner = pn.rect.shrink(3.0);
                            let mut child = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(inner)
                                    .layout(egui::Layout::top_down(egui::Align::Center)),
                            );

                            let text_color = node_text_color(state);

                            // Row 1: auto-buy checkbox + label
                            child.horizontal(|ui| {
                                if !matches!(state, NodeState::Locked) {
                                    upgrade.auto.ensure_towns(town_idx + 1);
                                    let count = upgrade_count();
                                    upgrade.auto.flags[town_idx].resize(count, false);
                                    let auto_flag =
                                        &mut upgrade.auto.flags[town_idx][node_idx];
                                    let prev = *auto_flag;
                                    ui.push_id(("auto_checkbox", node_idx), |ui| {
                                        ui.add(egui::Checkbox::without_text(auto_flag))
                                            .on_hover_text("Auto-buy each game hour");
                                    });
                                    if *auto_flag != prev {
                                        settings.auto_upgrades =
                                            upgrade.auto.flags[town_idx].clone();
                                    }
                                    ui.label(
                                        egui::RichText::new("Auto")
                                            .size(10.0)
                                            .color(egui::Color32::from_gray(150)),
                                    );
                                } else {
                                    ui.add_space(40.0);
                                }
                                ui.label(
                                    egui::RichText::new(node.label)
                                        .size(12.5)
                                        .color(text_color),
                                );
                            });

                            // Row 2: status (level + effect)
                            let (current_effect, _next) = upgrade_effect_summary(node_idx, lv);
                            let status = if matches!(state, NodeState::Maxed) {
                                if node.max_level == Some(1) {
                                    "Unlocked".to_string()
                                } else {
                                    format!("Lv{}  {}", lv, current_effect)
                                }
                            } else if matches!(state, NodeState::Locked) {
                                "Locked".to_string()
                            } else if lv == 0 {
                                "Lv0".to_string()
                            } else {
                                format!("Lv{}  {}", lv, current_effect)
                            };
                            let status_color = match state {
                                NodeState::Available => egui::Color32::from_rgb(160, 220, 100),
                                NodeState::Maxed => egui::Color32::from_rgb(120, 120, 200),
                                _ => egui::Color32::from_gray(150),
                            };
                            child.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(&status)
                                        .size(10.0)
                                        .color(status_color),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let cost_text = format_upgrade_cost(node_idx, lv);
                                        let cost_color = match state {
                                            NodeState::Available => {
                                                egui::Color32::from_rgb(140, 220, 140)
                                            }
                                            NodeState::Locked => egui::Color32::from_gray(105),
                                            _ => egui::Color32::from_gray(165),
                                        };
                                        ui.label(
                                            egui::RichText::new(cost_text)
                                                .size(10.0)
                                                .color(cost_color),
                                        );
                                    },
                                );
                            });

                            // Advance parent cursor so ScrollArea knows the space
                            ui.advance_cursor_after_rect(pn.rect);

                            // Tooltip
                            node_resp.clone().on_hover_ui(|ui| {
                                ui.label(egui::RichText::new(node.label).strong());
                                ui.label(node.tooltip);
                                let (now, next) = upgrade_effect_summary(node_idx, lv);
                                ui.label(format!("{} -> {}", now, next));
                                if matches!(state, NodeState::Available | NodeState::Unlocked) {
                                    let cost = format_upgrade_cost(node_idx, lv);
                                    ui.label(format!("Cost: {}", cost));
                                }
                                if let Some(msg) = missing_prereqs(&levels, node_idx) {
                                    ui.label(
                                        egui::RichText::new(msg)
                                            .color(egui::Color32::from_rgb(200, 100, 100)),
                                    );
                                }
                            });

                            // Click to buy
                            if node_resp.clicked() && matches!(state, NodeState::Available) {
                                upgrade.queue.write(UpgradeMsg {
                                    town_idx,
                                    upgrade_idx: node_idx,
                                });
                            }
                        }

                        // Reserve space for scroll area
                        ui.allocate_space(egui::vec2(content_w, content_h));
                    });
            }
        });
    ui_state.tech_tree_open = open;

    Ok(())
}

