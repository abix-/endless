use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    Name,
    Job,
    Level,
    Hp,
    State,
    Trait,
}

#[derive(Default)]
pub struct RosterState {
    sort_column: Option<SortColumn>,
    sort_descending: bool,
    job_filter: i32,
    frame_counter: u32,
    cached_rows: Vec<RosterRow>,
    rename_slot: i32,
    rename_text: String,
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

#[derive(SystemParam)]
pub struct RosterParams<'w, 's> {
    pub selected: ResMut<'w, SelectedNpc>,
    pub entity_map: Res<'w, EntityMap>,
    pub npc_stats_q: Query<'w, 's, &'static mut NpcStats>,
    pub camera_query: Query<'w, 's, &'static mut Transform, With<crate::render::MainCamera>>,
    gpu_state: Res<'w, GpuReadState>,
    activity_q: Query<'w, 's, &'static Activity>,
    health_q: Query<'w, 's, &'static Health, Without<Building>>,
    cached_stats_q: Query<'w, 's, &'static CachedStats>,
    combat_state_q: Query<'w, 's, &'static CombatState>,
    personality_q: Query<'w, 's, &'static Personality>,
}

// ============================================================================
// ROSTER CONTENT
// ============================================================================

pub(crate) fn roster_content(
    ui: &mut egui::Ui,
    roster: &mut RosterParams,
    state: &mut RosterState,
    debug_all: bool,
) {
    // Rebuild cache every 30 frames
    state.frame_counter += 1;
    if state.frame_counter % 30 == 1 || state.cached_rows.is_empty() {
        let mut rows = Vec::new();
        for npc in roster.entity_map.iter_npcs() {
            if npc.dead {
                continue;
            }
            let idx = npc.slot;
            // Player faction only unless debug
            if !debug_all && npc.faction != crate::constants::FACTION_PLAYER {
                continue;
            }
            let job_i32 = npc.job as i32;
            if state.job_filter >= 0 && job_i32 != state.job_filter {
                continue;
            }
            let stats = roster.npc_stats_q.get(npc.entity).ok();
            let state_str = if roster
                .combat_state_q
                .get(npc.entity)
                .is_ok_and(|cs| cs.is_fighting())
            {
                roster
                    .combat_state_q
                    .get(npc.entity)
                    .map(|cs| cs.name().to_string())
                    .unwrap_or_default()
            } else {
                roster
                    .activity_q
                    .get(npc.entity)
                    .map(|a| a.name().to_string())
                    .unwrap_or_else(|_| "Unknown".to_string())
            };
            rows.push(RosterRow {
                slot: idx,
                name: stats.map(|s| s.name.clone()).unwrap_or_default(),
                job: job_i32,
                level: stats
                    .map(|s| crate::systems::stats::level_from_xp(s.xp))
                    .unwrap_or(0),
                hp: roster.health_q.get(npc.entity).map(|h| h.0).unwrap_or(0.0),
                max_hp: roster
                    .cached_stats_q
                    .get(npc.entity)
                    .map(|s| s.max_health)
                    .unwrap_or(100.0),
                state: state_str,
                trait_name: roster
                    .personality_q
                    .get(npc.entity)
                    .map(|p| p.trait_summary())
                    .unwrap_or_default(),
            });
        }

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
                if state.sort_descending {
                    ord.reverse()
                } else {
                    ord
                }
            });
        } else {
            rows.sort_by(|a, b| b.level.cmp(&a.level));
        }
        state.cached_rows = rows;
    }

    // Filter row
    ui.horizontal(|ui| {
        if ui.selectable_label(state.job_filter == -1, "All").clicked() {
            state.job_filter = -1;
            state.frame_counter = 0;
        }
        // Military first, then civilian
        for &military_first in &[true, false] {
            for def in crate::constants::NPC_REGISTRY.iter() {
                if def.is_military != military_first {
                    continue;
                }
                if def.job == Job::Raider && !debug_all {
                    continue;
                }
                let job_id = def.job as i32;
                if ui
                    .selectable_label(state.job_filter == job_id, def.label_plural)
                    .clicked()
                {
                    state.job_filter = job_id;
                    state.frame_counter = 0;
                }
            }
        }
    });

    // Miner target control — set how many villagers should be miners
    ui.label(format!("{} NPCs", state.cached_rows.len()));

    let selected_idx = roster.selected.0;
    if selected_idx >= 0 {
        let idx = selected_idx as usize;
        if let Some(npc) = roster.entity_map.get_npc(idx) {
            if state.rename_slot != selected_idx {
                state.rename_slot = selected_idx;
                state.rename_text = roster
                    .npc_stats_q
                    .get(npc.entity)
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
            }

            ui.horizontal(|ui| {
                ui.label("Rename:");
                let edit = ui.text_edit_singleline(&mut state.rename_text);
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button("Apply").clicked() || enter) && !state.rename_text.trim().is_empty() {
                    let new_name = state.rename_text.trim().to_string();
                    if let Ok(mut stats) = roster.npc_stats_q.get_mut(npc.entity) {
                        stats.name = new_name.clone();
                    }
                    state.rename_text = new_name;
                    state.frame_counter = 0;
                }
            });
        }
    } else {
        state.rename_slot = -1;
        state.rename_text.clear();
    }

    ui.separator();

    // Sort headers
    fn arrow_str(state: &RosterState, col: SortColumn) -> &'static str {
        if state.sort_column == Some(col) {
            if state.sort_descending {
                " \u{25BC}"
            } else {
                " \u{25B2}"
            }
        } else {
            ""
        }
    }

    let name_arrow = arrow_str(state, SortColumn::Name);
    let job_arrow = arrow_str(state, SortColumn::Job);
    let level_arrow = arrow_str(state, SortColumn::Level);
    let hp_arrow = arrow_str(state, SortColumn::Hp);
    let state_arrow = arrow_str(state, SortColumn::State);
    let trait_arrow = arrow_str(state, SortColumn::Trait);

    let mut clicked_col: Option<SortColumn> = None;
    ui.horizontal(|ui| {
        if ui.button(format!("Name{}", name_arrow)).clicked() {
            clicked_col = Some(SortColumn::Name);
        }
        if ui.button(format!("Job{}", job_arrow)).clicked() {
            clicked_col = Some(SortColumn::Job);
        }
        if ui.button(format!("Lv{}", level_arrow)).clicked() {
            clicked_col = Some(SortColumn::Level);
        }
        if ui.button(format!("HP{}", hp_arrow)).clicked() {
            clicked_col = Some(SortColumn::Hp);
        }
        if ui.button(format!("State{}", state_arrow)).clicked() {
            clicked_col = Some(SortColumn::State);
        }
        if ui.button(format!("Trait{}", trait_arrow)).clicked() {
            clicked_col = Some(SortColumn::Trait);
        }
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
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut new_selected: Option<i32> = None;
            let mut follow_idx: Option<usize> = None;

            for row in &state.cached_rows {
                let is_selected = selected_idx == row.slot as i32;
                let (r, g, b) = npc_def(Job::from_i32(row.job)).ui_color;
                let job_color = egui::Color32::from_rgb(r, g, b);

                let response = ui.horizontal(|ui| {
                    if is_selected {
                        let rect = ui.available_rect_before_wrap();
                        ui.painter().rect_filled(
                            rect,
                            0.0,
                            egui::Color32::from_rgba_premultiplied(60, 60, 100, 80),
                        );
                    }

                    let name_text = if row.name.len() > 16 {
                        &row.name[..16]
                    } else {
                        &row.name
                    };
                    ui.colored_label(job_color, name_text);
                    ui.label(crate::job_name(row.job));
                    ui.label(format!("{}", row.level));

                    let hp_frac = if row.max_hp > 0.0 {
                        row.hp / row.max_hp
                    } else {
                        0.0
                    };
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

                    if ui.small_button("◎").clicked() {
                        new_selected = Some(row.slot as i32);
                    }
                    if ui.small_button("▶").clicked() {
                        new_selected = Some(row.slot as i32);
                        follow_idx = Some(row.slot);
                    }
                });

                if response.response.clicked() {
                    new_selected = Some(row.slot as i32);
                }
            }

            if let Some(idx) = new_selected {
                roster.selected.0 = idx;
            }

            if let Some(idx) = follow_idx {
                if idx * 2 + 1 < roster.gpu_state.positions.len() {
                    let x = roster.gpu_state.positions[idx * 2];
                    let y = roster.gpu_state.positions[idx * 2 + 1];
                    if let Ok(mut transform) = roster.camera_query.single_mut() {
                        transform.translation.x = x;
                        transform.translation.y = y;
                    }
                }
            }
        });
}
