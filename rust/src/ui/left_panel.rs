//! Left panel — tabbed container for Roster, Upgrades, Policies, and Patrols.

use std::collections::HashMap;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;

use crate::components::*;
use crate::resources::*;
use crate::settings::{self, UserSettings};
use crate::systems::stats::{TownUpgrades, UpgradeQueue, UPGRADE_COUNT, UPGRADE_REGISTRY, UPGRADE_RENDER_ORDER, upgrade_unlocked, upgrade_available, missing_prereqs, format_upgrade_cost, upgrade_effect_summary, branch_total};
use crate::systems::{AiPlayerState, AiKind};
use crate::world::WorldData;

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
    meta_cache: Res<'w, NpcMetaCache>,
    health_query: Query<'w, 's, (
        &'static NpcIndex,
        &'static Health,
        &'static CachedStats,
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
    meta_cache: Res<'w, NpcMetaCache>,
    gpu_state: Res<'w, GpuReadState>,
    // Query: archers with SquadId (for dismiss)
    squad_guards: Query<'w, 's, (Entity, &'static NpcIndex, &'static SquadId), (With<Archer>, Without<Dead>)>,
}

// ============================================================================
// INTEL TYPES
// ============================================================================

#[derive(SystemParam)]
pub struct IntelParams<'w> {
    ai_state: Res<'w, AiPlayerState>,
    food_storage: Res<'w, FoodStorage>,
    spawner_state: Res<'w, SpawnerState>,
    faction_stats: Res<'w, FactionStats>,
    upgrades: Res<'w, TownUpgrades>,
}

#[derive(Clone)]
struct AiSnapshot {
    faction: i32,
    town_name: String,
    kind_name: &'static str,
    personality_name: &'static str,
    food: i32,
    farmers: usize,
    archers: usize,
    raiders: usize,
    miners: usize,
    farmer_homes: usize,
    archer_homes: usize,
    tents: usize,
    miner_homes: usize,
    farms: usize,
    guard_posts: usize,
    alive: i32,
    dead: i32,
    kills: i32,
    upgrades: [u8; UPGRADE_COUNT],
    last_actions: Vec<String>,
    archer_aggressive: bool,
    archer_leash: bool,
    archer_flee_hp: f32,
    farmer_flee_hp: f32,
    prioritize_healing: bool,
    center: Vec2,
}

#[derive(Default)]
pub struct IntelCache {
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
    mut world_data: ResMut<WorldData>,
    mut policies: ResMut<TownPolicies>,
    mut roster: RosterParams,
    mut upgrade: UpgradeParams,
    mut squad: SquadParams,
    intel: IntelParams,
    timings: Res<SystemTimings>,
    mut commands: Commands,
    mut roster_state: Local<RosterState>,
    mut intel_cache: Local<IntelCache>,
    settings: Res<UserSettings>,
    catalog: Res<HelpCatalog>,
    mut prev_tab: Local<LeftPanelTab>,
) -> Result {
    if !ui_state.left_panel_open {
        *prev_tab = LeftPanelTab::Roster;
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;
    let debug_all = settings.debug_all_npcs;

    let tab_name = match ui_state.left_panel_tab {
        LeftPanelTab::Roster => "Roster",
        LeftPanelTab::Upgrades => "Upgrades",
        LeftPanelTab::Policies => "Policies",
        LeftPanelTab::Patrols => "Patrols",
        LeftPanelTab::Squads => "Squads",
        LeftPanelTab::Intel => "Intel",
        LeftPanelTab::Profiler => "Profiler",
    };

    // Look up the help key for the current tab
    let tab_help_key = match ui_state.left_panel_tab {
        LeftPanelTab::Roster => "tab_roster",
        LeftPanelTab::Upgrades => "tab_upgrades",
        LeftPanelTab::Policies => "tab_policies",
        LeftPanelTab::Patrols => "tab_patrols",
        LeftPanelTab::Squads => "tab_squads",
        LeftPanelTab::Intel => "tab_intel",
        LeftPanelTab::Profiler => "tab_profiler",
    };

    let mut open = ui_state.left_panel_open;
    let mut jump_target: Option<Vec2> = None;
    egui::Window::new(tab_name)
        .open(&mut open)
        .resizable(false)
        .default_width(340.0)
        .anchor(egui::Align2::LEFT_TOP, [4.0, 30.0])
        .show(ctx, |ui| {
            // Inline help text at the top of every tab
            if let Some(tip) = catalog.0.get(tab_help_key) {
                ui.label(egui::RichText::new(*tip).size(settings.help_text_size));
                ui.separator();
            }

            match ui_state.left_panel_tab {
                LeftPanelTab::Roster => roster_content(ui, &mut roster, &mut roster_state, debug_all),
                LeftPanelTab::Upgrades => upgrade_content(ui, &mut upgrade, &world_data),
                LeftPanelTab::Policies => policies_content(ui, &mut policies, &world_data),
                LeftPanelTab::Patrols => patrols_content(ui, &mut world_data),
                LeftPanelTab::Squads => squads_content(ui, &mut squad, &world_data, &mut commands),
                LeftPanelTab::Intel => intel_content(ui, &intel, &world_data, &policies, &mut intel_cache, &mut jump_target),
                LeftPanelTab::Profiler => profiler_content(ui, &timings),
            }
        });

    // Apply camera jump from Intel panel
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

fn roster_content(ui: &mut egui::Ui, roster: &mut RosterParams, state: &mut RosterState, debug_all: bool) {
    // Rebuild cache every 30 frames
    state.frame_counter += 1;
    if state.frame_counter % 30 == 1 || state.cached_rows.is_empty() {
        let mut rows = Vec::new();
        for (npc_idx, health, cached, activity, combat, faction) in roster.health_query.iter() {
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
                trait_name: crate::trait_name(meta.trait_id).to_string(),
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
        if ui.selectable_label(state.job_filter == 0, "Farmers").clicked() {
            state.job_filter = 0;
            state.frame_counter = 0;
        }
        if ui.selectable_label(state.job_filter == 1, "Archers").clicked() {
            state.job_filter = 1;
            state.frame_counter = 0;
        }
        if ui.selectable_label(state.job_filter == 4, "Miners").clicked() {
            state.job_filter = 4;
            state.frame_counter = 0;
        }
        if debug_all {
            if ui.selectable_label(state.job_filter == 2, "Raiders").clicked() {
                state.job_filter = 2;
                state.frame_counter = 0;
            }
        }
    });

    // Miner target control — set how many villagers should be miners
    ui.label(format!("{} NPCs", state.cached_rows.len()));
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
        let selected_idx = roster.selected.0;
        let mut new_selected: Option<i32> = None;
        let mut follow_idx: Option<usize> = None;

        for row in &state.cached_rows {
            let is_selected = selected_idx == row.slot as i32;
            let job_color = match row.job {
                0 => egui::Color32::from_rgb(80, 200, 80),   // Farmer green
                1 => egui::Color32::from_rgb(80, 100, 220),  // Archer blue
                2 => egui::Color32::from_rgb(220, 80, 80),   // Raider red
                4 => egui::Color32::from_rgb(160, 110, 60),  // Miner brown
                _ => egui::Color32::from_rgb(220, 220, 80),
            };

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

fn upgrade_content(ui: &mut egui::Ui, upgrade: &mut UpgradeParams, world_data: &WorldData) {
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
    let total: u32 = levels.iter().map(|&l| l as u32).sum();
    ui.horizontal(|ui| {
        for (branch, _) in UPGRADE_RENDER_ORDER {
            let bt = branch_total(&levels, branch);
            ui.label(egui::RichText::new(format!("{}: {}", branch, bt)).small());
        }
        ui.label(egui::RichText::new(format!("Total: {}", total)).small().strong());
    });
    ui.separator();

    // Tree-ordered upgrade list
    for (branch, nodes) in UPGRADE_RENDER_ORDER {
        let bt = branch_total(&levels, branch);
        ui.add_space(4.0);
        ui.label(egui::RichText::new(format!("{} ({})", branch, bt)).strong());

        for &(i, depth) in *nodes {
            let upg = &UPGRADE_REGISTRY[i];
            let unlocked = upgrade_unlocked(&levels, i);
            let available = upgrade_available(&levels, i, food, gold);
            let indent = depth as f32 * 16.0;

            ui.horizontal(|ui| {
                ui.add_space(indent);

                // Auto-upgrade checkbox
                if upgrade.auto.flags.len() <= town_idx {
                    upgrade.auto.flags.resize(town_idx + 1, [false; UPGRADE_COUNT]);
                }
                let auto_flag = &mut upgrade.auto.flags[town_idx][i];
                let prev_auto = *auto_flag;
                ui.add_enabled(unlocked, egui::Checkbox::new(auto_flag, ""))
                    .on_hover_text("Auto-buy each game hour");
                if *auto_flag != prev_auto {
                    let mut saved = settings::load_settings();
                    saved.auto_upgrades = upgrade.auto.flags[town_idx].to_vec();
                    settings::save_settings(&saved);
                }

                // Label (dimmed when locked)
                let label_text = egui::RichText::new(upg.label);
                ui.label(if unlocked { label_text } else { label_text.weak() });

                // Effect summary (now/next)
                let (now, next) = upgrade_effect_summary(i, levels[i]);
                ui.label(egui::RichText::new(format!("{} \u{2192} {}", now, next)).small().weak());

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let cost_text = format_upgrade_cost(i, levels[i]);
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

                    ui.label(format!("Lv{}", levels[i]));
                });
            });
        }
    }
}

// ============================================================================
// POLICIES CONTENT
// ============================================================================

fn policies_content(ui: &mut egui::Ui, policies: &mut TownPolicies, world_data: &WorldData) {
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
}

// ============================================================================
// PATROLS CONTENT
// ============================================================================

fn patrols_content(ui: &mut egui::Ui, world_data: &mut WorldData) {
    let town_pair_idx = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0) as u32;

    if let Some(town) = world_data.towns.get(town_pair_idx as usize) {
        ui.small(format!("Town: {}", town.name));
    }

    // Collect non-tombstoned posts for this town, sorted by patrol_order
    let mut posts: Vec<(usize, u32, Vec2)> = world_data.guard_posts.iter().enumerate()
        .filter(|(_, p)| p.town_idx == town_pair_idx && p.position.x > -9000.0)
        .map(|(i, p)| (i, p.patrol_order, p.position))
        .collect();
    posts.sort_by_key(|(_, order, _)| *order);

    ui.label(format!("{} guard posts", posts.len()));
    ui.separator();

    let mut swap: Option<(usize, usize)> = None;

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (list_idx, &(data_idx, order, pos)) in posts.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", order));
                ui.label(format!("({:.0}, {:.0})", pos.x, pos.y));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if list_idx + 1 < posts.len() {
                        if ui.small_button("\u{25BC}").on_hover_text("Move down").clicked() {
                            swap = Some((data_idx, posts[list_idx + 1].0));
                        }
                    }
                    if list_idx > 0 {
                        if ui.small_button("\u{25B2}").on_hover_text("Move up").clicked() {
                            swap = Some((data_idx, posts[list_idx - 1].0));
                        }
                    }
                });
            });
        }
    });

    // Apply swap — mutates WorldData which triggers rebuild_patrol_routes_system
    if let Some((a, b)) = swap {
        let order_a = world_data.guard_posts[a].patrol_order;
        let order_b = world_data.guard_posts[b].patrol_order;
        world_data.guard_posts[a].patrol_order = order_b;
        world_data.guard_posts[b].patrol_order = order_a;
    }
}

// ============================================================================
// SQUADS CONTENT
// ============================================================================

fn squads_content(ui: &mut egui::Ui, squad: &mut SquadParams, _world_data: &WorldData, commands: &mut Commands) {
    let selected = squad.squad_state.selected;

    // Squad list
    for i in 0..squad.squad_state.squads.len() {
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
    ui.strong(format!("{} {} — {} archers", header_name, si + 1, member_count));

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

    ui.add_space(4.0);

    // Recruit controls: move archers from Default Squad (1) into selected squad.
    let default_count = squad.squad_state.squads.first().map(|s| s.members.len()).unwrap_or(0);
    if si == 0 {
        ui.small(format!("Default squad pool: {} archers", default_count));
    } else {
        ui.small(format!("Default squad pool: {} archers", default_count));
        ui.horizontal_wrapped(|ui| {
            for amount in [1usize, 2, 4, 8, 16, 32] {
                let enabled = default_count >= amount;
                if ui.add_enabled(enabled, egui::Button::new(format!("+{}", amount))).clicked() {
                    // Pick first N members from default squad and transfer to selected squad.
                    let recruits: Vec<usize> = squad.squad_state.squads[0].members.iter().copied().take(amount).collect();

                    for slot in &recruits {
                        for (entity, npc_idx, sid) in squad.squad_guards.iter() {
                            if sid.0 == 0 && npc_idx.0 == *slot {
                                commands.entity(entity).insert(SquadId(si as i32));
                                break;
                            }
                        }
                    }

                    squad.squad_state.squads[0].members.retain(|slot| !recruits.contains(slot));
                    for slot in recruits {
                        if !squad.squad_state.squads[si].members.contains(&slot) {
                            squad.squad_state.squads[si].members.push(slot);
                        }
                    }

                    // Keep auto-recruit logic from immediately dismissing newly added members.
                    let selected_len = squad.squad_state.squads[si].members.len();
                    let selected_target = squad.squad_state.squads[si].target_size;
                    squad.squad_state.squads[si].target_size = selected_target.max(selected_len);
                }
            }
        });
    }

    // Dismiss all
    if member_count > 0 {
        if ui.button("Dismiss All").clicked() {
            for (entity, _, sid) in squad.squad_guards.iter() {
                if sid.0 == selected {
                    commands.entity(entity).remove::<SquadId>();
                }
            }
            squad.squad_state.squads[si].members.clear();
            squad.squad_state.squads[si].target_size = 0;
        }
    }

    ui.separator();

    // Member list
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let members = &squad.squad_state.squads[si].members;
        for &slot in members {
            if slot >= squad.meta_cache.0.len() { continue; }
            let meta = &squad.meta_cache.0[slot];
            if meta.name.is_empty() { continue; }

            // Try to get HP from GPU readback
            let hp_str = if slot < squad.gpu_state.health.len() {
                format!("HP {:.0}", squad.gpu_state.health[slot])
            } else {
                String::new()
            };

            ui.horizontal(|ui| {
                let job_color = egui::Color32::from_rgb(80, 100, 220);
                ui.colored_label(job_color, &meta.name);
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


fn rebuild_intel_cache(
    intel: &IntelParams,
    world_data: &WorldData,
    policies: &TownPolicies,
    cache: &mut IntelCache,
) {
    cache.snapshots.clear();
    for player in intel.ai_state.players.iter() {
        let tdi = player.town_data_idx;
        let ti = tdi as u32;

        let town_name = world_data.towns.get(tdi)
            .map(|t| t.name.clone()).unwrap_or_default();
        let center = world_data.towns.get(tdi)
            .map(|t| t.center).unwrap_or_default();

        let kind_name = match player.kind {
            AiKind::Builder => "Builder",
            AiKind::Raider => "Raider",
        };

        let alive_check = |pos: Vec2, idx: u32| idx == ti && pos.x > -9000.0;
        let farms = world_data.farms.iter().filter(|f| alive_check(f.position, f.town_idx)).count();
        let farmer_homes = world_data.farmer_homes.iter().filter(|h| alive_check(h.position, h.town_idx)).count();
        let archer_homes = world_data.archer_homes.iter().filter(|b| alive_check(b.position, b.town_idx)).count();
        let guard_posts = world_data.guard_posts.iter().filter(|g| alive_check(g.position, g.town_idx)).count();
        let tents = world_data.tents.iter().filter(|t| alive_check(t.position, t.town_idx)).count();
        let miner_homes = world_data.miner_homes.iter().filter(|ms| alive_check(ms.position, ms.town_idx)).count();

        // Count alive NPCs by job from spawner state
        let ti_i32 = tdi as i32;
        let alive_spawner = |kind: i32| intel.spawner_state.0.iter()
            .filter(|s| s.building_kind == kind && s.town_idx == ti_i32 && s.npc_slot >= 0 && s.position.x > -9000.0).count();
        let farmers = alive_spawner(0);
        let archers = alive_spawner(1);
        let raiders = alive_spawner(2);
        let miners = alive_spawner(3);

        let food = intel.food_storage.food.get(tdi).copied().unwrap_or(0);
        let faction = world_data.towns.get(tdi).map(|t| t.faction).unwrap_or(0);
        let (alive, dead, kills) = intel.faction_stats.stats.get(faction as usize)
            .map(|s| (s.alive, s.dead, s.kills))
            .unwrap_or((0, 0, 0));

        let upgrades = intel.upgrades.levels.get(tdi).copied().unwrap_or([0; UPGRADE_COUNT]);

        let last_actions: Vec<String> = player.last_actions.iter().rev().cloned().collect();

        let policy = policies.policies.get(tdi);
        let archer_aggressive = policy.map(|p| p.archer_aggressive).unwrap_or(false);
        let archer_leash = policy.map(|p| p.archer_leash).unwrap_or(true);
        let archer_flee_hp = policy.map(|p| p.archer_flee_hp).unwrap_or(0.15);
        let farmer_flee_hp = policy.map(|p| p.farmer_flee_hp).unwrap_or(0.30);
        let prioritize_healing = policy.map(|p| p.prioritize_healing).unwrap_or(true);

        cache.snapshots.push(AiSnapshot {
            faction,
            town_name,
            kind_name,
            personality_name: player.personality.name(),
            food,
            farmers,
            archers,
            raiders,
            miners,
            farmer_homes,
            archer_homes,
            tents,
            miner_homes,
            farms,
            guard_posts,
            alive,
            dead,
            kills,
            upgrades,
            last_actions,
            archer_aggressive,
            archer_leash,
            archer_flee_hp,
            farmer_flee_hp,
            prioritize_healing,
            center,
        });
    }
}

fn intel_content(
    ui: &mut egui::Ui,
    intel: &IntelParams,
    world_data: &WorldData,
    policies: &TownPolicies,
    cache: &mut IntelCache,
    jump_target: &mut Option<Vec2>,
) {
    // Rebuild cache every 30 frames
    cache.frame_counter += 1;
    if cache.frame_counter % 30 == 1 || cache.snapshots.is_empty() {
        rebuild_intel_cache(intel, world_data, policies, cache);
    }

    if cache.snapshots.is_empty() {
        ui.label("No AI settlements");
        return;
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
    });
    ui.label(format!("Alive: {}  Dead: {}  Kills: {}", snap.alive, snap.dead, snap.kills));
    ui.separator();

    ui.label("Units");
    ui.label(format!("Farmers: {}/{}", snap.farmers, snap.farmer_homes));
    ui.label(format!("Archers: {}/{}", snap.archers, snap.archer_homes));
    ui.label(format!("Raiders: {}/{}", snap.raiders, snap.tents));
    ui.label(format!("Miners: {}/{}", snap.miners, snap.miner_homes));
    ui.separator();

    ui.label("Buildings");
    ui.label(format!("Farms: {}", snap.farms));
    ui.label(format!("Guard Posts: {}", snap.guard_posts));
    ui.label(format!("Farmer Homes: {}", snap.farmer_homes));
    ui.label(format!("Archer Homes: {}", snap.archer_homes));
    ui.label(format!("Tents: {}", snap.tents));
    ui.label(format!("Miner Homes: {}", snap.miner_homes));
    ui.separator();

    ui.label("Upgrades");
    egui::Grid::new("intel_upgrades_grid").num_columns(2).striped(true).show(ui, |ui| {
        for (j, &level) in snap.upgrades.iter().enumerate() {
            let label = UPGRADE_REGISTRY.get(j).map(|n| n.label).unwrap_or("?");
            ui.label(label);
            ui.label(format!("Lv{}", level));
            ui.end_row();
        }
    });

    ui.separator();
    ui.label("Policies");
    ui.label(format!("Archer Aggressive: {}", if snap.archer_aggressive { "On" } else { "Off" }));
    ui.label(format!("Archer Leash: {}", if snap.archer_leash { "On" } else { "Off" }));
    ui.label(format!("Prioritize Healing: {}", if snap.prioritize_healing { "On" } else { "Off" }));
    ui.label(format!("Flee HP: Archer {:.0}% / Farmer {:.0}%", snap.archer_flee_hp * 100.0, snap.farmer_flee_hp * 100.0));

    if !snap.last_actions.is_empty() {
        ui.separator();
        ui.label("Recent Actions");
        for action in &snap.last_actions {
            ui.small(action);
        }
    }
}

// ============================================================================
// PROFILER CONTENT
// ============================================================================

fn profiler_content(ui: &mut egui::Ui, timings: &SystemTimings) {
    let frame_ms = timings.get_frame_ms();
    ui.label(egui::RichText::new(format!("Frame: {:.2} ms", frame_ms)).strong());
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
