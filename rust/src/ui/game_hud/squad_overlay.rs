//! Squad target overlay and faction commander arrows.

use super::dc_slots;
use crate::components::*;
use crate::gpu::EntityGpuState;
use crate::resources::*;
use crate::settings::UserSettings;
use crate::world::{WorldData, WorldGrid};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use std::collections::HashMap;

// ============================================================================
// TARGET OVERLAY
// ============================================================================

/// Draw a target indicator line from selected NPC to its movement target.
/// Shows full A* path through all remaining waypoints when pathfinding is active.
/// Uses egui painter on the background layer so it renders over the game viewport.
pub fn target_overlay_system(
    mut contexts: EguiContexts,
    selected: Res<SelectedNpc>,
    gpu_state: Res<GpuReadState>,
    buffer_writes: Res<EntityGpuState>,
    entity_map: Res<crate::entity_map::EntityMap>,
    grid: Res<WorldGrid>,
    path_q: Query<&NpcPath>,
    camera_query: Query<(&Transform, &Projection), With<crate::render::MainCamera>>,
    windows: Query<&Window>,
) -> Result {
    if selected.0 < 0 {
        return Ok(());
    }
    let idx = selected.0 as usize;

    let positions = &gpu_state.positions;
    let targets = &buffer_writes.targets;
    if idx * 2 + 1 >= positions.len() || idx * 2 + 1 >= targets.len() {
        return Ok(());
    }

    let npc_x = positions[idx * 2];
    let npc_y = positions[idx * 2 + 1];
    if npc_x < -9000.0 {
        return Ok(());
    }

    let tgt_x = targets[idx * 2];
    let tgt_y = targets[idx * 2 + 1];

    // Skip if target == position (stationary)
    let dx = tgt_x - npc_x;
    let dy = tgt_y - npc_y;
    if dx * dx + dy * dy < 4.0 {
        return Ok(());
    }

    let Ok(window) = windows.single() else {
        return Ok(());
    };
    let Ok((transform, projection)) = camera_query.single() else {
        return Ok(());
    };

    let zoom = match projection {
        Projection::Orthographic(ortho) => 1.0 / ortho.scale,
        _ => 1.0,
    };
    let cam = transform.translation.truncate();
    let viewport = egui::Vec2::new(window.width(), window.height());
    let center = viewport * 0.5;

    let to_screen = |wx: f32, wy: f32| -> egui::Pos2 {
        egui::Pos2::new(
            center.x + (wx - cam.x) * zoom,
            center.y - (wy - cam.y) * zoom,
        )
    };

    let npc_screen = to_screen(npc_x, npc_y);

    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::background());
    let line_color = egui::Color32::from_rgba_unmultiplied(255, 220, 50, 200);
    let stroke = egui::Stroke::new(2.5, line_color);
    let dot_color = egui::Color32::from_rgba_unmultiplied(255, 220, 50, 160);

    // Try to get the full A* path for this NPC
    let npc_path = entity_map
        .entities
        .get(&idx)
        .and_then(|&entity| path_q.get(entity).ok());

    let final_screen = if let Some(path) = npc_path.filter(|p| p.current < p.waypoints.len()) {
        // Draw full path: NPC → remaining waypoints → goal_world
        let mut prev = npc_screen;
        for wp in &path.waypoints[path.current..] {
            let world_pos = grid.grid_to_world(wp.x as usize, wp.y as usize);
            let wp_screen = to_screen(world_pos.x, world_pos.y);
            painter.line_segment([prev, wp_screen], stroke);
            // Small dot at intermediate waypoint
            painter.circle_filled(wp_screen, 3.0, dot_color);
            prev = wp_screen;
        }
        // Final segment from last waypoint to goal_world
        let goal_screen = to_screen(path.goal_world.x, path.goal_world.y);
        painter.line_segment([prev, goal_screen], stroke);
        goal_screen
    } else {
        // No A* path — single line to GPU target (direct movement)
        let tgt_screen = to_screen(tgt_x, tgt_y);
        painter.line_segment([npc_screen, tgt_screen], stroke);
        tgt_screen
    };

    // Diamond marker at final destination
    let s = 7.0;
    let diamond = [
        egui::Pos2::new(final_screen.x, final_screen.y - s),
        egui::Pos2::new(final_screen.x + s, final_screen.y),
        egui::Pos2::new(final_screen.x, final_screen.y + s),
        egui::Pos2::new(final_screen.x - s, final_screen.y),
    ];
    let fill = egui::Color32::from_rgba_unmultiplied(255, 220, 50, 240);
    painter.add(egui::Shape::convex_polygon(
        diamond.to_vec(),
        fill,
        egui::Stroke::NONE,
    ));

    Ok(())
}

// ============================================================================
// SQUAD TARGET OVERLAY
// ============================================================================

/// Draw numbered markers at each squad's target position.
pub fn squad_overlay_system(
    mut contexts: EguiContexts,
    squad_state: Res<SquadState>,
    ui_state: Res<UiState>,
    gpu_state: Res<GpuReadState>,
    camera_query: Query<(&Transform, &Projection), With<crate::render::MainCamera>>,
    windows: Query<&Window>,
    entity_map: Res<EntityMap>,
    manual_target_q: Query<&ManualTarget>,
    npc_flags_q: Query<&NpcFlags>,
) -> Result {
    let Ok(window) = windows.single() else {
        return Ok(());
    };
    let Ok((transform, projection)) = camera_query.single() else {
        return Ok(());
    };

    let zoom = match projection {
        Projection::Orthographic(ortho) => 1.0 / ortho.scale,
        _ => 1.0,
    };
    let cam = transform.translation.truncate();
    let viewport = egui::Vec2::new(window.width(), window.height());
    let center = viewport * 0.5;

    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::background());

    // Squad colors (distinct per squad)
    let colors = [
        egui::Color32::from_rgb(255, 80, 80),   // red
        egui::Color32::from_rgb(80, 180, 255),  // blue
        egui::Color32::from_rgb(80, 220, 80),   // green
        egui::Color32::from_rgb(255, 200, 40),  // yellow
        egui::Color32::from_rgb(200, 80, 255),  // purple
        egui::Color32::from_rgb(255, 140, 40),  // orange
        egui::Color32::from_rgb(40, 220, 200),  // teal
        egui::Color32::from_rgb(255, 100, 180), // pink
        egui::Color32::from_rgb(180, 180, 80),  // olive
        egui::Color32::from_rgb(140, 140, 255), // light blue
    ];

    if !squad_state.box_selecting {
        for (i, squad) in squad_state.squads.iter().enumerate() {
            if !squad.is_player() {
                continue;
            }
            let Some(target) = squad.target else { continue };
            if squad.members.is_empty() {
                continue;
            }

            let screen = egui::Pos2::new(
                center.x + (target.x - cam.x) * zoom,
                center.y - (target.y - cam.y) * zoom,
            );

            let color = colors[i % colors.len()];
            let fill = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 120);

            // Filled circle with border
            painter.circle(screen, 10.0, fill, egui::Stroke::new(2.0, color));

            // Squad number label
            painter.text(
                screen,
                egui::Align2::CENTER_CENTER,
                format!("{}", i + 1),
                egui::FontId::proportional(11.0),
                egui::Color32::BLACK,
            );
        }
    }

    // Crosshair on DirectControl attack targets
    let positions = &gpu_state.positions;
    let xh_color = egui::Color32::from_rgba_unmultiplied(80, 220, 80, 200);
    // Collect unique target positions to avoid drawing multiple crosshairs on same spot
    let mut drawn_targets: Vec<egui::Pos2> = Vec::new();
    let dc_targets: Vec<ManualTarget> = dc_slots(&squad_state, &entity_map, |e| {
        npc_flags_q.get(e).is_ok_and(|f| f.direct_control)
    })
    .iter()
    .filter_map(|&slot| entity_map.get_npc(slot))
    .filter_map(|npc| manual_target_q.get(npc.entity).ok().cloned())
    .collect();
    for mt in &dc_targets {
        let world_pos = match mt {
            ManualTarget::Npc(slot) => {
                if *slot * 2 + 1 >= positions.len() {
                    continue;
                }
                let px = positions[*slot * 2];
                let py = positions[*slot * 2 + 1];
                if px < -9000.0 {
                    continue;
                }
                Vec2::new(px, py)
            }
            ManualTarget::Building(pos) => *pos,
            ManualTarget::Position(pos) => *pos,
        };
        let sp = egui::Pos2::new(
            center.x + (world_pos.x - cam.x) * zoom,
            center.y - (world_pos.y - cam.y) * zoom,
        );
        // Skip if we already drew a crosshair near this screen position
        if drawn_targets
            .iter()
            .any(|p| (p.x - sp.x).abs() < 2.0 && (p.y - sp.y).abs() < 2.0)
        {
            continue;
        }
        drawn_targets.push(sp);
        let r = 10.0_f32;
        let gap = 3.0_f32;
        painter.line_segment(
            [
                egui::Pos2::new(sp.x - r, sp.y),
                egui::Pos2::new(sp.x - gap, sp.y),
            ],
            egui::Stroke::new(2.0, xh_color),
        );
        painter.line_segment(
            [
                egui::Pos2::new(sp.x + gap, sp.y),
                egui::Pos2::new(sp.x + r, sp.y),
            ],
            egui::Stroke::new(2.0, xh_color),
        );
        painter.line_segment(
            [
                egui::Pos2::new(sp.x, sp.y - r),
                egui::Pos2::new(sp.x, sp.y - gap),
            ],
            egui::Stroke::new(2.0, xh_color),
        );
        painter.line_segment(
            [
                egui::Pos2::new(sp.x, sp.y + gap),
                egui::Pos2::new(sp.x, sp.y + r),
            ],
            egui::Stroke::new(2.0, xh_color),
        );
        painter.circle_stroke(sp, r, egui::Stroke::new(1.5, xh_color));
    }

    // Placement mode cursor hint
    if squad_state.placing_target && squad_state.selected >= 0 {
        if let Some(cursor_pos) = window.cursor_position() {
            let cursor_egui = egui::Pos2::new(cursor_pos.x, cursor_pos.y);
            let hint_color = egui::Color32::from_rgba_unmultiplied(255, 255, 100, 160);
            painter.circle_stroke(cursor_egui, 12.0, egui::Stroke::new(2.0, hint_color));
        }
    }

    // Box-select drag rectangle
    if squad_state.box_selecting {
        if let Some(start) = squad_state.drag_start {
            if let Some(cursor_pos) = window.cursor_position() {
                let start_screen = egui::Pos2::new(
                    center.x + (start.x - cam.x) * zoom,
                    center.y - (start.y - cam.y) * zoom,
                );
                let end_screen = egui::Pos2::new(cursor_pos.x, cursor_pos.y);
                let rect = egui::Rect::from_two_pos(start_screen, end_screen);
                let fill = egui::Color32::from_rgba_unmultiplied(80, 220, 80, 30);
                let stroke =
                    egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(80, 220, 80, 180));
                painter.rect_filled(rect, 0.0, fill);
                painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Outside);
            }
        }
    }

    // Mine assignment cursor hint
    if ui_state.assigning_mine.is_some() {
        if let Some(cursor_pos) = window.cursor_position() {
            let cursor_egui = egui::Pos2::new(cursor_pos.x, cursor_pos.y);
            let hint_color = egui::Color32::from_rgba_unmultiplied(200, 180, 40, 180);
            painter.circle_stroke(cursor_egui, 12.0, egui::Stroke::new(2.0, hint_color));
            painter.text(
                egui::Pos2::new(cursor_egui.x, cursor_egui.y + 18.0),
                egui::Align2::CENTER_TOP,
                "Click a gold mine",
                egui::FontId::proportional(12.0),
                hint_color,
            );
        }
    }

    Ok(())
}

/// Draw commander arrows for currently selected faction in Factions tab.
/// Arrows run from faction fountain/town center to each squad target.
pub fn faction_squad_overlay_system(
    mut contexts: EguiContexts,
    ui_state: Res<UiState>,
    settings: Res<UserSettings>,
    world_data: Res<WorldData>,
    squad_state: Res<SquadState>,
    camera_query: Query<(&Transform, &Projection), With<crate::render::MainCamera>>,
    windows: Query<&Window>,
) -> Result {
    let show_all = settings.show_all_faction_squad_lines;
    let selected_faction =
        if ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Factions {
            ui_state.factions_overlay_faction
        } else {
            None
        };
    if !show_all && selected_faction.is_none() {
        return Ok(());
    }

    let Ok(window) = windows.single() else {
        return Ok(());
    };
    let Ok((transform, projection)) = camera_query.single() else {
        return Ok(());
    };

    let zoom = match projection {
        Projection::Orthographic(ortho) => 1.0 / ortho.scale,
        _ => 1.0,
    };
    let cam = transform.translation.truncate();
    let viewport = egui::Vec2::new(window.width(), window.height());
    let center = viewport * 0.5;
    let to_screen = |p: Vec2| -> egui::Pos2 {
        egui::Pos2::new(
            center.x + (p.x - cam.x) * zoom,
            center.y - (p.y - cam.y) * zoom,
        )
    };

    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::background());
    let palette = [
        egui::Color32::from_rgb(255, 90, 90),
        egui::Color32::from_rgb(90, 180, 255),
        egui::Color32::from_rgb(100, 230, 120),
        egui::Color32::from_rgb(255, 210, 70),
        egui::Color32::from_rgb(210, 120, 255),
    ];

    // Build per-faction arrow start positions once (prefer fountain; fallback to town center).
    let mut start_by_faction: HashMap<i32, Vec2> = HashMap::new();
    for town in world_data.towns.iter() {
        let entry = start_by_faction.entry(town.faction).or_insert(town.center);
        if !town.is_raider() {
            *entry = town.center;
        }
    }

    // Per-faction color index so multiple factions can be drawn together without rescanning.
    let mut color_idx_by_faction: HashMap<i32, usize> = HashMap::new();

    for (si, squad) in squad_state.squads.iter().enumerate() {
        let faction = match squad.owner {
            SquadOwner::Player => 0,
            SquadOwner::Town(tdi) => match world_data.towns.get(tdi) {
                Some(t) => t.faction,
                None => continue,
            },
        };
        if !show_all && selected_faction != Some(faction) {
            continue;
        }
        let Some(target_world) = squad.target else {
            continue;
        };
        if squad.members.is_empty() {
            continue;
        }
        let Some(start_world) = start_by_faction.get(&faction).copied() else {
            continue;
        };

        let color_idx = color_idx_by_faction.entry(faction).or_insert(0usize);
        let color = palette[*color_idx % palette.len()];
        *color_idx += 1;
        let start = to_screen(start_world);
        let end = to_screen(target_world);
        let line = end - start;
        let len = line.length();
        if len < 6.0 {
            continue;
        }
        let dir = line / len;
        let perp = egui::vec2(-dir.y, dir.x);

        painter.line_segment([start, end], egui::Stroke::new(2.0, color));

        let head_len = 12.0;
        let head_w = 6.0;
        let base = end - dir * head_len;
        let p1 = base + perp * head_w;
        let p2 = base - perp * head_w;
        painter.add(egui::Shape::convex_polygon(
            vec![end, p1, p2],
            color,
            egui::Stroke::NONE,
        ));

        let label_pos = end + perp * 10.0;
        let label = if show_all {
            let town_label = world_data
                .towns
                .iter()
                .find(|t| t.faction == faction)
                .map(|t| t.name.as_str())
                .unwrap_or("?");
            format!("{} Squad {}", town_label, si + 1)
        } else {
            format!("Squad {}", si + 1)
        };
        painter.text(
            label_pos,
            egui::Align2::LEFT_BOTTOM,
            label,
            egui::FontId::proportional(12.0),
            color,
        );
    }

    Ok(())
}
