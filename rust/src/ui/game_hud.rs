//! In-game HUD — top resource bar, bottom panel (inspector + combat log), target overlay.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::components::*;
use crate::gpu::NpcBufferWrites;
use crate::resources::*;
use crate::settings::{self, UserSettings};
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
    faction_stats: Res<FactionStats>,
    kill_stats: Res<KillStats>,
    slots: Res<SlotAllocator>,
    world_data: Res<WorldData>,
    settings: Res<UserSettings>,
    mut ui_state: ResMut<UiState>,
    spawner_state: Res<SpawnerState>,
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
                    // Debug: enemy info (rightmost)
                    if settings.debug_enemy_info {
                        // Sum food at odd indices (raider camps in interleaved array)
                        let camp_food: i32 = food_storage.food.iter().skip(1).step_by(2).sum();
                        ui.label(format!("Camp: {}", camp_food));
                        ui.label(format!("Kills: g{} r{}",
                            kill_stats.guard_kills, kill_stats.villager_kills));
                        let raider_alive: i32 = faction_stats.stats.iter().skip(1).map(|s| s.alive).sum();
                        let raider_dead: i32 = faction_stats.stats.iter().skip(1).map(|s| s.dead).sum();
                        ui.label(format!("Raiders: {}/{}", raider_alive, raider_dead));
                        ui.label(format!("Total: {}", slots.alive()));
                        ui.separator();
                    }

                    // Player stats (right-aligned) — player's town is index 0
                    let town_food = food_storage.food.first().copied().unwrap_or(0);
                    ui.label(format!("Food: {}", town_food));

                    let farmers = pop_stats.0.get(&(0, 0)).map(|s| s.alive).unwrap_or(0);
                    let guards = pop_stats.0.get(&(1, 0)).map(|s| s.alive).unwrap_or(0);
                    // Filter to player's town (pair_idx 0) to match pop_stats town 0
                    let huts = spawner_state.0.iter().filter(|s| s.building_kind == 0 && s.town_idx == 0 && s.position.x > -9000.0).count();
                    let barracks = spawner_state.0.iter().filter(|s| s.building_kind == 1 && s.town_idx == 0 && s.position.x > -9000.0).count();
                    ui.label(format!("Guards: {}/{}", guards, barracks));
                    ui.label(format!("Farmers: {}/{}", farmers, huts));
                });
            });
        });

    Ok(())
}

// ============================================================================
// BOTTOM PANEL (INSPECTOR + COMBAT LOG)
// ============================================================================

const INSPECTOR_WIDTH: f32 = 260.0;

/// Query bundle for NPC state display.
#[derive(SystemParam)]
pub struct NpcStateQuery<'w, 's> {
    states: Query<'w, 's, (
        &'static NpcIndex,
        &'static Home,
        &'static Faction,
        &'static TownId,
        &'static Activity,
        &'static CombatState,
        Option<&'static AtDestination>,
        Option<&'static Starving>,
        Option<&'static Healing>,
    ), Without<Dead>>,
}

/// Bundled readonly resources for bottom panel.
#[derive(SystemParam)]
pub struct BottomPanelData<'w> {
    game_time: Res<'w, GameTime>,
    meta_cache: Res<'w, NpcMetaCache>,
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
}

#[derive(Default)]
pub struct LogFilterState {
    pub show_kills: bool,
    pub show_spawns: bool,
    pub show_raids: bool,
    pub show_harvests: bool,
    pub show_levelups: bool,
    pub show_npc_activity: bool,
    pub initialized: bool,
}

/// Bottom panel: inspector (left, always visible) + combat log (right).
pub fn bottom_panel_system(
    mut contexts: EguiContexts,
    data: BottomPanelData,
    bld_data: BuildingInspectorData,
    world_data: Res<WorldData>,
    health_query: Query<(&NpcIndex, &Health, &CachedStats, &Energy), Without<Dead>>,
    npc_states: NpcStateQuery,
    gpu_state: Res<GpuReadState>,
    buffer_writes: Res<NpcBufferWrites>,
    mut follow: ResMut<FollowSelected>,
    mut settings: ResMut<UserSettings>,
    mut filter_state: Local<LogFilterState>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    // Init filter state from saved settings
    if !filter_state.initialized {
        filter_state.show_kills = settings.log_kills;
        filter_state.show_spawns = settings.log_spawns;
        filter_state.show_raids = settings.log_raids;
        filter_state.show_harvests = settings.log_harvests;
        filter_state.show_levelups = settings.log_levelups;
        filter_state.show_npc_activity = settings.log_npc_activity;
        filter_state.initialized = true;
    }

    let prev_filters = (
        filter_state.show_kills, filter_state.show_spawns, filter_state.show_raids,
        filter_state.show_harvests, filter_state.show_levelups, filter_state.show_npc_activity,
    );

    let mut copy_text: Option<String> = None;

    egui::TopBottomPanel::bottom("bottom_panel")
        .default_height(160.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                // LEFT: Inspector (fixed width)
                ui.vertical(|ui| {
                    ui.set_width(INSPECTOR_WIDTH);
                    inspector_content(
                        ui, &data, &bld_data, &world_data, &health_query, &npc_states,
                        &gpu_state, &buffer_writes, &mut follow, &settings, &mut copy_text,
                    );
                });

                ui.separator();

                // RIGHT: Combat log (fills remaining width)
                ui.vertical(|ui| {
                    // Filter checkboxes
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut filter_state.show_kills, "Deaths");
                        ui.checkbox(&mut filter_state.show_spawns, "Spawns");
                        ui.checkbox(&mut filter_state.show_raids, "Raids");
                        ui.checkbox(&mut filter_state.show_harvests, "Harvests");
                        ui.checkbox(&mut filter_state.show_levelups, "Levels");
                        ui.checkbox(&mut filter_state.show_npc_activity, "NPC");
                    });

                    ui.separator();

                    // Scrollable log — merge combat + NPC logs chronologically
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            // Collect visible entries: (sort_key, color, timestamp, message)
                            let mut merged: Vec<(i64, egui::Color32, String, &str)> = Vec::new();

                            for entry in &data.combat_log.entries {
                                let show = match entry.kind {
                                    CombatEventKind::Kill => filter_state.show_kills,
                                    CombatEventKind::Spawn => filter_state.show_spawns,
                                    CombatEventKind::Raid => filter_state.show_raids,
                                    CombatEventKind::Harvest => filter_state.show_harvests,
                                    CombatEventKind::LevelUp => filter_state.show_levelups,
                                };
                                if !show { continue; }

                                let color = match entry.kind {
                                    CombatEventKind::Kill => egui::Color32::from_rgb(220, 80, 80),
                                    CombatEventKind::Spawn => egui::Color32::from_rgb(80, 200, 80),
                                    CombatEventKind::Raid => egui::Color32::from_rgb(220, 160, 40),
                                    CombatEventKind::Harvest => egui::Color32::from_rgb(200, 200, 60),
                                    CombatEventKind::LevelUp => egui::Color32::from_rgb(80, 180, 255),
                                };

                                let key = (entry.day as i64) * 10000 + (entry.hour as i64) * 100 + entry.minute as i64;
                                let ts = format!("[D{} {:02}:{:02}]", entry.day, entry.hour, entry.minute);
                                merged.push((key, color, ts, &entry.message));
                            }

                            // Selected NPC activity log entries
                            if filter_state.show_npc_activity && data.selected.0 >= 0 {
                                let idx = data.selected.0 as usize;
                                if idx < data.npc_logs.0.len() {
                                    let npc_color = egui::Color32::from_rgb(180, 180, 220);
                                    for entry in data.npc_logs.0[idx].iter() {
                                        let key = (entry.day as i64) * 10000 + (entry.hour as i64) * 100 + entry.minute as i64;
                                        let ts = format!("[D{} {:02}:{:02}]", entry.day, entry.hour, entry.minute);
                                        merged.push((key, npc_color, ts, &entry.message));
                                    }
                                }
                            }

                            merged.sort_by_key(|(key, ..)| *key);

                            for (_, color, ts, msg) in &merged {
                                ui.horizontal(|ui| {
                                    ui.small(ts);
                                    ui.colored_label(*color, *msg);
                                });
                            }
                        });
                });
            });
        });

    // Persist filter changes
    let curr_filters = (
        filter_state.show_kills, filter_state.show_spawns, filter_state.show_raids,
        filter_state.show_harvests, filter_state.show_levelups, filter_state.show_npc_activity,
    );
    if curr_filters != prev_filters {
        settings.log_kills = filter_state.show_kills;
        settings.log_spawns = filter_state.show_spawns;
        settings.log_raids = filter_state.show_raids;
        settings.log_harvests = filter_state.show_harvests;
        settings.log_levelups = filter_state.show_levelups;
        settings.log_npc_activity = filter_state.show_npc_activity;
        settings::save_settings(&settings);
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

/// Render inspector content into a ui region (left side of bottom panel).
fn inspector_content(
    ui: &mut egui::Ui,
    data: &BottomPanelData,
    bld_data: &BuildingInspectorData,
    world_data: &WorldData,
    health_query: &Query<(&NpcIndex, &Health, &CachedStats, &Energy), Without<Dead>>,
    npc_states: &NpcStateQuery,
    gpu_state: &GpuReadState,
    buffer_writes: &NpcBufferWrites,
    follow: &mut FollowSelected,
    settings: &UserSettings,
    copy_text: &mut Option<String>,
) {
    let sel = data.selected.0;
    if sel < 0 {
        if bld_data.selected_building.active {
            building_inspector_content(ui, bld_data, world_data, &data.meta_cache);
            return;
        }
        ui.label("Click an NPC or building to inspect");
        return;
    }
    let idx = sel as usize;
    if idx >= data.meta_cache.0.len() { return; }

    let meta = &data.meta_cache.0[idx];

    ui.strong(format!("{}", meta.name));
    let next_level_xp = (meta.level + 1) * (meta.level + 1) * 100;
    ui.label(format!("{} Lv.{}  XP: {}/{}", crate::job_name(meta.job), meta.level, meta.xp, next_level_xp));

    let trait_str = crate::trait_name(meta.trait_id);
    if !trait_str.is_empty() {
        ui.label(format!("Trait: {}", trait_str));
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
        ui.label("EN:");
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

    if let Some((_, home, faction, town_id, activity, combat, at_dest, starving, healing))
        = npc_states.states.iter().find(|(ni, ..)| ni.0 == idx)
    {
        home_str = format!("({:.0}, {:.0})", home.0.x, home.0.y);
        faction_str = format!("{} (town {})", faction.0, town_id.0);

        let mut parts: Vec<&str> = Vec::new();
        let combat_name = combat.name();
        if !combat_name.is_empty() { parts.push(combat_name); }
        parts.push(activity.name());
        if at_dest.is_some() { parts.push("AtDest"); }
        if starving.is_some() { parts.push("Starving"); }
        if healing.is_some() { parts.push("Healing"); }
        state_str = parts.join(", ");
    }

    ui.label(format!("State: {}", state_str));

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
        Building::Hut { .. } => "Hut",
        Building::Barracks { .. } => "Barracks",
    }
}

fn building_town_idx(building: &Building) -> u32 {
    match building {
        Building::Fountain { town_idx }
        | Building::Farm { town_idx }
        | Building::Bed { town_idx }
        | Building::GuardPost { town_idx, .. }
        | Building::Camp { town_idx }
        | Building::Hut { town_idx }
        | Building::Barracks { town_idx } => *town_idx,
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

        Building::Hut { .. } | Building::Barracks { .. } => {
            let is_hut = matches!(building, Building::Hut { .. });
            let kind = if is_hut { 0 } else { 1 };
            let world_pos = bld.grid.grid_to_world(col, row);

            // Find matching spawner entry
            if let Some(entry) = bld.spawner_state.0.iter().find(|e| {
                e.building_kind == kind
                    && (e.position - world_pos).length() < 1.0
                    && e.position.x > -9000.0
            }) {
                ui.label(format!("Spawns: {}", if is_hut { "Farmer" } else { "Guard" }));

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
    }
}

// ============================================================================
// TARGET OVERLAY
// ============================================================================

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
    let line_color = egui::Color32::from_rgba_unmultiplied(255, 220, 50, 140);
    painter.line_segment([npc_screen, tgt_screen], egui::Stroke::new(1.5, line_color));

    // Diamond marker at target
    let s = 5.0;
    let diamond = [
        egui::Pos2::new(tgt_screen.x, tgt_screen.y - s),
        egui::Pos2::new(tgt_screen.x + s, tgt_screen.y),
        egui::Pos2::new(tgt_screen.x, tgt_screen.y + s),
        egui::Pos2::new(tgt_screen.x - s, tgt_screen.y),
    ];
    let fill = egui::Color32::from_rgba_unmultiplied(255, 220, 50, 200);
    painter.add(egui::Shape::convex_polygon(diamond.to_vec(), fill, egui::Stroke::NONE));

    // Small circle highlight on NPC
    let npc_color = egui::Color32::from_rgba_unmultiplied(100, 200, 255, 160);
    painter.circle_stroke(npc_screen, 8.0, egui::Stroke::new(1.5, npc_color));

    Ok(())
}

// ============================================================================
// FPS DISPLAY
// ============================================================================

/// Always-visible FPS counter at bottom-right. Smoothed with exponential moving average.
pub fn fps_display_system(
    mut contexts: EguiContexts,
    time: Res<Time>,
    mut avg_fps: Local<f32>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let dt = time.delta_secs();
    if dt > 0.0 {
        let fps = 1.0 / dt;
        // EMA smoothing: 5% weight on new sample
        *avg_fps = if *avg_fps == 0.0 { fps } else { *avg_fps * 0.95 + fps * 0.05 };
    }

    egui::Area::new(egui::Id::new("fps_display"))
        .anchor(egui::Align2::RIGHT_BOTTOM, [-8.0, -8.0])
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(format!("FPS: {:.0}", *avg_fps))
                .size(14.0)
                .color(egui::Color32::from_rgba_unmultiplied(200, 200, 200, 180)));
        });

    Ok(())
}
