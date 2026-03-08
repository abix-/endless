//! Visual tech tree window — full-width immersive upgrade tree with left-to-right node graph.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::constants::FACTION_PLAYER;
use crate::resources::*;
use crate::settings::UserSettings;
use crate::systems::stats::{
    UPGRADES, UpgradeBranch, UpgradeMsg, branch_total, format_upgrade_cost,
    missing_prereqs, upgrade_available, upgrade_count, upgrade_effect_summary, upgrade_unlocked,
};
use crate::world::WorldData;

// Layout constants
const NODE_W: f32 = 110.0;
const NODE_H: f32 = 38.0;
const COL_SPACING: f32 = 150.0;
const ROW_SPACING: f32 = 6.0;
const BRANCH_SPACING: f32 = 16.0;
const SECTION_SPACING: f32 = 24.0;

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
        NodeState::Locked => egui::Color32::from_gray(35),
        NodeState::Unlocked => egui::Color32::from_gray(55),
        NodeState::Available => egui::Color32::from_rgb(30, 70, 30),
        NodeState::Maxed => egui::Color32::from_rgb(35, 35, 90),
    }
}

fn node_border(state: NodeState) -> egui::Stroke {
    match state {
        NodeState::Locked => egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
        NodeState::Unlocked => egui::Stroke::new(1.0, egui::Color32::from_gray(100)),
        NodeState::Available => egui::Stroke::new(2.0, egui::Color32::from_rgb(180, 180, 50)),
        NodeState::Maxed => egui::Stroke::new(1.5, egui::Color32::from_rgb(80, 80, 180)),
    }
}

fn node_text_color(state: NodeState) -> egui::Color32 {
    match state {
        NodeState::Locked => egui::Color32::from_gray(90),
        _ => egui::Color32::from_gray(220),
    }
}

fn line_color(unlocked: bool) -> egui::Color32 {
    if unlocked {
        egui::Color32::from_gray(120)
    } else {
        egui::Color32::from_gray(45)
    }
}

/// Positioned node for drawing.
struct PlacedNode {
    idx: usize,
    rect: egui::Rect,
}

/// Compute node positions for a branch. Returns (placed_nodes, total_height).
fn layout_branch(branch: &UpgradeBranch, origin: egui::Pos2) -> (Vec<PlacedNode>, f32) {
    let mut placed: Vec<PlacedNode> = Vec::new();

    // Group entries by depth, preserving DFS order
    let max_depth = branch.entries.iter().map(|&(_, d)| d).max().unwrap_or(0);

    // For each root node, place it and its children aligned vertically
    let mut y_cursor = origin.y;

    // Track which nodes are roots (depth 0) and build parent→children map
    let reg = &*UPGRADES;
    let branch_indices: Vec<usize> = branch.entries.iter().map(|&(idx, _)| idx).collect();

    // Walk DFS order from entries (already in DFS order from emit_tree)
    // Place roots at depth 0, children at depth 1, etc.
    // Children should be vertically adjacent to their parent

    // First pass: count nodes per depth column to allocate rows
    let mut depth_rows: Vec<Vec<usize>> = vec![Vec::new(); (max_depth + 1) as usize];
    for &(idx, depth) in &branch.entries {
        depth_rows[depth as usize].push(idx);
    }

    // Place nodes by walking the DFS-ordered entries
    // For nodes with children, center the parent vertically across its children
    // Simple approach: assign rows by DFS order, place children next to parent

    // Simpler layout: for each entry in DFS order, track Y by parent relationship
    let mut node_y: std::collections::HashMap<usize, f32> = std::collections::HashMap::new();

    for &(idx, depth) in &branch.entries {
        let x = origin.x + depth as f32 * COL_SPACING;
        let y = if depth == 0 {
            // Root node: place at current cursor
            let y = y_cursor;
            y_cursor = y + NODE_H + ROW_SPACING;
            y
        } else {
            // Child node: place at cursor (will be near parent since DFS order)
            let y = y_cursor;
            y_cursor = y + NODE_H + ROW_SPACING;
            y
        };
        node_y.insert(idx, y);
        placed.push(PlacedNode {
            idx,
            rect: egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(NODE_W, NODE_H)),
        });
    }

    // Second pass: for root nodes with children, vertically center the parent
    // across its children's Y range
    for &(idx, depth) in &branch.entries {
        if depth != 0 {
            continue;
        }
        // Find children of this root
        let children: Vec<usize> = branch_indices
            .iter()
            .filter(|&&ci| {
                ci != idx && reg.nodes[ci].prereqs.iter().any(|&(pi, _)| pi == idx)
            })
            .copied()
            .collect();
        if children.is_empty() {
            continue;
        }
        // Get Y range of children
        let child_ys: Vec<f32> = children
            .iter()
            .filter_map(|ci| node_y.get(ci).copied())
            .collect();
        if child_ys.is_empty() {
            continue;
        }
        let min_y = child_ys.iter().copied().fold(f32::MAX, f32::min);
        let max_y = child_ys.iter().copied().fold(f32::MIN, f32::max);
        let center = (min_y + max_y + NODE_H) / 2.0 - NODE_H / 2.0;

        // Re-center parent
        if let Some(p) = placed.iter_mut().find(|n| n.idx == idx) {
            p.rect = egui::Rect::from_min_size(
                egui::pos2(p.rect.min.x, center),
                egui::vec2(NODE_W, NODE_H),
            );
        }
    }

    let total_h = y_cursor - origin.y;
    (placed, total_h)
}

pub fn tech_tree_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    world_data: Res<WorldData>,
    mut upgrade: super::UpgradeParams,
    gold_storage: Res<GoldStorage>,
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
    let food = upgrade
        .food_storage
        .food
        .get(town_idx)
        .copied()
        .unwrap_or(0);
    let gold = gold_storage.gold.get(town_idx).copied().unwrap_or(0);
    let player_faction = world_data
        .towns
        .get(town_idx)
        .map(|t| t.faction as usize)
        .unwrap_or(0);
    let villager_stats = upgrade.faction_stats.stats.get(player_faction);
    let alive = villager_stats.map(|s| s.alive).unwrap_or(0);
    let levels = upgrade.upgrades.town_levels(town_idx);
    let reg = &*UPGRADES;

    let mut open = ui_state.tech_tree_open;
    egui::Window::new("Tech Tree")
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_size([900.0, 600.0])
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            // Header row
            ui.horizontal(|ui| {
                ui.label(format!("Food: {food}"));
                ui.separator();
                ui.label(format!("Gold: {gold}"));
                ui.separator();
                ui.label(format!("Villagers: {alive}"));
                ui.add_space(20.0);
                for branch in &reg.branches {
                    let bt = branch_total(&levels, branch.label);
                    ui.label(
                        egui::RichText::new(format!("{}:{}", branch.label, bt))
                            .small()
                            .weak(),
                    );
                }
            });
            ui.separator();

            // Scrollable tree canvas
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Reserve space for the full tree, then draw with painter
                    let origin = ui.cursor().min;
                    let mut y_offset = 0.0_f32;

                    for section_name in ["Economy", "Military"] {
                        // Section header
                        let section_y = origin.y + y_offset;
                        ui.painter().text(
                            egui::pos2(origin.x + 4.0, section_y),
                            egui::Align2::LEFT_TOP,
                            section_name,
                            egui::FontId::proportional(16.0),
                            egui::Color32::from_gray(200),
                        );
                        y_offset += SECTION_SPACING;

                        for branch in reg.branches.iter().filter(|b| b.section == section_name) {
                            // Branch label
                            let branch_y = origin.y + y_offset;
                            let bt = branch_total(&levels, branch.label);
                            ui.painter().text(
                                egui::pos2(origin.x + 8.0, branch_y),
                                egui::Align2::LEFT_TOP,
                                format!("{} ({})", branch.label, bt),
                                egui::FontId::proportional(13.0),
                                egui::Color32::from_gray(160),
                            );
                            y_offset += 18.0;

                            // Layout and draw branch tree
                            let branch_origin =
                                egui::pos2(origin.x + 12.0, origin.y + y_offset);
                            let (placed, tree_h) = layout_branch(branch, branch_origin);

                            // Draw connection lines
                            let painter = ui.painter();
                            for pn in &placed {
                                let node = &reg.nodes[pn.idx];
                                for &(prereq_idx, _min_lv) in &node.prereqs {
                                    if let Some(parent) =
                                        placed.iter().find(|p| p.idx == prereq_idx)
                                    {
                                        let unlocked = upgrade_unlocked(&levels, pn.idx);
                                        let color = line_color(unlocked);
                                        let from = egui::pos2(
                                            parent.rect.right(),
                                            parent.rect.center().y,
                                        );
                                        let to = egui::pos2(
                                            pn.rect.left(),
                                            pn.rect.center().y,
                                        );
                                        let mid_x = (from.x + to.x) / 2.0;
                                        // Right-angle connector
                                        painter.line_segment(
                                            [from, egui::pos2(mid_x, from.y)],
                                            egui::Stroke::new(1.5, color),
                                        );
                                        painter.line_segment(
                                            [
                                                egui::pos2(mid_x, from.y),
                                                egui::pos2(mid_x, to.y),
                                            ],
                                            egui::Stroke::new(1.5, color),
                                        );
                                        painter.line_segment(
                                            [egui::pos2(mid_x, to.y), to],
                                            egui::Stroke::new(1.5, color),
                                        );
                                    }
                                }
                            }

                            // Draw node boxes
                            for pn in &placed {
                                let node = &reg.nodes[pn.idx];
                                let lv = levels.get(pn.idx).copied().unwrap_or(0);
                                let state = node_state(&levels, pn.idx, food, gold);

                                // Background + border
                                let painter = ui.painter();
                                painter.rect_filled(
                                    pn.rect,
                                    4.0,
                                    node_bg(state),
                                );
                                painter.rect_stroke(
                                    pn.rect,
                                    4.0,
                                    node_border(state),
                                    egui::StrokeKind::Outside,
                                );

                                // Label text (short name + level)
                                let text_color = node_text_color(state);
                                let label = if node.max_level == Some(1) {
                                    if lv >= 1 {
                                        format!("{} ✓", node.short)
                                    } else {
                                        node.short.to_string()
                                    }
                                } else {
                                    format!("{} Lv{}", node.short, lv)
                                };
                                painter.text(
                                    pn.rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    &label,
                                    egui::FontId::proportional(12.0),
                                    text_color,
                                );

                                // Interaction (click + hover)
                                let resp = ui.allocate_rect(
                                    pn.rect,
                                    egui::Sense::click() | egui::Sense::hover(),
                                );

                                // Click to buy (before hover which consumes resp)
                                let clicked = resp.clicked();
                                let node_idx = pn.idx;

                                // Tooltip
                                resp.on_hover_ui(|ui| {
                                    ui.label(
                                        egui::RichText::new(node.label).strong(),
                                    );
                                    ui.label(node.tooltip);
                                    let (now, next) = upgrade_effect_summary(node_idx, lv);
                                    ui.label(format!("{} → {}", now, next));
                                    if matches!(state, NodeState::Available | NodeState::Unlocked)
                                    {
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

                                if clicked && matches!(state, NodeState::Available) {
                                    upgrade.queue.write(UpgradeMsg {
                                        town_idx,
                                        upgrade_idx: node_idx,
                                    });
                                }

                                // Auto-upgrade checkbox (small, top-right corner)
                                if !matches!(state, NodeState::Locked) {
                                    let cb_size = 14.0;
                                    let cb_rect = egui::Rect::from_min_size(
                                        egui::pos2(
                                            pn.rect.right() - cb_size - 2.0,
                                            pn.rect.top() + 2.0,
                                        ),
                                        egui::vec2(cb_size, cb_size),
                                    );
                                    upgrade.auto.ensure_towns(town_idx + 1);
                                    let count = upgrade_count();
                                    upgrade.auto.flags[town_idx].resize(count, false);
                                    let is_auto = upgrade.auto.flags[town_idx]
                                        .get(pn.idx)
                                        .copied()
                                        .unwrap_or(false);
                                    let cb_resp = ui.allocate_rect(
                                        cb_rect,
                                        egui::Sense::click(),
                                    );
                                    // Draw checkbox visual
                                    let painter = ui.painter();
                                    painter.rect_stroke(
                                        cb_rect,
                                        2.0,
                                        egui::Stroke::new(1.0, egui::Color32::from_gray(100)),
                                        egui::StrokeKind::Outside,
                                    );
                                    if is_auto {
                                        painter.text(
                                            cb_rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "A",
                                            egui::FontId::proportional(9.0),
                                            egui::Color32::from_rgb(100, 200, 100),
                                        );
                                    }
                                    let cb_clicked = cb_resp.clicked();
                                    cb_resp.on_hover_text("Auto-buy each game hour");
                                    if cb_clicked {
                                        upgrade.auto.flags[town_idx][pn.idx] = !is_auto;
                                        settings.auto_upgrades =
                                            upgrade.auto.flags[town_idx].clone();
                                    }
                                }
                            }

                            y_offset += tree_h.max(NODE_H) + BRANCH_SPACING;
                        }

                        y_offset += SECTION_SPACING * 0.5;
                    }

                    // Reserve the total space so ScrollArea knows the content size
                    let max_x = reg
                        .branches
                        .iter()
                        .map(|b| {
                            b.entries
                                .iter()
                                .map(|&(_, d)| d)
                                .max()
                                .unwrap_or(0) as f32
                                * COL_SPACING
                                + NODE_W
                                + 24.0
                        })
                        .fold(0.0_f32, f32::max);
                    ui.allocate_space(egui::vec2(max_x, y_offset));
                });
        });
    ui_state.tech_tree_open = open;

    Ok(())
}
