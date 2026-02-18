//! Left panel — tabbed container for Roster, Upgrades, Policies, and Patrols.

use std::collections::HashMap;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;

use crate::constants::{BUILDING_REGISTRY, DisplayCategory, FOUNTAIN_TOWER, npc_def};
use crate::components::*;
use crate::resources::*;
use crate::settings::{self, UserSettings};
use crate::systems::stats::{CombatConfig, TownUpgrades, UpgradeQueue, UPGRADES, upgrade_count, upgrade_unlocked, upgrade_available, missing_prereqs, format_upgrade_cost, upgrade_effect_summary, branch_total, resolve_town_tower_stats};
use crate::constants::UpgradeStatKind;
use crate::systems::{AiPlayerState, AiKind};
use crate::systems::ai_player::{AiPersonality, cheapest_gold_upgrade_cost};
use crate::world::{WorldData, BuildingKind, is_alive};

// ============================================================================
// PROFILER PARAMS
// ============================================================================

#[derive(SystemParam)]
pub struct ProfilerParams<'w> {
    timings: Res<'w, SystemTimings>,
    migration: ResMut<'w, MigrationState>,
    spawner_state: Res<'w, SpawnerState>,
    mining_policy: ResMut<'w, MiningPolicy>,
}

// ============================================================================
// ROSTER TYPES
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn { Name, Job, Level, Hp, State, Trait }

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


// ============================================================================
// POLICIES CONSTANTS
// ============================================================================

const SCHEDULE_OPTIONS: &[&str] = &["Both Shifts", "Day Only", "Night Only"];
const OFF_DUTY_OPTIONS: &[&str] = &["Go to Bed", "Stay at Fountain", "Wander Town"];

// ============================================================================
// SYSTEM PARAM BUNDLES
// ============================================================================

#[derive(SystemParam)]
pub struct RosterParams<'w, 's> {
    selected: ResMut<'w, SelectedNpc>,
    meta_cache: ResMut<'w, NpcMetaCache>,
    health_query: Query<'w, 's, (
        &'static NpcIndex,
        &'static Health,
        &'static CachedStats,
        &'static Personality,
        &'static Activity,
        &'static CombatState,
        &'static Faction,
    ), Without<Dead>>,
    camera_query: Query<'w, 's, &'static mut Transform, With<crate::render::MainCamera>>,
    gpu_state: Res<'w, GpuReadState>,
}

#[derive(SystemParam)]
pub struct UpgradeParams<'w> {
    food_storage: Res<'w, FoodStorage>,
    gold_storage: Res<'w, GoldStorage>,
    faction_stats: Res<'w, FactionStats>,
    upgrades: Res<'w, TownUpgrades>,
    queue: ResMut<'w, UpgradeQueue>,
    auto: ResMut<'w, AutoUpgrade>,
}

// ============================================================================
// SQUAD TYPES
// ============================================================================

#[derive(SystemParam)]
pub struct SquadParams<'w, 's> {
    squad_state: ResMut<'w, SquadState>,
    gpu_state: Res<'w, GpuReadState>,
    // Query: military units with SquadId (for dismiss/recruit)
    squad_guards: Query<'w, 's, (Entity, &'static NpcIndex, &'static SquadId, &'static Job), (With<SquadUnit>, Without<Dead>)>,
}

// ============================================================================
// INTEL TYPES
// ============================================================================

#[derive(SystemParam)]
pub struct FactionsParams<'w> {
    ai_state: Res<'w, AiPlayerState>,
    food_storage: Res<'w, FoodStorage>,
    gold_storage: Res<'w, GoldStorage>,
    spawner_state: Res<'w, SpawnerState>,
    faction_stats: Res<'w, FactionStats>,
    upgrades: Res<'w, TownUpgrades>,
    combat_config: Res<'w, CombatConfig>,
}

#[derive(Clone)]
struct SquadSnapshot {
    squad_idx: usize,
    members: usize,
    target_size: usize,
    patrol_enabled: bool,
    rest_when_tired: bool,
    target: Option<Vec2>,
    commander_kind: Option<crate::world::BuildingKind>,
    commander_index: Option<usize>,
    commander_cooldown: Option<f32>,
}

#[derive(Clone)]
struct AiSnapshot {
    faction: i32,
    town_name: String,
    kind_name: &'static str,
    personality_name: &'static str,
    food: i32,
    gold: i32,
    npcs: std::collections::HashMap<crate::world::BuildingKind, usize>,
    buildings: std::collections::HashMap<crate::world::BuildingKind, usize>,
    alive: i32,
    dead: i32,
    kills: i32,
    upgrades: Vec<u8>,
    last_actions: Vec<String>,
    mining_radius: f32,
    mines_in_radius: usize,
    mines_discovered: usize,
    mines_enabled: usize,
    reserve_food: i32,
    food_desire: Option<f32>,
    military_desire: Option<f32>,
    gold_desire: Option<f32>,
    food_desire_tip: String,
    military_desire_tip: String,
    gold_desire_tip: String,
    center: Vec2,
    squads: Vec<SquadSnapshot>,
    next_upgrade: Option<NextUpgradeSnapshot>,
}

#[derive(Clone)]
struct NextUpgradeSnapshot {
    label: String,
    cost: String,
    affordable: bool,
}

#[derive(Default)]
pub struct FactionsCache {
    frame_counter: u32,
    snapshots: Vec<AiSnapshot>,
    selected_idx: usize,
}

// ============================================================================
// MAIN SYSTEM
// ============================================================================

pub fn left_panel_system(
    mut contexts: bevy_egui::EguiContexts,
    mut ui_state: ResMut<UiState>,
    world_data: Res<WorldData>,
    mut policies: ResMut<TownPolicies>,
    mut roster: RosterParams,
    mut upgrade: UpgradeParams,
    mut squad: SquadParams,
    factions: FactionsParams,
    mut profiler: ProfilerParams,
    mut commands: Commands,
    mut roster_state: Local<RosterState>,
    mut factions_cache: Local<FactionsCache>,
    mut settings: ResMut<UserSettings>,
    catalog: Res<HelpCatalog>,
    mut prev_tab: Local<LeftPanelTab>,
    mut dirty: ResMut<DirtyFlags>,
) -> Result {
    let _t = profiler.timings.scope("ui_left_panel");
    if !ui_state.left_panel_open {
        ui_state.factions_overlay_faction = None;
        *prev_tab = LeftPanelTab::Roster;
        return Ok(());
    }
    if ui_state.left_panel_tab != LeftPanelTab::Factions {
        ui_state.factions_overlay_faction = None;
    }

    let ctx = contexts.ctx_mut()?;
    let debug_all = settings.debug_all_npcs;
    let help_text_size = settings.help_text_size;

    let tab_name = match ui_state.left_panel_tab {
        LeftPanelTab::Roster => "Roster",
        LeftPanelTab::Upgrades => "Upgrades",
        LeftPanelTab::Policies => "Policies",
        LeftPanelTab::Patrols => "Patrols",
        LeftPanelTab::Squads => "Squads",
        LeftPanelTab::Factions => "Factions",
        LeftPanelTab::Profiler => "Profiler",
        LeftPanelTab::Help => "Help",
    };

    // Look up the help key for the current tab
    let tab_help_key = match ui_state.left_panel_tab {
        LeftPanelTab::Roster => "tab_roster",
        LeftPanelTab::Upgrades => "tab_upgrades",
        LeftPanelTab::Policies => "tab_policies",
        LeftPanelTab::Patrols => "tab_patrols",
        LeftPanelTab::Squads => "tab_squads",
        LeftPanelTab::Factions => "tab_factions",
        LeftPanelTab::Profiler => "tab_profiler",
        LeftPanelTab::Help => "tab_help",
    };

    let mut open = ui_state.left_panel_open;
    let mut jump_target: Option<Vec2> = None;
    let mut patrol_swap: Option<(usize, usize)> = None;
    egui::Window::new(tab_name)
        .open(&mut open)
        .resizable(false)
        .default_width(340.0)
        .anchor(egui::Align2::LEFT_TOP, [4.0, 30.0])
        .show(ctx, |ui| {
            // Inline help text at the top of every tab
            if let Some(tip) = catalog.0.get(tab_help_key) {
                ui.label(egui::RichText::new(*tip).size(help_text_size));
                ui.separator();
            }

            match ui_state.left_panel_tab {
                LeftPanelTab::Roster => roster_content(ui, &mut roster, &mut roster_state, debug_all),
                LeftPanelTab::Upgrades => upgrade_content(ui, &mut upgrade, &world_data, &mut settings),
                LeftPanelTab::Policies => policies_content(ui, &mut policies, &world_data, &profiler.spawner_state, &mut profiler.mining_policy, &mut dirty, &mut jump_target),
                LeftPanelTab::Patrols => { patrol_swap = patrols_content(ui, &world_data, &mut jump_target); },
                LeftPanelTab::Squads => squads_content(ui, &mut squad, &roster.meta_cache, &world_data, &mut commands, &mut dirty),
                LeftPanelTab::Factions => factions_content(ui, &factions, &squad.squad_state, &world_data, &policies, &profiler.mining_policy, &mut factions_cache, &mut jump_target, &mut ui_state),
                LeftPanelTab::Profiler => profiler_content(ui, &profiler.timings, &mut profiler.migration, &mut settings),
                LeftPanelTab::Help => help_content(ui),
            }
        });

    // Queue patrol swap — applied in rebuild_patrol_routes_system which has ResMut<WorldData>
    if let Some((a, b)) = patrol_swap {
        dirty.patrol_swap = Some((a, b));
        dirty.patrols = true;
    }

    // Apply camera jump from Factions panel
    if let Some(target) = jump_target {
        if let Ok(mut transform) = roster.camera_query.single_mut() {
            transform.translation.x = target.x;
            transform.translation.y = target.y;
        }
    }

    if !open {
        ui_state.left_panel_open = false;
    }

    // Save policies when leaving Policies tab or closing panel
    let was_policies = *prev_tab == LeftPanelTab::Policies;
    let is_policies = ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Policies;
    if was_policies && !is_policies {
        let town_idx = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);
        if town_idx < policies.policies.len() {
            let mut saved = settings::load_settings();
            saved.policy = policies.policies[town_idx].clone();
            settings::save_settings(&saved);
        }
    }
    *prev_tab = if ui_state.left_panel_open { ui_state.left_panel_tab } else { LeftPanelTab::Roster };

    Ok(())
}

// ============================================================================
// ROSTER CONTENT
// ============================================================================

fn roster_content(
    ui: &mut egui::Ui,
    roster: &mut RosterParams,
    state: &mut RosterState,
    debug_all: bool,
) {
    // Rebuild cache every 30 frames
    state.frame_counter += 1;
    if state.frame_counter % 30 == 1 || state.cached_rows.is_empty() {
        let mut rows = Vec::new();
        for (npc_idx, health, cached, personality, activity, combat, faction) in roster.health_query.iter() {
            let idx = npc_idx.0;
            let meta = &roster.meta_cache.0[idx];
            // Player faction only unless debug
            if !debug_all && faction.0 != 0 { continue; }
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
                trait_name: personality.trait_summary(),
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
                if state.sort_descending { ord.reverse() } else { ord }
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
                if def.is_military != military_first { continue; }
                if def.job == Job::Raider && !debug_all { continue; }
                let job_id = def.job as i32;
                if ui.selectable_label(state.job_filter == job_id, def.label_plural).clicked() {
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
        if idx < roster.meta_cache.0.len() {
            if state.rename_slot != selected_idx {
                state.rename_slot = selected_idx;
                state.rename_text = roster.meta_cache.0[idx].name.clone();
            }

            ui.horizontal(|ui| {
                ui.label("Rename:");
                let edit = ui.text_edit_singleline(&mut state.rename_text);
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button("Apply").clicked() || enter) && !state.rename_text.trim().is_empty() {
                    let new_name = state.rename_text.trim().to_string();
                    roster.meta_cache.0[idx].name = new_name.clone();
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
            if state.sort_descending { " \u{25BC}" } else { " \u{25B2}" }
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
        let mut new_selected: Option<i32> = None;
        let mut follow_idx: Option<usize> = None;

        for row in &state.cached_rows {
            let is_selected = selected_idx == row.slot as i32;
            let (r, g, b) = npc_def(Job::from_i32(row.job)).ui_color;
            let job_color = egui::Color32::from_rgb(r, g, b);

            let response = ui.horizontal(|ui| {
                if is_selected {
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgba_premultiplied(60, 60, 100, 80));
                }

                let name_text = if row.name.len() > 16 { &row.name[..16] } else { &row.name };
                ui.colored_label(job_color, name_text);
                ui.label(crate::job_name(row.job));
                ui.label(format!("{}", row.level));

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

// ============================================================================
// UPGRADE CONTENT
// ============================================================================

fn upgrade_content(ui: &mut egui::Ui, upgrade: &mut UpgradeParams, world_data: &WorldData, settings: &mut UserSettings) {
    let town_idx = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);
    let food = upgrade.food_storage.food.get(town_idx).copied().unwrap_or(0);
    let gold = upgrade.gold_storage.gold.get(town_idx).copied().unwrap_or(0);
    let villager_stats = upgrade.faction_stats.stats.first();
    let alive = villager_stats.map(|s| s.alive).unwrap_or(0);
    let levels = upgrade.upgrades.town_levels(town_idx);

    // Header: resources + town name
    ui.horizontal(|ui| {
        ui.label(format!("Food: {}", food));
        ui.separator();
        ui.label(format!("Gold: {}", gold));
        ui.separator();
        ui.label(format!("Villagers: {}", alive));
    });
    if let Some(town) = world_data.towns.get(town_idx) {
        ui.small(format!("Town: {}", town.name));
    }

    // Branch totals + overall total
    let reg = &*UPGRADES;
    let total: u32 = levels.iter().map(|&l| l as u32).sum();
    ui.horizontal(|ui| {
        for branch in &reg.branches {
            let bt = branch_total(&levels, branch.label);
            ui.label(egui::RichText::new(format!("{}: {}", branch.label, bt)).small());
        }
        ui.label(egui::RichText::new(format!("Total: {}", total)).small().strong());
    });
    ui.separator();

    // Tree-ordered upgrade list grouped by section (driven by dynamic registry)
    for section_name in ["Economy", "Military"] {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(section_name).strong().size(16.0));
        ui.separator();

        for branch in reg.branches.iter().filter(|b| b.section == section_name) {
            let bt = branch_total(&levels, branch.label);
            let is_expanded = settings.upgrade_expanded.iter().any(|s| s == branch.label);
            let id = ui.make_persistent_id(format!("upg_{}", branch.label));
            let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, is_expanded);
            let header_res = state.show_header(ui, |ui| {
                ui.label(egui::RichText::new(format!("{} ({})", branch.label, bt)).strong());
            });
            header_res.body(|ui| {
                    for &(i, depth) in &branch.entries {
                        let upg = &reg.nodes[i];
                        let unlocked = upgrade_unlocked(&levels, i);
                        let lv_i = levels.get(i).copied().unwrap_or(0);
                        let available = upgrade_available(&levels, i, food, gold);
                        let indent = depth as f32 * 16.0;

                        ui.horizontal(|ui| {
                            ui.add_space(indent);

                            // Auto-upgrade checkbox
                            upgrade.auto.ensure_towns(town_idx + 1);
                            let count = upgrade_count();
                            upgrade.auto.flags[town_idx].resize(count, false);
                            let auto_flag = &mut upgrade.auto.flags[town_idx][i];
                            let prev_auto = *auto_flag;
                            ui.add_enabled(unlocked, egui::Checkbox::new(auto_flag, ""))
                                .on_hover_text("Auto-buy each game hour");
                            if *auto_flag != prev_auto {
                                let mut saved = settings::load_settings();
                                saved.auto_upgrades = upgrade.auto.flags[town_idx].clone();
                                settings::save_settings(&saved);
                            }

                            // Label (dimmed when locked)
                            let label_text = egui::RichText::new(upg.label);
                            ui.label(if unlocked { label_text } else { label_text.weak() });

                            // Effect summary (now/next)
                            let (now, next) = upgrade_effect_summary(i, lv_i);
                            ui.label(egui::RichText::new(format!("{} -> {}", now, next)).small().weak());

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let cost_text = format_upgrade_cost(i, lv_i);
                                let response = ui.add_enabled(available, egui::Button::new(&cost_text));

                                let response = if !unlocked {
                                    if let Some(msg) = missing_prereqs(&levels, i) {
                                        response.on_hover_text(msg)
                                    } else {
                                        response
                                    }
                                } else {
                                    response.on_hover_text(upg.tooltip)
                                };
                                if response.clicked() {
                                    upgrade.queue.0.push((town_idx, i));
                                }

                                ui.label(format!("Lv{}", lv_i));
                            });
                        });
                    }
                });
            // Persist expand/collapse changes after body renders (borrow on ui released)
            let now_open = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false).is_open();
            if now_open != is_expanded {
                if now_open {
                    settings.upgrade_expanded.push(branch.label.to_string());
                } else {
                    settings.upgrade_expanded.retain(|s| s != branch.label);
                }
                settings::save_settings(settings);
            }
        }
    }
}

// ============================================================================
// POLICIES CONTENT
// ============================================================================

fn policies_content(
    ui: &mut egui::Ui,
    policies: &mut TownPolicies,
    world_data: &WorldData,
    spawner_state: &SpawnerState,
    mining_policy: &mut MiningPolicy,
    dirty: &mut DirtyFlags,
    jump_target: &mut Option<Vec2>,
) {
    let town_idx = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);

    if town_idx >= policies.policies.len() {
        policies.policies.resize(town_idx + 1, PolicySet::default());
    }
    let policy = &mut policies.policies[town_idx];

    if let Some(town) = world_data.towns.get(town_idx) {
        ui.small(format!("Town: {}", town.name));
        ui.separator();
    }

    // -- General --
    ui.label(egui::RichText::new("General").strong());
    ui.checkbox(&mut policy.eat_food, "Eat Food")
        .on_hover_text("NPCs consume food to restore HP and energy");
    ui.checkbox(&mut policy.prioritize_healing, "Prioritize Healing")
        .on_hover_text("Wounded NPCs go to fountain before resuming work");
    let mut recovery_pct = policy.recovery_hp * 100.0;
    ui.horizontal(|ui| {
        ui.label("Recovery HP:");
        ui.add(egui::Slider::new(&mut recovery_pct, 0.0..=100.0).suffix("%"));
    });
    policy.recovery_hp = recovery_pct / 100.0;

    // -- Archers --
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Archers").strong());
    ui.checkbox(&mut policy.archer_aggressive, "Aggressive")
        .on_hover_text("Archers never flee combat");
    ui.checkbox(&mut policy.archer_leash, "Leash")
        .on_hover_text("Archers return home if too far from post");
    let mut archer_flee_pct = policy.archer_flee_hp * 100.0;
    ui.horizontal(|ui| {
        ui.label("Flee HP:");
        ui.add(egui::Slider::new(&mut archer_flee_pct, 0.0..=100.0).suffix("%"));
    });
    policy.archer_flee_hp = archer_flee_pct / 100.0;
    let mut archer_sched_idx = policy.archer_schedule as usize;
    ui.horizontal(|ui| {
        ui.label("Schedule:");
        egui::ComboBox::from_id_salt("archer_schedule")
            .selected_text(SCHEDULE_OPTIONS[archer_sched_idx])
            .show_index(ui, &mut archer_sched_idx, SCHEDULE_OPTIONS.len(), |i| SCHEDULE_OPTIONS[i]);
    });
    policy.archer_schedule = match archer_sched_idx {
        1 => WorkSchedule::DayOnly,
        2 => WorkSchedule::NightOnly,
        _ => WorkSchedule::Both,
    };
    let mut archer_off_idx = policy.archer_off_duty as usize;
    ui.horizontal(|ui| {
        ui.label("Off-duty:");
        egui::ComboBox::from_id_salt("archer_off_duty")
            .selected_text(OFF_DUTY_OPTIONS[archer_off_idx])
            .show_index(ui, &mut archer_off_idx, OFF_DUTY_OPTIONS.len(), |i| OFF_DUTY_OPTIONS[i]);
    });
    policy.archer_off_duty = match archer_off_idx {
        1 => OffDutyBehavior::StayAtFountain,
        2 => OffDutyBehavior::WanderTown,
        _ => OffDutyBehavior::GoToBed,
    };

    // -- Farmers --
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Farmers").strong());
    ui.checkbox(&mut policy.farmer_fight_back, "Fight Back")
        .on_hover_text("Farmers attack enemies instead of fleeing");
    let mut farmer_flee_pct = policy.farmer_flee_hp * 100.0;
    ui.horizontal(|ui| {
        ui.label("Flee HP:");
        ui.add(egui::Slider::new(&mut farmer_flee_pct, 0.0..=100.0).suffix("%"));
    });
    policy.farmer_flee_hp = farmer_flee_pct / 100.0;
    let mut farmer_sched_idx = policy.farmer_schedule as usize;
    ui.horizontal(|ui| {
        ui.label("Schedule:");
        egui::ComboBox::from_id_salt("farmer_schedule")
            .selected_text(SCHEDULE_OPTIONS[farmer_sched_idx])
            .show_index(ui, &mut farmer_sched_idx, SCHEDULE_OPTIONS.len(), |i| SCHEDULE_OPTIONS[i]);
    });
    policy.farmer_schedule = match farmer_sched_idx {
        1 => WorkSchedule::DayOnly,
        2 => WorkSchedule::NightOnly,
        _ => WorkSchedule::Both,
    };
    let mut farmer_off_idx = policy.farmer_off_duty as usize;
    ui.horizontal(|ui| {
        ui.label("Off-duty:");
        egui::ComboBox::from_id_salt("farmer_off_duty")
            .selected_text(OFF_DUTY_OPTIONS[farmer_off_idx])
            .show_index(ui, &mut farmer_off_idx, OFF_DUTY_OPTIONS.len(), |i| OFF_DUTY_OPTIONS[i]);
    });
    policy.farmer_off_duty = match farmer_off_idx {
        1 => OffDutyBehavior::StayAtFountain,
        2 => OffDutyBehavior::WanderTown,
        _ => OffDutyBehavior::GoToBed,
    };

    // -- Mining --
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Mining").strong());

    let mut mining_radius = policy.mining_radius;
    let slider = egui::Slider::new(&mut mining_radius, 0.0..=5000.0)
        .step_by(100.0)
        .suffix(" px");
    if ui.add(slider).changed() {
        policy.mining_radius = mining_radius;
        dirty.mining = true;
    }

    if mining_policy.discovered_mines.len() <= town_idx {
        mining_policy.discovered_mines.resize(town_idx + 1, Vec::new());
    }
    if mining_policy.mine_enabled.len() < world_data.gold_mines().len() {
        mining_policy.mine_enabled.resize(world_data.gold_mines().len(), true);
    }

    let discovered = mining_policy.discovered_mines[town_idx].clone();
    let mut enabled_count = 0usize;
    for &mine_idx in &discovered {
        if mine_idx < mining_policy.mine_enabled.len() && mining_policy.mine_enabled[mine_idx] {
            enabled_count += 1;
        }
    }

    let mut assigned_per_mine: Vec<usize> = vec![0; world_data.gold_mines().len()];
    spawner_state.0.iter()
        .filter(|e| e.building_kind == crate::constants::tileset_index(BuildingKind::MinerHome) as i32 && e.town_idx == town_idx as i32 && e.npc_slot >= 0 && is_alive(e.position))
        .filter_map(|e| world_data.miner_home_at(e.position))
        .filter(|&mh_idx| {
            world_data.miner_homes().get(mh_idx)
                .map(|m| !m.manual_mine && m.assigned_mine.is_some())
                .unwrap_or(false)
        })
        .for_each(|mh_idx| {
            let Some(mine_pos) = world_data.miner_homes().get(mh_idx).and_then(|m| m.assigned_mine) else { return; };
            let Some(mine_idx) = world_data.gold_mine_at(mine_pos) else { return; };
            if let Some(c) = assigned_per_mine.get_mut(mine_idx) {
                *c += 1;
            }
        });
    let assigned_auto: usize = assigned_per_mine.iter().sum();

    ui.label(format!("{}/{} mines enabled, {} miners assigned", enabled_count, discovered.len(), assigned_auto));

    if discovered.is_empty() {
        ui.small("No discovered mines in radius.");
    } else {
        for &mine_idx in &discovered {
            let Some(mine) = world_data.gold_mines().get(mine_idx) else { continue };
            let dist = mine.position.distance(world_data.towns[town_idx].center);
            let mut enabled = mining_policy.mine_enabled.get(mine_idx).copied().unwrap_or(true);
            let mine_name = crate::ui::gold_mine_name(mine_idx);
            let assigned_here = assigned_per_mine.get(mine_idx).copied().unwrap_or(0);
            ui.horizontal(|ui| {
                if ui.checkbox(&mut enabled, "").changed() {
                    if mine_idx >= mining_policy.mine_enabled.len() {
                        mining_policy.mine_enabled.resize(mine_idx + 1, true);
                    }
                    mining_policy.mine_enabled[mine_idx] = enabled;
                    dirty.mining = true;
                }
                if ui.button(mine_name).on_hover_text("Jump to mine").clicked() {
                    *jump_target = Some(mine.position);
                }
                ui.small(format!("{:.0}px, {} assigned", dist, assigned_here));
            });
        }
    }
}

// ============================================================================
// PATROLS CONTENT
// ============================================================================

/// Returns swap indices if the user clicked a reorder button.
fn patrols_content(ui: &mut egui::Ui, world_data: &WorldData, jump_target: &mut Option<Vec2>) -> Option<(usize, usize)> {
    let town_pair_idx = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0) as u32;

    if let Some(town) = world_data.towns.get(town_pair_idx as usize) {
        ui.small(format!("Town: {}", town.name));
    }

    // Collect non-tombstoned posts for this town, sorted by patrol_order
    let mut posts: Vec<(usize, u32, Vec2)> = world_data.waypoints().iter().enumerate()
        .filter(|(_, p)| p.town_idx == town_pair_idx && is_alive(p.position))
        .map(|(i, p)| (i, p.patrol_order, p.position))
        .collect();
    posts.sort_by_key(|(_, order, _)| *order);

    ui.label(format!("{} waypoints", posts.len()));
    ui.separator();

    let mut swap: Option<(usize, usize)> = None;

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (list_idx, &(data_idx, order, pos)) in posts.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", order));
                if ui.button(format!("({:.0}, {:.0})", pos.x, pos.y)).on_hover_text("Jump to this post").clicked() {
                    *jump_target = Some(pos);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if list_idx + 1 < posts.len() {
                        if ui.small_button("Down").on_hover_text("Move down").clicked() {
                            swap = Some((data_idx, posts[list_idx + 1].0));
                        }
                    }
                    if list_idx > 0 {
                        if ui.small_button("Up").on_hover_text("Move up").clicked() {
                            swap = Some((data_idx, posts[list_idx - 1].0));
                        }
                    }
                });
            });
        }
    });

    swap
}

// ============================================================================
// SQUADS CONTENT
// ============================================================================

fn squads_content(ui: &mut egui::Ui, squad: &mut SquadParams, meta_cache: &NpcMetaCache, _world_data: &WorldData, commands: &mut Commands, dirty: &mut DirtyFlags) {
    let selected = squad.squad_state.selected;

    // Squad list (player-owned only — AI squads are hidden from UI)
    for i in 0..squad.squad_state.squads.len() {
        if !squad.squad_state.squads[i].is_player() { continue; }
        let count = squad.squad_state.squads[i].members.len();
        let has_target = squad.squad_state.squads[i].target.is_some();
        let patrol_on = squad.squad_state.squads[i].patrol_enabled;
        let rest_on = squad.squad_state.squads[i].rest_when_tired;
        let is_selected = selected == i as i32;

        let target_str = if has_target { "target set" } else { "---" };
        let patrol_str = if patrol_on { "patrol:on" } else { "patrol:off" };
        let rest_str = if rest_on { "rest:on" } else { "rest:off" };
        let squad_name = if i == 0 { "Default Squad" } else { "Squad" };
        let label = format!("{}. {} {}  [{}]  {}  {}  {}", i + 1, squad_name, i + 1, count, target_str, patrol_str, rest_str);

        if ui.selectable_label(is_selected, label).clicked() {
            squad.squad_state.selected = if is_selected { -1 } else { i as i32 };
        }
    }

    ui.separator();

    // Selected squad details
    if selected < 0 || selected as usize >= squad.squad_state.squads.len() {
        ui.label("Select a squad above");
        return;
    }
    let si = selected as usize;
    let member_count = squad.squad_state.squads[si].members.len();

    let header_name = if si == 0 { "Default Squad" } else { "Squad" };
    ui.strong(format!("{} {} — {} members", header_name, si + 1, member_count));

    // Target controls
    ui.horizontal(|ui| {
        if squad.squad_state.placing_target {
            ui.colored_label(egui::Color32::YELLOW, "Click map to set target...");
        } else {
            if ui.button("Set Target").clicked() {
                squad.squad_state.placing_target = true;
            }
        }
        if squad.squad_state.squads[si].target.is_some() {
            if ui.button("Clear Target").clicked() {
                squad.squad_state.squads[si].target = None;
            }
        }
    });

    if let Some(target) = squad.squad_state.squads[si].target {
        ui.small(format!("Target: ({:.0}, {:.0})", target.x, target.y));
    }

    let mut patrol_enabled = squad.squad_state.squads[si].patrol_enabled;
    if ui.checkbox(&mut patrol_enabled, "Patrol when no target").changed() {
        squad.squad_state.squads[si].patrol_enabled = patrol_enabled;
    }
    let mut rest_when_tired = squad.squad_state.squads[si].rest_when_tired;
    if ui.checkbox(&mut rest_when_tired, "Go home to rest when tired").changed() {
        squad.squad_state.squads[si].rest_when_tired = rest_when_tired;
    }
    let mut hold_fire = squad.squad_state.squads[si].hold_fire;
    if ui.checkbox(&mut hold_fire, "Hold fire (attack on command only)").changed() {
        squad.squad_state.squads[si].hold_fire = hold_fire;
    }

    // Show attack target if set
    match squad.squad_state.squads[si].attack_target {
        Some(crate::resources::AttackTarget::Npc(slot)) => {
            ui.small(format!("Attack target: NPC #{}", slot));
        }
        Some(crate::resources::AttackTarget::Building(pos)) => {
            ui.small(format!("Attack target: building ({:.0}, {:.0})", pos.x, pos.y));
        }
        None => {}
    }

    ui.add_space(4.0);

    // Per-job recruit controls — one row per military NPC type from registry
    for def in crate::constants::NPC_REGISTRY.iter() {
        if !def.is_military { continue; }
        if def.job == Job::Raider { continue; }
        let job_id = def.job as i32;
        // Available units of this job in default squad (squad 0)
        let available: Vec<usize> = squad.squad_state.squads[0].members.iter().copied()
            .filter(|&slot| slot < meta_cache.0.len() && meta_cache.0[slot].job == job_id)
            .collect();
        let avail_count = available.len();
        if avail_count == 0 && si == 0 { continue; }

        let (r, g, b) = def.ui_color;
        let label_color = egui::Color32::from_rgb(r, g, b);

        if si == 0 {
            ui.colored_label(label_color, format!("{}: {}", def.label_plural, avail_count));
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(label_color, format!("{}: {}", def.label_plural, avail_count));
                for amount in [1usize, 2, 4, 8, 16, 32] {
                    if amount > avail_count { break; }
                    if ui.small_button(format!("+{}", amount)).clicked() {
                        let recruits: Vec<usize> = available.iter().copied().take(amount).collect();
                        for &slot in &recruits {
                            for (entity, npc_idx, sid, _) in squad.squad_guards.iter() {
                                if sid.0 == 0 && npc_idx.0 == slot {
                                    commands.entity(entity).insert(SquadId(si as i32));
                                    break;
                                }
                            }
                        }
                        squad.squad_state.squads[0].members.retain(|s| !recruits.contains(s));
                        for slot in recruits {
                            if !squad.squad_state.squads[si].members.contains(&slot) {
                                squad.squad_state.squads[si].members.push(slot);
                            }
                        }
                        let selected_len = squad.squad_state.squads[si].members.len();
                        let selected_target = squad.squad_state.squads[si].target_size;
                        squad.squad_state.squads[si].target_size = selected_target.max(selected_len);
                        dirty.squads = true;
                    }
                }
            });
        }
    }

    // Dismiss all
    if member_count > 0 {
        if ui.button("Dismiss All").clicked() {
            for (entity, _, sid, _) in squad.squad_guards.iter() {
                if sid.0 == selected {
                    commands.entity(entity).remove::<SquadId>();
                }
            }
            squad.squad_state.squads[si].members.clear();
            squad.squad_state.squads[si].target_size = 0;
            dirty.squads = true;
        }
    }

    ui.separator();

    // Member list
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let members = &squad.squad_state.squads[si].members;
        for &slot in members {
            if slot >= meta_cache.0.len() { continue; }
            let meta = &meta_cache.0[slot];
            if meta.name.is_empty() { continue; }

            // Try to get HP from GPU readback
            let hp_str = if slot < squad.gpu_state.health.len() {
                format!("HP {:.0}", squad.gpu_state.health[slot])
            } else {
                String::new()
            };

            ui.horizontal(|ui| {
                let (r, g, b) = npc_def(Job::from_i32(meta.job)).ui_color;
                let job_color = egui::Color32::from_rgb(r, g, b);
                ui.colored_label(job_color, &meta.name);
                ui.label(Job::from_i32(meta.job).label());
                ui.label(format!("Lv.{}", meta.level));
                if !hp_str.is_empty() {
                    ui.label(hp_str);
                }
            });
        }
    });
}

// ============================================================================
// INTEL CONTENT
// ============================================================================


fn rebuild_factions_cache(
    factions: &FactionsParams,
    squad_state: &SquadState,
    world_data: &WorldData,
    policies: &TownPolicies,
    mining_policy: &MiningPolicy,
    cache: &mut FactionsCache,
) {
    fn push_snapshot(
        factions: &FactionsParams,
        squad_state: &SquadState,
        world_data: &WorldData,
        policies: &TownPolicies,
        mining_policy: &MiningPolicy,
        cache: &mut FactionsCache,
        tdi: usize,
        kind_name: &'static str,
        personality_name: &'static str,
        personality: Option<AiPersonality>,
        last_actions: Vec<String>,
    ) {
        let ti = tdi as u32;
        let town_name = world_data.towns.get(tdi)
            .map(|t| t.name.clone()).unwrap_or_default();
        let center = world_data.towns.get(tdi)
            .map(|t| t.center).unwrap_or_default();
        let faction = world_data.towns.get(tdi).map(|t| t.faction).unwrap_or(0);

        let buildings = world_data.building_counts(ti);

        let ti_i32 = tdi as i32;
        let npcs: std::collections::HashMap<BuildingKind, usize> = crate::constants::BUILDING_REGISTRY.iter()
            .filter(|def| def.spawner.is_some())
            .map(|def| {
                let ti = crate::constants::tileset_index(def.kind) as i32;
                let count = factions.spawner_state.0.iter()
                    .filter(|s| s.building_kind == ti && s.town_idx == ti_i32 && s.npc_slot >= 0 && is_alive(s.position)).count();
                (def.kind, count)
            })
            .collect();

        let food = factions.food_storage.food.get(tdi).copied().unwrap_or(0);
        let gold = factions.gold_storage.gold.get(tdi).copied().unwrap_or(0);
        let (alive, dead, kills) = factions.faction_stats.stats.get(faction as usize)
            .map(|s| (s.alive, s.dead, s.kills))
            .unwrap_or((0, 0, 0));
        let upgrades = factions.upgrades.town_levels(tdi);
        let next_upgrade = UPGRADES.nodes.iter().enumerate()
            .find_map(|(idx, node)| {
                let level = upgrades.get(idx).copied().unwrap_or(0);
                if level >= 20 || !upgrade_unlocked(&upgrades, idx) {
                    return None;
                }
                Some(NextUpgradeSnapshot {
                    label: node.label.to_string(),
                    cost: format_upgrade_cost(idx, level),
                    affordable: upgrade_available(&upgrades, idx, food, gold),
                })
            });

        let policy = policies.policies.get(tdi);
        let mining_radius = policy.map(|p| p.mining_radius).unwrap_or(crate::constants::DEFAULT_MINING_RADIUS);
        let mines_in_radius = world_data.gold_mines().iter()
            .filter(|m| is_alive(m.position))
            .filter(|m| (m.position - center).length_squared() <= mining_radius * mining_radius)
            .count();
        let discovered = mining_policy.discovered_mines.get(tdi);
        let mines_discovered = discovered.map(|v| v.len()).unwrap_or(0);
        let mines_enabled = discovered.map(|v| {
            v.iter()
                .filter(|&&mine_idx| mining_policy.mine_enabled.get(mine_idx).copied().unwrap_or(true))
                .count()
        }).unwrap_or(0);
        let spawner_count = factions.spawner_state.0.iter()
            .filter(|s| is_alive(s.position))
            .filter(|s| s.town_idx == tdi as i32)
            .filter(|s| s.is_population_spawner())
            .count() as i32;
        let reserve_food = personality
            .map(|p| p.food_reserve_per_spawner() * spawner_count)
            .unwrap_or(0);

        let farmer_homes = buildings.get(&BuildingKind::FarmerHome).copied().unwrap_or(0);
        let archer_homes = buildings.get(&BuildingKind::ArcherHome).copied().unwrap_or(0);
        let crossbow_homes = buildings.get(&BuildingKind::CrossbowHome).copied().unwrap_or(0);
        let military_homes = archer_homes + crossbow_homes;
        let waypoints = buildings.get(&BuildingKind::Waypoint).copied().unwrap_or(0);

        let (food_desire, military_desire, food_desire_tip, military_desire_tip) = if let Some(p) = personality {
            let food_desire = if reserve_food > 0 {
                (1.0 - (food - reserve_food) as f32 / reserve_food as f32).clamp(0.0, 1.0)
            } else if food < 5 {
                0.8
            } else if food < 10 {
                0.4
            } else {
                0.0
            };

            let food_tip = if reserve_food > 0 {
                format!(
                    "Food desire = clamp(1 - (food - reserve) / reserve, 0..1)\nfood = {food}, reserve = {reserve_food}\n=> {:.0}%",
                    food_desire * 100.0
                )
            } else {
                format!(
                    "Reserve <= 0 fallback:\nfood < 5 => 80%, food < 10 => 40%, else 0%\nfood = {food}\n=> {:.0}%",
                    food_desire * 100.0
                )
            };

            let barracks_target = p.archer_home_target(farmer_homes).max(1);
            let barracks_gap = barracks_target.saturating_sub(military_homes) as f32 / barracks_target as f32;
            let waypoint_gap = if military_homes > 0 {
                military_homes.saturating_sub(waypoints) as f32 / military_homes as f32
            } else {
                0.0
            };
            let military_desire = (barracks_gap * 0.75 + waypoint_gap * 0.25).clamp(0.0, 1.0);
            let military_tip = format!(
                "Military desire = clamp(barracks_gap*0.75 + waypoint_gap*0.25, 0..1)\n\
                 barracks_target = max(1, archer_home_target(farmer_homes)) = {barracks_target}\n\
                 farmer_homes = {farmer_homes}, military_homes = {military_homes}, waypoints = {waypoints}\n\
                 barracks_gap = {barracks_gap:.2}, waypoint_gap = {waypoint_gap:.2}\n\
                 => {:.0}%",
                military_desire * 100.0,
            );

            (Some(food_desire), Some(military_desire), food_tip, military_tip)
        } else {
            (
                None,
                None,
                "Not applicable: desire metrics are only computed for AI factions.".to_string(),
                "Not applicable: desire metrics are only computed for AI factions.".to_string(),
            )
        };

        // Gold desire: mirrors ai_player.rs logic.
        let (gold_desire, gold_desire_tip) = if let Some(p) = personality {
            let uw = p.upgrade_weights(crate::systems::ai_player::AiKind::Builder);
            let levels = factions.upgrades.town_levels(tdi);
            let cheapest = cheapest_gold_upgrade_cost(&uw, &levels, gold);
            let base = p.base_mining_desire();
            let gd = if cheapest > 0 {
                ((1.0 - gold as f32 / cheapest as f32) * p.gold_desire_mult()).clamp(0.0, 1.0)
            } else {
                base
            };
            let tip = format!(
                "Gold desire: cheapest_gold_upgrade={cheapest}, gold={gold}, base_mining={base:.1}\n=> {:.0}%",
                gd * 100.0
            );
            (Some(gd), tip)
        } else {
            (None, "Not applicable: desire metrics are only computed for AI factions.".to_string())
        };

        let ai_player = factions.ai_state.players.iter().find(|p| p.town_data_idx == tdi);
        let squads = squad_state.squads.iter().enumerate()
            .filter_map(|(si, squad)| {
                let owned = match squad.owner {
                    SquadOwner::Player => faction == 0,
                    SquadOwner::Town(owner_tdi) => owner_tdi == tdi,
                };
                if !owned || squad.members.is_empty() {
                    return None;
                }

                let (commander_kind, commander_index, commander_cooldown) = ai_player
                    .and_then(|p| p.squad_cmd.get(&si))
                    .map(|cmd| (cmd.target_kind, Some(cmd.target_index), Some(cmd.cooldown)))
                    .unwrap_or((None, None, None));

                Some(SquadSnapshot {
                    squad_idx: si,
                    members: squad.members.len(),
                    target_size: squad.target_size,
                    patrol_enabled: squad.patrol_enabled,
                    rest_when_tired: squad.rest_when_tired,
                    target: squad.target,
                    commander_kind,
                    commander_index,
                    commander_cooldown,
                })
            })
            .collect();

        cache.snapshots.push(AiSnapshot {
            faction,
            town_name,
            kind_name,
            personality_name,
            food,
            gold,
            npcs,
            buildings,
            alive,
            dead,
            kills,
            upgrades,
            last_actions,
            mining_radius,
            mines_in_radius,
            mines_discovered,
            mines_enabled,
            reserve_food,
            food_desire,
            military_desire,
            gold_desire,
            food_desire_tip,
            military_desire_tip,
            gold_desire_tip,
            center,
            squads,
            next_upgrade,
        });
    }

    cache.snapshots.clear();

    // Include player faction (faction 0) in Factions view.
    if let Some(player_tdi) = world_data.towns.iter().position(|t| t.faction == 0) {
        push_snapshot(factions, squad_state, world_data, policies, mining_policy, cache, player_tdi, "Player", "Human", None, Vec::new());
    }

    for player in factions.ai_state.players.iter() {
        let tdi = player.town_data_idx;

        let kind_name = match player.kind {
            AiKind::Builder => "Builder",
            AiKind::Raider => "Raider",
        };

        let last_actions: Vec<String> = player.last_actions.iter().rev().cloned().collect();
        push_snapshot(
            factions,
            squad_state,
            world_data,
            policies,
            mining_policy,
            cache,
            tdi,
            kind_name,
            player.personality.name(),
            Some(player.personality),
            last_actions,
        );
    }
}

fn factions_content(
    ui: &mut egui::Ui,
    factions: &FactionsParams,
    squad_state: &SquadState,
    world_data: &WorldData,
    policies: &TownPolicies,
    mining_policy: &MiningPolicy,
    cache: &mut FactionsCache,
    jump_target: &mut Option<Vec2>,
    ui_state: &mut UiState,
) {
    // Rebuild cache every 30 frames
    cache.frame_counter += 1;
    if cache.frame_counter % 30 == 1 || cache.snapshots.is_empty() {
        rebuild_factions_cache(factions, squad_state, world_data, policies, mining_policy, cache);
    }

    if cache.snapshots.is_empty() {
        ui.label("No AI settlements");
        return;
    }

    // Consume pending faction selection from double-click
    if let Some(faction) = ui_state.pending_faction_select.take() {
        if let Some(idx) = cache.snapshots.iter().position(|s| s.faction == faction) {
            cache.selected_idx = idx;
        }
    }

    if cache.selected_idx >= cache.snapshots.len() {
        cache.selected_idx = 0;
    }

    ui.horizontal(|ui| {
        ui.label("Faction:");
        egui::ComboBox::from_id_salt("intel_faction_select")
            .selected_text({
                let s = &cache.snapshots[cache.selected_idx];
                format!("F{} {} [{} {}]", s.faction, s.town_name, s.personality_name, s.kind_name)
            })
            .show_ui(ui, |ui| {
                for (i, s) in cache.snapshots.iter().enumerate() {
                    let label = format!("F{} {} [{} {}]", s.faction, s.town_name, s.personality_name, s.kind_name);
                    ui.selectable_value(&mut cache.selected_idx, i, label);
                }
            });
    });
    ui.separator();

    let snap = &cache.snapshots[cache.selected_idx];
    ui_state.factions_overlay_faction = Some(snap.faction);
    let kind_color = match snap.kind_name {
        "Builder" => egui::Color32::from_rgb(80, 180, 255),
        _ => egui::Color32::from_rgb(220, 80, 80),
    };
    ui.colored_label(kind_color, format!("F{} {} [{} {}]", snap.faction, snap.town_name, snap.personality_name, snap.kind_name));

    ui.horizontal(|ui| {
        if ui.small_button("Jump").clicked() {
            *jump_target = Some(snap.center);
        }
        ui.label(format!("Food: {}", snap.food));
        ui.separator();
        ui.label(format!("Gold: {}", snap.gold));
        ui.separator();
        let food_desire = snap.food_desire
            .map(|v| format!("{:.0}%", v * 100.0))
            .unwrap_or_else(|| "-".to_string());
        ui.label(format!("Food Desire: {}", food_desire))
            .on_hover_text(&snap.food_desire_tip);
        ui.separator();
        let military_desire = snap.military_desire
            .map(|v| format!("{:.0}%", v * 100.0))
            .unwrap_or_else(|| "-".to_string());
        ui.label(format!("Military Desire: {}", military_desire))
            .on_hover_text(&snap.military_desire_tip);
        ui.separator();
        let gold_desire = snap.gold_desire
            .map(|v| format!("{:.0}%", v * 100.0))
            .unwrap_or_else(|| "-".to_string());
        ui.label(format!("Gold Desire: {}", gold_desire))
            .on_hover_text(&snap.gold_desire_tip);
        ui.separator();
        if let Some(next) = &snap.next_upgrade {
            ui.label(format!("Next Upgrade Cost: {}", next.cost))
                .on_hover_text(format!("Next Upgrade: {}", next.label));
            let afford_color = if next.affordable {
                egui::Color32::from_rgb(80, 190, 120)
            } else {
                egui::Color32::from_rgb(210, 95, 95)
            };
            ui.colored_label(
                afford_color,
                if next.affordable { "Can Afford: Yes" } else { "Can Afford: No" },
            );
        } else {
            ui.label("Next Upgrade Cost: -");
            ui.label("Can Afford: N/A");
        }
    });
    ui.label(format!("Alive: {}  Dead: {}  Kills: {}", snap.alive, snap.dead, snap.kills));
    ui.separator();

    let lv = &snap.upgrades;
    let archer_base = factions.combat_config.jobs.get(&Job::Archer);
    let fighter_base = factions.combat_config.jobs.get(&Job::Fighter);
    let crossbow_base = factions.combat_config.jobs.get(&Job::Crossbow);
    let crossbow_atk = npc_def(Job::Crossbow).attack_override.as_ref();
    let farmer_base = factions.combat_config.jobs.get(&Job::Farmer);
    let miner_base = factions.combat_config.jobs.get(&Job::Miner);
    let ranged_base = factions.combat_config.attacks.get(&BaseAttackType::Ranged);
    let melee_base = factions.combat_config.attacks.get(&BaseAttackType::Melee);

    let archer_hp_mult = UPGRADES.stat_mult(lv, "Archer", UpgradeStatKind::Hp);
    let archer_dmg_mult = UPGRADES.stat_mult(lv, "Archer", UpgradeStatKind::Attack);
    let archer_range_mult = UPGRADES.stat_mult(lv, "Archer", UpgradeStatKind::Range);
    let archer_speed_mult = UPGRADES.stat_mult(lv, "Archer", UpgradeStatKind::MoveSpeed);
    let archer_cd_mult = 1.0 / UPGRADES.stat_mult(lv, "Archer", UpgradeStatKind::AttackSpeed);
    let archer_cd_reduction = (1.0 - archer_cd_mult) * 100.0;
    let archer_alert_mult = UPGRADES.stat_mult(lv, "Archer", UpgradeStatKind::Alert);

    let fighter_hp_mult = UPGRADES.stat_mult(lv, "Fighter", UpgradeStatKind::Hp);
    let fighter_dmg_mult = UPGRADES.stat_mult(lv, "Fighter", UpgradeStatKind::Attack);
    let fighter_speed_mult = UPGRADES.stat_mult(lv, "Fighter", UpgradeStatKind::MoveSpeed);
    let fighter_cd_mult = 1.0 / UPGRADES.stat_mult(lv, "Fighter", UpgradeStatKind::AttackSpeed);
    let fighter_cd_reduction = (1.0 - fighter_cd_mult) * 100.0;

    let xbow_hp_mult = UPGRADES.stat_mult(lv, "Crossbow", UpgradeStatKind::Hp);
    let xbow_dmg_mult = UPGRADES.stat_mult(lv, "Crossbow", UpgradeStatKind::Attack);
    let xbow_range_mult = UPGRADES.stat_mult(lv, "Crossbow", UpgradeStatKind::Range);
    let xbow_speed_mult = UPGRADES.stat_mult(lv, "Crossbow", UpgradeStatKind::MoveSpeed);
    let xbow_cd_mult = 1.0 / UPGRADES.stat_mult(lv, "Crossbow", UpgradeStatKind::AttackSpeed);

    let farmer_hp_mult = UPGRADES.stat_mult(lv, "Farmer", UpgradeStatKind::Hp);
    let farmer_speed_mult = UPGRADES.stat_mult(lv, "Farmer", UpgradeStatKind::MoveSpeed);
    let farm_yield_mult = UPGRADES.stat_mult(lv, "Farmer", UpgradeStatKind::Yield);

    let miner_hp_mult = UPGRADES.stat_mult(lv, "Miner", UpgradeStatKind::Hp);
    let miner_speed_mult = UPGRADES.stat_mult(lv, "Miner", UpgradeStatKind::MoveSpeed);
    let gold_yield_mult = UPGRADES.stat_mult(lv, "Miner", UpgradeStatKind::Yield);

    let healing_mult = UPGRADES.stat_mult(lv, "Town", UpgradeStatKind::Healing);
    let fountain_bonus = UPGRADES.stat_level(lv, "Town", UpgradeStatKind::FountainRange) as f32 * 24.0;
    let tower = resolve_town_tower_stats(lv);

    let npc = |k: BuildingKind| snap.npcs.get(&k).copied().unwrap_or(0);
    let bld = |k: BuildingKind| snap.buildings.get(&k).copied().unwrap_or(0);

    ui.columns(2, |columns| {
        let (left_slice, right_slice) = columns.split_at_mut(1);
        let left = &mut left_slice[0];
        let right = &mut right_slice[0];

        left.label("Economy");
        let econ_spawners: Vec<_> = BUILDING_REGISTRY.iter()
            .filter(|d| d.display == DisplayCategory::Economy && d.spawner.is_some())
            .collect();
        let workforce: usize = econ_spawners.iter().map(|d| npc(d.kind)).sum();
        let parts: Vec<String> = econ_spawners.iter()
            .map(|d| format!("{} {}", npc(d.kind), npc_def(Job::from_i32(d.spawner.unwrap().job)).label_plural))
            .collect();
        left.label(format!("Workforce: {} ({})", workforce, parts.join(" + ")));
        for def in &econ_spawners {
            let label = npc_def(Job::from_i32(def.spawner.unwrap().job)).label_plural;
            left.label(format!("{}: {}/{}", label, npc(def.kind), bld(def.kind)));
        }
        left.separator();

        left.label("Economy Buildings");
        for def in BUILDING_REGISTRY.iter().filter(|d| d.display == DisplayCategory::Economy) {
            left.label(format!("{}: {}", def.label, bld(def.kind)));
        }
        left.separator();

        left.label("Mining Policy");
        left.label(format!("Radius: {:.0}px", snap.mining_radius));
        left.label(format!("Reserve Food: {}", snap.reserve_food));
        left.label(format!("Mines in Radius: {}", snap.mines_in_radius));
        left.label(format!("Discovered: {}  Enabled: {}", snap.mines_discovered, snap.mines_enabled));
        left.separator();

        left.label("Economy Stats");
        egui::Grid::new(format!("intel_economy_stats_grid_{}_{}", snap.faction, cache.selected_idx))
            .num_columns(2)
            .striped(true)
            .show(left, |ui| {
                if let Some(base) = farmer_base {
                    ui.label("Farmer HP");
                    ui.label(format!("{:.0} -> {:.0}", base.max_health, base.max_health * farmer_hp_mult));
                    ui.end_row();

                    ui.label("Farmer Speed");
                    ui.label(format!("{:.0} -> {:.0}", base.speed, base.speed * farmer_speed_mult));
                    ui.end_row();
                }

                if let Some(base) = miner_base {
                    ui.label("Miner HP");
                    ui.label(format!("{:.0} -> {:.0}", base.max_health, base.max_health * miner_hp_mult));
                    ui.end_row();

                    ui.label("Miner Speed");
                    ui.label(format!("{:.0} -> {:.0}", base.speed, base.speed * miner_speed_mult));
                    ui.end_row();
                }

                ui.label("Food Yield");
                ui.label(format!("{:.0}% of base", farm_yield_mult * 100.0));
                ui.end_row();

                ui.label("Gold Yield");
                ui.label(format!("{:.0}% of base", gold_yield_mult * 100.0));
                ui.end_row();

                ui.label("Healing Rate");
                ui.label(format!("{:.1}/s -> {:.1}/s", factions.combat_config.heal_rate, factions.combat_config.heal_rate * healing_mult));
                ui.end_row();

                ui.label("Tower/Heal Radius");
                ui.label(format!("{:.0}px -> {:.0}px", factions.combat_config.heal_radius, factions.combat_config.heal_radius + fountain_bonus));
                ui.end_row();

                ui.label("Fountain Cooldown");
                ui.label(format!("{:.2}s -> {:.2}s", FOUNTAIN_TOWER.cooldown, tower.cooldown));
                ui.end_row();

                ui.label("Fountain Projectile Life");
                ui.label(format!("{:.2}s -> {:.2}s", FOUNTAIN_TOWER.proj_lifetime, tower.proj_lifetime));
                ui.end_row();

                ui.label("Build Area Expansion");
                ui.label(format!("+{}", UPGRADES.stat_level(lv, "Town", UpgradeStatKind::Expansion)));
                ui.end_row();
            });

        right.label("Military");
        let mil_spawners: Vec<_> = BUILDING_REGISTRY.iter()
            .filter(|d| d.display == DisplayCategory::Military && d.spawner.is_some())
            .collect();
        let total_mil: usize = mil_spawners.iter().map(|d| npc(d.kind)).sum();
        let parts: Vec<String> = mil_spawners.iter()
            .map(|d| format!("{} {}", npc(d.kind), npc_def(Job::from_i32(d.spawner.unwrap().job)).label_plural))
            .collect();
        right.label(format!("Force: {} ({})", total_mil, parts.join(" + ")));
        for def in &mil_spawners {
            let label = npc_def(Job::from_i32(def.spawner.unwrap().job)).label_plural;
            right.label(format!("{}: {}/{}", label, npc(def.kind), bld(def.kind)));
        }
        right.separator();

        right.label("Military Buildings");
        for def in BUILDING_REGISTRY.iter().filter(|d| d.display == DisplayCategory::Military) {
            right.label(format!("{}: {}", def.label, bld(def.kind)));
        }
        right.separator();

        right.label("Military Stats");
        egui::Grid::new(format!("intel_military_stats_grid_{}_{}", snap.faction, cache.selected_idx))
            .num_columns(2)
            .striped(true)
            .show(right, |ui| {
                if let Some(base) = archer_base {
                    ui.label("HP (Archer)");
                    ui.label(format!("{:.0} -> {:.0}", base.max_health, base.max_health * archer_hp_mult));
                    ui.end_row();

                    ui.label("Damage (Archer)");
                    ui.label(format!("{:.1} -> {:.1}", base.damage, base.damage * archer_dmg_mult));
                    ui.end_row();

                    ui.label("Move Speed (Archer)");
                    ui.label(format!("{:.0} -> {:.0}", base.speed, base.speed * archer_speed_mult));
                    ui.end_row();
                }

                if let Some(base) = ranged_base {
                    ui.label("Detection Range (Archer)");
                    ui.label(format!("{:.0} -> {:.0}", base.range, base.range * archer_range_mult));
                    ui.end_row();

                    ui.label("Attack Cooldown (Archer)");
                    ui.label(format!("{:.2}s -> {:.2}s ({:.0}% faster)", base.cooldown, base.cooldown * archer_cd_mult, archer_cd_reduction));
                    ui.end_row();
                }

                ui.label("Alert (Archer)");
                ui.label(format!("{:.0}% of base", archer_alert_mult * 100.0));
                ui.end_row();

                ui.label("Dodge (Archer)");
                ui.label(if UPGRADES.stat_level(lv, "Archer", UpgradeStatKind::Dodge) > 0 { "Unlocked" } else { "Locked" });
                ui.end_row();

                ui.separator();
                ui.separator();
                ui.end_row();

                if let Some(base) = fighter_base {
                    ui.label("HP (Fighter)");
                    ui.label(format!("{:.0} -> {:.0}", base.max_health, base.max_health * fighter_hp_mult));
                    ui.end_row();

                    ui.label("Damage (Fighter)");
                    ui.label(format!("{:.1} -> {:.1}", base.damage, base.damage * fighter_dmg_mult));
                    ui.end_row();

                    ui.label("Move Speed (Fighter)");
                    ui.label(format!("{:.0} -> {:.0}", base.speed, base.speed * fighter_speed_mult));
                    ui.end_row();
                }

                if let Some(base) = melee_base {
                    ui.label("Attack Cooldown (Fighter)");
                    ui.label(format!("{:.2}s -> {:.2}s ({:.0}% faster)", base.cooldown, base.cooldown * fighter_cd_mult, fighter_cd_reduction));
                    ui.end_row();
                }

                ui.label("Dodge (Fighter)");
                ui.label(if UPGRADES.stat_level(lv, "Fighter", UpgradeStatKind::Dodge) > 0 { "Unlocked" } else { "Locked" });
                ui.end_row();

                ui.separator();
                ui.separator();
                ui.end_row();

                if let Some(base) = crossbow_base {
                    ui.label("HP (Crossbow)");
                    ui.label(format!("{:.0} -> {:.0}", base.max_health, base.max_health * xbow_hp_mult));
                    ui.end_row();

                    ui.label("Damage (Crossbow)");
                    ui.label(format!("{:.1} -> {:.1}", base.damage, base.damage * xbow_dmg_mult));
                    ui.end_row();

                    ui.label("Move Speed (Crossbow)");
                    ui.label(format!("{:.0} -> {:.0}", base.speed, base.speed * xbow_speed_mult));
                    ui.end_row();
                }

                if let Some(base) = crossbow_atk {
                    ui.label("Detection Range (Crossbow)");
                    ui.label(format!("{:.0} -> {:.0}", base.range, base.range * xbow_range_mult));
                    ui.end_row();

                    ui.label("Attack Cooldown (Crossbow)");
                    let xbow_cd_red = (1.0 - xbow_cd_mult) * 100.0;
                    ui.label(format!("{:.2}s -> {:.2}s ({:.0}% faster)", base.cooldown, base.cooldown * xbow_cd_mult, xbow_cd_red));
                    ui.end_row();
                }
            });

        right.separator();
        right.label("Squad Commander");
        if snap.squads.is_empty() {
            right.label("No squads with members.");
        } else {
            let mut squads = snap.squads.clone();
            squads.sort_by_key(|s| s.squad_idx);

            let role_for = |i: usize, s: &SquadSnapshot| -> &'static str {
                if snap.faction == 0 {
                    "MANUAL"
                } else if i == 0 {
                    "DEF"
                } else if s.target_size == 0 {
                    "IDLE"
                } else {
                    "ATK"
                }
            };

            let mut defense_archers = 0usize;
            let mut offense_archers = 0usize;
            let mut attack_squads_active = 0usize;
            for (i, s) in squads.iter().enumerate() {
                match role_for(i, s) {
                    "DEF" => defense_archers += s.members,
                    "ATK" => {
                        offense_archers += s.members;
                        if s.members > 0 {
                            attack_squads_active += 1;
                        }
                    }
                    _ => {}
                }
            }

            right.label(format!("Active squads: {}", squads.len()));
            if snap.faction == 0 {
                right.label("Commander: Manual");
            } else {
                right.label("Commander: AI");
                right.label(format!(
                    "Defense: {}  Offense: {}  Active attack squads: {}",
                    defense_archers, offense_archers, attack_squads_active
                ));
            }

            egui::Grid::new(format!("intel_squads_grid_{}", snap.faction))
                .striped(true)
                .num_columns(7)
                .show(right, |ui| {
                    ui.label("Role");
                    ui.label("Squad");
                    ui.label("Members");
                    ui.label("State");
                    ui.label("Target");
                    ui.label("CD");
                    ui.label("Jump");
                    ui.end_row();

                    for (i, squad) in squads.iter().enumerate() {
                        let role = role_for(i, squad);
                        let mut state_bits: Vec<&str> = Vec::new();
                        if squad.patrol_enabled { state_bits.push("PATROL"); }
                        if squad.rest_when_tired { state_bits.push("REST"); }
                        if squad.commander_kind.is_some() { state_bits.push("LOCK"); }
                        let state = if state_bits.is_empty() {
                            "-".to_string()
                        } else {
                            state_bits.join(" ")
                        };

                        let target = if let Some(kind) = squad.commander_kind {
                            let idx = squad.commander_index.unwrap_or(0);
                            format!("{:?} #{}", kind, idx)
                        } else if squad.target.is_some() {
                            "Map target".to_string()
                        } else {
                            "None".to_string()
                        };
                        let cd = squad.commander_cooldown.unwrap_or(0.0).max(0.0);

                        ui.label(role);
                        ui.label(format!("#{}", squad.squad_idx + 1));
                        ui.label(format!("{}/{}", squad.members, squad.target_size));
                        ui.label(&state).on_hover_text("PATROL = holds local patrol when idle, REST = returns home when tired, LOCK = commander has an active target lock");
                        ui.label(target);
                        ui.label(format!("{:.1}s", cd));
                        if let Some(target) = squad.target {
                            if ui.button("Jump").clicked() {
                                *jump_target = Some(target);
                            }
                        } else {
                            ui.label("-");
                        }
                        ui.end_row();
                    }
                });
        }

        if !snap.last_actions.is_empty() {
            right.separator();
            right.label("Recent Actions");
            for action in &snap.last_actions {
                right.small(action);
            }
        }
    });
}

// ============================================================================
// PROFILER CONTENT
// ============================================================================

fn profiler_content(
    ui: &mut egui::Ui,
    timings: &SystemTimings,
    migration: &mut MigrationState,
    user_settings: &mut UserSettings,
) {
    let frame_ms = timings.get_frame_ms();
    ui.label(egui::RichText::new(format!("Frame: {:.2} ms", frame_ms)).strong());
    ui.separator();

    // Debug actions
    egui::CollapsingHeader::new(egui::RichText::new("Debug Actions").strong())
        .default_open(false)
        .show(ui, |ui| {
            let prev_terrain = user_settings.show_terrain_sprites;
            ui.checkbox(&mut user_settings.show_terrain_sprites, "Show Terrain Sprites");
            if prev_terrain != user_settings.show_terrain_sprites {
                settings::save_settings(user_settings);
            }
            ui.separator();
            let has_active = migration.active.is_some();
            let btn = ui.add_enabled(!has_active, egui::Button::new("Spawn Migration Group"));
            if btn.clicked() {
                migration.debug_spawn = true;
            }
            if has_active {
                let count = migration.active.as_ref().map(|g| g.member_slots.len()).unwrap_or(0);
                ui.label(format!("Migration active: {} raiders", count));
            }
        });
    ui.separator();

    let all = timings.get_timings();
    if all.is_empty() {
        ui.label("Enable profiler in pause menu settings");
        return;
    }

    // Separate timings from counts (keys containing "/n_")
    let mut timing_entries: Vec<(&str, f32)> = Vec::new();
    let mut count_map: HashMap<&str, f32> = HashMap::new();
    for (&name, &val) in &all {
        if name.contains("/n_") {
            count_map.insert(name, val);
        } else {
            timing_entries.push((name, val));
        }
    }
    timing_entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Compute untracked time (render pipeline, vsync, ECS scheduling)
    let systems_total: f32 = timing_entries.iter().map(|(_, ms)| ms).sum();
    let untracked = (frame_ms - systems_total).max(0.0);
    timing_entries.push(("render + other", untracked));

    // Check if any entry has a paired count
    let has_counts = !count_map.is_empty();

    if ui.button("Copy Top 10").clicked() {
        let top10: String = timing_entries.iter().take(10)
            .map(|(name, ms)| format!("{}: {:.3} ms", name, ms))
            .collect::<Vec<_>>()
            .join("\n");
        let text = format!("Frame: {:.2} ms\n{}", frame_ms, top10);
        ui.ctx().copy_text(text);
    }
    ui.separator();

    let cols = if has_counts { 3 } else { 2 };
    egui::Grid::new("profiler_grid").num_columns(cols).striped(true).show(ui, |ui| {
        // Header
        ui.label(egui::RichText::new("system").strong());
        ui.label(egui::RichText::new("ms").strong());
        if has_counts { ui.label(egui::RichText::new("count").strong()); }
        ui.end_row();

        for (name, ms) in &timing_entries {
            ui.label(*name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("{:.3}", ms)).monospace());
            });
            if has_counts {
                // Look for paired count: "decision/arrival" → "decision/n_arrival"
                let count_key = if let Some(slash) = name.rfind('/') {
                    let (prefix, suffix) = name.split_at(slash + 1);
                    let candidate = format!("{prefix}n_{suffix}");
                    count_map.iter().find(|(k, _)| **k == candidate).map(|(_, &v)| v)
                } else {
                    None
                };
                if let Some(c) = count_key {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(format!("{:.0}", c)).monospace());
                    });
                } else {
                    ui.label("");
                }
            }
            ui.end_row();
        }
    });
}

// ============================================================================
// HELP TAB
// ============================================================================

fn help_content(ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        egui::CollapsingHeader::new(egui::RichText::new("Quick Start").strong())
            .default_open(true)
            .show(ui, |ui| {
                ui.label("1. B > build Farms, then Farmer Homes");
                ui.label("2. Waypoints near farms, then Archer Homes");
                ui.label("3. Food buys buildings + upgrades (U). Gold for advanced upgrades.");
                ui.label("4. Click to inspect. ESC for settings.");
            });

        egui::CollapsingHeader::new(egui::RichText::new("Economy").strong())
            .show(ui, |ui| {
                ui.label("- Farms grow food. Farmer Homes spawn farmers to harvest them.");
                ui.label("- Gold Mines between towns. Miner Homes spawn miners.");
                ui.label("- Food = buildings + upgrades. Gold = advanced upgrades.");
                ui.label("- Dead NPCs respawn after 12 game-hours.");
            });

        egui::CollapsingHeader::new(egui::RichText::new("Military").strong())
            .show(ui, |ui| {
                ui.label("- Waypoints are patrol points for archers. Archer Homes spawn archers.");
                ui.label("- Archers level up from kills (+1% stats/level).");
                ui.label("- Policies (P): set work schedules, off-duty behavior, flee/aggro.");
                ui.label("- Squads (Q): all archers join Squad 1. Set sizes for 2-9 to split into groups.");
                ui.label("- Press 1-9 to pick a squad, click the map to send them.");
                ui.label("- Patrols (T): reorder waypoint patrol routes.");
            });

        egui::CollapsingHeader::new(egui::RichText::new("Controls").strong())
            .show(ui, |ui| {
                ui.label("WASD - scroll    Wheel - zoom    Click - select");
                ui.label("Space - pause    +/- - speed (0.25x-128x)");
                ui.label("B - Build   R - Roster   U - Upgrades   P - Policies");
                ui.label("T - Patrols   Q - Squads   I - Factions   L - Log   H - Help");
                ui.label("F - Follow NPC   1-9 - Squad target   ESC - Menu");
                ui.label("F5 - Quicksave   F9 - Quickload");
            });

        egui::CollapsingHeader::new(egui::RichText::new("Tips").strong())
            .show(ui, |ui| {
                ui.label("- Build farms before homes -- no farm, no work.");
                ui.label("- Waypoints between farms and enemy camps.");
                ui.label("- Day Only schedule (P) keeps farmers safe at night.");
                ui.label("- Upgrade Fountain Radius early for faster healing.");
            });
    });
}
