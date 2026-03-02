//! Left panel — tabbed container for Roster, Upgrades, Policies, and Patrols.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;
use std::collections::HashMap;

use crate::components::*;
use crate::constants::UpgradeStatKind;
use crate::constants::{ALL_EQUIPMENT_SLOTS, BUILDING_REGISTRY, DisplayCategory, EquipmentSlot, FOUNTAIN_TOWER, Rarity, npc_def};
use crate::resources::*;
use crate::settings::{self, UserSettings};
use crate::systems::ai_player::{
    AiPersonality, RoadStyle, cheapest_gold_upgrade_cost, debug_food_military_desire,
};
use crate::systems::stats::{
    CombatConfig, TownUpgrades, UPGRADES, UpgradeMsg, branch_total, format_upgrade_cost,
    missing_prereqs, resolve_town_tower_stats, upgrade_available, upgrade_count,
    upgrade_effect_summary, upgrade_unlocked,
};
use crate::systems::{AiKind, AiPlayerState};
use crate::world::{BuildingKind, TownGrids, WorldData, WorldGrid, is_alive};

// ============================================================================
// PROFILER PARAMS
// ============================================================================

#[derive(SystemParam)]
pub struct ProfilerParams<'w> {
    timings: Res<'w, SystemTimings>,
    migration: ResMut<'w, MigrationState>,
    mining_policy: ResMut<'w, MiningPolicy>,
    target_thrash: Res<'w, NpcTargetThrashDebug>,
}

#[derive(Default)]
pub struct ProfilerCache {
    frame_counter: u32,
    frame_ms: f32,
    game_entries: Vec<(String, f32)>,
    engine_entries: Vec<(String, f32)>,
    game_sum: f32,
    engine_sum: f32,
    render_entries: Vec<(String, f32)>,
    top_flips: Vec<(usize, u16, u16, u16, u16, String)>,
    total_changes: u32,
    sink_window_key: i64,
    dirty_counts: Vec<(String, u32)>,
}

// ============================================================================
// ROSTER TYPES
// ============================================================================

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
    entity_map: Res<'w, EntityMap>,
    camera_query: Query<'w, 's, &'static mut Transform, With<crate::render::MainCamera>>,
    gpu_state: Res<'w, GpuReadState>,
    activity_q: Query<'w, 's, &'static Activity>,
    health_q: Query<'w, 's, &'static Health, Without<Building>>,
    cached_stats_q: Query<'w, 's, &'static CachedStats>,
    combat_state_q: Query<'w, 's, &'static CombatState>,
    personality_q: Query<'w, 's, &'static Personality>,
}

#[derive(SystemParam)]
pub struct UpgradeParams<'w> {
    food_storage: Res<'w, FoodStorage>,
    gold_storage: ResMut<'w, GoldStorage>,
    faction_stats: Res<'w, FactionStats>,
    upgrades: Res<'w, TownUpgrades>,
    queue: MessageWriter<'w, UpgradeMsg>,
    auto: ResMut<'w, AutoUpgrade>,
}

// ============================================================================
// SQUAD TYPES
// ============================================================================

#[derive(SystemParam)]
pub struct SquadParams<'w> {
    squad_state: ResMut<'w, SquadState>,
    gpu_state: Res<'w, GpuReadState>,
    entity_map: Res<'w, EntityMap>,
}

// ============================================================================
// INTEL TYPES
// ============================================================================

#[derive(SystemParam)]
pub struct FactionsParams<'w, 's> {
    ai_state: ResMut<'w, AiPlayerState>,
    food_storage: Res<'w, FoodStorage>,
    gold_storage: ResMut<'w, GoldStorage>,
    reputation: ResMut<'w, Reputation>,
    faction_stats: Res<'w, FactionStats>,
    upgrades: Res<'w, TownUpgrades>,
    combat_config: Res<'w, CombatConfig>,
    town_grids: Res<'w, TownGrids>,
    world_grid: Res<'w, WorldGrid>,
    entity_map: Res<'w, EntityMap>,
    gpu_state: Res<'w, GpuReadState>,
    pop_stats: Res<'w, PopulationStats>,
    faction_select: MessageReader<'w, 's, crate::messages::SelectFactionMsg>,
}

#[derive(Clone)]
struct SquadSnapshot {
    squad_idx: usize,
    members: usize,
    target_size: usize,
    patrol_enabled: bool,
    rest_when_tired: bool,
    target: Option<Vec2>,
    commander_uid: Option<crate::components::EntityUid>,
    commander_cooldown: Option<f32>,
}

#[derive(Clone)]
struct AiSnapshot {
    town_data_idx: usize,
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
    last_actions: Vec<(String, i32, i32)>,
    mining_radius: f32,
    mines_in_radius: usize,
    mines_discovered: usize,
    mines_enabled: usize,
    reserve_food: i32,
    food_desire: Option<f32>,
    military_desire: Option<f32>,
    gold_desire: Option<f32>,
    economy_desire: Option<f32>,
    food_desire_tip: String,
    military_desire_tip: String,
    gold_desire_tip: String,
    economy_desire_tip: String,
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
// TAB STRING CONVERSION & COLLAPSING SECTION PERSISTENCE
// ============================================================================

/// All collapsing section names we track for persistence.
/// Each entry is (name, default_open).
const TRACKED_SECTIONS: &[(&str, bool)] = &[
    ("Desires", true),
    ("Economy", true),
    ("Policies", false),
    ("Military", true),
    ("Economy Stats", false),
    ("Military Stats", false),
    ("Squad Commander", true),
    ("Recent Actions", true),
    ("Debug Actions", false),
    ("NPC Target Thrash (sink, 1s window)", true),
];

/// Read current collapsed state from egui and store in settings.
fn snapshot_collapsed_sections(ctx: &egui::Context, settings: &mut UserSettings) {
    settings.collapsed_sections.clear();
    for &(name, default_open) in TRACKED_SECTIONS {
        let id = egui::Id::new(name);
        let open = egui::collapsing_header::CollapsingState::load_with_default_open(
            ctx, id, default_open,
        )
        .is_open();
        if !open {
            settings.collapsed_sections.push(name.to_string());
        }
    }
}

/// Apply saved collapsed state to egui collapsing headers.
fn restore_collapsed_sections(ctx: &egui::Context, settings: &UserSettings) {
    for &(name, default_open) in TRACKED_SECTIONS {
        let id = egui::Id::new(name);
        let should_open = if settings.collapsed_sections.contains(&name.to_string()) {
            false
        } else {
            default_open
        };
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ctx, id, should_open,
        );
        state.set_open(should_open);
        state.store(ctx);
    }
}

fn tab_to_str(tab: LeftPanelTab) -> &'static str {
    match tab {
        LeftPanelTab::Roster => "Roster",
        LeftPanelTab::Upgrades => "Upgrades",
        LeftPanelTab::Policies => "Policies",
        LeftPanelTab::Patrols => "Patrols",
        LeftPanelTab::Squads => "Squads",
        LeftPanelTab::Inventory => "Inventory",
        LeftPanelTab::Factions => "Factions",
        LeftPanelTab::Blackjack => "Blackjack",
        LeftPanelTab::Profiler => "Profiler",
        LeftPanelTab::Help => "Help",
    }
}



// ============================================================================
// INVENTORY PARAMS
// ============================================================================

#[derive(SystemParam)]
pub struct InventoryParams<'w, 's> {
    pub town_inventory: Res<'w, TownInventory>,
    pub equipment_q: Query<'w, 's, (&'static NpcEquipment, &'static Job, &'static TownId)>,
    pub equip_writer: MessageWriter<'w, crate::systems::stats::EquipItemMsg>,
    pub unequip_writer: MessageWriter<'w, crate::systems::stats::UnequipItemMsg>,
    pub catalog: Res<'w, HelpCatalog>,
}

// ============================================================================
// MAIN SYSTEM
// ============================================================================

/// Tracks previous-frame state for detecting panel open/close and tab changes.
#[derive(Default)]
pub struct PanelState {
    was_open: bool,
    prev_tab: LeftPanelTab,
    pub blackjack: crate::ui::blackjack::BlackjackState,
}

pub fn left_panel_system(
    mut contexts: bevy_egui::EguiContexts,
    mut ui_state: ResMut<UiState>,
    world_data: Res<WorldData>,
    mut policies: ResMut<TownPolicies>,
    mut roster: RosterParams,
    mut upgrade: UpgradeParams,
    mut squad: SquadParams,
    mut factions: FactionsParams,
    mut profiler: ProfilerParams,
    mut roster_state: Local<RosterState>,
    mut factions_cache: Local<FactionsCache>,
    mut settings: ResMut<UserSettings>,
    mut inventory: InventoryParams,
    mut panel_state: Local<PanelState>,
    mut profiler_cache: Local<ProfilerCache>,
    mut dirty_writers: crate::messages::DirtyWriters,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    // Detect panel close → snapshot collapsed state + save all to disk once
    if !ui_state.left_panel_open {
        if panel_state.was_open {
            panel_state.was_open = false;
            snapshot_collapsed_sections(ctx, &mut settings);
            save_left_panel_state(&ui_state, &settings, &policies, &world_data);
        }
        ui_state.factions_overlay_faction = None;
        panel_state.prev_tab = LeftPanelTab::Roster;
        return Ok(());
    }
    if !panel_state.was_open {
        panel_state.was_open = true;
        restore_collapsed_sections(ctx, &settings);
    }
    if ui_state.left_panel_tab != LeftPanelTab::Factions {
        ui_state.factions_overlay_faction = None;
    }

    let debug_all = settings.debug_all_npcs;
    let help_text_size = settings.help_text_size;

    let tab_name = match ui_state.left_panel_tab {
        LeftPanelTab::Roster => "Roster",
        LeftPanelTab::Upgrades => "Upgrades",
        LeftPanelTab::Policies => "Policies",
        LeftPanelTab::Patrols => "Patrols",
        LeftPanelTab::Squads => "Squads",
        LeftPanelTab::Inventory => "Inventory",
        LeftPanelTab::Factions => "Factions",
        LeftPanelTab::Blackjack => "Blackjack",
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
        LeftPanelTab::Inventory => "tab_inventory",
        LeftPanelTab::Factions => "tab_factions",
        LeftPanelTab::Blackjack => "tab_blackjack",
        LeftPanelTab::Profiler => "tab_profiler",
        LeftPanelTab::Help => "tab_help",
    };

    let mut open = ui_state.left_panel_open;
    let mut jump_target: Option<Vec2> = None;
    let mut patrol_swap: Option<(usize, usize)> = None;
    let mut copy_text: Option<String> = None;
    let mut requested_faction: Option<i32> = None;
    for msg in factions.faction_select.read() {
        requested_faction = Some(msg.0);
    }
    egui::Window::new(tab_name)
        .open(&mut open)
        .resizable(false)
        .default_width(340.0)
        .anchor(egui::Align2::LEFT_TOP, [4.0, 30.0])
        .show(ctx, |ui| {
            // Inline help text at the top of every tab
            if let Some(tip) = inventory.catalog.0.get(tab_help_key) {
                ui.label(egui::RichText::new(*tip).size(help_text_size));
                ui.separator();
            }

            match ui_state.left_panel_tab {
                LeftPanelTab::Roster => {
                    roster_content(ui, &mut roster, &mut roster_state, debug_all)
                }
                LeftPanelTab::Upgrades => {
                    upgrade_content(ui, &mut upgrade, &world_data, &mut settings)
                }
                LeftPanelTab::Policies => policies_content(
                    ui,
                    &mut policies,
                    &world_data,
                    &factions.entity_map,
                    &mut profiler.mining_policy,
                    &mut dirty_writers,
                    &mut jump_target,
                    &mut factions.ai_state,
                ),
                LeftPanelTab::Patrols => {
                    patrol_swap =
                        patrols_content(ui, &world_data, &factions.entity_map, &mut jump_target);
                }
                LeftPanelTab::Squads => squads_content(
                    ui,
                    &mut squad,
                    &roster.meta_cache,
                    &world_data,
                    &mut dirty_writers,
                ),
                LeftPanelTab::Inventory => inventory_content(
                    ui,
                    &mut inventory,
                    &roster.selected,
                    &roster.meta_cache,
                    &factions.entity_map,
                ),
                LeftPanelTab::Factions => factions_content(
                    ui,
                    &factions,
                    &squad.squad_state,
                    &world_data,
                    &policies,
                    &profiler.mining_policy,
                    &mut factions_cache,
                    &mut jump_target,
                    &mut ui_state,
                    &mut copy_text,
                    requested_faction,
                ),
                LeftPanelTab::Blackjack => {
                    crate::ui::blackjack::blackjack_content(
                        ui,
                        &mut panel_state.blackjack,
                        &mut factions.gold_storage,
                        &mut factions.reputation,
                        &world_data,
                    );
                }
                LeftPanelTab::Profiler => profiler_content(
                    ui,
                    &profiler.timings,
                    &profiler.target_thrash,
                    &mut profiler.migration,
                    &mut settings,
                    &mut profiler_cache,
                ),
                LeftPanelTab::Help => help_content(ui),
            }
        });

    // Queue patrol swap — applied in rebuild_patrol_routes_system which reads PatrolSwapMsg
    if let Some((a, b)) = patrol_swap {
        dirty_writers
            .patrols
            .write(crate::messages::PatrolsDirtyMsg);
        // PatrolSwapMsg is a separate message type — written directly via the system param below
        dirty_writers
            .patrol_swap
            .write(crate::messages::PatrolSwapMsg {
                slot_a: a,
                slot_b: b,
            });
    }

    // Apply camera jump from Factions panel
    if let Some(target) = jump_target {
        if let Ok(mut transform) = roster.camera_query.single_mut() {
            transform.translation.x = target.x;
            transform.translation.y = target.y;
        }
    }

    // Clipboard copy from Factions "Copy Debug" button
    if let Some(text) = copy_text {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            let _ = cb.set_text(text);
        }
    }

    if !open {
        ui_state.left_panel_open = false;
    }

    panel_state.prev_tab = if ui_state.left_panel_open {
        ui_state.left_panel_tab
    } else {
        LeftPanelTab::Roster
    };

    Ok(())
}

/// Save all left-panel state to settings file in a single write.
fn save_left_panel_state(
    ui_state: &UiState,
    settings: &UserSettings,
    policies: &TownPolicies,
    world_data: &WorldData,
) {
    let mut saved = settings::load_settings();
    saved.left_panel_tab = tab_to_str(ui_state.left_panel_tab).to_string();
    saved.upgrade_expanded = settings.upgrade_expanded.clone();
    saved.auto_upgrades = settings.auto_upgrades.clone();
    saved.show_terrain_sprites = settings.show_terrain_sprites;
    saved.collapsed_sections = settings.collapsed_sections.clone();
    // Save policies from player town
    let town_idx = world_data
        .towns
        .iter()
        .position(|t| t.faction == 0)
        .unwrap_or(0);
    if town_idx < policies.policies.len() {
        saved.policy = policies.policies[town_idx].clone();
    }
    settings::save_settings(&saved);
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
        for npc in roster.entity_map.iter_npcs() {
            if npc.dead {
                continue;
            }
            let idx = npc.slot;
            let meta = &roster.meta_cache.0[idx];
            // Player faction only unless debug
            if !debug_all && npc.faction != 0 {
                continue;
            }
            if state.job_filter >= 0 && meta.job != state.job_filter {
                continue;
            }
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
                name: meta.name.clone(),
                job: meta.job,
                level: meta.level,
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

// ============================================================================
// UPGRADE CONTENT
// ============================================================================

fn upgrade_content(
    ui: &mut egui::Ui,
    upgrade: &mut UpgradeParams,
    world_data: &WorldData,
    settings: &mut UserSettings,
) {
    let town_idx = world_data
        .towns
        .iter()
        .position(|t| t.faction == 0)
        .unwrap_or(0);
    let food = upgrade
        .food_storage
        .food
        .get(town_idx)
        .copied()
        .unwrap_or(0);
    let gold = upgrade
        .gold_storage
        .gold
        .get(town_idx)
        .copied()
        .unwrap_or(0);
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
        ui.label(
            egui::RichText::new(format!("Total: {}", total))
                .small()
                .strong(),
        );
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
            let state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                is_expanded,
            );
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
                            settings.auto_upgrades = upgrade.auto.flags[town_idx].clone();
                        }

                        // Label (dimmed when locked)
                        let label_text = egui::RichText::new(upg.label);
                        ui.label(if unlocked {
                            label_text
                        } else {
                            label_text.weak()
                        });

                        // Effect summary (now/next)
                        let (now, next) = upgrade_effect_summary(i, lv_i);
                        ui.label(
                            egui::RichText::new(format!("{} -> {}", now, next))
                                .small()
                                .weak(),
                        );

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
                                upgrade.queue.write(UpgradeMsg {
                                    town_idx,
                                    upgrade_idx: i,
                                });
                            }

                            ui.label(format!("Lv{}", lv_i));
                        });
                    });
                }
            });
            // Persist expand/collapse changes after body renders (borrow on ui released)
            let now_open = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                false,
            )
            .is_open();
            if now_open != is_expanded {
                if now_open {
                    settings.upgrade_expanded.push(branch.label.to_string());
                } else {
                    settings.upgrade_expanded.retain(|s| s != branch.label);
                }
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
    entity_map: &EntityMap,
    mining_policy: &mut MiningPolicy,
    dirty_writers: &mut crate::messages::DirtyWriters,
    jump_target: &mut Option<Vec2>,
    ai_state: &mut AiPlayerState,
) {
    let town_idx = world_data
        .towns
        .iter()
        .position(|t| t.faction == 0)
        .unwrap_or(0);

    if town_idx >= policies.policies.len() {
        policies.policies.resize(town_idx + 1, PolicySet::default());
    }
    let policy = &mut policies.policies[town_idx];

    if let Some(town) = world_data.towns.get(town_idx) {
        ui.small(format!("Town: {}", town.name));
        ui.separator();
    }

    // -- AI Manager --
    ui.label(egui::RichText::new("AI Manager").strong());

    if let Some(player) = ai_state
        .players
        .iter_mut()
        .find(|p| p.town_data_idx == town_idx)
    {
        ui.checkbox(&mut player.active, "Enable AI Manager")
            .on_hover_text("AI automatically builds and upgrades your town");

        if player.active {
            ui.checkbox(&mut player.build_enabled, "Auto-Build")
                .on_hover_text("AI places buildings");
            ui.checkbox(&mut player.upgrade_enabled, "Auto-Upgrade")
                .on_hover_text("AI purchases upgrades");

            let personalities = ["Aggressive", "Balanced", "Economic"];
            let mut idx = player.personality as usize;
            ui.horizontal(|ui| {
                ui.label("Strategy:");
                egui::ComboBox::from_id_salt("ai_personality")
                    .selected_text(personalities[idx])
                    .show_index(ui, &mut idx, personalities.len(), |i| personalities[i]);
            });
            player.personality = match idx {
                0 => AiPersonality::Aggressive,
                2 => AiPersonality::Economic,
                _ => AiPersonality::Balanced,
            };

            let road_styles = ["None", "Cardinal", "Grid 4", "Grid 5"];
            let mut rs_idx = player.road_style as usize;
            ui.horizontal(|ui| {
                ui.label("Roads:");
                egui::ComboBox::from_id_salt("ai_road_style")
                    .selected_text(road_styles[rs_idx])
                    .show_index(ui, &mut rs_idx, road_styles.len(), |i| road_styles[i]);
            });
            player.road_style = match rs_idx {
                0 => RoadStyle::None,
                1 => RoadStyle::Cardinal,
                3 => RoadStyle::Grid5,
                _ => RoadStyle::Grid4,
            };
        }
    }

    // -- General --
    ui.add_space(8.0);
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
            .show_index(ui, &mut archer_sched_idx, SCHEDULE_OPTIONS.len(), |i| {
                SCHEDULE_OPTIONS[i]
            });
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
            .show_index(ui, &mut archer_off_idx, OFF_DUTY_OPTIONS.len(), |i| {
                OFF_DUTY_OPTIONS[i]
            });
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
            .show_index(ui, &mut farmer_sched_idx, SCHEDULE_OPTIONS.len(), |i| {
                SCHEDULE_OPTIONS[i]
            });
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
            .show_index(ui, &mut farmer_off_idx, OFF_DUTY_OPTIONS.len(), |i| {
                OFF_DUTY_OPTIONS[i]
            });
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
        dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
    }

    if mining_policy.discovered_mines.len() <= town_idx {
        mining_policy
            .discovered_mines
            .resize(town_idx + 1, Vec::new());
    }

    let discovered = mining_policy.discovered_mines[town_idx].clone();
    let mut enabled_count = 0usize;
    for &slot in &discovered {
        if *mining_policy.mine_enabled.get(&slot).unwrap_or(&true) {
            enabled_count += 1;
        }
    }

    // Count auto-assigned miners per mine (keyed by mine slot)
    let mut assigned_per_mine: HashMap<usize, usize> = HashMap::new();
    for inst in entity_map.iter_kind_for_town(BuildingKind::MinerHome, town_idx as u32) {
        if inst.manual_mine {
            continue;
        }
        let Some(mine_pos) = inst.assigned_mine else {
            continue;
        };
        if let Some(mine_inst) = entity_map.find_by_position(mine_pos) {
            *assigned_per_mine.entry(mine_inst.slot).or_default() += 1;
        }
    }
    let assigned_auto: usize = assigned_per_mine.values().sum();

    ui.label(format!(
        "{}/{} mines enabled, {} miners assigned",
        enabled_count,
        discovered.len(),
        assigned_auto
    ));

    if discovered.is_empty() {
        ui.small("No discovered mines in radius.");
    } else {
        for (display_idx, &slot) in discovered.iter().enumerate() {
            let Some(mine_inst) = entity_map.get_instance(slot) else {
                continue;
            };
            let dist = mine_inst
                .position
                .distance(world_data.towns[town_idx].center);
            let mut enabled = *mining_policy.mine_enabled.get(&slot).unwrap_or(&true);
            let mine_name = crate::ui::gold_mine_name(display_idx);
            let assigned_here = assigned_per_mine.get(&slot).copied().unwrap_or(0);
            ui.horizontal(|ui| {
                if ui.checkbox(&mut enabled, "").changed() {
                    mining_policy.mine_enabled.insert(slot, enabled);
                    dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
                }
                if ui.button(mine_name).on_hover_text("Jump to mine").clicked() {
                    *jump_target = Some(mine_inst.position);
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
fn patrols_content(
    ui: &mut egui::Ui,
    world_data: &WorldData,
    entity_map: &EntityMap,
    jump_target: &mut Option<Vec2>,
) -> Option<(usize, usize)> {
    let town_pair_idx = world_data
        .towns
        .iter()
        .position(|t| t.faction == 0)
        .unwrap_or(0) as u32;

    if let Some(town) = world_data.towns.get(town_pair_idx as usize) {
        ui.small(format!("Town: {}", town.name));
    }

    // Collect waypoints for this town from EntityMap, sorted by patrol_order
    // Collect waypoints: (slot, patrol_order, position), sorted by patrol_order
    let mut posts: Vec<(usize, u32, Vec2)> = entity_map
        .iter_kind_for_town(BuildingKind::Waypoint, town_pair_idx)
        .map(|inst| (inst.slot, inst.patrol_order, inst.position))
        .collect();
    posts.sort_by_key(|(_, order, _)| *order);

    ui.label(format!("{} waypoints", posts.len()));
    ui.separator();

    let mut swap: Option<(usize, usize)> = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (list_idx, &(slot, order, pos)) in posts.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("#{}", order));
                    if ui
                        .button(format!("({:.0}, {:.0})", pos.x, pos.y))
                        .on_hover_text("Jump to this post")
                        .clicked()
                    {
                        *jump_target = Some(pos);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if list_idx + 1 < posts.len() {
                            if ui.small_button("Down").on_hover_text("Move down").clicked() {
                                swap = Some((slot, posts[list_idx + 1].0));
                            }
                        }
                        if list_idx > 0 {
                            if ui.small_button("Up").on_hover_text("Move up").clicked() {
                                swap = Some((slot, posts[list_idx - 1].0));
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

fn squads_content(
    ui: &mut egui::Ui,
    squad: &mut SquadParams,
    meta_cache: &NpcMetaCache,
    _world_data: &WorldData,
    dirty_writers: &mut crate::messages::DirtyWriters,
) {
    let selected = squad.squad_state.selected;

    // Squad list (player-owned only — AI squads are hidden from UI)
    for i in 0..squad.squad_state.squads.len() {
        if !squad.squad_state.squads[i].is_player() {
            continue;
        }
        let count = squad.squad_state.squads[i].members.len();
        let has_target = squad.squad_state.squads[i].target.is_some();
        let patrol_on = squad.squad_state.squads[i].patrol_enabled;
        let rest_on = squad.squad_state.squads[i].rest_when_tired;
        let is_selected = selected == i as i32;

        let target_str = if has_target { "target set" } else { "---" };
        let patrol_str = if patrol_on { "patrol:on" } else { "patrol:off" };
        let rest_str = if rest_on { "rest:on" } else { "rest:off" };
        let squad_name = if i == 0 { "Default Squad" } else { "Squad" };
        let label = format!(
            "{}. {} {}  [{}]  {}  {}  {}",
            i + 1,
            squad_name,
            i + 1,
            count,
            target_str,
            patrol_str,
            rest_str
        );

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
    ui.strong(format!(
        "{} {} — {} members",
        header_name,
        si + 1,
        member_count
    ));

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
    if ui
        .checkbox(&mut patrol_enabled, "Patrol when no target")
        .changed()
    {
        squad.squad_state.squads[si].patrol_enabled = patrol_enabled;
    }
    let mut rest_when_tired = squad.squad_state.squads[si].rest_when_tired;
    if ui
        .checkbox(&mut rest_when_tired, "Go home to rest when tired")
        .changed()
    {
        squad.squad_state.squads[si].rest_when_tired = rest_when_tired;
    }
    let mut hold_fire = squad.squad_state.squads[si].hold_fire;
    if ui
        .checkbox(&mut hold_fire, "Hold fire (attack on command only)")
        .changed()
    {
        squad.squad_state.squads[si].hold_fire = hold_fire;
    }

    ui.add_space(4.0);

    // Per-job recruit controls — one row per military NPC type from registry
    for def in crate::constants::NPC_REGISTRY.iter() {
        if !def.is_military {
            continue;
        }
        if def.job == Job::Raider {
            continue;
        }
        let job_id = def.job as i32;
        // Available units of this job in default squad (squad 0)
        let available: Vec<crate::components::EntityUid> = squad.squad_state.squads[0]
            .members
            .iter()
            .copied()
            .filter(|uid| {
                squad.entity_map.slot_for_uid(*uid).is_some_and(|slot| {
                    slot < meta_cache.0.len() && meta_cache.0[slot].job == job_id
                })
            })
            .collect();
        let avail_count = available.len();
        if avail_count == 0 && si == 0 {
            continue;
        }

        let (r, g, b) = def.ui_color;
        let label_color = egui::Color32::from_rgb(r, g, b);

        if si == 0 {
            ui.colored_label(
                label_color,
                format!("{}: {}", def.label_plural, avail_count),
            );
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    label_color,
                    format!("{}: {}", def.label_plural, avail_count),
                );
                for amount in [1usize, 2, 4, 8, 16, 32] {
                    if amount > avail_count {
                        break;
                    }
                    if ui.small_button(format!("+{}", amount)).clicked() {
                        let recruits: Vec<crate::components::EntityUid> =
                            available.iter().copied().take(amount).collect();
                        squad.squad_state.squads[0]
                            .members
                            .retain(|s| !recruits.contains(s));
                        for uid in recruits {
                            if !squad.squad_state.squads[si].members.contains(&uid) {
                                squad.squad_state.squads[si].members.push(uid);
                            }
                        }
                        let selected_len = squad.squad_state.squads[si].members.len();
                        let selected_target = squad.squad_state.squads[si].target_size;
                        squad.squad_state.squads[si].target_size =
                            selected_target.max(selected_len);
                        dirty_writers.squads.write(crate::messages::SquadsDirtyMsg);
                    }
                }
            });
        }
    }

    // Dismiss all
    if member_count > 0 {
        if ui.button("Dismiss All").clicked() {
            squad.squad_state.squads[si].members.clear();
            squad.squad_state.squads[si].target_size = 0;
            dirty_writers.squads.write(crate::messages::SquadsDirtyMsg);
        }
    }

    ui.separator();

    // Member list
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let members = &squad.squad_state.squads[si].members;
            for &uid in members {
                let Some(slot) = squad.entity_map.slot_for_uid(uid) else {
                    continue;
                };
                if slot >= meta_cache.0.len() {
                    continue;
                }
                let meta = &meta_cache.0[slot];
                if meta.name.is_empty() {
                    continue;
                }

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
    entity_map: &EntityMap,
    policies: &TownPolicies,
    mining_policy: &MiningPolicy,
    cache: &mut FactionsCache,
) {
    fn push_snapshot(
        factions: &FactionsParams,
        squad_state: &SquadState,
        world_data: &WorldData,
        entity_map: &EntityMap,
        policies: &TownPolicies,
        mining_policy: &MiningPolicy,
        cache: &mut FactionsCache,
        tdi: usize,
        kind_name: &'static str,
        personality_name: &'static str,
        personality: Option<AiPersonality>,
        last_actions: Vec<(String, i32, i32)>,
    ) {
        let ti = tdi as u32;
        let town_name = world_data
            .towns
            .get(tdi)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        let center = world_data
            .towns
            .get(tdi)
            .map(|t| t.center)
            .unwrap_or_default();
        let faction = world_data.towns.get(tdi).map(|t| t.faction).unwrap_or(0);

        let buildings = entity_map.building_counts(ti);

        let npcs: std::collections::HashMap<BuildingKind, usize> =
            crate::constants::BUILDING_REGISTRY
                .iter()
                .filter(|def| def.spawner.is_some())
                .map(|def| {
                    let count = factions
                        .entity_map
                        .iter_kind_for_town(def.kind, tdi as u32)
                        .filter(|i| i.npc_uid.is_some() && is_alive(i.position))
                        .count();
                    (def.kind, count)
                })
                .collect();

        let food = factions.food_storage.food.get(tdi).copied().unwrap_or(0);
        let gold = factions.gold_storage.gold.get(tdi).copied().unwrap_or(0);
        let (alive, dead, kills) = factions
            .faction_stats
            .stats
            .get(faction as usize)
            .map(|s| (s.alive, s.dead, s.kills))
            .unwrap_or((0, 0, 0));
        let upgrades = factions.upgrades.town_levels(tdi);
        let next_upgrade = UPGRADES.nodes.iter().enumerate().find_map(|(idx, node)| {
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
        let mining_radius = policy
            .map(|p| p.mining_radius)
            .unwrap_or(crate::constants::DEFAULT_MINING_RADIUS);
        let mines_in_radius = entity_map
            .iter_kind(BuildingKind::GoldMine)
            .filter(|inst| {
                (inst.position - center).length_squared() <= mining_radius * mining_radius
            })
            .count();
        let discovered = mining_policy.discovered_mines.get(tdi);
        let mines_discovered = discovered.map(|v| v.len()).unwrap_or(0);
        let mines_enabled = discovered
            .map(|v| {
                v.iter()
                    .filter(|&&slot| *mining_policy.mine_enabled.get(&slot).unwrap_or(&true))
                    .count()
            })
            .unwrap_or(0);
        let spawner_count = factions
            .entity_map
            .iter_instances()
            .filter(|i| {
                is_alive(i.position)
                    && i.town_idx == tdi as u32
                    && crate::constants::building_def(i.kind).spawner.is_some()
            })
            .count() as i32;
        let reserve_food = personality
            .map(|p| p.food_reserve_per_spawner() * spawner_count)
            .unwrap_or(0);

        let farmer_homes = buildings
            .get(&BuildingKind::FarmerHome)
            .copied()
            .unwrap_or(0);
        let miner_homes = buildings
            .get(&BuildingKind::MinerHome)
            .copied()
            .unwrap_or(0);
        let civilian_homes = farmer_homes + miner_homes;
        let archer_homes = buildings
            .get(&BuildingKind::ArcherHome)
            .copied()
            .unwrap_or(0);
        let crossbow_homes = buildings
            .get(&BuildingKind::CrossbowHome)
            .copied()
            .unwrap_or(0);
        let military_homes = archer_homes + crossbow_homes;
        let waypoints = buildings.get(&BuildingKind::Waypoint).copied().unwrap_or(0);

        let (food_desire, military_desire, food_desire_tip, military_desire_tip) = if let Some(p) =
            personality
        {
            let threat = entity_map
                .iter_kind_for_town(BuildingKind::Fountain, tdi as u32)
                .next()
                .map(|inst| inst.slot)
                .and_then(|slot| factions.gpu_state.threat_counts.get(slot).copied())
                .map(|packed| {
                    let enemies = (packed >> 16) as f32;
                    (enemies / 10.0).min(1.0)
                })
                .unwrap_or(0.0);
            let town_key = tdi as i32;
            let pop_alive = |job: Job| {
                factions
                    .pop_stats
                    .0
                    .get(&(job as i32, town_key))
                    .map(|ps| ps.alive)
                    .unwrap_or(0)
                    .max(0) as usize
            };
            let civilians = pop_alive(Job::Farmer) + pop_alive(Job::Miner);
            let military =
                pop_alive(Job::Archer) + pop_alive(Job::Fighter) + pop_alive(Job::Crossbow);
            let (food_desire, military_desire) = debug_food_military_desire(
                p,
                food,
                reserve_food,
                civilian_homes,
                military_homes,
                waypoints,
                threat,
                civilians,
                military,
            );

            let food_tip = format!(
                "Food desire (shared AI path)\nfood={food}, reserve={reserve_food}, civilians={civilians}, military={military}\n=> {:.0}%",
                food_desire * 100.0
            );
            let military_tip = format!(
                "Military desire (shared AI path)\n\
                 includes waypoint cap, threat, and population-ratio correction\n\
                 civilian_homes={civilian_homes} (farmer={farmer_homes} + miner={miner_homes}), military_homes={military_homes}, waypoints={waypoints}\n\
                 threat={threat:.2}, civilians={civilians}, military={military}\n\
                 => {:.0}%",
                military_desire * 100.0,
            );

            (
                Some(food_desire),
                Some(military_desire),
                food_tip,
                military_tip,
            )
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
            (
                None,
                "Not applicable: desire metrics are only computed for AI factions.".to_string(),
            )
        };

        // Economy desire = 1 - slot_fullness = empty_slots / total_slots (mirrors ai_player.rs).
        let (economy_desire, economy_desire_tip) = if personality.is_some() {
            let (empty, total, fullness) = factions
                .town_grids
                .grids
                .iter()
                .find(|tg| tg.town_data_idx == tdi)
                .map(|tg| {
                    let empty = crate::world::empty_slots(
                        tg,
                        center,
                        &factions.world_grid,
                        &factions.entity_map,
                    )
                    .len();
                    let (min_r, max_r, min_c, max_c) = crate::world::build_bounds(tg);
                    let total = ((max_r - min_r + 1) * (max_c - min_c + 1) - 1) as f32;
                    (empty, total, 1.0 - empty as f32 / total.max(1.0))
                })
                .unwrap_or((0, 0.0, 0.0));
            let ed = 1.0 - fullness;
            let tip = format!(
                "Economy desire = 1 - slot_fullness\nempty={empty}, total={total:.0}, fullness={fullness:.2}\n=> {:.0}%",
                ed * 100.0
            );
            (Some(ed), tip)
        } else {
            (
                None,
                "Not applicable: desire metrics are only computed for AI factions.".to_string(),
            )
        };

        let ai_player = factions
            .ai_state
            .players
            .iter()
            .find(|p| p.town_data_idx == tdi);
        let squads = squad_state
            .squads
            .iter()
            .enumerate()
            .filter_map(|(si, squad)| {
                let owned = match squad.owner {
                    SquadOwner::Player => faction == 0,
                    SquadOwner::Town(owner_tdi) => owner_tdi == tdi,
                };
                if !owned || squad.members.is_empty() {
                    return None;
                }

                let (commander_uid, commander_cooldown) = ai_player
                    .and_then(|p| p.squad_cmd.get(&si))
                    .map(|cmd| (cmd.building_uid, Some(cmd.cooldown)))
                    .unwrap_or((None, None));

                Some(SquadSnapshot {
                    squad_idx: si,
                    members: squad.members.len(),
                    target_size: squad.target_size,
                    patrol_enabled: squad.patrol_enabled,
                    rest_when_tired: squad.rest_when_tired,
                    target: squad.target,
                    commander_uid,
                    commander_cooldown,
                })
            })
            .collect();

        cache.snapshots.push(AiSnapshot {
            town_data_idx: tdi,
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
            economy_desire,
            food_desire_tip,
            military_desire_tip,
            gold_desire_tip,
            economy_desire_tip,
            center,
            squads,
            next_upgrade,
        });
    }

    cache.snapshots.clear();

    // Include player faction (faction 0) in Factions view.
    if let Some(player_tdi) = world_data.towns.iter().position(|t| t.faction == 0) {
        push_snapshot(
            factions,
            squad_state,
            world_data,
            entity_map,
            policies,
            mining_policy,
            cache,
            player_tdi,
            "Player",
            "Human",
            None,
            Vec::new(),
        );
    }

    for player in factions.ai_state.players.iter() {
        let tdi = player.town_data_idx;

        let kind_name = match player.kind {
            AiKind::Builder => "Builder",
            AiKind::Raider => "Raider",
        };

        let last_actions: Vec<(String, i32, i32)> =
            player.last_actions.iter().rev().cloned().collect();
        push_snapshot(
            factions,
            squad_state,
            world_data,
            entity_map,
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

fn build_faction_debug_string(snap: &AiSnapshot) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(2048);

    let _ = writeln!(
        s,
        "=== F{} {} [{} {}] ===",
        snap.faction, snap.town_name, snap.personality_name, snap.kind_name
    );
    let _ = writeln!(s, "Center: ({:.0}, {:.0})", snap.center.x, snap.center.y);

    let _ = writeln!(s, "\n--- Resources ---");
    let _ = writeln!(s, "Food: {} (reserve: {})", snap.food, snap.reserve_food);
    let _ = writeln!(s, "Gold: {}", snap.gold);

    let _ = writeln!(s, "\n--- Desires ---");
    let fd = snap
        .food_desire
        .map(|v| format!("{:.0}%", v * 100.0))
        .unwrap_or("-".into());
    let _ = writeln!(s, "Food: {} — {}", fd, snap.food_desire_tip);
    let md = snap
        .military_desire
        .map(|v| format!("{:.0}%", v * 100.0))
        .unwrap_or("-".into());
    let _ = writeln!(s, "Military: {} — {}", md, snap.military_desire_tip);
    let gd = snap
        .gold_desire
        .map(|v| format!("{:.0}%", v * 100.0))
        .unwrap_or("-".into());
    let _ = writeln!(s, "Gold: {} — {}", gd, snap.gold_desire_tip);
    let ed = snap
        .economy_desire
        .map(|v| format!("{:.0}%", v * 100.0))
        .unwrap_or("-".into());
    let _ = writeln!(s, "Economy: {} — {}", ed, snap.economy_desire_tip);

    let _ = writeln!(s, "\n--- Buildings ---");
    let mut bld_sorted: Vec<_> = snap.buildings.iter().collect();
    bld_sorted.sort_by_key(|(k, _)| format!("{:?}", k));
    for (kind, count) in &bld_sorted {
        let _ = writeln!(s, "{:?}: {}", kind, count);
    }

    let _ = writeln!(s, "\n--- NPCs (alive per spawner) ---");
    let mut npc_sorted: Vec<_> = snap.npcs.iter().collect();
    npc_sorted.sort_by_key(|(k, _)| format!("{:?}", k));
    for (kind, count) in &npc_sorted {
        let _ = writeln!(s, "{:?}: {}", kind, count);
    }

    let _ = writeln!(s, "\n--- Population ---");
    let _ = writeln!(
        s,
        "Alive: {}  Dead: {}  Kills: {}",
        snap.alive, snap.dead, snap.kills
    );

    let _ = writeln!(s, "\n--- Mining ---");
    let _ = writeln!(s, "Radius: {:.0}px", snap.mining_radius);
    let _ = writeln!(s, "Mines in radius: {}", snap.mines_in_radius);
    let _ = writeln!(
        s,
        "Discovered: {}  Enabled: {}",
        snap.mines_discovered, snap.mines_enabled
    );

    let _ = writeln!(s, "\n--- Upgrades ---");
    for (idx, &level) in snap.upgrades.iter().enumerate() {
        if level > 0 {
            if let Some(node) = UPGRADES.nodes.get(idx) {
                let _ = writeln!(s, "{}: {}", node.label, level);
            }
        }
    }
    if snap.upgrades.iter().all(|&l| l == 0) {
        let _ = writeln!(s, "(none)");
    }
    if let Some(next) = &snap.next_upgrade {
        let _ = writeln!(
            s,
            "Next: {} (cost: {}, affordable: {})",
            next.label, next.cost, next.affordable
        );
    }

    let _ = writeln!(s, "\n--- Squads ---");
    if snap.squads.is_empty() {
        let _ = writeln!(s, "(none)");
    } else {
        let mut squads = snap.squads.clone();
        squads.sort_by_key(|sq| sq.squad_idx);
        for (i, sq) in squads.iter().enumerate() {
            let role = if snap.faction == 0 {
                "MANUAL"
            } else if i == 0 {
                "DEF"
            } else if sq.target_size == 0 {
                "IDLE"
            } else {
                "ATK"
            };
            let target = if let Some(uid) = sq.commander_uid {
                format!("uid#{}", uid.0)
            } else if sq.target.is_some() {
                "Map target".into()
            } else {
                "None".into()
            };
            let cd = sq.commander_cooldown.unwrap_or(0.0).max(0.0);
            let _ = writeln!(
                s,
                "#{} [{}]: {}/{} target={} cd={:.1}s patrol={} rest={}",
                sq.squad_idx + 1,
                role,
                sq.members,
                sq.target_size,
                target,
                cd,
                sq.patrol_enabled,
                sq.rest_when_tired
            );
        }
    }

    let _ = writeln!(s, "\n--- Recent Actions ---");
    if snap.last_actions.is_empty() {
        let _ = writeln!(s, "(none)");
    } else {
        for (action, day, hour) in &snap.last_actions {
            let _ = writeln!(s, "D{} {:02}:00  {}", day, hour, action);
        }
    }

    // Upgrade stat multipliers
    let lv = &snap.upgrades;
    let _ = writeln!(s, "\n--- Stat Multipliers ---");
    for &(unit, stats) in &[
        (
            "Archer",
            &[
                "Hp",
                "Attack",
                "Range",
                "MoveSpeed",
                "AttackSpeed",
                "Alert",
                "Dodge",
            ] as &[&str],
        ),
        (
            "Fighter",
            &["Hp", "Attack", "MoveSpeed", "AttackSpeed", "Dodge"],
        ),
        (
            "Crossbow",
            &["Hp", "Attack", "Range", "MoveSpeed", "AttackSpeed"],
        ),
        ("Farmer", &["Hp", "MoveSpeed", "Yield"]),
        ("Miner", &["Hp", "MoveSpeed", "Yield"]),
        ("Town", &["Healing", "FountainRange", "Expansion"]),
    ] {
        let mults: Vec<String> = stats
            .iter()
            .filter_map(|stat_name| {
                let kind = match *stat_name {
                    "Hp" => UpgradeStatKind::Hp,
                    "Attack" => UpgradeStatKind::Attack,
                    "Range" => UpgradeStatKind::Range,
                    "MoveSpeed" => UpgradeStatKind::MoveSpeed,
                    "AttackSpeed" => UpgradeStatKind::AttackSpeed,
                    "Alert" => UpgradeStatKind::Alert,
                    "Dodge" => UpgradeStatKind::Dodge,
                    "Yield" => UpgradeStatKind::Yield,
                    "Healing" => UpgradeStatKind::Healing,
                    "FountainRange" => UpgradeStatKind::FountainRange,
                    "Expansion" => UpgradeStatKind::Expansion,
                    _ => return None,
                };
                let val = UPGRADES.stat_mult(lv, unit, kind);
                if (val - 1.0).abs() > 0.001
                    || matches!(
                        kind,
                        UpgradeStatKind::Expansion | UpgradeStatKind::FountainRange
                    )
                {
                    let level = UPGRADES.stat_level(lv, unit, kind);
                    if level > 0 || (val - 1.0).abs() > 0.001 {
                        Some(format!("{}={:.2}x", stat_name, val))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        if !mults.is_empty() {
            let _ = writeln!(s, "{}: {}", unit, mults.join(", "));
        }
    }

    s
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
    copy_text: &mut Option<String>,
    requested_faction: Option<i32>,
) {
    // Rebuild cache every 30 frames
    cache.frame_counter += 1;
    if cache.frame_counter % 30 == 1 || cache.snapshots.is_empty() {
        rebuild_factions_cache(
            factions,
            squad_state,
            world_data,
            &factions.entity_map,
            policies,
            mining_policy,
            cache,
        );
    }

    if cache.snapshots.is_empty() {
        ui.label("No AI settlements");
        return;
    }

    // Consume requested faction selection from click/double-click messages.
    if let Some(faction) = requested_faction {
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
                format!(
                    "F{} {} [{} {}]",
                    s.faction, s.town_name, s.personality_name, s.kind_name
                )
            })
            .show_ui(ui, |ui| {
                for (i, s) in cache.snapshots.iter().enumerate() {
                    let label = format!(
                        "F{} {} [{} {}]",
                        s.faction, s.town_name, s.personality_name, s.kind_name
                    );
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
    ui.colored_label(
        kind_color,
        format!(
            "F{} {} [{} {}]",
            snap.faction, snap.town_name, snap.personality_name, snap.kind_name
        ),
    );

    // Compact header: buttons + resources + population
    ui.horizontal(|ui| {
        if ui.small_button("Jump").clicked() {
            *jump_target = Some(snap.center);
        }
        if ui.small_button("Copy Debug").clicked() {
            *copy_text = Some(build_faction_debug_string(snap));
        }
        ui.label(format!("Food: {}", snap.food));
        ui.separator();
        ui.label(format!("Gold: {}", snap.gold));
        ui.separator();
        ui.label(format!(
            "Alive: {}  Dead: {}  Kills: {}",
            snap.alive, snap.dead, snap.kills
        ));
    });
    ui.separator();

    let fmt_desire = |v: Option<f32>| {
        v.map(|v| format!("{:.0}%", v * 100.0))
            .unwrap_or_else(|| "-".into())
    };

    // -- Desires --
    egui::CollapsingHeader::new("Desires")
        .default_open(true)
        .show(ui, |ui| {
            egui::Grid::new(format!("intel_desires_grid_{}", snap.faction))
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Food Desire").on_hover_text(&snap.food_desire_tip);
                    ui.label(fmt_desire(snap.food_desire))
                        .on_hover_text(&snap.food_desire_tip);
                    ui.end_row();

                    ui.label("Military Desire")
                        .on_hover_text(&snap.military_desire_tip);
                    ui.label(fmt_desire(snap.military_desire))
                        .on_hover_text(&snap.military_desire_tip);
                    ui.end_row();

                    ui.label("Gold Desire").on_hover_text(&snap.gold_desire_tip);
                    ui.label(fmt_desire(snap.gold_desire))
                        .on_hover_text(&snap.gold_desire_tip);
                    ui.end_row();

                    ui.label("Economy Desire")
                        .on_hover_text(&snap.economy_desire_tip);
                    ui.label(fmt_desire(snap.economy_desire))
                        .on_hover_text(&snap.economy_desire_tip);
                    ui.end_row();

                    ui.label("Reserve Food");
                    ui.label(format!("{}", snap.reserve_food));
                    ui.end_row();

                    if let Some(next) = &snap.next_upgrade {
                        ui.label("Next Upgrade");
                        ui.label(&next.label);
                        ui.end_row();

                        ui.label("Upgrade Cost");
                        let afford_color = if next.affordable {
                            egui::Color32::from_rgb(80, 190, 120)
                        } else {
                            egui::Color32::from_rgb(210, 95, 95)
                        };
                        ui.colored_label(
                            afford_color,
                            format!(
                                "{} ({})",
                                next.cost,
                                if next.affordable {
                                    "affordable"
                                } else {
                                    "too expensive"
                                }
                            ),
                        );
                        ui.end_row();
                    } else {
                        ui.label("Next Upgrade");
                        ui.label("None");
                        ui.end_row();
                    }
                });
        });

    let lv = &snap.upgrades;
    let npc = |k: BuildingKind| snap.npcs.get(&k).copied().unwrap_or(0);
    let bld = |k: BuildingKind| snap.buildings.get(&k).copied().unwrap_or(0);

    // -- Economy --
    egui::CollapsingHeader::new("Economy")
        .default_open(true)
        .show(ui, |ui| {
            let econ_spawners: Vec<_> = BUILDING_REGISTRY
                .iter()
                .filter(|d| d.display == DisplayCategory::Economy && d.spawner.is_some())
                .collect();
            let workforce: usize = econ_spawners.iter().map(|d| npc(d.kind)).sum();
            let parts: Vec<String> = econ_spawners
                .iter()
                .map(|d| {
                    format!(
                        "{} {}",
                        npc(d.kind),
                        npc_def(Job::from_i32(d.spawner.unwrap().job)).label_plural
                    )
                })
                .collect();
            ui.label(format!("Workforce: {} ({})", workforce, parts.join(" + ")));
            for def in &econ_spawners {
                let label = npc_def(Job::from_i32(def.spawner.unwrap().job)).label_plural;
                ui.label(format!("{}: {}/{}", label, npc(def.kind), bld(def.kind)));
            }
            ui.separator();

            ui.label("Buildings");
            for def in BUILDING_REGISTRY
                .iter()
                .filter(|d| d.display == DisplayCategory::Economy)
            {
                ui.label(format!("{}: {}", def.label, bld(def.kind)));
            }
            ui.separator();

            ui.label("Mining");
            ui.label(format!("Radius: {:.0}px", snap.mining_radius));
            ui.label(format!("Reserve Food: {}", snap.reserve_food));
            ui.label(format!("Mines in Radius: {}", snap.mines_in_radius));
            ui.label(format!(
                "Discovered: {}  Enabled: {}",
                snap.mines_discovered, snap.mines_enabled
            ));
        });

    // -- Policies --
    egui::CollapsingHeader::new("Policies")
        .default_open(false)
        .show(ui, |ui| {
            if let Some(policy) = policies.policies.get(snap.town_data_idx) {
                let schedule_label = |s: WorkSchedule| SCHEDULE_OPTIONS[s as usize];
                let off_duty_label = |o: OffDutyBehavior| OFF_DUTY_OPTIONS[o as usize];
                egui::Grid::new(format!("intel_policies_grid_{}", snap.faction))
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Eat Food");
                        ui.label(if policy.eat_food { "Yes" } else { "No" });
                        ui.end_row();

                        ui.label("Prioritize Healing");
                        ui.label(if policy.prioritize_healing {
                            "Yes"
                        } else {
                            "No"
                        });
                        ui.end_row();

                        ui.label("Recovery HP");
                        ui.label(format!("{:.0}%", policy.recovery_hp * 100.0));
                        ui.end_row();

                        ui.label("Archer Aggressive");
                        ui.label(if policy.archer_aggressive {
                            "Yes"
                        } else {
                            "No"
                        });
                        ui.end_row();

                        ui.label("Archer Leash");
                        ui.label(if policy.archer_leash { "Yes" } else { "No" });
                        ui.end_row();

                        ui.label("Archer Flee HP");
                        ui.label(format!("{:.0}%", policy.archer_flee_hp * 100.0));
                        ui.end_row();

                        ui.label("Archer Schedule");
                        ui.label(schedule_label(policy.archer_schedule));
                        ui.end_row();

                        ui.label("Archer Off-duty");
                        ui.label(off_duty_label(policy.archer_off_duty));
                        ui.end_row();

                        ui.label("Farmer Fight Back");
                        ui.label(if policy.farmer_fight_back {
                            "Yes"
                        } else {
                            "No"
                        });
                        ui.end_row();

                        ui.label("Farmer Flee HP");
                        ui.label(format!("{:.0}%", policy.farmer_flee_hp * 100.0));
                        ui.end_row();

                        ui.label("Farmer Schedule");
                        ui.label(schedule_label(policy.farmer_schedule));
                        ui.end_row();

                        ui.label("Farmer Off-duty");
                        ui.label(off_duty_label(policy.farmer_off_duty));
                        ui.end_row();

                        ui.label("Mining Radius");
                        ui.label(format!("{:.0}px", policy.mining_radius));
                        ui.end_row();
                    });
            } else {
                ui.label("No policy data for this faction.");
            }
        });

    // -- Military --
    egui::CollapsingHeader::new("Military")
        .default_open(true)
        .show(ui, |ui| {
            let mil_spawners: Vec<_> = BUILDING_REGISTRY
                .iter()
                .filter(|d| d.display == DisplayCategory::Military && d.spawner.is_some())
                .collect();
            let total_mil: usize = mil_spawners.iter().map(|d| npc(d.kind)).sum();
            let parts: Vec<String> = mil_spawners
                .iter()
                .map(|d| {
                    format!(
                        "{} {}",
                        npc(d.kind),
                        npc_def(Job::from_i32(d.spawner.unwrap().job)).label_plural
                    )
                })
                .collect();
            ui.label(format!("Force: {} ({})", total_mil, parts.join(" + ")));
            for def in &mil_spawners {
                let label = npc_def(Job::from_i32(def.spawner.unwrap().job)).label_plural;
                ui.label(format!("{}: {}/{}", label, npc(def.kind), bld(def.kind)));
            }
            ui.separator();

            ui.label("Buildings");
            for def in BUILDING_REGISTRY
                .iter()
                .filter(|d| d.display == DisplayCategory::Military)
            {
                ui.label(format!("{}: {}", def.label, bld(def.kind)));
            }
        });

    // -- Economy Stats (collapsed by default) --
    let archer_base = factions.combat_config.jobs.get(&Job::Archer);
    let fighter_base = factions.combat_config.jobs.get(&Job::Fighter);
    let crossbow_base = factions.combat_config.jobs.get(&Job::Crossbow);
    let crossbow_atk = npc_def(Job::Crossbow).attack_override.as_ref();
    let farmer_base = factions.combat_config.jobs.get(&Job::Farmer);
    let miner_base = factions.combat_config.jobs.get(&Job::Miner);
    let ranged_base = factions.combat_config.attacks.get(&BaseAttackType::Ranged);
    let melee_base = factions.combat_config.attacks.get(&BaseAttackType::Melee);

    let farmer_hp_mult = UPGRADES.stat_mult(lv, "Farmer", UpgradeStatKind::Hp);
    let farmer_speed_mult = UPGRADES.stat_mult(lv, "Farmer", UpgradeStatKind::MoveSpeed);
    let farm_yield_mult = UPGRADES.stat_mult(lv, "Farmer", UpgradeStatKind::Yield);
    let miner_hp_mult = UPGRADES.stat_mult(lv, "Miner", UpgradeStatKind::Hp);
    let miner_speed_mult = UPGRADES.stat_mult(lv, "Miner", UpgradeStatKind::MoveSpeed);
    let gold_yield_mult = UPGRADES.stat_mult(lv, "Miner", UpgradeStatKind::Yield);
    let healing_mult = UPGRADES.stat_mult(lv, "Town", UpgradeStatKind::Healing);
    let fountain_bonus =
        UPGRADES.stat_level(lv, "Town", UpgradeStatKind::FountainRange) as f32 * 24.0;
    let tower = resolve_town_tower_stats(lv);

    egui::CollapsingHeader::new("Economy Stats")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new(format!(
                "intel_economy_stats_grid_{}_{}",
                snap.faction, cache.selected_idx
            ))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                if let Some(base) = farmer_base {
                    ui.label("Farmer HP");
                    ui.label(format!(
                        "{:.0} -> {:.0}",
                        base.max_health,
                        base.max_health * farmer_hp_mult
                    ));
                    ui.end_row();
                    ui.label("Farmer Speed");
                    ui.label(format!(
                        "{:.0} -> {:.0}",
                        base.speed,
                        base.speed * farmer_speed_mult
                    ));
                    ui.end_row();
                }
                if let Some(base) = miner_base {
                    ui.label("Miner HP");
                    ui.label(format!(
                        "{:.0} -> {:.0}",
                        base.max_health,
                        base.max_health * miner_hp_mult
                    ));
                    ui.end_row();
                    ui.label("Miner Speed");
                    ui.label(format!(
                        "{:.0} -> {:.0}",
                        base.speed,
                        base.speed * miner_speed_mult
                    ));
                    ui.end_row();
                }
                ui.label("Food Yield");
                ui.label(format!("{:.0}% of base", farm_yield_mult * 100.0));
                ui.end_row();
                ui.label("Gold Yield");
                ui.label(format!("{:.0}% of base", gold_yield_mult * 100.0));
                ui.end_row();
                ui.label("Healing Rate");
                ui.label(format!(
                    "{:.1}/s -> {:.1}/s",
                    factions.combat_config.heal_rate,
                    factions.combat_config.heal_rate * healing_mult
                ));
                ui.end_row();
                ui.label("Tower/Heal Radius");
                ui.label(format!(
                    "{:.0}px -> {:.0}px",
                    factions.combat_config.heal_radius,
                    factions.combat_config.heal_radius + fountain_bonus
                ));
                ui.end_row();
                ui.label("Fountain Cooldown");
                ui.label(format!(
                    "{:.2}s -> {:.2}s",
                    FOUNTAIN_TOWER.cooldown, tower.cooldown
                ));
                ui.end_row();
                ui.label("Fountain Projectile Life");
                ui.label(format!(
                    "{:.2}s -> {:.2}s",
                    FOUNTAIN_TOWER.proj_lifetime, tower.proj_lifetime
                ));
                ui.end_row();
                ui.label("Build Area Expansion");
                ui.label(format!(
                    "+{}",
                    UPGRADES.stat_level(lv, "Town", UpgradeStatKind::Expansion)
                ));
                ui.end_row();
            });
        });

    // -- Military Stats (collapsed by default) --
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

    egui::CollapsingHeader::new("Military Stats")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new(format!(
                "intel_military_stats_grid_{}_{}",
                snap.faction, cache.selected_idx
            ))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                if let Some(base) = archer_base {
                    ui.label("HP (Archer)");
                    ui.label(format!(
                        "{:.0} -> {:.0}",
                        base.max_health,
                        base.max_health * archer_hp_mult
                    ));
                    ui.end_row();
                    ui.label("Damage (Archer)");
                    ui.label(format!(
                        "{:.1} -> {:.1}",
                        base.damage,
                        base.damage * archer_dmg_mult
                    ));
                    ui.end_row();
                    ui.label("Move Speed (Archer)");
                    ui.label(format!(
                        "{:.0} -> {:.0}",
                        base.speed,
                        base.speed * archer_speed_mult
                    ));
                    ui.end_row();
                }
                if let Some(base) = ranged_base {
                    ui.label("Detection Range (Archer)");
                    ui.label(format!(
                        "{:.0} -> {:.0}",
                        base.range,
                        base.range * archer_range_mult
                    ));
                    ui.end_row();
                    ui.label("Attack Cooldown (Archer)");
                    ui.label(format!(
                        "{:.2}s -> {:.2}s ({:.0}% faster)",
                        base.cooldown,
                        base.cooldown * archer_cd_mult,
                        archer_cd_reduction
                    ));
                    ui.end_row();
                }
                ui.label("Alert (Archer)");
                ui.label(format!("{:.0}% of base", archer_alert_mult * 100.0));
                ui.end_row();
                ui.label("Dodge (Archer)");
                ui.label(
                    if UPGRADES.stat_level(lv, "Archer", UpgradeStatKind::Dodge) > 0 {
                        "Unlocked"
                    } else {
                        "Locked"
                    },
                );
                ui.end_row();

                ui.separator();
                ui.separator();
                ui.end_row();

                if let Some(base) = fighter_base {
                    ui.label("HP (Fighter)");
                    ui.label(format!(
                        "{:.0} -> {:.0}",
                        base.max_health,
                        base.max_health * fighter_hp_mult
                    ));
                    ui.end_row();
                    ui.label("Damage (Fighter)");
                    ui.label(format!(
                        "{:.1} -> {:.1}",
                        base.damage,
                        base.damage * fighter_dmg_mult
                    ));
                    ui.end_row();
                    ui.label("Move Speed (Fighter)");
                    ui.label(format!(
                        "{:.0} -> {:.0}",
                        base.speed,
                        base.speed * fighter_speed_mult
                    ));
                    ui.end_row();
                }
                if let Some(base) = melee_base {
                    ui.label("Attack Cooldown (Fighter)");
                    ui.label(format!(
                        "{:.2}s -> {:.2}s ({:.0}% faster)",
                        base.cooldown,
                        base.cooldown * fighter_cd_mult,
                        fighter_cd_reduction
                    ));
                    ui.end_row();
                }
                ui.label("Dodge (Fighter)");
                ui.label(
                    if UPGRADES.stat_level(lv, "Fighter", UpgradeStatKind::Dodge) > 0 {
                        "Unlocked"
                    } else {
                        "Locked"
                    },
                );
                ui.end_row();

                ui.separator();
                ui.separator();
                ui.end_row();

                if let Some(base) = crossbow_base {
                    ui.label("HP (Crossbow)");
                    ui.label(format!(
                        "{:.0} -> {:.0}",
                        base.max_health,
                        base.max_health * xbow_hp_mult
                    ));
                    ui.end_row();
                    ui.label("Damage (Crossbow)");
                    ui.label(format!(
                        "{:.1} -> {:.1}",
                        base.damage,
                        base.damage * xbow_dmg_mult
                    ));
                    ui.end_row();
                    ui.label("Move Speed (Crossbow)");
                    ui.label(format!(
                        "{:.0} -> {:.0}",
                        base.speed,
                        base.speed * xbow_speed_mult
                    ));
                    ui.end_row();
                }
                if let Some(base) = crossbow_atk {
                    ui.label("Detection Range (Crossbow)");
                    ui.label(format!(
                        "{:.0} -> {:.0}",
                        base.range,
                        base.range * xbow_range_mult
                    ));
                    ui.end_row();
                    ui.label("Attack Cooldown (Crossbow)");
                    let xbow_cd_red = (1.0 - xbow_cd_mult) * 100.0;
                    ui.label(format!(
                        "{:.2}s -> {:.2}s ({:.0}% faster)",
                        base.cooldown,
                        base.cooldown * xbow_cd_mult,
                        xbow_cd_red
                    ));
                    ui.end_row();
                }
            });
        });

    // -- Squad Commander --
    egui::CollapsingHeader::new("Squad Commander")
        .default_open(true)
        .show(ui, |ui| {
            if snap.squads.is_empty() {
                ui.label("No squads with members.");
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

                ui.label(format!("Active squads: {}", squads.len()));
                if snap.faction == 0 {
                    ui.label("Commander: Manual");
                } else {
                    ui.label("Commander: AI");
                    ui.label(format!(
                        "Defense: {}  Offense: {}  Active attack squads: {}",
                        defense_archers, offense_archers, attack_squads_active
                    ));
                }

                egui::Grid::new(format!("intel_squads_grid_{}", snap.faction))
                    .striped(true)
                    .num_columns(7)
                    .show(ui, |ui| {
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
                            if squad.commander_uid.is_some() { state_bits.push("LOCK"); }
                            let state = if state_bits.is_empty() {
                                "-".to_string()
                            } else {
                                state_bits.join(" ")
                            };

                            let target = if let Some(uid) = squad.commander_uid {
                                format!("uid#{}", uid.0)
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
        });

    // -- Recent Actions --
    if !snap.last_actions.is_empty() {
        egui::CollapsingHeader::new("Recent Actions")
            .default_open(true)
            .show(ui, |ui| {
                for (action, day, hour) in &snap.last_actions {
                    ui.label(format!("D{} {:02}:00  {}", day, hour, action));
                }
            });
    }
}

// ============================================================================
// PROFILER CONTENT
// ============================================================================

fn profiler_content(
    ui: &mut egui::Ui,
    timings: &SystemTimings,
    target_thrash: &NpcTargetThrashDebug,
    migration: &mut MigrationState,
    user_settings: &mut UserSettings,
    cache: &mut ProfilerCache,
) {
    // Refresh cached data every 15 frames
    cache.frame_counter += 1;
    if cache.frame_counter % 15 == 1 || cache.game_entries.is_empty() {
        cache.frame_ms = timings.get_frame_ms();

        let traced = timings.get_traced_timings();
        cache.game_entries.clear();
        cache.engine_entries.clear();
        for (name, &ms) in &traced {
            if name.starts_with("endless::") {
                cache.game_entries.push((name.clone(), ms));
            } else {
                cache.engine_entries.push((name.clone(), ms));
            }
        }
        cache
            .game_entries
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        cache
            .engine_entries
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        cache.game_sum = cache.game_entries.iter().map(|(_, ms)| ms).sum();
        cache.engine_sum = cache.engine_entries.iter().map(|(_, ms)| ms).sum();
        cache.game_entries.truncate(10);
        cache.engine_entries.truncate(10);

        let render_timings = timings.get_timings();
        cache.render_entries.clear();
        cache
            .render_entries
            .extend(render_timings.iter().map(|(&n, &v)| (n.to_string(), v)));
        cache
            .render_entries
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let flips = target_thrash.top_offenders(8);
        cache.total_changes = flips.iter().map(|(_, c, _, _, _, _)| *c as u32).sum();
        cache.sink_window_key = target_thrash.sink_window_key;
        cache.top_flips = flips
            .into_iter()
            .map(|(idx, c, pp, rf, w, r)| (idx, c, pp, rf, w, r.to_string()))
            .collect();

        // Extract dirty counts from atomic globals
        use crate::messages::{DC_COUNT, DC_NAMES, EXTRACT_DIRTY_COUNTS};
        cache.dirty_counts.clear();
        for i in 0..DC_COUNT {
            let v = EXTRACT_DIRTY_COUNTS[i].load(std::sync::atomic::Ordering::Relaxed);
            cache.dirty_counts.push((DC_NAMES[i].to_string(), v));
        }
    }

    ui.label(egui::RichText::new(format!("Frame: {:.2} ms", cache.frame_ms)).strong());
    ui.separator();

    // Debug actions (not cached — cheap interactive widgets)
    egui::CollapsingHeader::new(egui::RichText::new("Debug Actions").strong())
        .default_open(false)
        .show(ui, |ui| {
            ui.checkbox(
                &mut user_settings.show_terrain_sprites,
                "Show Terrain Sprites",
            );
            ui.separator();
            let has_active = migration.active.is_some();
            let btn = ui.add_enabled(!has_active, egui::Button::new("Spawn Migration Group"));
            if btn.clicked() {
                migration.debug_spawn = true;
            }
            if has_active {
                let count = migration
                    .active
                    .as_ref()
                    .map(|g| g.member_slots.len())
                    .unwrap_or(0);
                ui.label(format!("Migration active: {} raiders", count));
            }
        });
    ui.separator();

    egui::CollapsingHeader::new(egui::RichText::new("NPC Target Thrash (sink, 1s window)").strong())
        .default_open(true)
        .show(ui, |ui| {
            ui.label(format!("Window key: {}", cache.sink_window_key));
            ui.label(format!("Top-8 sink target-change sum: {}", cache.total_changes));
            if cache.top_flips.is_empty() {
                ui.label("No target changes yet.");
            } else {
                if ui.button("Copy Thrash Top 8").clicked() {
                    let body = cache.top_flips.iter()
                        .map(|(idx, changes, ping_pong, reason_flips, writes, reason)| {
                            format!("#{idx}: sink_target_changes={changes} sink_ping_pong={ping_pong} reason_flips={reason_flips} sink_writes={writes} last={reason}")
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    ui.ctx().copy_text(format!("Window key: {}\n{}", cache.sink_window_key, body));
                }
                egui::Grid::new("target_thrash_grid").num_columns(6).striped(true).show(ui, |ui| {
                    ui.label(egui::RichText::new("npc").strong());
                    ui.label(egui::RichText::new("target changes").strong());
                    ui.label(egui::RichText::new("ping-pong").strong());
                    ui.label(egui::RichText::new("reason flips").strong());
                    ui.label(egui::RichText::new("writes").strong());
                    ui.label(egui::RichText::new("last reason").strong());
                    ui.end_row();
                    for (idx, changes, ping_pong, reason_flips, writes, reason) in &cache.top_flips {
                        ui.label(format!("#{idx}"));
                        ui.label(format!("{changes}"));
                        ui.label(format!("{ping_pong}"));
                        ui.label(format!("{reason_flips}"));
                        ui.label(format!("{writes}"));
                        ui.label(reason.as_str());
                        ui.end_row();
                    }
                });
            }
        });
    ui.separator();

    if cache.game_entries.is_empty() && cache.engine_entries.is_empty() {
        ui.label("Enable profiler in pause menu settings");
        return;
    }

    ui.label(
        egui::RichText::new("(cpu sums include parallel overlap)")
            .weak()
            .small(),
    );

    if ui.button("Copy Top 10").clicked() {
        let top_game: String = cache
            .game_entries
            .iter()
            .map(|(name, ms)| format!("{}: {:.3} ms", name, ms))
            .collect::<Vec<_>>()
            .join("\n");
        let top_engine: String = cache
            .engine_entries
            .iter()
            .map(|(name, ms)| format!("{}: {:.3} ms", name, ms))
            .collect::<Vec<_>>()
            .join("\n");
        let render: String = cache
            .render_entries
            .iter()
            .map(|(name, ms)| format!("{}: {:.3} ms", name, ms))
            .collect::<Vec<_>>()
            .join("\n");
        let dirty: String = cache
            .dirty_counts
            .iter()
            .map(|(name, count)| format!("{name}={count}"))
            .collect::<Vec<_>>()
            .join(" ");
        ui.ctx().copy_text(format!(
            "Frame: {:.2} ms\n\nGame Systems (cpu sum: {:.2} ms)\n{}\n\nEngine Systems (cpu sum: {:.2} ms)\n{}\n\nRender Pipeline\n{}\n\nExtract dirty: {}",
            cache.frame_ms, cache.game_sum, top_game, cache.engine_sum, top_engine, render, dirty
        ));
    }
    ui.separator();

    // Game systems (top 10, pre-sorted)
    egui::CollapsingHeader::new(format!("Game Systems ({:.2} ms)", cache.game_sum))
        .id_salt("prof_game")
        .default_open(true)
        .show(ui, |ui| {
            egui::Grid::new("prof_game_grid")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("system").strong());
                    ui.label(egui::RichText::new("ms").strong());
                    ui.end_row();
                    for (name, ms) in &cache.game_entries {
                        ui.label(name.as_str());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(format!("{:.3}", ms)).monospace());
                        });
                        ui.end_row();
                    }
                });
        });

    // Engine systems (top 10, pre-sorted)
    egui::CollapsingHeader::new(format!("Engine Systems ({:.2} ms)", cache.engine_sum))
        .id_salt("prof_engine")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("prof_engine_grid")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("system").strong());
                    ui.label(egui::RichText::new("ms").strong());
                    ui.end_row();
                    for (name, ms) in &cache.engine_entries {
                        ui.label(name.as_str());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(format!("{:.3}", ms)).monospace());
                        });
                        ui.end_row();
                    }
                });
        });
    ui.separator();

    // Render pipeline timings
    if !cache.render_entries.is_empty() {
        egui::CollapsingHeader::new(egui::RichText::new("Render Pipeline").strong())
            .id_salt("prof_render")
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("prof_render_grid")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("stage").strong());
                        ui.label(egui::RichText::new("ms").strong());
                        ui.end_row();
                        for (name, ms) in &cache.render_entries {
                            ui.label(name.as_str());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(egui::RichText::new(format!("{:.3}", ms)).monospace());
                                },
                            );
                            ui.end_row();
                        }
                    });
            });
    }

    // Extract dirty counts
    if !cache.dirty_counts.is_empty() {
        ui.separator();
        egui::CollapsingHeader::new(egui::RichText::new("Extract Dirty Counts").strong())
            .id_salt("prof_dirty")
            .default_open(true)
            .show(ui, |ui| {
                let total: u32 = cache.dirty_counts.iter().map(|(_, v)| v).sum();
                ui.label(format!("Total dirty indices: {total}"));
                egui::Grid::new("prof_dirty_grid")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("buffer").strong());
                        ui.label(egui::RichText::new("dirty").strong());
                        ui.end_row();
                        for (name, count) in &cache.dirty_counts {
                            ui.label(name.as_str());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(egui::RichText::new(format!("{count}")).monospace());
                                },
                            );
                            ui.end_row();
                        }
                    });
            });
    }
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
                ui.label("Space - pause    +/- - speed (0x, 0.25x-128x)");
                ui.label("B - Build   R - Roster   U - Upgrades   P - Policies");
                ui.label("T - Patrols   Q - Squads   I - Factions   L - Log   H - Help");
                ui.label("F - Follow NPC   1-9 - Squad target   ESC - Menu");
                ui.label("F5 - Quicksave   F9 - Quickload");
            });

        egui::CollapsingHeader::new(egui::RichText::new("Tips").strong())
            .show(ui, |ui| {
                ui.label("- Build farms before homes -- no farm, no work.");
                ui.label("- Waypoints between farms and enemy towns.");
                ui.label("- Day Only schedule (P) keeps farmers safe at night.");
                ui.label("- Upgrade Fountain Radius early for faster healing.");
            });
    });
}

// ============================================================================
// INVENTORY TAB
// ============================================================================

fn rarity_color(rarity: Rarity) -> egui::Color32 {
    let (r, g, b) = rarity.color();
    egui::Color32::from_rgb(r, g, b)
}

fn slot_label(slot: EquipmentSlot) -> &'static str {
    match slot {
        EquipmentSlot::Helm => "Helm",
        EquipmentSlot::Armor => "Armor",
        EquipmentSlot::Weapon => "Weapon",
        EquipmentSlot::Shield => "Shield",
        EquipmentSlot::Gloves => "Gloves",
        EquipmentSlot::Boots => "Boots",
        EquipmentSlot::Belt => "Belt",
        EquipmentSlot::Amulet => "Amulet",
        EquipmentSlot::Ring => "Ring",
    }
}

fn inventory_content(
    ui: &mut egui::Ui,
    inv: &mut InventoryParams,
    selected_npc: &SelectedNpc,
    meta_cache: &NpcMetaCache,
    entity_map: &EntityMap,
) {
    let town_idx: usize = 0; // player town

    // --- Selected NPC equipment section ---
    let sel = selected_npc.0;
    if sel >= 0 {
        if let Some(npc) = entity_map.get_npc(sel as usize) {
            if let Ok((equip, job, _town_id)) = inv.equipment_q.get(npc.entity) {
                let def = npc_def(*job);
                let name = meta_cache.0.get(sel as usize)
                    .map(|m| m.name.as_str())
                    .unwrap_or("NPC");
                ui.label(
                    egui::RichText::new(format!("{} — {:?}", name, job))
                        .strong()
                        .size(14.0),
                );

                if def.equip_slots.is_empty() {
                    ui.label("(non-military — cannot equip)");
                } else {
                    for &slot in ALL_EQUIPMENT_SLOTS {
                        if slot == EquipmentSlot::Ring {
                            for (ring_idx, ring) in
                                [&equip.ring1, &equip.ring2].iter().enumerate()
                            {
                                ui.horizontal(|ui| {
                                    let label = format!("Ring {}", ring_idx + 1);
                                    if let Some(item) = ring {
                                        ui.label(format!("{}:", label));
                                        ui.label(
                                            egui::RichText::new(&item.name)
                                                .color(rarity_color(item.rarity)),
                                        );
                                        ui.label(format!(
                                            "(+{:.0}%)",
                                            item.stat_bonus * 100.0
                                        ));
                                        if ui.small_button("Unequip").clicked() {
                                            inv.unequip_writer.write(
                                                crate::systems::stats::UnequipItemMsg {
                                                    npc_entity: npc.entity,
                                                    slot,
                                                    ring_index: ring_idx as u8,
                                                },
                                            );
                                        }
                                    } else {
                                        ui.label(format!("{}: —", label));
                                    }
                                });
                            }
                        } else {
                            ui.horizontal(|ui| {
                                let item_opt = equip.slot(slot);
                                ui.label(format!("{}:", slot_label(slot)));
                                if let Some(item) = item_opt {
                                    ui.label(
                                        egui::RichText::new(&item.name)
                                            .color(rarity_color(item.rarity)),
                                    );
                                    ui.label(format!("(+{:.0}%)", item.stat_bonus * 100.0));
                                    if ui.small_button("Unequip").clicked() {
                                        inv.unequip_writer.write(
                                            crate::systems::stats::UnequipItemMsg {
                                                npc_entity: npc.entity,
                                                slot,
                                                ring_index: 0,
                                            },
                                        );
                                    }
                                } else {
                                    ui.label("—");
                                }
                            });
                        }
                    }
                }
                ui.separator();
            }
        }
    } else {
        ui.label("Select a military NPC to equip items.");
        ui.separator();
    }

    // --- Town inventory list ---
    let items = inv
        .town_inventory
        .items
        .get(town_idx)
        .map(|v| v.as_slice())
        .unwrap_or(&[]);

    ui.label(
        egui::RichText::new(format!("Town Inventory ({} items)", items.len()))
            .strong()
            .size(14.0),
    );

    if items.is_empty() {
        ui.label("No unequipped items. Defeat raiders to collect loot.");
        return;
    }

    // Can equip if a valid military NPC from player town is selected
    let npc_entity = if sel >= 0 {
        entity_map.get_npc(sel as usize).and_then(|npc| {
            inv.equipment_q.get(npc.entity).ok().and_then(|(_, job, tid)| {
                let def = npc_def(*job);
                if tid.0 == town_idx as i32 && !def.equip_slots.is_empty() {
                    Some(npc.entity)
                } else {
                    None
                }
            })
        })
    } else {
        None
    };

    egui::ScrollArea::vertical()
        .max_height(400.0)
        .show(ui, |ui| {
            for item in items {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&item.name)
                            .color(rarity_color(item.rarity)),
                    );
                    ui.label(format!(
                        "{} +{:.0}%",
                        slot_label(item.slot),
                        item.stat_bonus * 100.0
                    ));
                    if let Some(ent) = npc_entity {
                        if ui.small_button("Equip").clicked() {
                            inv.equip_writer.write(
                                crate::systems::stats::EquipItemMsg {
                                    npc_entity: ent,
                                    item_id: item.id,
                                    town_idx,
                                },
                            );
                        }
                    }
                });
            }
        });
}
