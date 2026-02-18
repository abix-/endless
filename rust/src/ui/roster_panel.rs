//! Roster panel — NPC list with sorting and filtering (mirrors roster_panel.gd).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::components::*;
use crate::resources::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn { Name, Job, Level, Hp, State, Trait }

#[derive(Default)]
pub struct RosterState {
    sort_column: Option<SortColumn>,
    sort_descending: bool,
    job_filter: i32, // -1=all, 0=farmer, 1=archer, 2=raider
    frame_counter: u32,
    /// Cached rows, rebuilt every 30 frames
    cached_rows: Vec<RosterRow>,
}

#[derive(Clone)]
struct RosterRow {
    slot: usize,
    name: String,
    job: i32,
    level: i32,
    hp: f32,
    max_hp: f32,
    state: String,
    trait_name: String,
}

pub fn roster_panel_system(
    mut contexts: EguiContexts,
    ui_state: Res<UiState>,
    mut selected: ResMut<SelectedNpc>,
    meta_cache: Res<NpcMetaCache>,
    health_query: Query<(&NpcIndex, &Health, &CachedStats, &Activity, &CombatState), Without<Dead>>,
    mut state: Local<RosterState>,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    gpu_state: Res<GpuReadState>,
) -> Result {
    if !ui_state.roster_open {
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;

    // Rebuild cache every 30 frames
    state.frame_counter += 1;
    if state.frame_counter % 30 == 1 || state.cached_rows.is_empty() {
        let mut rows = Vec::new();
        for (npc_idx, health, cached, activity, combat) in health_query.iter() {
            let idx = npc_idx.0;
            let meta = &meta_cache.0[idx];

            // Job filter
            if state.job_filter >= 0 && meta.job != state.job_filter {
                continue;
            }

            let state_str = if combat.is_fighting() {
                combat.name().to_string()
            } else {
                activity.name().to_string()
            };

            rows.push(RosterRow {
                slot: idx,
                name: meta.name.clone(),
                job: meta.job,
                level: meta.level,
                hp: health.0,
                max_hp: cached.max_health,
                state: state_str,
                trait_name: crate::trait_name(meta.trait_id).to_string(),
            });
        }

        // Sort
        if let Some(col) = state.sort_column {
            rows.sort_by(|a, b| {
                let ord = match col {
                    SortColumn::Name => a.name.cmp(&b.name),
                    SortColumn::Job => a.job.cmp(&b.job),
                    SortColumn::Level => a.level.cmp(&b.level),
                    SortColumn::Hp => a.hp.partial_cmp(&b.hp).unwrap_or(std::cmp::Ordering::Equal),
                    SortColumn::State => a.state.cmp(&b.state),
                    SortColumn::Trait => a.trait_name.cmp(&b.trait_name),
                };
                if state.sort_descending { ord.reverse() } else { ord }
            });
        } else {
            // Default: sort by level descending
            rows.sort_by(|a, b| b.level.cmp(&a.level));
        }

        state.cached_rows = rows;
    }

    egui::SidePanel::right("roster").default_width(480.0).show(ctx, |ui| {
        ui.heading("Roster");

        // Filter row
        ui.horizontal(|ui| {
            if ui.selectable_label(state.job_filter == -1, "All").clicked() {
                state.job_filter = -1;
                state.frame_counter = 0; // force refresh
            }
            if ui.selectable_label(state.job_filter == 1, "Archers").clicked() {
                state.job_filter = 1;
                state.frame_counter = 0;
            }
            if ui.selectable_label(state.job_filter == 0, "Farmers").clicked() {
                state.job_filter = 0;
                state.frame_counter = 0;
            }
            if ui.selectable_label(state.job_filter == 4, "Miners").clicked() {
                state.job_filter = 4;
                state.frame_counter = 0;
            }
            if ui.selectable_label(state.job_filter == 2, "Raiders").clicked() {
                state.job_filter = 2;
                state.frame_counter = 0;
            }
        });

        ui.label(format!("{} NPCs", state.cached_rows.len()));
        ui.separator();

        // Sort headers — pre-compute arrow strings to avoid borrow conflict
        fn arrow_str(state: &RosterState, col: SortColumn) -> &'static str {
            if state.sort_column == Some(col) {
                if state.sort_descending { " v" } else { " ^" }
            } else {
                ""
            }
        }

        let name_arrow = arrow_str(&state, SortColumn::Name);
        let job_arrow = arrow_str(&state, SortColumn::Job);
        let level_arrow = arrow_str(&state, SortColumn::Level);
        let hp_arrow = arrow_str(&state, SortColumn::Hp);
        let state_arrow = arrow_str(&state, SortColumn::State);
        let trait_arrow = arrow_str(&state, SortColumn::Trait);

        // Header row
        let mut clicked_col: Option<SortColumn> = None;
        ui.horizontal(|ui| {
            if ui.button(format!("Name{}", name_arrow)).clicked() { clicked_col = Some(SortColumn::Name); }
            if ui.button(format!("Job{}", job_arrow)).clicked() { clicked_col = Some(SortColumn::Job); }
            if ui.button(format!("Lv{}", level_arrow)).clicked() { clicked_col = Some(SortColumn::Level); }
            if ui.button(format!("HP{}", hp_arrow)).clicked() { clicked_col = Some(SortColumn::Hp); }
            if ui.button(format!("State{}", state_arrow)).clicked() { clicked_col = Some(SortColumn::State); }
            if ui.button(format!("Trait{}", trait_arrow)).clicked() { clicked_col = Some(SortColumn::Trait); }
        });

        if let Some(col) = clicked_col {
            if state.sort_column == Some(col) {
                state.sort_descending = !state.sort_descending;
            } else {
                state.sort_column = Some(col);
                state.sort_descending = true;
            }
            state.frame_counter = 0;
        }

        ui.separator();

        // Scrollable NPC list
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let selected_idx = selected.0;
            let mut new_selected: Option<i32> = None;
            let mut follow_idx: Option<usize> = None;

            for row in &state.cached_rows {
                let is_selected = selected_idx == row.slot as i32;
                let job_name = crate::job_name(row.job);

                // Job color indicator
                let job_color = match row.job {
                    0 => egui::Color32::from_rgb(80, 200, 80),   // Farmer green
                    1 => egui::Color32::from_rgb(80, 100, 220),  // Archer blue
                    2 => egui::Color32::from_rgb(220, 80, 80),   // Raider red
                    4 => egui::Color32::from_rgb(160, 110, 60),  // Miner brown
                    _ => egui::Color32::from_rgb(220, 220, 80),
                };

                let response = ui.horizontal(|ui| {
                    // Highlight selected row
                    if is_selected {
                        let rect = ui.available_rect_before_wrap();
                        ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgba_premultiplied(60, 60, 100, 80));
                    }

                    // Name (truncated)
                    let name_text = if row.name.len() > 16 { &row.name[..16] } else { &row.name };
                    ui.colored_label(job_color, name_text);

                    ui.label(job_name);
                    ui.label(format!("{}", row.level));

                    // HP compact
                    let hp_frac = if row.max_hp > 0.0 { row.hp / row.max_hp } else { 0.0 };
                    let hp_color = if hp_frac > 0.6 {
                        egui::Color32::from_rgb(80, 200, 80)
                    } else if hp_frac > 0.3 {
                        egui::Color32::from_rgb(200, 200, 40)
                    } else {
                        egui::Color32::from_rgb(200, 60, 60)
                    };
                    ui.colored_label(hp_color, format!("{:.0}/{:.0}", row.hp, row.max_hp));

                    ui.label(&row.state);

                    if !row.trait_name.is_empty() {
                        ui.small(&row.trait_name);
                    }

                    // Select button
                    if ui.small_button("◎").clicked() {
                        new_selected = Some(row.slot as i32);
                    }

                    // Follow button
                    if ui.small_button("▶").clicked() {
                        new_selected = Some(row.slot as i32);
                        follow_idx = Some(row.slot);
                    }

                });

                // Click anywhere on row to select
                if response.response.clicked() {
                    new_selected = Some(row.slot as i32);
                }
            }

            if let Some(idx) = new_selected {
                selected.0 = idx;
            }

            // Follow: move camera to NPC position
            if let Some(idx) = follow_idx {
                if idx * 2 + 1 < gpu_state.positions.len() {
                    let x = gpu_state.positions[idx * 2];
                    let y = gpu_state.positions[idx * 2 + 1];
                    if let Ok(mut transform) = camera_query.single_mut() {
                        transform.translation.x = x;
                        transform.translation.y = y;
                    }
                }
            }

        });
    });

    Ok(())
}
