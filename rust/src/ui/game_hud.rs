//! In-game HUD — top resource bar, bottom panel (inspector + combat log), target overlay.

use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::constants::{building_def, npc_def, tileset_index};
use crate::components::*;
use crate::gpu::NpcGpuState;
use crate::resources::*;
use crate::settings::{self, UserSettings};
use crate::ui::tipped;
use crate::world::{WorldData, WorldGrid, BuildingKind, BuildingOccupancy, is_alive};
use crate::systems::stats::{CombatConfig, TownUpgrades, UPGRADES, resolve_town_tower_stats};
use crate::constants::UpgradeStatKind;

// ============================================================================
// TOP RESOURCE BAR
// ============================================================================

/// Full-width opaque top bar (WC3 style): buttons left, town name center, stats right.
pub fn top_bar_system(
    mut contexts: EguiContexts,
    game_time: Res<GameTime>,
    pop_stats: Res<PopulationStats>,
    food_storage: Res<FoodStorage>,
    gold_storage: Res<GoldStorage>,
    world_data: Res<WorldData>,
    mut ui_state: ResMut<UiState>,
    spawner_state: Res<SpawnerState>,
    catalog: Res<HelpCatalog>,
    time: Res<Time>,
    mut avg_fps: Local<f32>,
    settings: Res<crate::settings::UserSettings>,
    timings: Res<SystemTimings>,
) -> Result {
    let _t = timings.scope("ui_top_bar");
    let ctx = contexts.ctx_mut()?;

    let frame = egui::Frame::new()
        .fill(egui::Color32::from_rgb(30, 30, 35))
        .inner_margin(egui::Margin::symmetric(8, 4));

    egui::TopBottomPanel::top("resource_bar")
        .frame(frame)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                // LEFT: panel toggle buttons
                if ui.selectable_label(ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Roster, "Roster").clicked() {
                    ui_state.toggle_left_tab(LeftPanelTab::Roster);
                }
                if ui.selectable_label(ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Upgrades, "Upgrades").clicked() {
                    ui_state.toggle_left_tab(LeftPanelTab::Upgrades);
                }
                if ui.selectable_label(ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Policies, "Policies").clicked() {
                    ui_state.toggle_left_tab(LeftPanelTab::Policies);
                }
                if ui.selectable_label(ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Patrols, "Patrols").clicked() {
                    ui_state.toggle_left_tab(LeftPanelTab::Patrols);
                }
                if ui.selectable_label(ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Squads, "Squads").clicked() {
                    ui_state.toggle_left_tab(LeftPanelTab::Squads);
                }
                if ui.selectable_label(ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Factions, "Factions").clicked() {
                    ui_state.toggle_left_tab(LeftPanelTab::Factions);
                }
                if ui.selectable_label(ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Help, "Help").clicked() {
                    ui_state.toggle_left_tab(LeftPanelTab::Help);
                }
                if settings.debug_profiler {
                    if ui.selectable_label(ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Profiler, "Profiler").clicked() {
                        ui_state.toggle_left_tab(LeftPanelTab::Profiler);
                    }
                }

                // CENTER: town name + time (painted at true center of bar)
                let town_name = world_data.towns.first()
                    .map(|t| t.name.as_str())
                    .unwrap_or("Unknown");
                let period = if game_time.is_daytime() { "Day" } else { "Night" };
                let center_text = format!("{}  -  Day {} {:02}:{:02} ({}) {:.0}x{}",
                    town_name,
                    game_time.day(), game_time.hour(), game_time.minute(), period,
                    game_time.time_scale,
                    if game_time.paused { " [PAUSED]" } else { "" });
                ui.painter().text(
                    ui.max_rect().center(),
                    egui::Align2::CENTER_CENTER,
                    &center_text,
                    egui::FontId::default(),
                    ui.style().visuals.text_color(),
                );

                // RIGHT: stats pushed to the right edge
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // FPS (far right)
                    let dt = time.delta_secs();
                    if dt > 0.0 {
                        let fps = 1.0 / dt;
                        *avg_fps = if *avg_fps == 0.0 { fps } else { *avg_fps * 0.95 + fps * 0.05 };
                    }
                    ui.label(egui::RichText::new(format!("FPS: {:.0}", *avg_fps))
                        .size(12.0).strong());
                    ui.separator();

                    // Player stats (right-aligned) — player's town is index 0
                    let town_food = food_storage.food.first().copied().unwrap_or(0);
                    let town_gold = gold_storage.gold.first().copied().unwrap_or(0);
                    tipped(ui, egui::RichText::new(format!("Gold: {}", town_gold)).color(egui::Color32::from_rgb(220, 190, 50)), catalog.0.get("gold").unwrap_or(&""));
                    tipped(ui, format!("Food: {}", town_food), catalog.0.get("food").unwrap_or(&""));

                    let farmers = pop_stats.0.get(&(0, 0)).map(|s| s.alive).unwrap_or(0);
                    let guards = pop_stats.0.get(&(1, 0)).map(|s| s.alive).unwrap_or(0);
                    let crossbows = pop_stats.0.get(&(5, 0)).map(|s| s.alive).unwrap_or(0);
                    let sk_farmer = crate::constants::tileset_index(BuildingKind::FarmerHome) as i32;
                    let sk_archer = crate::constants::tileset_index(BuildingKind::ArcherHome) as i32;
                    let sk_xbow = crate::constants::tileset_index(BuildingKind::CrossbowHome) as i32;
                    let houses = spawner_state.0.iter().filter(|s| s.building_kind == sk_farmer && s.town_idx == 0 && is_alive(s.position)).count();
                    let barracks = spawner_state.0.iter().filter(|s| s.building_kind == sk_archer && s.town_idx == 0 && is_alive(s.position)).count();
                    let xbow_homes = spawner_state.0.iter().filter(|s| s.building_kind == sk_xbow && s.town_idx == 0 && is_alive(s.position)).count();
                    tipped(ui, format!("Archers: {}/{}", guards, barracks), catalog.0.get("archers").unwrap_or(&""));
                    tipped(ui, format!("Crossbow: {}/{}", crossbows, xbow_homes), catalog.0.get("crossbow").unwrap_or(&""));
                    tipped(ui, format!("Farmers: {}/{}", farmers, houses), catalog.0.get("farmers").unwrap_or(&""));
                    let total_alive: i32 = pop_stats.0.values().map(|s| s.alive).sum();
                    let total_spawners = spawner_state.0.iter().filter(|s| is_alive(s.position)).count();
                    tipped(ui, format!("Pop: {}/{}", total_alive, total_spawners), catalog.0.get("pop").unwrap_or(&""));
                });
            });
        });

    Ok(())
}

// ============================================================================
// BOTTOM PANEL (INSPECTOR + COMBAT LOG)
// ============================================================================

/// Query bundle for NPC state display.
#[derive(SystemParam)]
pub struct NpcStateQuery<'w, 's> {
    states: Query<'w, 's, (
        &'static NpcIndex,
        &'static Personality,
        &'static Home,
        &'static Faction,
        &'static TownId,
        &'static Activity,
        &'static CombatState,
        Option<&'static SquadId>,
        Option<&'static PatrolRoute>,
    ), Without<Dead>>,
}

/// Bundled readonly resources for bottom panel.
#[derive(SystemParam)]
pub struct BottomPanelData<'w> {
    game_time: Res<'w, GameTime>,
    npc_logs: Res<'w, NpcLogCache>,
    selected: Res<'w, SelectedNpc>,
    combat_log: Res<'w, CombatLog>,
}

/// Bundled resources for building inspector.
#[derive(SystemParam)]
pub struct BuildingInspectorData<'w> {
    selected_building: Res<'w, SelectedBuilding>,
    grid: Res<'w, WorldGrid>,
    farm_states: Res<'w, GrowthStates>,
    farm_occupancy: Res<'w, BuildingOccupancy>,
    spawner_state: Res<'w, SpawnerState>,
    food_storage: Res<'w, FoodStorage>,
    combat_config: Res<'w, CombatConfig>,
    town_upgrades: Res<'w, TownUpgrades>,
    building_hp: Res<'w, BuildingHpState>,
}

#[derive(SystemParam)]
pub struct BottomPanelUiState<'w> {
    destroy_request: ResMut<'w, DestroyRequest>,
    ui_state: ResMut<'w, UiState>,
    mining_policy: ResMut<'w, MiningPolicy>,
    dirty: ResMut<'w, DirtyFlags>,
}

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
    /// -1 = all factions, 0 = my faction only
    pub faction_filter: i32,
    pub initialized: bool,
    // Cached merged log entries — skip rebuild when sources unchanged
    cached_selected_npc: i32,
    cached_filters: (bool, bool, bool, bool, bool, bool, bool, bool, i32),
    cached_entries: Vec<(i64, egui::Color32, String, String, Option<bevy::math::Vec2>)>,
}

#[derive(Default)]
pub struct InspectorRenameState {
    slot: i32,
    text: String,
}

#[derive(Default)]
pub struct InspectorTabState {
    /// true = NPC tab, false = Building tab
    show_npc: bool,
}

#[derive(Default)]
pub struct InspectorUiState {
    rename: InspectorRenameState,
    tabs: InspectorTabState,
    last_click_seq: u64,
}

/// Bottom panel: NPC/building inspector.
pub fn bottom_panel_system(
    mut contexts: EguiContexts,
    data: BottomPanelData,
    mut meta_cache: ResMut<NpcMetaCache>,
    bld_data: BuildingInspectorData,
    mut world_data: ResMut<WorldData>,
    health_query: Query<(&NpcIndex, &Health, &CachedStats, &Energy), Without<Dead>>,
    equip_query: Query<(
        &NpcIndex, Option<&EquippedWeapon>, Option<&EquippedHelmet>, Option<&EquippedArmor>,
        Option<&Starving>, Option<&SquadId>, Option<&CarriedGold>, &BaseAttackType, &Speed,
    ), Without<Dead>>,
    npc_states: NpcStateQuery,
    gpu_state: Res<GpuReadState>,
    buffer_writes: Res<NpcGpuState>,
    mut follow: ResMut<FollowSelected>,
    settings: Res<UserSettings>,
    catalog: Res<HelpCatalog>,
    mut panel_state: BottomPanelUiState,
    mut inspector_state: Local<InspectorUiState>,
    timings: Res<SystemTimings>,
) -> Result {
    let _t = timings.scope("ui_bottom");
    let ctx = contexts.ctx_mut()?;

    let mut copy_text: Option<String> = None;

    // Only show inspector when something is selected
    let has_npc = data.selected.0 >= 0;
    let has_building = bld_data.selected_building.active;
    if has_npc || has_building {
        if has_npc && !has_building {
            inspector_state.tabs.show_npc = true;
        } else if has_building && !has_npc {
            inspector_state.tabs.show_npc = false;
        } else if has_npc && has_building && inspector_state.last_click_seq != panel_state.ui_state.inspector_click_seq {
            inspector_state.tabs.show_npc = panel_state.ui_state.inspector_prefer_npc;
            inspector_state.last_click_seq = panel_state.ui_state.inspector_click_seq;
        }

        let frame = egui::Frame::new()
            .fill(egui::Color32::from_rgba_unmultiplied(30, 30, 35, 220))
            .inner_margin(egui::Margin::same(6));

        egui::Window::new("Inspector")
            .anchor(egui::Align2::LEFT_BOTTOM, [2.0, -2.0])
            .fixed_size([300.0, 280.0])
            .collapsible(true)
            .movable(false)
            .frame(frame)
            .show(ctx, |ui| {
                if has_npc && has_building {
                    let npc_label = if data.selected.0 >= 0 && (data.selected.0 as usize) < meta_cache.0.len() {
                        format!("NPC: {}", meta_cache.0[data.selected.0 as usize].name)
                    } else {
                        "NPC".to_string()
                    };
                    let bld_label = selected_building_info(&bld_data.selected_building, &bld_data.grid, &world_data)
                        .map(|(k, _, _, _, _)| format!("Building: {}", building_def(k).label))
                        .unwrap_or_else(|| "Building".to_string());

                    ui.horizontal(|ui| {
                        if ui.selectable_label(inspector_state.tabs.show_npc, npc_label).clicked() {
                            inspector_state.tabs.show_npc = true;
                        }
                        if ui.selectable_label(!inspector_state.tabs.show_npc, bld_label).clicked() {
                            inspector_state.tabs.show_npc = false;
                        }
                    });
                    ui.separator();
                }

                let show_npc = has_npc && (!has_building || inspector_state.tabs.show_npc);
                inspector_content(
                    ui, &data, &mut meta_cache, &mut inspector_state.rename, &bld_data, &mut world_data, &health_query,
                    &equip_query, &npc_states, &gpu_state, &buffer_writes, &mut follow, &settings, &catalog, &mut copy_text,
                    &mut panel_state.ui_state, &mut panel_state.mining_policy, &mut panel_state.dirty, show_npc,
                );
                // Destroy button for selected buildings (not fountains/camps)
                let show_building = has_building && (!has_npc || !show_npc);
                if show_building {
                    let selected_info = selected_building_info(&bld_data.selected_building, &bld_data.grid, &world_data);
                    let is_destructible = selected_info
                        .as_ref()
                        .map(|(k, _, _, _, _)| !matches!(k, BuildingKind::Fountain | BuildingKind::Camp | BuildingKind::GoldMine))
                        .unwrap_or(false);
                    if is_destructible {
                        ui.separator();
                        if ui.button(egui::RichText::new("Destroy").color(egui::Color32::from_rgb(220, 80, 80))).clicked() {
                            if let Some((_, _, _, col, row)) = selected_info {
                                panel_state.destroy_request.0 = Some((col, row));
                            }
                        }
                    }
                }
            });
    }

    // Handle clipboard copy (must be outside egui closure)
    if let Some(text) = copy_text {
        info!("Copy button clicked, {} bytes", text.len());
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                match cb.set_text(text) {
                    Ok(_) => info!("Clipboard: text copied successfully"),
                    Err(e) => error!("Clipboard: set_text failed: {e}"),
                }
            }
            Err(e) => error!("Clipboard: failed to open: {e}"),
        }
    }

    Ok(())
}

/// Combat log window anchored at bottom-right.
pub fn combat_log_system(
    mut contexts: EguiContexts,
    data: BottomPanelData,
    mut settings: ResMut<UserSettings>,
    mut filter_state: Local<LogFilterState>,
    ui_state: Res<UiState>,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
) -> Result {
    if !ui_state.combat_log_visible { return Ok(()); }
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
        filter_state.faction_filter = settings.log_faction_filter;
        filter_state.initialized = true;
    }

    let prev_filters = (
        filter_state.show_kills, filter_state.show_spawns, filter_state.show_raids,
        filter_state.show_harvests, filter_state.show_levelups, filter_state.show_npc_activity,
        filter_state.show_ai, filter_state.show_building_damage, filter_state.faction_filter,
    );

    let frame = egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(30, 30, 35, 220))
        .inner_margin(egui::Margin::same(6));

    egui::Window::new("Combat Log")
        .anchor(egui::Align2::RIGHT_BOTTOM, [-2.0, -2.0])
        .default_size([450.0, 140.0])
        .collapsible(false)
        .resizable(true)
        .movable(false)
        .frame(frame)
        .title_bar(true)
        .show(ctx, |ui| {
            // Filter checkboxes
            ui.horizontal_wrapped(|ui| {
                let faction_label = if filter_state.faction_filter == -1 { "All" } else { "Mine" };
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
            });

            ui.separator();

            // Rebuild merged entries only when sources changed
            let curr_filters = (
                filter_state.show_kills, filter_state.show_spawns, filter_state.show_raids,
                filter_state.show_harvests, filter_state.show_levelups, filter_state.show_npc_activity,
                filter_state.show_ai, filter_state.show_building_damage, filter_state.faction_filter,
            );
            let needs_rebuild = data.combat_log.is_changed()
                || data.npc_logs.is_changed()
                || data.selected.0 != filter_state.cached_selected_npc
                || curr_filters != filter_state.cached_filters;

            if needs_rebuild {
                filter_state.cached_entries.clear();

                for entry in &data.combat_log.entries {
                    let show = match entry.kind {
                        CombatEventKind::Kill => filter_state.show_kills,
                        CombatEventKind::Spawn => filter_state.show_spawns,
                        CombatEventKind::Raid => filter_state.show_raids,
                        CombatEventKind::Harvest => filter_state.show_harvests,
                        CombatEventKind::LevelUp => filter_state.show_levelups,
                        CombatEventKind::Ai => filter_state.show_ai,
                        CombatEventKind::BuildingDamage => filter_state.show_building_damage,
                    };
                    if !show { continue; }
                    // Faction filter: "Mine" shows player (0) + global (-1) events only
                    if filter_state.faction_filter == 0 && entry.faction != 0 && entry.faction != -1 {
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
                    };

                    let key = (entry.day as i64) * 10000 + (entry.hour as i64) * 100 + entry.minute as i64;
                    let ts = format!("[D{} {:02}:{:02}]", entry.day, entry.hour, entry.minute);
                    filter_state.cached_entries.push((key, color, ts, entry.message.clone(), entry.location));
                }

                if filter_state.show_npc_activity && data.selected.0 >= 0 {
                    let idx = data.selected.0 as usize;
                    if idx < data.npc_logs.0.len() {
                        let npc_color = egui::Color32::from_rgb(180, 180, 220);
                        for entry in data.npc_logs.0[idx].iter() {
                            let key = (entry.day as i64) * 10000 + (entry.hour as i64) * 100 + entry.minute as i64;
                            let ts = format!("[D{} {:02}:{:02}]", entry.day, entry.hour, entry.minute);
                            filter_state.cached_entries.push((key, npc_color, ts, entry.message.to_string(), None));
                        }
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
                        ui.horizontal(|ui| {
                            ui.small(ts);
                            if let Some(pos) = loc {
                                if ui.small_button(">>").on_hover_text("Pan camera to target").clicked() {
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
            }
        });

    // Persist filter changes
    let curr_filters = (
        filter_state.show_kills, filter_state.show_spawns, filter_state.show_raids,
        filter_state.show_harvests, filter_state.show_levelups, filter_state.show_npc_activity,
        filter_state.show_ai, filter_state.show_building_damage, filter_state.faction_filter,
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
        settings.log_faction_filter = filter_state.faction_filter;
        settings::save_settings(&settings);
    }

    Ok(())
}

/// Render inspector content into a ui region (left side of bottom panel).
fn inspector_content(
    ui: &mut egui::Ui,
    data: &BottomPanelData,
    meta_cache: &mut NpcMetaCache,
    rename_state: &mut InspectorRenameState,
    bld_data: &BuildingInspectorData,
    world_data: &mut WorldData,
    health_query: &Query<(&NpcIndex, &Health, &CachedStats, &Energy), Without<Dead>>,
    equip_query: &Query<(
        &NpcIndex, Option<&EquippedWeapon>, Option<&EquippedHelmet>, Option<&EquippedArmor>,
        Option<&Starving>, Option<&SquadId>, Option<&CarriedGold>, &BaseAttackType, &Speed,
    ), Without<Dead>>,
    npc_states: &NpcStateQuery,
    gpu_state: &GpuReadState,
    buffer_writes: &NpcGpuState,
    follow: &mut FollowSelected,
    settings: &UserSettings,
    catalog: &HelpCatalog,
    copy_text: &mut Option<String>,
    ui_state: &mut UiState,
    mining_policy: &mut MiningPolicy,
    dirty: &mut DirtyFlags,
    show_npc: bool,
) {
    if !show_npc {
        rename_state.slot = -1;
        rename_state.text.clear();
        if bld_data.selected_building.active {
            building_inspector_content(ui, bld_data, world_data, mining_policy, dirty, meta_cache, ui_state, copy_text, &data.game_time, settings, &data.combat_log, npc_states, gpu_state);
            return;
        }
        ui.label("Click an NPC or building to inspect");
        return;
    }

    let sel = data.selected.0;
    if sel < 0 {
        rename_state.slot = -1;
        rename_state.text.clear();
        if bld_data.selected_building.active {
            building_inspector_content(ui, bld_data, world_data, mining_policy, dirty, meta_cache, ui_state, copy_text, &data.game_time, settings, &data.combat_log, npc_states, gpu_state);
            return;
        }
        ui.label("Click an NPC or building to inspect");
        return;
    }
    let idx = sel as usize;
    if idx >= meta_cache.0.len() { return; }
    if !health_query.iter().any(|(npc_idx, ..)| npc_idx.0 == idx) {
        rename_state.slot = -1;
        rename_state.text.clear();
        if bld_data.selected_building.active {
            building_inspector_content(ui, bld_data, world_data, mining_policy, dirty, meta_cache, ui_state, copy_text, &data.game_time, settings, &data.combat_log, npc_states, gpu_state);
        } else {
            ui.label("Click an NPC or building to inspect");
        }
        return;
    }

    if rename_state.slot != sel {
        rename_state.slot = sel;
        rename_state.text = meta_cache.0[idx].name.clone();
    }

    ui.horizontal(|ui| {
        ui.label("Name:");
        let edit = ui.text_edit_singleline(&mut rename_state.text);
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Rename").clicked() || enter) && !rename_state.text.trim().is_empty() {
            let new_name = rename_state.text.trim().to_string();
            meta_cache.0[idx].name = new_name.clone();
            rename_state.text = new_name;
        }
    });

    let meta = &meta_cache.0[idx];

    tipped(ui, format!("{} Lv.{}  XP: {}/{}", crate::job_name(meta.job), meta.level, meta.xp, (meta.level + 1) * (meta.level + 1) * 100), catalog.0.get("npc_level").unwrap_or(&""));

    if let Some((_, personality, ..)) = npc_states.states.iter().find(|(ni, ..)| ni.0 == idx) {
        let trait_str = personality.trait_summary();
        if !trait_str.is_empty() {
            tipped(ui, format!("Trait: {}", trait_str), catalog.0.get("npc_trait").unwrap_or(&""));
        }
    }

    // Find HP, energy, combat stats from query
    let mut hp = 0.0f32;
    let mut max_hp = 100.0f32;
    let mut energy = 0.0f32;
    let mut cached_stats: Option<&CachedStats> = None;
    for (npc_idx, health, cached, npc_energy) in health_query.iter() {
        if npc_idx.0 == idx {
            hp = health.0;
            max_hp = cached.max_health;
            energy = npc_energy.0;
            cached_stats = Some(cached);
            break;
        }
    }

    // HP bar
    let hp_frac = if max_hp > 0.0 { (hp / max_hp).clamp(0.0, 1.0) } else { 0.0 };
    let hp_color = if hp_frac > 0.6 {
        egui::Color32::from_rgb(80, 200, 80)
    } else if hp_frac > 0.3 {
        egui::Color32::from_rgb(200, 200, 40)
    } else {
        egui::Color32::from_rgb(200, 60, 60)
    };
    ui.horizontal(|ui| {
        ui.label("HP:");
        ui.add(egui::ProgressBar::new(hp_frac)
            .text(egui::RichText::new(format!("{:.0}/{:.0}", hp, max_hp)).color(egui::Color32::BLACK))
            .fill(hp_color));
    });

    // Energy bar
    let energy_frac = (energy / 100.0).clamp(0.0, 1.0);
    ui.horizontal(|ui| {
        tipped(ui, "EN:", catalog.0.get("npc_energy").unwrap_or(&""));
        ui.add(egui::ProgressBar::new(energy_frac)
            .text(egui::RichText::new(format!("{:.0}", energy)).color(egui::Color32::BLACK))
            .fill(egui::Color32::from_rgb(60, 120, 200)));
    });

    // Combat stats
    if let Some(stats) = cached_stats {
        ui.label(format!("Dmg: {:.0}  Rng: {:.0}  CD: {:.1}s  Spd: {:.0}",
            stats.damage, stats.range, stats.cooldown, stats.speed));
    }

    // Equipment + status from equip_query
    if let Some((_, weapon, helmet, armor, starving, squad, gold, atk_type, _speed))
        = equip_query.iter().find(|(ni, ..)| ni.0 == idx)
    {
        let atk_str = match atk_type { BaseAttackType::Melee => "Melee", BaseAttackType::Ranged => "Ranged" };
        let mut equip_parts: Vec<&str> = Vec::new();
        if weapon.is_some() { equip_parts.push("Weapon"); }
        if helmet.is_some() { equip_parts.push("Helmet"); }
        if armor.is_some() { equip_parts.push("Armor"); }
        let equip_str = if equip_parts.is_empty() { "None".to_string() } else { equip_parts.join(" + ") };
        ui.label(format!("{} | {}", atk_str, equip_str));

        // Status markers
        if starving.is_some() {
            ui.colored_label(egui::Color32::from_rgb(200, 60, 60), "Starving");
        }
        if let Some(sq) = squad {
            ui.label(format!("Squad: {}", sq.0));
        }
        if let Some(g) = gold {
            if g.0 > 0 { ui.label(format!("Carrying: {} gold", g.0)); }
        }
    }

    // Town name
    if meta.town_id >= 0 {
        if let Some(town) = world_data.towns.get(meta.town_id as usize) {
            ui.label(format!("Town: {}", town.name));
        }
    }

    // State, faction, home
    let mut state_str = String::new();
    let mut home_str = String::new();
    let mut faction_str = String::new();
    let mut faction_id: Option<i32> = None;
    let mut home_pos: Option<Vec2> = None;
    let mut is_mining_at_mine = false;

    if let Some((_, _, home, faction, town_id, activity, combat, ..))
        = npc_states.states.iter().find(|(ni, ..)| ni.0 == idx)
    {
        home_pos = Some(home.0);
        home_str = format!("({:.0}, {:.0})", home.0.x, home.0.y);
        faction_str = format!("{} (town {})", faction.0, town_id.0);
        faction_id = Some(faction.0);
        is_mining_at_mine = matches!(activity, Activity::MiningAtMine);

        let mut parts: Vec<&str> = Vec::new();
        let combat_name = combat.name();
        if !combat_name.is_empty() { parts.push(combat_name); }
        parts.push(activity.name());
        state_str = parts.join(", ");
    }

    tipped(ui, format!("State: {}", state_str), catalog.0.get("npc_state").unwrap_or(&""));
    ui.horizontal(|ui| {
        if let Some(fid) = faction_id {
            if ui.link(format!("Faction: {}", faction_str)).clicked() {
                ui_state.left_panel_open = true;
                ui_state.left_panel_tab = LeftPanelTab::Factions;
                ui_state.pending_faction_select = Some(fid);
            }
        } else {
            ui.label(format!("Faction: {}", faction_str));
        }
        ui.label(format!("Home: {}", home_str));
    });

    // Mine assignment for miners (same UI as MinerHome building inspector)
    if meta.job == 4 {
        if let Some(hp) = home_pos {
            let mh_idx = world_data.miner_home_at(hp);
            if let Some(mh_idx) = mh_idx {
                ui.separator();
                mine_assignment_ui(ui, world_data, mh_idx, hp, dirty, ui_state);
                // Show mine productivity when actively mining
                if is_mining_at_mine {
                    if let Some(mine_pos) = world_data.miner_homes().get(mh_idx).and_then(|mh| mh.assigned_mine) {
                        let occupants = bld_data.farm_occupancy.count(mine_pos);
                        if occupants > 0 {
                            let mult = crate::constants::mine_productivity_mult(occupants);
                            ui.label(format!("Mine productivity: {:.0}% ({} miners)", mult * 100.0, occupants));
                        }
                    }
                }
            }
        }
    }

    // Follow toggle
    ui.horizontal(|ui| {
        if ui.selectable_label(follow.0, "Follow (F)").clicked() {
            follow.0 = !follow.0;
        }
    });

    // Debug: coordinates, copy button
    if settings.debug_coordinates {
        ui.separator();

        let positions = &gpu_state.positions;
        let targets = &buffer_writes.targets;

        let pos = if idx * 2 + 1 < positions.len() {
            format!("({:.0}, {:.0})", positions[idx * 2], positions[idx * 2 + 1])
        } else {
            "?".into()
        };
        let target = if idx * 2 + 1 < targets.len() {
            format!("({:.0}, {:.0})", targets[idx * 2], targets[idx * 2 + 1])
        } else {
            "?".into()
        };

        ui.label(format!("Pos: {}  Target: {}", pos, target));

        if ui.button("Copy Debug Info").clicked() {
            let xp_next = (meta.level + 1) * (meta.level + 1) * 100;
            let mut info = format!(
                "NPC #{idx} \"{name}\" {job} Lv.{level}  XP: {xp}/{xp_next}\n\
                 HP: {hp:.0}/{max_hp:.0}  EN: {energy:.0}\n\
                 Pos: {pos}  Target: {target}\n",
                idx = idx,
                name = meta.name,
                job = crate::job_name(meta.job),
                level = meta.level,
                xp = meta.xp,
                xp_next = xp_next,
                hp = hp,
                max_hp = max_hp,
                energy = energy,
                pos = pos,
                target = target,
            );
            if let Some((_, personality, ..)) = npc_states.states.iter().find(|(ni, ..)| ni.0 == idx) {
                let trait_str = personality.trait_summary();
                if !trait_str.is_empty() {
                    info.push_str(&format!("Trait: {}\n", trait_str));
                }
            }
            if let Some(stats) = cached_stats {
                info.push_str(&format!(
                    "Dmg: {:.0}  Rng: {:.0}  CD: {:.1}s  Spd: {:.0}\n",
                    stats.damage, stats.range, stats.cooldown, stats.speed
                ));
            }
            if let Some((_, weapon, helmet, armor, starving, squad, gold, atk_type, _speed))
                = equip_query.iter().find(|(ni, ..)| ni.0 == idx)
            {
                let atk_str = match atk_type {
                    BaseAttackType::Melee => "Melee",
                    BaseAttackType::Ranged => "Ranged",
                };
                let mut equip_parts: Vec<&str> = Vec::new();
                if weapon.is_some() { equip_parts.push("Weapon"); }
                if helmet.is_some() { equip_parts.push("Helmet"); }
                if armor.is_some() { equip_parts.push("Armor"); }
                let equip_str = if equip_parts.is_empty() { "None".to_string() } else { equip_parts.join(" + ") };
                info.push_str(&format!("{} | {}\n", atk_str, equip_str));
                if starving.is_some() {
                    info.push_str("Starving\n");
                }
                if let Some(sq) = squad {
                    info.push_str(&format!("Squad: {}\n", sq.0));
                }
                if let Some(g) = gold {
                    if g.0 > 0 {
                        info.push_str(&format!("Carrying: {} gold\n", g.0));
                    }
                }
            }
            if meta.town_id >= 0 {
                if let Some(town) = world_data.towns.get(meta.town_id as usize) {
                    info.push_str(&format!("Town: {}\n", town.name));
                }
            }
            info.push_str(&format!(
                "Home: {home}  Faction: {faction}\n\
                 State: {state}\n",
                home = home_str,
                faction = faction_str,
                state = state_str,
            ));
            if meta.job == 4 {
                if let Some(hp) = home_pos {
                    if let Some(mh_idx) = world_data.miner_home_at(hp) {
                        let assigned = world_data.miner_homes()[mh_idx].assigned_mine;
                        let manual = world_data.miner_homes()[mh_idx].manual_mine;
                        if let Some(mine_pos) = assigned {
                            let dist = mine_pos.distance(hp);
                            if let Some(mine_idx) = world_data.gold_mine_at(mine_pos) {
                                info.push_str(&format!("Mine: {} - {:.0}px\n", crate::ui::gold_mine_name(mine_idx), dist));
                            } else {
                                info.push_str(&format!("Mine: ({:.0}, {:.0}) - {:.0}px\n", mine_pos.x, mine_pos.y, dist));
                            }
                        } else {
                            info.push_str("Mine: Auto (nearest)\n");
                        }
                        info.push_str(if manual { "Mode: Manual\n" } else { "Mode: Auto-policy\n" });
                        if is_mining_at_mine {
                            if let Some(mine_pos) = world_data.miner_homes().get(mh_idx).and_then(|mh| mh.assigned_mine) {
                                let occupants = bld_data.farm_occupancy.count(mine_pos);
                                if occupants > 0 {
                                    let mult = crate::constants::mine_productivity_mult(occupants);
                                    info.push_str(&format!("Mine productivity: {:.0}% ({} miners)\n", mult * 100.0, occupants));
                                }
                            }
                        }
                    }
                }
            }
            info.push_str(&format!(
                "Follow: {}\n\
                 Day {day} {hour:02}:{min:02}\n\
                 ---\n",
                if follow.0 { "ON" } else { "OFF" },
                day = data.game_time.day(),
                hour = data.game_time.hour(),
                min = data.game_time.minute(),
            ));
            if idx < data.npc_logs.0.len() {
                for entry in data.npc_logs.0[idx].iter() {
                    info.push_str(&format!("D{}:{:02}:{:02} {}\n",
                        entry.day, entry.hour, entry.minute, entry.message));
                }
            }
            *copy_text = Some(info);
        }
    }
}

// ============================================================================
// BUILDING INSPECTOR
// ============================================================================

fn selected_building_info(
    selected: &SelectedBuilding,
    grid: &WorldGrid,
    world_data: &WorldData,
) -> Option<(BuildingKind, u32, Vec2, usize, usize)> {
    if !selected.active { return None; }

    if let (Some(kind), Some(index)) = (selected.kind, selected.index) {
        let def = crate::constants::building_def(kind);
        if let Some((pos, town_idx)) = (def.pos_town)(world_data, index) {
            let (col, row) = grid.world_to_grid(pos);
            return Some((kind, town_idx, pos, col, row));
        }
    }

    let col = selected.col;
    let row = selected.row;
    let cell = grid.cell(col, row)?;
    let (kind, town_idx) = cell.building?;
    let pos = grid.grid_to_world(col, row);
    Some((kind, town_idx, pos, col, row))
}

/// Mine assignment UI: show assigned mine, "Set Mine" / "Clear" buttons.
/// Shared by building inspector (MinerHome) and NPC inspector (Miner).
fn mine_assignment_ui(
    ui: &mut egui::Ui,
    world_data: &mut WorldData,
    mh_idx: usize,
    ref_pos: Vec2,
    dirty: &mut DirtyFlags,
    ui_state: &mut UiState,
) {
    let assigned = world_data.miner_homes_mut()[mh_idx].assigned_mine;
    let manual = world_data.miner_homes_mut()[mh_idx].manual_mine;
    if let Some(mine_pos) = assigned {
        let dist = mine_pos.distance(ref_pos);
        if let Some(mine_idx) = world_data.gold_mine_at(mine_pos) {
            ui.label(format!("Mine: {} - {:.0}px", crate::ui::gold_mine_name(mine_idx), dist));
        } else {
            ui.label(format!("Mine: ({:.0}, {:.0}) - {:.0}px", mine_pos.x, mine_pos.y, dist));
        }
    } else {
        ui.label("Mine: Auto (nearest)");
    }
    ui.small(if manual { "Mode: Manual" } else { "Mode: Auto-policy" });
    ui.horizontal(|ui| {
        if ui.button("Set Mine").clicked() {
            world_data.miner_homes_mut()[mh_idx].manual_mine = true;
            dirty.mining = true;
            ui_state.assigning_mine = Some(mh_idx);
        }
        if assigned.is_some() || manual {
            if ui.button("Clear").clicked() {
                world_data.miner_homes_mut()[mh_idx].manual_mine = false;
                world_data.miner_homes_mut()[mh_idx].assigned_mine = None;
                dirty.mining = true;
            }
        }
    });
}

/// Render building inspector content when a building cell is selected.
fn building_inspector_content(
    ui: &mut egui::Ui,
    bld: &BuildingInspectorData,
    world_data: &mut WorldData,
    mining_policy: &mut MiningPolicy,
    dirty: &mut DirtyFlags,
    meta_cache: &NpcMetaCache,
    ui_state: &mut UiState,
    copy_text: &mut Option<String>,
    game_time: &GameTime,
    settings: &UserSettings,
    combat_log: &CombatLog,
    npc_states: &NpcStateQuery,
    gpu_state: &GpuReadState,
) {
    let Some((kind, bld_town_idx, world_pos, col, row)) =
        selected_building_info(&bld.selected_building, &bld.grid, world_data)
    else { return };

    let def = building_def(kind);
    let town_idx = bld_town_idx as usize;

    // Header
    ui.strong(def.label);

    // Town + faction
    if let Some(town) = world_data.towns.get(town_idx) {
        ui.label(format!("Town: {}", town.name));
        if ui.link(format!("Faction: {}", town.faction)).clicked() {
            ui_state.left_panel_open = true;
            ui_state.left_panel_tab = LeftPanelTab::Factions;
            ui_state.pending_faction_select = Some(town.faction);
        }
    } else if kind == BuildingKind::GoldMine {
        ui.label("Faction: Unowned");
    }

    // Per-type details
    match kind {
        BuildingKind::Farm => {
            // Find farm index by matching grid position
            if let Some(farm_idx) = world_data.farms().iter().position(|f| {
                (f.position - world_pos).length() < 1.0
            }) {
                if let Some(state) = bld.farm_states.states.get(farm_idx) {
                    let state_name = match state {
                        FarmGrowthState::Growing => "Growing",
                        FarmGrowthState::Ready => "Ready to harvest",
                    };
                    ui.label(format!("Status: {}", state_name));

                    if let Some(&progress) = bld.farm_states.progress.get(farm_idx) {
                        let color = if *state == FarmGrowthState::Ready {
                            egui::Color32::from_rgb(200, 200, 60)
                        } else {
                            egui::Color32::from_rgb(80, 180, 80)
                        };
                        ui.horizontal(|ui| {
                            ui.label("Growth:");
                            ui.add(egui::ProgressBar::new(progress)
                                .text(format!("{:.0}%", progress * 100.0))
                                .fill(color));
                        });
                    }
                }

                // Show farmer working here
                let occupants = bld.farm_occupancy.count(world_pos);
                ui.label(format!("Farmers: {}", occupants));
            }
        }

        BuildingKind::Waypoint => {
            if let Some(wp_idx) = world_data.get(BuildingKind::Waypoint).iter()
                .position(|w| (w.position - world_pos).length() < 1.0)
            {
                ui.label(format!("Patrol order: {}", world_data.get(BuildingKind::Waypoint)[wp_idx].patrol_order));
            }
        }

        BuildingKind::Fountain => {
            // Healing + tower info
            let base_radius = bld.combat_config.heal_radius;
            let levels = bld.town_upgrades.town_levels(town_idx);
            let upgrade_bonus = UPGRADES.stat_level(&levels, "Town", UpgradeStatKind::FountainRange) as f32 * 24.0;
            let tower = resolve_town_tower_stats(&levels);
            ui.label(format!("Heal radius: {:.0}px", base_radius + upgrade_bonus));
            ui.label(format!("Heal rate: {:.0}/s", bld.combat_config.heal_rate));
            ui.separator();
            ui.label(format!("Tower range: {:.0}px", tower.range));
            ui.label(format!("Tower damage: {:.1}", tower.damage));
            ui.label(format!("Tower cooldown: {:.2}s", tower.cooldown));
            ui.label(format!("Tower projectile life: {:.2}s", tower.proj_lifetime));

            // Town food — town_idx is direct index into food_storage
            if let Some(&food) = bld.food_storage.food.get(town_idx) {
                ui.label(format!("Food: {}", food));
            }
        }

        BuildingKind::Camp => {
            // Camp food — town_idx is direct index into food_storage
            if let Some(&food) = bld.food_storage.food.get(town_idx) {
                ui.label(format!("Camp food: {}", food));
            }
        }

        BuildingKind::Bed => {
            ui.label("Rest point");
        }

        BuildingKind::GoldMine => {
            if let Some(mine_idx) = world_data.gold_mine_at(world_pos) {
                ui.label(format!("Name: {}", crate::ui::gold_mine_name(mine_idx)));
                if mine_idx >= mining_policy.mine_enabled.len() {
                    mining_policy.mine_enabled.resize(mine_idx + 1, true);
                }
                let enabled = mining_policy.mine_enabled[mine_idx];
                let label = if enabled { "Auto-mining: ON" } else { "Auto-mining: OFF" };
                if ui.button(label).clicked() {
                    mining_policy.mine_enabled[mine_idx] = !enabled;
                    dirty.mining = true;
                }
            }
            // Find mine in GrowthStates
            if let Some(gi) = bld.farm_states.positions.iter().position(|p| (*p - world_pos).length() < 1.0) {
                let progress = bld.farm_states.progress.get(gi).copied().unwrap_or(0.0);
                let ready = bld.farm_states.states.get(gi) == Some(&FarmGrowthState::Ready);
                let label = if ready { "Ready to harvest" } else { &format!("Growing: {:.0}%", progress * 100.0) };
                ui.label(label);
                let color = if ready {
                    egui::Color32::from_rgb(200, 180, 40)
                } else if progress > 0.0 {
                    egui::Color32::from_rgb(160, 140, 40)
                } else {
                    egui::Color32::from_rgb(100, 100, 100)
                };
                ui.add(egui::ProgressBar::new(progress)
                    .text(format!("{:.0}%", progress * 100.0))
                    .fill(color));
                let occupants = bld.farm_occupancy.count(world_pos);
                if occupants > 0 {
                    let mult = crate::constants::mine_productivity_mult(occupants);
                    ui.label(format!("Miners: {} ({:.0}% speed)", occupants, mult * 100.0));
                }
            }
        }

        _ => {
            if let Some(spawner) = def.spawner {
                let spawner_kind = tileset_index(def.kind) as i32;
                let spawns_label = npc_def(Job::from_i32(spawner.job)).label;
                if let Some(entry) = bld.spawner_state.0.iter().find(|e| {
                    e.building_kind == spawner_kind
                        && (e.position - world_pos).length() < 1.0
                        && is_alive(e.position)
                }) {
                    ui.label(format!("Spawns: {}", spawns_label));
                    if entry.npc_slot >= 0 {
                        let slot = entry.npc_slot as usize;
                        if slot < meta_cache.0.len() {
                            let meta = &meta_cache.0[slot];
                            ui.label(format!("NPC: {} (Lv.{})", meta.name, meta.level));
                        }
                        ui.colored_label(egui::Color32::from_rgb(80, 200, 80), "Alive");
                        // Show NPC state from ECS
                        if let Some((_, _, home, _, _, activity, combat, squad_id, patrol_route))
                            = npc_states.states.iter().find(|(ni, ..)| ni.0 == slot)
                        {
                            let mut parts: Vec<&str> = Vec::new();
                            let combat_name = combat.name();
                            if !combat_name.is_empty() { parts.push(combat_name); }
                            parts.push(activity.name());
                            ui.label(format!("State: {}", parts.join(", ")));
                            if let Some(sq) = squad_id {
                                ui.label(format!("Squad: {}", sq.0 + 1));
                            }
                            let has_patrol = patrol_route.is_some_and(|r| !r.posts.is_empty());
                            ui.label(format!("Patrol route: {}", if has_patrol { "yes" } else { "none" }));
                            if slot * 2 + 1 < gpu_state.positions.len() {
                                let px = gpu_state.positions[slot * 2];
                                let py = gpu_state.positions[slot * 2 + 1];
                                if px > -9000.0 {
                                    ui.label(format!("GPU pos: ({:.0}, {:.0})", px, py));
                                }
                            }
                            ui.label(format!("Home: ({:.0}, {:.0})", home.0.x, home.0.y));
                        }
                    } else if entry.respawn_timer > 0.0 {
                        ui.colored_label(egui::Color32::from_rgb(200, 200, 40),
                            format!("Respawning in {:.0}h", entry.respawn_timer));
                    } else {
                        ui.colored_label(egui::Color32::from_rgb(200, 200, 40), "Spawning...");
                    }
                }
                if def.kind == BuildingKind::MinerHome {
                    ui.separator();
                    let mh_idx = world_data.miner_home_at(world_pos);
                    if let Some(mh_idx) = mh_idx {
                        mine_assignment_ui(ui, world_data, mh_idx, world_pos, dirty, ui_state);
                    }
                }
            }
        }
    }

    // Copy Debug Info — gated behind debug_coordinates (same as NPC inspector)
    if settings.debug_coordinates {
        ui.separator();
        let data_idx = crate::world::find_building_data_index(world_data, kind, world_pos);
        let (hp, max_hp) = data_idx
            .and_then(|i| bld.building_hp.get(kind, i))
            .map(|hp| (hp, BuildingHpState::max_hp(kind)))
            .unwrap_or((0.0, 0.0));

        ui.label(format!("Pos: ({:.0}, {:.0})  Grid: ({}, {})", world_pos.x, world_pos.y, col, row));
        ui.label(format!("HP: {:.0}/{:.0}  Kind: {:?}", hp, max_hp, kind));

        if ui.button("Copy Debug Info").clicked() {
            let name = def.label;
            let town_name = world_data.towns.get(town_idx)
                .map(|t| t.name.as_str()).unwrap_or("?");
            let faction_text = world_data.towns.get(town_idx)
                .map(|t| t.faction.to_string())
                .unwrap_or_else(|| if kind == BuildingKind::GoldMine { "Unowned".to_string() } else { "?".to_string() });
            let mut info = format!(
                "{name} [{kind:?}]\n\
                 Town: {town}\n\
                 Faction: {faction}\n\
                 Pos: ({px:.0}, {py:.0})  Grid: ({col}, {row})\n\
                 HP: {hp:.0}/{max:.0}\n\
                 ",
                name = name,
                kind = kind,
                town = town_name,
                faction = faction_text,
                px = world_pos.x,
                py = world_pos.y,
                col = col,
                row = row,
                hp = hp,
                max = max_hp,
            );
            match kind {
                BuildingKind::Farm => {
                    if let Some(farm_idx) = world_data.farms().iter().position(|f| (f.position - world_pos).length() < 1.0) {
                        if let Some(state) = bld.farm_states.states.get(farm_idx) {
                            let state_name = match state {
                                FarmGrowthState::Growing => "Growing",
                                FarmGrowthState::Ready => "Ready to harvest",
                            };
                            info.push_str(&format!("Status: {}\n", state_name));
                            if let Some(&progress) = bld.farm_states.progress.get(farm_idx) {
                                info.push_str(&format!("Growth: {:.0}%\n", progress * 100.0));
                            }
                        }
                        let occupants = bld.farm_occupancy.count(world_pos);
                        info.push_str(&format!("Farmers: {}\n", occupants));
                    }
                }
                BuildingKind::Waypoint => {
                    if let Some(wp_idx) = world_data.get(BuildingKind::Waypoint).iter()
                        .position(|w| (w.position - world_pos).length() < 1.0)
                    {
                        info.push_str(&format!("Patrol order: {}\n", world_data.get(BuildingKind::Waypoint)[wp_idx].patrol_order));
                    }
                }
                BuildingKind::Fountain => {
                    let base_radius = bld.combat_config.heal_radius;
                    let levels = bld.town_upgrades.town_levels(town_idx);
                    let upgrade_bonus = UPGRADES.stat_level(&levels, "Town", UpgradeStatKind::FountainRange) as f32 * 24.0;
                    let tower = resolve_town_tower_stats(&levels);
                    info.push_str(&format!("Heal radius: {:.0}px\n", base_radius + upgrade_bonus));
                    info.push_str(&format!("Heal rate: {:.0}/s\n", bld.combat_config.heal_rate));
                    info.push_str(&format!("Tower range: {:.0}px\n", tower.range));
                    info.push_str(&format!("Tower damage: {:.1}\n", tower.damage));
                    info.push_str(&format!("Tower cooldown: {:.2}s\n", tower.cooldown));
                    info.push_str(&format!("Tower projectile life: {:.2}s\n", tower.proj_lifetime));
                    if let Some(&food) = bld.food_storage.food.get(town_idx) {
                        info.push_str(&format!("Food: {}\n", food));
                    }
                }
                BuildingKind::Camp => {
                    if let Some(&food) = bld.food_storage.food.get(town_idx) {
                        info.push_str(&format!("Camp food: {}\n", food));
                    }
                }
                BuildingKind::Bed => {
                    info.push_str("Rest point\n");
                }
                BuildingKind::GoldMine => {
                    if let Some(mine_idx) = world_data.gold_mine_at(world_pos) {
                        info.push_str(&format!("Name: {}\n", crate::ui::gold_mine_name(mine_idx)));
                        let enabled = mining_policy.mine_enabled.get(mine_idx).copied().unwrap_or(true);
                        info.push_str(if enabled { "Auto-mining: ON\n" } else { "Auto-mining: OFF\n" });
                    }
                    if let Some(gi) = bld.farm_states.positions.iter().position(|p| (*p - world_pos).length() < 1.0) {
                        let progress = bld.farm_states.progress.get(gi).copied().unwrap_or(0.0);
                        let ready = bld.farm_states.states.get(gi) == Some(&FarmGrowthState::Ready);
                        if ready {
                            info.push_str("Ready to harvest\n");
                        } else {
                            info.push_str(&format!("Growing: {:.0}%\n", progress * 100.0));
                        }
                        let occupants = bld.farm_occupancy.count(world_pos);
                        if occupants > 0 {
                            let mult = crate::constants::mine_productivity_mult(occupants);
                            info.push_str(&format!("Miners: {} ({:.0}% speed)\n", occupants, mult * 100.0));
                        }
                    }
                }
                _ => {}
            }
            // Append spawner NPC state
            if let Some(spawner) = def.spawner {
                let spawner_kind = tileset_index(def.kind) as i32;
                if let Some(entry) = bld.spawner_state.0.iter().find(|e| {
                    e.building_kind == spawner_kind
                        && (e.position - world_pos).length() < 1.0
                        && is_alive(e.position)
                }) {
                    let spawns_label = npc_def(Job::from_i32(spawner.job)).label;
                    info.push_str(&format!("Spawns: {}\n", spawns_label));
                    if entry.npc_slot >= 0 {
                        let slot = entry.npc_slot as usize;
                        if slot < meta_cache.0.len() {
                            let meta = &meta_cache.0[slot];
                            info.push_str(&format!("NPC: {} (Lv.{}) slot={}\n", meta.name, meta.level, slot));
                        }
                        if let Some((_, _, home, _, _, activity, combat, squad_id, patrol_route))
                            = npc_states.states.iter().find(|(ni, ..)| ni.0 == slot)
                        {
                            let combat_name = combat.name();
                            info.push_str(&format!("State: {}{}\n",
                                if combat_name.is_empty() { "" } else { combat_name },
                                if combat_name.is_empty() { activity.name().to_string() } else { format!(", {}", activity.name()) }));
                            if let Some(sq) = squad_id {
                                info.push_str(&format!("Squad: {}\n", sq.0 + 1));
                            }
                            let has_patrol = patrol_route.is_some_and(|r| !r.posts.is_empty());
                            info.push_str(&format!("Patrol route: {}\n", if has_patrol { "yes" } else { "none" }));
                            if slot * 2 + 1 < gpu_state.positions.len() {
                                let px = gpu_state.positions[slot * 2];
                                let py = gpu_state.positions[slot * 2 + 1];
                                if px > -9000.0 {
                                    info.push_str(&format!("GPU pos: ({:.0}, {:.0})\n", px, py));
                                }
                            }
                            info.push_str(&format!("Home: ({:.0}, {:.0})\n", home.0.x, home.0.y));
                        }
                    } else if entry.respawn_timer > 0.0 {
                        info.push_str(&format!("Respawning in {:.0}h\n", entry.respawn_timer));
                    }
                }
            }
            info.push_str(&format!(
                "Day {day} {hour:02}:{min:02}\n\
                 ---\n",
                day = game_time.day(),
                hour = game_time.hour(),
                min = game_time.minute(),
            ));
            // Append building damage log entries (same pattern as NPC log in copy)
            let prefix = format!("{:?} in {}", kind, town_name);
            for entry in &combat_log.entries {
                if entry.kind == CombatEventKind::BuildingDamage && entry.message.starts_with(&prefix) {
                    info.push_str(&format!("D{}:{:02}:{:02} {}\n",
                        entry.day, entry.hour, entry.minute, entry.message));
                }
            }
            *copy_text = Some(info);
        }
    }
}

// ============================================================================
// TARGET OVERLAY
// ============================================================================

fn draw_corner_brackets(
    painter: &egui::Painter,
    center: egui::Pos2,
    half_w: f32,
    half_h: f32,
    color: egui::Color32,
) {
    let hw = half_w.max(6.0);
    let hh = half_h.max(6.0);
    let x0 = center.x - hw;
    let x1 = center.x + hw;
    let y0 = center.y - hh;
    let y1 = center.y + hh;
    let len_x = (hw * 0.55).clamp(6.0, 18.0);
    let len_y = (hh * 0.55).clamp(6.0, 18.0);
    let stroke = egui::Stroke::new(2.0, color);

    // Top-left
    painter.line_segment([egui::pos2(x0, y0), egui::pos2(x0 + len_x, y0)], stroke);
    painter.line_segment([egui::pos2(x0, y0), egui::pos2(x0, y0 + len_y)], stroke);
    // Top-right
    painter.line_segment([egui::pos2(x1 - len_x, y0), egui::pos2(x1, y0)], stroke);
    painter.line_segment([egui::pos2(x1, y0), egui::pos2(x1, y0 + len_y)], stroke);
    // Bottom-left
    painter.line_segment([egui::pos2(x0, y1 - len_y), egui::pos2(x0, y1)], stroke);
    painter.line_segment([egui::pos2(x0, y1), egui::pos2(x0 + len_x, y1)], stroke);
    // Bottom-right
    painter.line_segment([egui::pos2(x1 - len_x, y1), egui::pos2(x1, y1)], stroke);
    painter.line_segment([egui::pos2(x1, y1 - len_y), egui::pos2(x1, y1)], stroke);
}

/// Draw corner-bracket selection indicators for clicked NPCs/buildings.
/// NPC and building boxes use different world sizes so the brackets scale correctly.
pub fn selection_overlay_system(
    mut contexts: EguiContexts,
    selected: Res<SelectedNpc>,
    selected_building: Res<SelectedBuilding>,
    gpu_state: Res<GpuReadState>,
    building_slots: Res<BuildingSlotMap>,
    grid: Res<WorldGrid>,
    world_data: Res<WorldData>,
    camera_query: Query<(&Transform, &Projection), With<crate::render::MainCamera>>,
    windows: Query<&Window>,
) -> Result {
    if selected.0 < 0 && !selected_building.active { return Ok(()); }

    let Ok(window) = windows.single() else { return Ok(()); };
    let Ok((transform, projection)) = camera_query.single() else { return Ok(()); };
    let zoom = match projection {
        Projection::Orthographic(ortho) => 1.0 / ortho.scale,
        _ => 1.0,
    };
    let cam = transform.translation.truncate();
    let viewport = egui::Vec2::new(window.width(), window.height());
    let center = viewport * 0.5;

    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::background());

    // NPC selection: sprite is ~16 world units.
    if selected.0 >= 0 {
        let idx = selected.0 as usize;
        let positions = &gpu_state.positions;
        if idx * 2 + 1 < positions.len() {
            let x = positions[idx * 2];
            let y = positions[idx * 2 + 1];
            if is_alive(Vec2::new(x, y)) {
                let screen = egui::Pos2::new(
                    center.x + (x - cam.x) * zoom,
                    center.y - (y - cam.y) * zoom,
                );
                let half = (10.0 * zoom).max(7.0);
                draw_corner_brackets(
                    &painter,
                    screen,
                    half,
                    half,
                    egui::Color32::from_rgba_unmultiplied(100, 200, 255, 220),
                );
            }
        }
    }

    // Building selection: tile/building footprint is larger (one grid cell ~= 32 world units).
    if selected_building.active {
        let bpos = if let Some(slot) = selected_building.slot {
            let i = slot * 2;
            if i + 1 < gpu_state.positions.len() {
                let x = gpu_state.positions[i];
                let y = gpu_state.positions[i + 1];
                { let p = Vec2::new(x, y); if is_alive(p) { Some(p) } else { None } }
            } else {
                None
            }
        } else if let (Some(kind), Some(index)) = (selected_building.kind, selected_building.index) {
            building_slots.get_slot(kind, index).and_then(|slot| {
                let i = slot * 2;
                if i + 1 < gpu_state.positions.len() {
                    let x = gpu_state.positions[i];
                    let y = gpu_state.positions[i + 1];
                    { let p = Vec2::new(x, y); if is_alive(p) { Some(p) } else { None } }
                } else {
                    None
                }
            })
        } else {
            None
        }.or_else(|| {
            selected_building_info(&selected_building, &grid, &world_data)
                .map(|(_, _, pos, _, _)| pos)
        }).or_else(|| {
            let col = selected_building.col;
            let row = selected_building.row;
            if grid.cell(col, row).and_then(|c| c.building.as_ref()).is_some() {
                Some(grid.grid_to_world(col, row))
            } else {
                None
            }
        });
        if let Some(wp) = bpos {
            let screen = egui::Pos2::new(
                center.x + (wp.x - cam.x) * zoom,
                center.y - (wp.y - cam.y) * zoom,
            );
            let half = (grid.cell_size * 0.60 * zoom).max(10.0);
            draw_corner_brackets(
                &painter,
                screen,
                half,
                half,
                egui::Color32::from_rgba_unmultiplied(255, 220, 90, 230),
            );
        }
    }

    Ok(())
}

/// Draw a target indicator line from selected NPC to its movement target.
/// Uses egui painter on the background layer so it renders over the game viewport.
pub fn target_overlay_system(
    mut contexts: EguiContexts,
    selected: Res<SelectedNpc>,
    gpu_state: Res<GpuReadState>,
    buffer_writes: Res<NpcGpuState>,
    camera_query: Query<(&Transform, &Projection), With<crate::render::MainCamera>>,
    windows: Query<&Window>,
) -> Result {
    if selected.0 < 0 { return Ok(()); }
    let idx = selected.0 as usize;

    let positions = &gpu_state.positions;
    let targets = &buffer_writes.targets;
    if idx * 2 + 1 >= positions.len() || idx * 2 + 1 >= targets.len() { return Ok(()); }

    let npc_x = positions[idx * 2];
    let npc_y = positions[idx * 2 + 1];
    if npc_x < -9000.0 { return Ok(()); }

    let tgt_x = targets[idx * 2];
    let tgt_y = targets[idx * 2 + 1];

    // Skip if target == position (stationary)
    let dx = tgt_x - npc_x;
    let dy = tgt_y - npc_y;
    if dx * dx + dy * dy < 4.0 { return Ok(()); }

    let Ok(window) = windows.single() else { return Ok(()); };
    let Ok((transform, projection)) = camera_query.single() else { return Ok(()); };

    let zoom = match projection {
        Projection::Orthographic(ortho) => 1.0 / ortho.scale,
        _ => 1.0,
    };
    let cam = transform.translation.truncate();
    let viewport = egui::Vec2::new(window.width(), window.height());
    let center = viewport * 0.5;

    // World->screen conversion (flip Y)
    let npc_screen = egui::Pos2::new(
        center.x + (npc_x - cam.x) * zoom,
        center.y - (npc_y - cam.y) * zoom,
    );
    let tgt_screen = egui::Pos2::new(
        center.x + (tgt_x - cam.x) * zoom,
        center.y - (tgt_y - cam.y) * zoom,
    );

    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::background());

    // Line from NPC to target
    let line_color = egui::Color32::from_rgba_unmultiplied(255, 220, 50, 200);
    painter.line_segment([npc_screen, tgt_screen], egui::Stroke::new(2.5, line_color));

    // Diamond marker at target
    let s = 7.0;
    let diamond = [
        egui::Pos2::new(tgt_screen.x, tgt_screen.y - s),
        egui::Pos2::new(tgt_screen.x + s, tgt_screen.y),
        egui::Pos2::new(tgt_screen.x, tgt_screen.y + s),
        egui::Pos2::new(tgt_screen.x - s, tgt_screen.y),
    ];
    let fill = egui::Color32::from_rgba_unmultiplied(255, 220, 50, 240);
    painter.add(egui::Shape::convex_polygon(diamond.to_vec(), fill, egui::Stroke::NONE));

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
    camera_query: Query<(&Transform, &Projection), With<crate::render::MainCamera>>,
    windows: Query<&Window>,
) -> Result {
    let Ok(window) = windows.single() else { return Ok(()); };
    let Ok((transform, projection)) = camera_query.single() else { return Ok(()); };

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
        egui::Color32::from_rgb(255, 80, 80),    // red
        egui::Color32::from_rgb(80, 180, 255),    // blue
        egui::Color32::from_rgb(80, 220, 80),     // green
        egui::Color32::from_rgb(255, 200, 40),    // yellow
        egui::Color32::from_rgb(200, 80, 255),    // purple
        egui::Color32::from_rgb(255, 140, 40),    // orange
        egui::Color32::from_rgb(40, 220, 200),    // teal
        egui::Color32::from_rgb(255, 100, 180),   // pink
        egui::Color32::from_rgb(180, 180, 80),    // olive
        egui::Color32::from_rgb(140, 140, 255),   // light blue
    ];

    for (i, squad) in squad_state.squads.iter().enumerate() {
        if !squad.is_player() { continue; }
        let Some(target) = squad.target else { continue };
        if squad.members.is_empty() { continue; }

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

    // Placement mode cursor hint
    if squad_state.placing_target && squad_state.selected >= 0 {
        if let Some(cursor_pos) = window.cursor_position() {
            let cursor_egui = egui::Pos2::new(cursor_pos.x, cursor_pos.y);
            let hint_color = egui::Color32::from_rgba_unmultiplied(255, 255, 100, 160);
            painter.circle_stroke(cursor_egui, 12.0, egui::Stroke::new(2.0, hint_color));
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
    let selected_faction = if ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Factions {
        ui_state.factions_overlay_faction
    } else {
        None
    };
    if !show_all && selected_faction.is_none() {
        return Ok(());
    }

    let Ok(window) = windows.single() else { return Ok(()); };
    let Ok((transform, projection)) = camera_query.single() else { return Ok(()); };

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
        if town.sprite_type == 0 {
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
        if !show_all && selected_faction != Some(faction) { continue; }
        let Some(target_world) = squad.target else { continue; };
        if squad.members.is_empty() { continue; }
        let Some(start_world) = start_by_faction.get(&faction).copied() else { continue; };

        let color_idx = color_idx_by_faction.entry(faction).or_insert(0usize);
        let color = palette[*color_idx % palette.len()];
        *color_idx += 1;
        let start = to_screen(start_world);
        let end = to_screen(target_world);
        let line = end - start;
        let len = line.length();
        if len < 6.0 { continue; }
        let dir = line / len;
        let perp = egui::vec2(-dir.y, dir.x);

        painter.line_segment([start, end], egui::Stroke::new(2.0, color));

        let head_len = 12.0;
        let head_w = 6.0;
        let base = end - dir * head_len;
        let p1 = base + perp * head_w;
        let p2 = base - perp * head_w;
        painter.add(egui::Shape::convex_polygon(vec![end, p1, p2], color, egui::Stroke::NONE));

        let label_pos = end + perp * 10.0;
        let label = if show_all {
            format!("F{} Squad {}", faction, si + 1)
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

// ============================================================================
// JUKEBOX UI
// ============================================================================

/// Small overlay at top-right (below top bar) showing current track + pause/skip/loop.
pub fn jukebox_ui_system(
    mut contexts: EguiContexts,
    mut audio: ResMut<GameAudio>,
    mut commands: Commands,
    music_query: Query<(Entity, &AudioSink), With<MusicTrack>>,
    mut settings: ResMut<crate::settings::UserSettings>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let Some(track_idx) = audio.last_track else { return Ok(()) };

    let frame = egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(30, 30, 35, 220))
        .inner_margin(egui::Margin::same(6));

    egui::Area::new(egui::Id::new("jukebox"))
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 30.0])
        .show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Track picker dropdown
                    let current_name = crate::systems::audio::track_display_name(track_idx);
                    let mut selected = track_idx;
                    egui::ComboBox::from_id_salt("jukebox_track")
                        .selected_text(format!("♪ {}", current_name))
                        .width(160.0)
                        .show_ui(ui, |ui| {
                            for i in 0..crate::systems::audio::track_count() {
                                ui.selectable_value(&mut selected, i, crate::systems::audio::track_display_name(i));
                            }
                        });
                    // Switch track if user picked a different one
                    if selected != track_idx {
                        audio.play_next = Some(selected);
                        if let Ok((entity, _)) = music_query.single() {
                            commands.entity(entity).despawn();
                        }
                    }

                    if let Ok((entity, sink)) = music_query.single() {
                        let paused = sink.is_paused();
                        if ui.small_button(if paused { "▶" } else { "⏸" }).clicked() {
                            if paused { sink.play() } else { sink.pause() }
                        }
                        if ui.small_button("⏭").clicked() {
                            commands.entity(entity).despawn();
                        }
                    }

                    // Speed dropdown
                    let prev_speed = audio.music_speed;
                    let speed_pct = (audio.music_speed * 100.0).round() as i32;
                    egui::ComboBox::from_id_salt("jukebox_speed")
                        .selected_text(format!("{}%", speed_pct))
                        .width(55.0)
                        .show_ui(ui, |ui| {
                            // 10% to 100% in 10% steps
                            for pct in (10..=100).step_by(10) {
                                let val = pct as f32 / 100.0;
                                ui.selectable_value(&mut audio.music_speed, val, format!("{}%", pct));
                            }
                            // 150% to 500% in 50% steps
                            for pct in (150..=500).step_by(50) {
                                let val = pct as f32 / 100.0;
                                ui.selectable_value(&mut audio.music_speed, val, format!("{}%", pct));
                            }
                        });
                    if let Ok((_, sink)) = music_query.single() {
                        sink.set_speed(audio.music_speed);
                    }
                    if audio.music_speed != prev_speed {
                        settings.music_speed = audio.music_speed;
                        crate::settings::save_settings(&settings);
                    }

                    // Loop toggle
                    let btn = egui::Button::new(egui::RichText::new("🔁").size(12.0));
                    let resp = ui.add(btn);
                    if resp.clicked() {
                        audio.loop_current = !audio.loop_current;
                    }
                    if resp.hovered() {
                        resp.clone().show_tooltip_text(if audio.loop_current { "Loop: ON" } else { "Loop: OFF" });
                    }
                    if audio.loop_current {
                        resp.highlight();
                    }
                });
            });
        });

    Ok(())
}

/// Toast notification for save/load feedback — centered top area, fades out.
pub fn save_toast_system(
    mut contexts: EguiContexts,
    toast: Res<crate::save::SaveToast>,
) -> Result {
    if toast.timer <= 0.0 { return Ok(()); }
    let ctx = contexts.ctx_mut()?;

    let alpha = (toast.timer.min(1.0) * 255.0) as u8;
    egui::Area::new(egui::Id::new("save_toast"))
        .anchor(egui::Align2::CENTER_TOP, [0.0, 60.0])
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_unmultiplied(0, 0, 0, alpha / 2))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(12, 6))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(&toast.message)
                        .size(18.0)
                        .strong()
                        .color(egui::Color32::from_rgba_unmultiplied(255, 255, 200, alpha)));
                });
        });

    Ok(())
}
