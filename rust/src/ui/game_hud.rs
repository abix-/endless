//! In-game HUD — top resource bar, bottom panel (inspector + combat log), target overlay.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::components::*;
use crate::gpu::NpcBufferWrites;
use crate::resources::*;
use crate::settings::{self, UserSettings};
use crate::ui::tipped;
use crate::world::{WorldData, WorldGrid, Building, BuildingOccupancy};
use crate::systems::stats::{CombatConfig, TownUpgrades, UpgradeType};

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
    slots: Res<SlotAllocator>,
    world_data: Res<WorldData>,
    mut ui_state: ResMut<UiState>,
    spawner_state: Res<SpawnerState>,
    catalog: Res<HelpCatalog>,
    time: Res<Time>,
    mut avg_fps: Local<f32>,
    settings: Res<crate::settings::UserSettings>,
) -> Result {
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
                    let houses = spawner_state.0.iter().filter(|s| s.building_kind == 0 && s.town_idx == 0 && s.position.x > -9000.0).count();
                    let barracks = spawner_state.0.iter().filter(|s| s.building_kind == 1 && s.town_idx == 0 && s.position.x > -9000.0).count();
                    tipped(ui, format!("Archers: {}/{}", guards, barracks), catalog.0.get("archers").unwrap_or(&""));
                    tipped(ui, format!("Farmers: {}/{}", farmers, houses), catalog.0.get("farmers").unwrap_or(&""));
                    let total_alive = slots.alive();
                    let total_spawners = spawner_state.0.iter().filter(|s| s.position.x > -9000.0).count();
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
    farm_states: Res<'w, FarmStates>,
    farm_occupancy: Res<'w, BuildingOccupancy>,
    spawner_state: Res<'w, SpawnerState>,
    guard_post_state: Res<'w, GuardPostState>,
    food_storage: Res<'w, FoodStorage>,
    combat_config: Res<'w, CombatConfig>,
    town_upgrades: Res<'w, TownUpgrades>,
    mine_states: Res<'w, MineStates>,
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
    pub initialized: bool,
    // Cached merged log entries — skip rebuild when sources unchanged
    cached_selected_npc: i32,
    cached_filters: (bool, bool, bool, bool, bool, bool, bool),
    cached_entries: Vec<(i64, egui::Color32, String, String)>,
}

#[derive(Default)]
pub struct InspectorRenameState {
    slot: i32,
    text: String,
}

/// Bottom panel: NPC/building inspector.
pub fn bottom_panel_system(
    mut contexts: EguiContexts,
    data: BottomPanelData,
    mut meta_cache: ResMut<NpcMetaCache>,
    bld_data: BuildingInspectorData,
    world_data: Res<WorldData>,
    health_query: Query<(&NpcIndex, &Health, &CachedStats, &Energy), Without<Dead>>,
    npc_states: NpcStateQuery,
    gpu_state: Res<GpuReadState>,
    buffer_writes: Res<NpcBufferWrites>,
    mut follow: ResMut<FollowSelected>,
    settings: Res<UserSettings>,
    catalog: Res<HelpCatalog>,
    mut destroy_request: ResMut<DestroyRequest>,
    mut rename_state: Local<InspectorRenameState>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    let mut copy_text: Option<String> = None;

    // Only show inspector when something is selected
    let has_npc = data.selected.0 >= 0;
    let has_building = bld_data.selected_building.active;
    if has_npc || has_building {
        let frame = egui::Frame::new()
            .fill(egui::Color32::from_rgba_unmultiplied(30, 30, 35, 220))
            .inner_margin(egui::Margin::same(6));

        egui::Window::new("Inspector")
            .anchor(egui::Align2::LEFT_BOTTOM, [2.0, -2.0])
            .fixed_size([300.0, 160.0])
            .collapsible(true)
            .movable(false)
            .frame(frame)
            .show(ctx, |ui| {
                inspector_content(
                    ui, &data, &mut meta_cache, &mut rename_state, &bld_data, &world_data, &health_query,
                    &npc_states, &gpu_state, &buffer_writes, &mut follow, &settings, &catalog, &mut copy_text,
                );
                // Destroy button for selected buildings (not fountains/camps)
                if has_building && !has_npc {
                    let col = bld_data.selected_building.col;
                    let row = bld_data.selected_building.row;
                    let is_destructible = bld_data.grid.cell(col, row)
                        .and_then(|c| c.building.as_ref())
                        .map(|b| !matches!(b, Building::Fountain { .. } | Building::Camp { .. } | Building::GoldMine))
                        .unwrap_or(false);
                    if is_destructible {
                        ui.separator();
                        if ui.button(egui::RichText::new("Destroy").color(egui::Color32::from_rgb(220, 80, 80))).clicked() {
                            destroy_request.0 = Some((col, row));
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
        filter_state.initialized = true;
    }

    let prev_filters = (
        filter_state.show_kills, filter_state.show_spawns, filter_state.show_raids,
        filter_state.show_harvests, filter_state.show_levelups, filter_state.show_npc_activity,
        filter_state.show_ai,
    );

    let frame = egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(30, 30, 35, 220))
        .inner_margin(egui::Margin::same(6));

    egui::Window::new("Combat Log")
        .anchor(egui::Align2::RIGHT_BOTTOM, [-2.0, -2.0])
        .fixed_size([450.0, 140.0])
        .collapsible(false)
        .movable(false)
        .frame(frame)
        .title_bar(true)
        .show(ctx, |ui| {
            // Filter checkboxes
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut filter_state.show_kills, "Deaths");
                ui.checkbox(&mut filter_state.show_spawns, "Spawns");
                ui.checkbox(&mut filter_state.show_raids, "Raids");
                ui.checkbox(&mut filter_state.show_harvests, "Harvests");
                ui.checkbox(&mut filter_state.show_levelups, "Levels");
                ui.checkbox(&mut filter_state.show_npc_activity, "NPC");
                ui.checkbox(&mut filter_state.show_ai, "AI");
            });

            ui.separator();

            // Rebuild merged entries only when sources changed
            let curr_filters = (
                filter_state.show_kills, filter_state.show_spawns, filter_state.show_raids,
                filter_state.show_harvests, filter_state.show_levelups, filter_state.show_npc_activity,
                filter_state.show_ai,
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
                    };
                    if !show { continue; }

                    let color = match entry.kind {
                        CombatEventKind::Kill => egui::Color32::from_rgb(220, 80, 80),
                        CombatEventKind::Spawn => egui::Color32::from_rgb(80, 200, 80),
                        CombatEventKind::Raid => egui::Color32::from_rgb(220, 160, 40),
                        CombatEventKind::Harvest => egui::Color32::from_rgb(200, 200, 60),
                        CombatEventKind::LevelUp => egui::Color32::from_rgb(80, 180, 255),
                        CombatEventKind::Ai => egui::Color32::from_rgb(180, 120, 220),
                    };

                    let key = (entry.day as i64) * 10000 + (entry.hour as i64) * 100 + entry.minute as i64;
                    let ts = format!("[D{} {:02}:{:02}]", entry.day, entry.hour, entry.minute);
                    filter_state.cached_entries.push((key, color, ts, entry.message.clone()));
                }

                if filter_state.show_npc_activity && data.selected.0 >= 0 {
                    let idx = data.selected.0 as usize;
                    if idx < data.npc_logs.0.len() {
                        let npc_color = egui::Color32::from_rgb(180, 180, 220);
                        for entry in data.npc_logs.0[idx].iter() {
                            let key = (entry.day as i64) * 10000 + (entry.hour as i64) * 100 + entry.minute as i64;
                            let ts = format!("[D{} {:02}:{:02}]", entry.day, entry.hour, entry.minute);
                            filter_state.cached_entries.push((key, npc_color, ts, entry.message.to_string()));
                        }
                    }
                }

                filter_state.cached_entries.sort_by_key(|(key, ..)| *key);
                filter_state.cached_selected_npc = data.selected.0;
                filter_state.cached_filters = curr_filters;
            }

            // Render from cache
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for (_, color, ts, msg) in &filter_state.cached_entries {
                        ui.horizontal(|ui| {
                            ui.small(ts);
                            ui.colored_label(*color, msg);
                        });
                    }
                });
        });

    // Persist filter changes
    let curr_filters = (
        filter_state.show_kills, filter_state.show_spawns, filter_state.show_raids,
        filter_state.show_harvests, filter_state.show_levelups, filter_state.show_npc_activity,
        filter_state.show_ai,
    );
    if curr_filters != prev_filters {
        settings.log_kills = filter_state.show_kills;
        settings.log_spawns = filter_state.show_spawns;
        settings.log_raids = filter_state.show_raids;
        settings.log_harvests = filter_state.show_harvests;
        settings.log_levelups = filter_state.show_levelups;
        settings.log_npc_activity = filter_state.show_npc_activity;
        settings.log_ai = filter_state.show_ai;
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
    world_data: &WorldData,
    health_query: &Query<(&NpcIndex, &Health, &CachedStats, &Energy), Without<Dead>>,
    npc_states: &NpcStateQuery,
    gpu_state: &GpuReadState,
    buffer_writes: &NpcBufferWrites,
    follow: &mut FollowSelected,
    settings: &UserSettings,
    catalog: &HelpCatalog,
    copy_text: &mut Option<String>,
) {
    let sel = data.selected.0;
    if sel < 0 {
        rename_state.slot = -1;
        rename_state.text.clear();
        if bld_data.selected_building.active {
            building_inspector_content(ui, bld_data, world_data, meta_cache);
            return;
        }
        ui.label("Click an NPC or building to inspect");
        return;
    }
    let idx = sel as usize;
    if idx >= meta_cache.0.len() { return; }

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

    ui.strong(format!("{}", meta.name));
    tipped(ui, format!("{} Lv.{}  XP: {}/{}", crate::job_name(meta.job), meta.level, meta.xp, (meta.level + 1) * (meta.level + 1) * 100), catalog.0.get("npc_level").unwrap_or(&""));

    if let Some((_, personality, ..)) = npc_states.states.iter().find(|(ni, ..)| ni.0 == idx) {
        let trait_str = personality.trait_summary();
        if !trait_str.is_empty() {
            tipped(ui, format!("Trait: {}", trait_str), catalog.0.get("npc_trait").unwrap_or(&""));
        }
    }

    // Find HP + energy from query
    let mut hp = 0.0f32;
    let mut max_hp = 100.0f32;
    let mut energy = 0.0f32;
    for (npc_idx, health, cached, npc_energy) in health_query.iter() {
        if npc_idx.0 == idx {
            hp = health.0;
            max_hp = cached.max_health;
            energy = npc_energy.0;
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
            .text(format!("{:.0}/{:.0}", hp, max_hp))
            .fill(hp_color));
    });

    // Energy bar
    let energy_frac = (energy / 100.0).clamp(0.0, 1.0);
    ui.horizontal(|ui| {
        tipped(ui, "EN:", catalog.0.get("npc_energy").unwrap_or(&""));
        ui.add(egui::ProgressBar::new(energy_frac)
            .text(format!("{:.0}", energy))
            .fill(egui::Color32::from_rgb(60, 120, 200)));
    });

    // Town name
    if meta.town_id >= 0 {
        if let Some(town) = world_data.towns.get(meta.town_id as usize) {
            ui.label(format!("Town: {}", town.name));
        }
    }

    // State
    let mut state_str = String::new();
    let mut home_str = String::new();
    let mut faction_str = String::new();

    if let Some((_, _, home, faction, town_id, activity, combat))
        = npc_states.states.iter().find(|(ni, ..)| ni.0 == idx)
    {
        home_str = format!("({:.0}, {:.0})", home.0.x, home.0.y);
        faction_str = format!("{} (town {})", faction.0, town_id.0);

        let mut parts: Vec<&str> = Vec::new();
        let combat_name = combat.name();
        if !combat_name.is_empty() { parts.push(combat_name); }
        parts.push(activity.name());
        state_str = parts.join(", ");
    }

    tipped(ui, format!("State: {}", state_str), catalog.0.get("npc_state").unwrap_or(&""));

    // Follow toggle
    ui.horizontal(|ui| {
        if ui.selectable_label(follow.0, "Follow (F)").clicked() {
            follow.0 = !follow.0;
        }
    });

    // Debug: coordinates, faction, copy button
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

        ui.label(format!("Pos: {}", pos));
        ui.label(format!("Target: {}", target));
        ui.label(format!("Home: {}", home_str));
        ui.label(format!("Faction: {}", faction_str));

        if ui.button("Copy Debug Info").clicked() {
            let mut info = format!(
                "NPC #{idx} \"{name}\" {job} Lv.{level}\n\
                 HP: {hp:.0}/{max_hp:.0}  EN: {energy:.0}\n\
                 Pos: {pos}  Target: {target}\n\
                 Home: {home}  Faction: {faction}\n\
                 State: {state}\n\
                 Day {day} {hour:02}:{min:02}\n\
                 ---\n",
                idx = idx,
                name = meta.name,
                job = crate::job_name(meta.job),
                level = meta.level,
                hp = hp,
                max_hp = max_hp,
                energy = energy,
                pos = pos,
                target = target,
                home = home_str,
                faction = faction_str,
                state = state_str,
                day = data.game_time.day(),
                hour = data.game_time.hour(),
                min = data.game_time.minute(),
            );
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

fn building_name(building: &Building) -> &'static str {
    match building {
        Building::Fountain { .. } => "Fountain",
        Building::Farm { .. } => "Farm",
        Building::Bed { .. } => "Bed",
        Building::GuardPost { .. } => "Guard Post",
        Building::Camp { .. } => "Camp",
        Building::FarmerHome { .. } => "Farmer Home",
        Building::ArcherHome { .. } => "Archer Home",
        Building::Tent { .. } => "Tent",
        Building::GoldMine => "Gold Mine",
        Building::MinerHome { .. } => "Miner Home",
    }
}

pub fn building_town_idx(building: &Building) -> u32 {
    match building {
        Building::Fountain { town_idx }
        | Building::Farm { town_idx }
        | Building::Bed { town_idx }
        | Building::GuardPost { town_idx, .. }
        | Building::Camp { town_idx }
        | Building::FarmerHome { town_idx }
        | Building::ArcherHome { town_idx }
        | Building::Tent { town_idx }
        | Building::MinerHome { town_idx } => *town_idx,
        Building::GoldMine => 0, // mines are unowned
    }
}

/// Render building inspector content when a building cell is selected.
fn building_inspector_content(
    ui: &mut egui::Ui,
    bld: &BuildingInspectorData,
    world_data: &WorldData,
    meta_cache: &NpcMetaCache,
) {
    let col = bld.selected_building.col;
    let row = bld.selected_building.row;
    let Some(cell) = bld.grid.cell(col, row) else { return };
    let Some(building) = &cell.building else { return };

    let town_idx = building_town_idx(building) as usize;

    // Header
    ui.strong(building_name(building));

    // Town name
    if let Some(town) = world_data.towns.get(town_idx) {
        ui.label(format!("Town: {}", town.name));
    }

    // Per-type details
    match building {
        Building::Farm { .. } => {
            // Find farm index by matching grid position
            let world_pos = bld.grid.grid_to_world(col, row);
            if let Some(farm_idx) = world_data.farms.iter().position(|f| {
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

        Building::FarmerHome { .. } | Building::ArcherHome { .. } | Building::Tent { .. } | Building::MinerHome { .. } => {
            let (kind, spawns_label) = match building {
                Building::FarmerHome { .. } => (0, "Farmer"),
                Building::ArcherHome { .. } => (1, "Archer"),
                Building::MinerHome { .. } => (3, "Miner"),
                _ => (2, "Raider"),
            };
            let world_pos = bld.grid.grid_to_world(col, row);

            // Find matching spawner entry
            if let Some(entry) = bld.spawner_state.0.iter().find(|e| {
                e.building_kind == kind
                    && (e.position - world_pos).length() < 1.0
                    && e.position.x > -9000.0
            }) {
                ui.label(format!("Spawns: {}", spawns_label));

                if entry.npc_slot >= 0 {
                    let slot = entry.npc_slot as usize;
                    if slot < meta_cache.0.len() {
                        let meta = &meta_cache.0[slot];
                        ui.label(format!("NPC: {} (Lv.{})", meta.name, meta.level));
                    }
                    ui.colored_label(egui::Color32::from_rgb(80, 200, 80), "Alive");
                } else if entry.respawn_timer > 0.0 {
                    ui.colored_label(
                        egui::Color32::from_rgb(200, 200, 40),
                        format!("Respawning in {:.0}h", entry.respawn_timer),
                    );
                } else {
                    ui.colored_label(egui::Color32::from_rgb(200, 200, 40), "Spawning...");
                }
            }
        }

        Building::GuardPost { patrol_order, .. } => {
            ui.label(format!("Patrol order: {}", patrol_order));

            // Find guard post index by matching grid position
            let world_pos = bld.grid.grid_to_world(col, row);
            if let Some(post_idx) = world_data.guard_posts.iter().position(|g| {
                (g.position - world_pos).length() < 1.0
            }) {
                if let Some(&enabled) = bld.guard_post_state.attack_enabled.get(post_idx) {
                    ui.label(format!("Turret: {}", if enabled { "Enabled" } else { "Disabled" }));
                }
            }
        }

        Building::Fountain { .. } => {
            // Healing info
            let base_radius = bld.combat_config.heal_radius;
            let upgrade_bonus = if let Some(town) = bld.town_upgrades.levels.get(town_idx) {
                town[UpgradeType::FountainRadius as usize] as f32 * 24.0
            } else {
                0.0
            };
            ui.label(format!("Heal radius: {:.0}px", base_radius + upgrade_bonus));
            ui.label(format!("Heal rate: {:.0}/s", bld.combat_config.heal_rate));

            // Town food — town_idx is direct index into food_storage
            if let Some(&food) = bld.food_storage.food.get(town_idx) {
                ui.label(format!("Food: {}", food));
            }
        }

        Building::Camp { .. } => {
            // Camp food — town_idx is direct index into food_storage
            if let Some(&food) = bld.food_storage.food.get(town_idx) {
                ui.label(format!("Camp food: {}", food));
            }
        }

        Building::Bed { .. } => {
            ui.label("Rest point");
        }

        Building::GoldMine => {
            let world_pos = bld.grid.grid_to_world(col, row);
            if let Some(mine_idx) = bld.mine_states.positions.iter().position(|p| {
                (*p - world_pos).length() < 1.0
            }) {
                let gold = bld.mine_states.gold.get(mine_idx).copied().unwrap_or(0.0);
                let max_gold = bld.mine_states.max_gold.get(mine_idx).copied().unwrap_or(1.0);
                ui.label(format!("Gold: {:.0} / {:.0}", gold, max_gold));
                let pct = gold / max_gold.max(1.0);
                let color = if pct > 0.5 {
                    egui::Color32::from_rgb(200, 180, 40)
                } else if pct > 0.0 {
                    egui::Color32::from_rgb(160, 140, 40)
                } else {
                    egui::Color32::from_rgb(100, 100, 100)
                };
                ui.add(egui::ProgressBar::new(pct)
                    .text(format!("{:.0}%", pct * 100.0))
                    .fill(color));
                let occupants = bld.farm_occupancy.count(world_pos);
                if occupants > 0 {
                    ui.label(format!("Miners: {}", occupants));
                }
            }
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
    grid: Res<WorldGrid>,
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
            if x > -9000.0 {
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
        let col = selected_building.col;
        let row = selected_building.row;
        if grid.cell(col, row).and_then(|c| c.building.as_ref()).is_some() {
            let wp = grid.grid_to_world(col, row);
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
    buffer_writes: Res<NpcBufferWrites>,
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
            egui::Color32::WHITE,
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
                        audio.last_track = Some(selected);
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
