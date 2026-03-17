//! Combat log window.

use super::BottomPanelData;
use crate::resources::*;
use crate::settings::{self, UserSettings};
use crate::world::{WorldData, WorldGrid};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

#[derive(Default)]
pub struct LogFilterState {
    pub show_kills: bool,
    pub show_spawns: bool,
    pub show_raids: bool,
    pub show_harvests: bool,
    pub show_levelups: bool,
    pub show_npc_activity: bool,
    pub show_ai: bool,
    pub show_building_damage: bool,
    pub show_loot: bool,
    pub show_llm: bool,
    pub show_chat: bool,
    /// -1 = all factions, 0 = my faction only
    pub faction_filter: i32,
    pub initialized: bool,
    pub chat_input: String,
    // Cached merged log entries — skip rebuild when sources unchanged
    cached_selected_npc: i32,
    cached_filters: (
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
        bool,
        i32,
    ),
    cached_entries: Vec<(i64, egui::Color32, String, String, Option<bevy::math::Vec2>)>,
}

pub fn combat_log_system(
    mut contexts: EguiContexts,
    mut data: BottomPanelData,
    mut settings: ResMut<UserSettings>,
    mut filter_state: Local<LogFilterState>,
    ui_state: Res<UiState>,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut chat_inbox: ResMut<crate::resources::ChatInbox>,
    allowed_towns: Res<crate::resources::RemoteAllowedTowns>,
    world_data: Res<WorldData>,
    entity_map: Res<EntityMap>,
    grid: Res<WorldGrid>,
    mut selected_building: ResMut<SelectedBuilding>,
) -> Result {
    if !ui_state.combat_log_visible {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;

    // Init filter state from saved settings
    if !filter_state.initialized {
        filter_state.show_kills = settings.log_kills;
        filter_state.show_spawns = settings.log_spawns;
        filter_state.show_raids = settings.log_raids;
        filter_state.show_harvests = settings.log_harvests;
        filter_state.show_levelups = settings.log_levelups;
        filter_state.show_npc_activity = settings.log_npc_activity;
        filter_state.show_ai = settings.log_ai;
        filter_state.show_building_damage = settings.log_building_damage;
        filter_state.show_loot = settings.log_loot;
        filter_state.show_llm = settings.log_llm;
        filter_state.show_chat = settings.log_chat;
        filter_state.faction_filter = settings.log_faction_filter;
        filter_state.initialized = true;
    }

    let prev_filters = (
        filter_state.show_kills,
        filter_state.show_spawns,
        filter_state.show_raids,
        filter_state.show_harvests,
        filter_state.show_levelups,
        filter_state.show_npc_activity,
        filter_state.show_ai,
        filter_state.show_building_damage,
        filter_state.show_loot,
        filter_state.show_llm,
        filter_state.show_chat,
        filter_state.faction_filter,
    );

    let has_llm_towns = !allowed_towns.towns.is_empty();
    let mut chat_send: Option<String> = None;

    let frame = egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(30, 30, 35, 220))
        .inner_margin(egui::Margin::same(6));

    egui::Window::new("Combat Log")
        .anchor(egui::Align2::RIGHT_BOTTOM, [-2.0, -2.0])
        .default_size([450.0, 140.0])
        .max_height(300.0)
        .collapsible(false)
        .resizable(true)
        .movable(false)
        .frame(frame)
        .title_bar(true)
        .show(ctx, |ui| {
            // Filter checkboxes
            ui.horizontal_wrapped(|ui| {
                let faction_label = if filter_state.faction_filter == -1 {
                    "All"
                } else {
                    "Mine"
                };
                egui::ComboBox::from_id_salt("log_faction_filter")
                    .selected_text(faction_label)
                    .width(50.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut filter_state.faction_filter, -1, "All");
                        ui.selectable_value(&mut filter_state.faction_filter, 0, "Mine");
                    });
                ui.checkbox(&mut filter_state.show_kills, "Deaths");
                ui.checkbox(&mut filter_state.show_spawns, "Spawns");
                ui.checkbox(&mut filter_state.show_raids, "Raids");
                ui.checkbox(&mut filter_state.show_harvests, "Harvests");
                ui.checkbox(&mut filter_state.show_levelups, "Levels");
                ui.checkbox(&mut filter_state.show_npc_activity, "NPC");
                ui.checkbox(&mut filter_state.show_ai, "AI");
                ui.checkbox(&mut filter_state.show_building_damage, "Buildings");
                ui.checkbox(&mut filter_state.show_loot, "Loot");
                ui.checkbox(&mut filter_state.show_llm, "LLM");
                ui.checkbox(&mut filter_state.show_chat, "Chat");
            });

            ui.separator();

            // Rebuild merged entries only when sources changed
            let curr_filters = (
                filter_state.show_kills,
                filter_state.show_spawns,
                filter_state.show_raids,
                filter_state.show_harvests,
                filter_state.show_levelups,
                filter_state.show_npc_activity,
                filter_state.show_ai,
                filter_state.show_building_damage,
                filter_state.show_loot,
                filter_state.show_llm,
                filter_state.show_chat,
                filter_state.faction_filter,
            );
            let needs_rebuild = data.combat_log.is_changed()
                || data.npc_logs.is_changed()
                || chat_inbox.is_changed()
                || data.selected.0 != filter_state.cached_selected_npc
                || curr_filters != filter_state.cached_filters;

            if needs_rebuild {
                filter_state.cached_entries.clear();

                for entry in data.combat_log.iter_all() {
                    let show = match entry.kind {
                        CombatEventKind::Kill => filter_state.show_kills,
                        CombatEventKind::Spawn => filter_state.show_spawns,
                        CombatEventKind::Raid => filter_state.show_raids,
                        CombatEventKind::Harvest => filter_state.show_harvests,
                        CombatEventKind::LevelUp => filter_state.show_levelups,
                        CombatEventKind::Ai => filter_state.show_ai,
                        CombatEventKind::BuildingDamage => filter_state.show_building_damage,
                        CombatEventKind::Loot => filter_state.show_loot,
                        CombatEventKind::Llm => filter_state.show_llm,
                        CombatEventKind::Chat => filter_state.show_chat,
                    };
                    if !show {
                        continue;
                    }
                    // Faction filter: "Mine" shows player (0) + global (-1) events only
                    if filter_state.faction_filter == 0
                        && entry.faction != crate::constants::FACTION_PLAYER
                        && entry.faction != crate::constants::FACTION_NEUTRAL
                    {
                        continue;
                    }

                    let color = match entry.kind {
                        CombatEventKind::Kill => egui::Color32::from_rgb(220, 80, 80),
                        CombatEventKind::Spawn => egui::Color32::from_rgb(80, 200, 80),
                        CombatEventKind::Raid => egui::Color32::from_rgb(220, 160, 40),
                        CombatEventKind::Harvest => egui::Color32::from_rgb(200, 200, 60),
                        CombatEventKind::LevelUp => egui::Color32::from_rgb(80, 180, 255),
                        CombatEventKind::Ai => egui::Color32::from_rgb(180, 120, 220),
                        CombatEventKind::BuildingDamage => egui::Color32::from_rgb(220, 130, 50),
                        CombatEventKind::Loot => egui::Color32::from_rgb(255, 215, 0),
                        CombatEventKind::Llm => egui::Color32::from_rgb(0, 200, 180),
                        CombatEventKind::Chat => egui::Color32::from_rgb(240, 200, 80),
                    };

                    let key = (entry.day as i64) * 10000
                        + (entry.hour as i64) * 100
                        + entry.minute as i64;
                    let ts = format!("[D{} {:02}:{:02}]", entry.day, entry.hour, entry.minute);
                    filter_state.cached_entries.push((
                        key,
                        color,
                        ts,
                        entry.message.clone(),
                        entry.location,
                    ));
                }

                if filter_state.show_npc_activity && data.selected.0 >= 0 {
                    let idx = data.selected.0 as usize;
                    if idx < data.npc_logs.logs.len() {
                        let npc_color = egui::Color32::from_rgb(180, 180, 220);
                        for entry in data.npc_logs.logs[idx].iter() {
                            let key = (entry.day as i64) * 10000
                                + (entry.hour as i64) * 100
                                + entry.minute as i64;
                            let ts =
                                format!("[D{} {:02}:{:02}]", entry.day, entry.hour, entry.minute);
                            filter_state.cached_entries.push((
                                key,
                                npc_color,
                                ts,
                                entry.message.to_string(),
                                None,
                            ));
                        }
                    }
                }

                // Chat messages from ChatInbox (single source of truth)
                if filter_state.show_chat {
                    let chat_color = egui::Color32::from_rgb(240, 200, 80);
                    for msg in chat_inbox.messages.iter() {
                        let other_town = if msg.from_town == 0 {
                            msg.to_town
                        } else {
                            msg.from_town
                        };
                        let town_name = world_data
                            .towns
                            .get(other_town)
                            .map(|t| t.name.as_str())
                            .unwrap_or("?");
                        let label = if msg.from_town == 0 {
                            format!("[chat to {}] {}", town_name, msg.text)
                        } else {
                            format!("[chat from {}] {}", town_name, msg.text)
                        };
                        let key =
                            (msg.day as i64) * 10000 + (msg.hour as i64) * 100 + msg.minute as i64;
                        let ts = format!("[D{} {:02}:{:02}]", msg.day, msg.hour, msg.minute);
                        filter_state
                            .cached_entries
                            .push((key, chat_color, ts, label, None));
                    }
                }

                filter_state.cached_entries.sort_by_key(|(key, ..)| *key);
                filter_state.cached_selected_npc = data.selected.0;
                filter_state.cached_filters = curr_filters;
            }

            // Render from cache
            let mut pan_to: Option<bevy::math::Vec2> = None;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for (_, color, ts, msg, loc) in &filter_state.cached_entries {
                        ui.horizontal_wrapped(|ui| {
                            ui.small(ts);
                            if let Some(pos) = loc {
                                if ui
                                    .small_button(">>")
                                    .on_hover_text("Pan camera to target")
                                    .clicked()
                                {
                                    pan_to = Some(*pos);
                                }
                            }
                            ui.colored_label(*color, msg);
                        });
                    }
                });
            if let Some(pos) = pan_to {
                if let Ok(mut transform) = camera_query.single_mut() {
                    transform.translation.x = pos.x;
                    transform.translation.y = pos.y;
                }
                // Select building at this position (if any)
                let (gc, gr) = grid.world_to_grid(pos);
                if let Some(inst) = entity_map.get_at_grid(gc as i32, gr as i32) {
                    data.selected.0 = -1;
                    *selected_building = SelectedBuilding {
                        col: gc,
                        row: gr,
                        active: true,
                        slot: Some(inst.slot),
                        kind: Some(inst.kind),
                    };
                }
            }

            // Chat input — send messages to LLM towns
            if has_llm_towns {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Chat:");
                    let response = ui.text_edit_singleline(&mut filter_state.chat_input);
                    if (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        || ui.small_button("Send").clicked()
                    {
                        let text = filter_state.chat_input.trim().to_string();
                        if !text.is_empty() {
                            chat_send = Some(text);
                            filter_state.chat_input.clear();
                        }
                    }
                });
            }
        });

    // Process chat send — write to ChatInbox only (displayed from there, not combat log)
    if let Some(text) = chat_send {
        let day = data.game_time.day();
        let hour = data.game_time.hour();
        let minute = data.game_time.minute();
        for &to_town in &allowed_towns.towns {
            chat_inbox.push(crate::resources::ChatMessage {
                from_town: 0,
                to_town,
                text: text.clone(),
                day,
                hour,
                minute,
                sent_to_llm: false,
                has_reply: false,
            });
        }
    }

    // Persist filter changes
    let curr_filters = (
        filter_state.show_kills,
        filter_state.show_spawns,
        filter_state.show_raids,
        filter_state.show_harvests,
        filter_state.show_levelups,
        filter_state.show_npc_activity,
        filter_state.show_ai,
        filter_state.show_building_damage,
        filter_state.show_loot,
        filter_state.show_llm,
        filter_state.show_chat,
        filter_state.faction_filter,
    );
    if curr_filters != prev_filters {
        settings.log_kills = filter_state.show_kills;
        settings.log_spawns = filter_state.show_spawns;
        settings.log_raids = filter_state.show_raids;
        settings.log_harvests = filter_state.show_harvests;
        settings.log_levelups = filter_state.show_levelups;
        settings.log_npc_activity = filter_state.show_npc_activity;
        settings.log_ai = filter_state.show_ai;
        settings.log_building_damage = filter_state.show_building_damage;
        settings.log_loot = filter_state.show_loot;
        settings.log_llm = filter_state.show_llm;
        settings.log_chat = filter_state.show_chat;
        settings.log_faction_filter = filter_state.faction_filter;
        settings::save_settings(&settings);
    }

    Ok(())
}
