//! Right panel — tabbed container for Roster, Upgrades, and Policies.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui;

use crate::components::*;
use crate::resources::*;
use crate::systems::stats::{TownUpgrades, UpgradeQueue, UPGRADE_COUNT, upgrade_cost};
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
    UpgradeDef { label: "Farmer Cap",      tooltip: "+2 max farmers per level",                 category: "Farm" },
    UpgradeDef { label: "Guard Cap",       tooltip: "+10 max guards per level",                 category: "Guard" },
    UpgradeDef { label: "Healing Rate",    tooltip: "+20% HP regen at fountain per level",      category: "Town" },
    UpgradeDef { label: "Food Efficiency", tooltip: "10% chance per level to not consume food", category: "Town" },
    UpgradeDef { label: "Fountain Radius", tooltip: "+24px fountain healing range per level",   category: "Town" },
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
    ), Without<Dead>>,
    camera_query: Query<'w, 's, &'static mut Transform, With<crate::render::MainCamera>>,
    gpu_state: Res<'w, GpuReadState>,
    reassign_queue: ResMut<'w, ReassignQueue>,
}

#[derive(SystemParam)]
pub struct UpgradeParams<'w> {
    food_storage: Res<'w, FoodStorage>,
    faction_stats: Res<'w, FactionStats>,
    upgrades: Res<'w, TownUpgrades>,
    queue: ResMut<'w, UpgradeQueue>,
}

// ============================================================================
// MAIN SYSTEM
// ============================================================================

pub fn right_panel_system(
    mut contexts: bevy_egui::EguiContexts,
    mut ui_state: ResMut<UiState>,
    world_data: Res<WorldData>,
    mut policies: ResMut<TownPolicies>,
    mut roster: RosterParams,
    mut upgrade: UpgradeParams,
    mut roster_state: Local<RosterState>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    // Panel width: narrow for just tabs, wide when content is open
    let width = if ui_state.right_panel_open { 340.0 } else { 200.0 };

    egui::SidePanel::right("right_panel")
        .exact_width(width)
        .show(ctx, |ui| {
            // Tab bar — always visible
            let open = ui_state.right_panel_open;
            let tab = ui_state.right_panel_tab;

            ui.horizontal(|ui| {
                for (label, variant) in [
                    ("Roster (R)", RightPanelTab::Roster),
                    ("Upgrades (U)", RightPanelTab::Upgrades),
                    ("Policies (P)", RightPanelTab::Policies),
                ] {
                    let active = open && tab == variant;
                    if ui.selectable_label(active, label).clicked() {
                        ui_state.toggle_right_tab(variant);
                    }
                }
            });

            if !ui_state.right_panel_open { return; }

            ui.separator();

            match ui_state.right_panel_tab {
                RightPanelTab::Roster => roster_content(ui, &mut roster, &mut roster_state),
                RightPanelTab::Upgrades => upgrade_content(ui, &mut upgrade, &world_data),
                RightPanelTab::Policies => policies_content(ui, &mut policies, &world_data),
            }
        });

    Ok(())
}

// ============================================================================
// ROSTER CONTENT
// ============================================================================

fn roster_content(ui: &mut egui::Ui, roster: &mut RosterParams, state: &mut RosterState) {
    // Rebuild cache every 30 frames
    state.frame_counter += 1;
    if state.frame_counter % 30 == 1 || state.cached_rows.is_empty() {
        let mut rows = Vec::new();
        for (npc_idx, health, cached, activity, combat) in roster.health_query.iter() {
            let idx = npc_idx.0;
            let meta = &roster.meta_cache.0[idx];
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
        if ui.selectable_label(state.job_filter == 2, "Raiders").clicked() {
            state.job_filter = 2;
            state.frame_counter = 0;
        }
    });

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
        let mut reassigns: Vec<(usize, i32)> = Vec::new();

        for row in &state.cached_rows {
            let is_selected = selected_idx == row.slot as i32;
            let job_color = match row.job {
                0 => egui::Color32::from_rgb(80, 200, 80),
                1 => egui::Color32::from_rgb(80, 100, 220),
                2 => egui::Color32::from_rgb(220, 80, 80),
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

                match row.job {
                    0 => {
                        if ui.small_button("→G").on_hover_text("Reassign to Guard").clicked() {
                            reassigns.push((row.slot, 1));
                        }
                    }
                    1 => {
                        if ui.small_button("→F").on_hover_text("Reassign to Farmer").clicked() {
                            reassigns.push((row.slot, 0));
                        }
                    }
                    _ => {}
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

        if !reassigns.is_empty() {
            roster.reassign_queue.0.extend(reassigns);
            state.frame_counter = 0;
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

    ui.label(egui::RichText::new("General").strong());
    ui.checkbox(&mut policy.eat_food, "Eat Food")
        .on_hover_text("NPCs consume food to restore HP and energy");
    ui.checkbox(&mut policy.prioritize_healing, "Prioritize Healing")
        .on_hover_text("Wounded NPCs go to fountain before resuming work");

    ui.add_space(4.0);
    ui.label(egui::RichText::new("Guard Behavior").strong());
    ui.checkbox(&mut policy.guard_aggressive, "Aggressive")
        .on_hover_text("Guards never flee combat");
    ui.checkbox(&mut policy.guard_leash, "Leash")
        .on_hover_text("Guards return home if too far from post");

    ui.add_space(4.0);
    ui.label(egui::RichText::new("Farmer Behavior").strong());
    ui.checkbox(&mut policy.farmer_fight_back, "Fight Back")
        .on_hover_text("Farmers attack enemies instead of fleeing");

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Thresholds").strong());

    let mut farmer_flee_pct = policy.farmer_flee_hp * 100.0;
    let mut guard_flee_pct = policy.guard_flee_hp * 100.0;
    let mut recovery_pct = policy.recovery_hp * 100.0;

    ui.horizontal(|ui| {
        ui.label("Farmer flee HP:");
        ui.add(egui::Slider::new(&mut farmer_flee_pct, 0.0..=100.0).suffix("%"));
    });
    ui.horizontal(|ui| {
        ui.label("Guard flee HP:");
        ui.add(egui::Slider::new(&mut guard_flee_pct, 0.0..=100.0).suffix("%"));
    });
    ui.horizontal(|ui| {
        ui.label("Recovery HP:");
        ui.add(egui::Slider::new(&mut recovery_pct, 0.0..=100.0).suffix("%"));
    });

    policy.farmer_flee_hp = farmer_flee_pct / 100.0;
    policy.guard_flee_hp = guard_flee_pct / 100.0;
    policy.recovery_hp = recovery_pct / 100.0;

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Schedules").strong());

    let mut schedule_idx = policy.work_schedule as usize;
    let mut farmer_off_idx = policy.farmer_off_duty as usize;
    let mut guard_off_idx = policy.guard_off_duty as usize;

    ui.horizontal(|ui| {
        ui.label("Work schedule:");
        egui::ComboBox::from_id_salt("work_schedule")
            .selected_text(SCHEDULE_OPTIONS[schedule_idx])
            .show_index(ui, &mut schedule_idx, SCHEDULE_OPTIONS.len(), |i| SCHEDULE_OPTIONS[i]);
    });
    ui.horizontal(|ui| {
        ui.label("Farmer off-duty:");
        egui::ComboBox::from_id_salt("farmer_off_duty")
            .selected_text(OFF_DUTY_OPTIONS[farmer_off_idx])
            .show_index(ui, &mut farmer_off_idx, OFF_DUTY_OPTIONS.len(), |i| OFF_DUTY_OPTIONS[i]);
    });
    ui.horizontal(|ui| {
        ui.label("Guard off-duty:");
        egui::ComboBox::from_id_salt("guard_off_duty")
            .selected_text(OFF_DUTY_OPTIONS[guard_off_idx])
            .show_index(ui, &mut guard_off_idx, OFF_DUTY_OPTIONS.len(), |i| OFF_DUTY_OPTIONS[i]);
    });

    policy.work_schedule = match schedule_idx {
        1 => WorkSchedule::DayOnly,
        2 => WorkSchedule::NightOnly,
        _ => WorkSchedule::Both,
    };
    policy.farmer_off_duty = match farmer_off_idx {
        1 => OffDutyBehavior::StayAtFountain,
        2 => OffDutyBehavior::WanderTown,
        _ => OffDutyBehavior::GoToBed,
    };
    policy.guard_off_duty = match guard_off_idx {
        1 => OffDutyBehavior::StayAtFountain,
        2 => OffDutyBehavior::WanderTown,
        _ => OffDutyBehavior::GoToBed,
    };
}
