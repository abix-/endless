//! Left panel — tabbed container for Roster, Upgrades, Policies, and Patrols.

use std::collections::HashMap;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;

use crate::components::*;
use crate::resources::*;
use crate::settings::{self, UserSettings};
use crate::systems::stats::{TownUpgrades, UpgradeQueue, UPGRADE_COUNT, upgrade_cost};
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
// UPGRADE TYPES
// ============================================================================

struct UpgradeDef {
    label: &'static str,
    tooltip: &'static str,
    category: &'static str,
}

const UPGRADES: &[UpgradeDef] = &[
    UpgradeDef { label: "Guard Health",    tooltip: "+10% guard HP per level",                  category: "Guard" },
    UpgradeDef { label: "Guard Attack",    tooltip: "+10% guard damage per level",              category: "Guard" },
    UpgradeDef { label: "Guard Range",     tooltip: "+5% guard attack range per level",         category: "Guard" },
    UpgradeDef { label: "Guard Size",      tooltip: "+5% guard size per level",                 category: "Guard" },
    UpgradeDef { label: "Attack Speed",    tooltip: "-8% attack cooldown per level",            category: "Guard" },
    UpgradeDef { label: "Move Speed",      tooltip: "+5% movement speed per level",             category: "Guard" },
    UpgradeDef { label: "Alert Radius",    tooltip: "+10% alert radius per level",              category: "Guard" },
    UpgradeDef { label: "Farm Yield",      tooltip: "+15% food production per level",           category: "Farm" },
    UpgradeDef { label: "Farmer HP",       tooltip: "+20% farmer HP per level",                 category: "Farm" },
    UpgradeDef { label: "Healing Rate",    tooltip: "+20% HP regen at fountain per level",      category: "Town" },
    UpgradeDef { label: "Food Efficiency", tooltip: "10% chance per level to not consume food", category: "Town" },
    UpgradeDef { label: "Fountain Radius", tooltip: "+24px fountain healing range per level",   category: "Town" },
    UpgradeDef { label: "Town Area",       tooltip: "+1 buildable radius per level",            category: "Town" },
];

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
    miner_target: ResMut<'w, MinerTarget>,
}

#[derive(SystemParam)]
pub struct UpgradeParams<'w> {
    food_storage: Res<'w, FoodStorage>,
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
    // Query: alive guards without SquadId (available for recruitment)
    available_guards: Query<'w, 's, (Entity, &'static NpcIndex, &'static TownId), (With<Guard>, Without<Dead>, Without<SquadId>)>,
    // Query: guards with SquadId (for dismiss)
    squad_guards: Query<'w, 's, (Entity, &'static NpcIndex, &'static SquadId), (With<Guard>, Without<Dead>)>,
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
    town_name: String,
    kind_name: &'static str,
    personality_name: &'static str,
    food: i32,
    farmers: usize,
    guards: usize,
    raiders: usize,
    houses: usize,
    barracks: usize,
    tents: usize,
    farms: usize,
    guard_posts: usize,
    alive: i32,
    dead: i32,
    kills: i32,
    upgrades: [u8; UPGRADE_COUNT],
    last_actions: Vec<String>,
    guard_aggressive: bool,
    guard_leash: bool,
    guard_flee_hp: f32,
    farmer_flee_hp: f32,
    prioritize_healing: bool,
    center: Vec2,
}

#[derive(Default)]
pub struct IntelCache {
    frame_counter: u32,
    snapshots: Vec<AiSnapshot>,
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
                ui.small(*tip);
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
        if ui.selectable_label(state.job_filter == 1, "Guards").clicked() {
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
    let player_town_idx = roster.meta_cache.0.iter()
        .position(|m| m.job == 0 || m.job == 4) // find any farmer/miner
        .and_then(|idx| {
            let ti = roster.meta_cache.0[idx].town_id;
            if ti >= 0 { Some(ti as usize) } else { None }
        })
        .unwrap_or(0);
    if player_town_idx < roster.miner_target.targets.len() {
        // Count total villagers (farmers + miners) for this town as max
        let total_villagers = roster.meta_cache.0.iter()
            .filter(|m| m.town_id == player_town_idx as i32 && (m.job == 0 || m.job == 4) && !m.name.is_empty())
            .count() as i32;
        let mut target = roster.miner_target.targets[player_town_idx];
        ui.horizontal(|ui| {
            ui.label("Miners:");
            ui.add(egui::DragValue::new(&mut target).range(0..=total_villagers));
            ui.small(format!("/ {} villagers", total_villagers));
        });
        roster.miner_target.targets[player_town_idx] = target;
    }

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
                1 => egui::Color32::from_rgb(80, 100, 220),  // Guard blue
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
    let villager_stats = upgrade.faction_stats.stats.first();
    let alive = villager_stats.map(|s| s.alive).unwrap_or(0);

    ui.horizontal(|ui| {
        ui.label(format!("Food: {}", food));
        ui.separator();
        ui.label(format!("Villagers: {}", alive));
    });
    if let Some(town) = world_data.towns.get(town_idx) {
        ui.small(format!("Town: {}", town.name));
    }
    ui.separator();

    let levels = upgrade.upgrades.levels.get(town_idx).copied().unwrap_or([0; UPGRADE_COUNT]);

    let mut last_category = "";
    for (i, upg) in UPGRADES.iter().enumerate() {
        if upg.category != last_category {
            if !last_category.is_empty() {
                ui.add_space(4.0);
            }
            ui.label(egui::RichText::new(upg.category).strong());
            last_category = upg.category;
        }

        let level = levels[i];
        let cost = upgrade_cost(level);
        let can_afford = food >= cost;

        ui.horizontal(|ui| {
            // Auto-upgrade checkbox
            if upgrade.auto.flags.len() <= town_idx {
                upgrade.auto.flags.resize(town_idx + 1, [false; UPGRADE_COUNT]);
            }
            let auto_flag = &mut upgrade.auto.flags[town_idx][i];
            ui.checkbox(auto_flag, "").on_hover_text("Auto-buy each game hour");

            ui.label(upg.label);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let btn = egui::Button::new(format!("{}", cost));
                if ui.add_enabled(can_afford, btn).on_hover_text(upg.tooltip).clicked() {
                    upgrade.queue.0.push((town_idx, i));
                }
                ui.label(format!("Lv{}", level));
            });
        });
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

    // -- Guards --
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Guards").strong());
    ui.checkbox(&mut policy.guard_aggressive, "Aggressive")
        .on_hover_text("Guards never flee combat");
    ui.checkbox(&mut policy.guard_leash, "Leash")
        .on_hover_text("Guards return home if too far from post");
    let mut guard_flee_pct = policy.guard_flee_hp * 100.0;
    ui.horizontal(|ui| {
        ui.label("Flee HP:");
        ui.add(egui::Slider::new(&mut guard_flee_pct, 0.0..=100.0).suffix("%"));
    });
    policy.guard_flee_hp = guard_flee_pct / 100.0;
    let mut guard_sched_idx = policy.guard_schedule as usize;
    ui.horizontal(|ui| {
        ui.label("Schedule:");
        egui::ComboBox::from_id_salt("guard_schedule")
            .selected_text(SCHEDULE_OPTIONS[guard_sched_idx])
            .show_index(ui, &mut guard_sched_idx, SCHEDULE_OPTIONS.len(), |i| SCHEDULE_OPTIONS[i]);
    });
    policy.guard_schedule = match guard_sched_idx {
        1 => WorkSchedule::DayOnly,
        2 => WorkSchedule::NightOnly,
        _ => WorkSchedule::Both,
    };
    let mut guard_off_idx = policy.guard_off_duty as usize;
    ui.horizontal(|ui| {
        ui.label("Off-duty:");
        egui::ComboBox::from_id_salt("guard_off_duty")
            .selected_text(OFF_DUTY_OPTIONS[guard_off_idx])
            .show_index(ui, &mut guard_off_idx, OFF_DUTY_OPTIONS.len(), |i| OFF_DUTY_OPTIONS[i]);
    });
    policy.guard_off_duty = match guard_off_idx {
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

fn squads_content(ui: &mut egui::Ui, squad: &mut SquadParams, world_data: &WorldData, commands: &mut Commands) {
    let player_town = world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0) as i32;
    let selected = squad.squad_state.selected;

    // Squad list
    for i in 0..squad.squad_state.squads.len() {
        let count = squad.squad_state.squads[i].members.len();
        let has_target = squad.squad_state.squads[i].target.is_some();
        let is_selected = selected == i as i32;

        let target_str = if has_target { "target set" } else { "---" };
        let label = format!("{}. Squad {}  [{}]  {}", i + 1, i + 1, count, target_str);

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

    ui.strong(format!("Squad {} — {} guards", si + 1, member_count));

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

    ui.add_space(4.0);

    // Count available unsquadded guards
    let avail_count = squad.available_guards.iter()
        .filter(|(_, _, town)| town.0 == player_town)
        .count();
    let max_size = member_count + avail_count;

    // Target size control
    let mut target_size = squad.squad_state.squads[si].target_size;
    ui.horizontal(|ui| {
        ui.label("Size:");
        ui.add(egui::DragValue::new(&mut target_size).range(0..=max_size));
        ui.small(format!("{} / {} available", member_count, avail_count));
    });
    squad.squad_state.squads[si].target_size = target_size;

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

const UPGRADE_SHORT: &[&str] = &[
    "G.HP", "G.Atk", "G.Rng", "G.Size", "AtkSpd", "MvSpd",
    "Alert", "FarmY", "F.HP", "Heal", "FoodEff", "Fount", "Area",
];

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
        let houses = world_data.houses.iter().filter(|h| alive_check(h.position, h.town_idx)).count();
        let barracks = world_data.barracks.iter().filter(|b| alive_check(b.position, b.town_idx)).count();
        let guard_posts = world_data.guard_posts.iter().filter(|g| alive_check(g.position, g.town_idx)).count();
        let tents = world_data.tents.iter().filter(|t| alive_check(t.position, t.town_idx)).count();

        // Count alive NPCs by job from spawner state
        let ti_i32 = tdi as i32;
        let farmers = intel.spawner_state.0.iter()
            .filter(|s| s.building_kind == 0 && s.town_idx == ti_i32 && s.npc_slot >= 0 && s.position.x > -9000.0).count();
        let guards = intel.spawner_state.0.iter()
            .filter(|s| s.building_kind == 1 && s.town_idx == ti_i32 && s.npc_slot >= 0 && s.position.x > -9000.0).count();
        let raiders = intel.spawner_state.0.iter()
            .filter(|s| s.building_kind == 2 && s.town_idx == ti_i32 && s.npc_slot >= 0 && s.position.x > -9000.0).count();

        let food = intel.food_storage.food.get(tdi).copied().unwrap_or(0);
        let faction = world_data.towns.get(tdi).map(|t| t.faction).unwrap_or(0);
        let (alive, dead, kills) = intel.faction_stats.stats.get(faction as usize)
            .map(|s| (s.alive, s.dead, s.kills))
            .unwrap_or((0, 0, 0));

        let upgrades = intel.upgrades.levels.get(tdi).copied().unwrap_or([0; UPGRADE_COUNT]);

        let last_actions: Vec<String> = player.last_actions.iter().rev().cloned().collect();

        let policy = policies.policies.get(tdi);
        let guard_aggressive = policy.map(|p| p.guard_aggressive).unwrap_or(false);
        let guard_leash = policy.map(|p| p.guard_leash).unwrap_or(true);
        let guard_flee_hp = policy.map(|p| p.guard_flee_hp).unwrap_or(0.15);
        let farmer_flee_hp = policy.map(|p| p.farmer_flee_hp).unwrap_or(0.30);
        let prioritize_healing = policy.map(|p| p.prioritize_healing).unwrap_or(true);

        cache.snapshots.push(AiSnapshot {
            town_name,
            kind_name,
            personality_name: player.personality.name(),
            food,
            farmers,
            guards,
            raiders,
            houses,
            barracks,
            tents,
            farms,
            guard_posts,
            alive,
            dead,
            kills,
            upgrades,
            last_actions,
            guard_aggressive,
            guard_leash,
            guard_flee_hp,
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

    ui.label(format!("{} AI settlements", cache.snapshots.len()));
    ui.separator();

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (i, snap) in cache.snapshots.iter().enumerate() {
            let header = format!("{} [{} {}]", snap.town_name, snap.personality_name, snap.kind_name);
            let kind_color = match snap.kind_name {
                "Builder" => egui::Color32::from_rgb(80, 180, 255),
                _ => egui::Color32::from_rgb(220, 80, 80),
            };

            let id = egui::Id::new("ai_intel").with(i);
            egui::CollapsingHeader::new(egui::RichText::new(&header).color(kind_color))
                .id_salt(id)
                .default_open(cache.snapshots.len() <= 4)
                .show(ui, |ui| {
                    // Jump + Food
                    ui.horizontal(|ui| {
                        if ui.small_button("Jump").clicked() {
                            *jump_target = Some(snap.center);
                        }
                        ui.label(format!("Food: {}", snap.food));
                    });

                    // Population
                    ui.horizontal(|ui| {
                        ui.label(format!("Alive: {}  Dead: {}  Kills: {}", snap.alive, snap.dead, snap.kills));
                    });

                    // NPCs by type
                    ui.horizontal(|ui| {
                        if snap.farmers > 0 || snap.houses > 0 {
                            ui.label(format!("Farmers: {}/{}", snap.farmers, snap.houses));
                        }
                        if snap.guards > 0 || snap.barracks > 0 {
                            ui.label(format!("Guards: {}/{}", snap.guards, snap.barracks));
                        }
                        if snap.raiders > 0 || snap.tents > 0 {
                            ui.label(format!("Raiders: {}/{}", snap.raiders, snap.tents));
                        }
                    });

                    // Buildings
                    ui.horizontal(|ui| {
                        if snap.farms > 0 { ui.label(format!("Farms: {}", snap.farms)); }
                        if snap.guard_posts > 0 { ui.label(format!("Posts: {}", snap.guard_posts)); }
                    });

                    // Upgrades — compact grid, only show non-zero
                    let has_upgrades = snap.upgrades.iter().any(|&l| l > 0);
                    if has_upgrades {
                        ui.add_space(2.0);
                        ui.horizontal_wrapped(|ui| {
                            for (j, &level) in snap.upgrades.iter().enumerate() {
                                if level == 0 { continue; }
                                let label = UPGRADE_SHORT.get(j).unwrap_or(&"?");
                                ui.small(format!("{} {}", label, level));
                            }
                        });
                    }

                    // Last 3 actions (most recent first)
                    if !snap.last_actions.is_empty() {
                        ui.add_space(2.0);
                        for action in &snap.last_actions {
                            ui.colored_label(
                                egui::Color32::from_rgb(180, 120, 220),
                                format!("  {}", action),
                            );
                        }
                    }

                    // Key policies
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        if snap.guard_aggressive {
                            ui.small(egui::RichText::new("Aggressive").color(egui::Color32::from_rgb(220, 80, 80)));
                        }
                        if !snap.guard_leash {
                            ui.small(egui::RichText::new("No Leash").color(egui::Color32::from_rgb(220, 160, 40)));
                        }
                        if snap.prioritize_healing {
                            ui.small(egui::RichText::new("Heal First").color(egui::Color32::from_rgb(80, 200, 80)));
                        }
                        ui.small(format!("Flee: G{:.0}% F{:.0}%", snap.guard_flee_hp * 100.0, snap.farmer_flee_hp * 100.0));
                    });

                    ui.add_space(4.0);
                });
        }
    });
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
