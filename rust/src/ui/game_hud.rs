//! In-game HUD — top resource bar, bottom panel (inspector + combat log), target overlay.

use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::components::*;
use crate::constants::{
    EffectDisplay, ResourceKind, UpgradeStatKind, WALL_TIER_HP,
    WALL_TIER_NAMES, WALL_UPGRADE_COSTS, building_def, npc_def,
};
use crate::gpu::EntityGpuState;
use crate::render::MainCamera;
use crate::resources::*;
use crate::settings::{self, UserSettings};
use crate::systems::stats::{
    CombatConfig, TownUpgrades, UPGRADES, UnequipItemMsg, level_from_xp,
    resolve_tower_instance_stats, resolve_town_tower_stats,
};
use crate::ui::tipped;
use crate::world::{BuildingKind, WorldData, WorldGrid};

// ============================================================================
// DIRECT-CONTROL HELPERS
// ============================================================================

/// Collect alive direct-control NPC slots from the selected player squad.
/// Only includes NPCs with `NpcFlags.direct_control == true` (set by box-select).
/// O(squad_size) instead of O(all_npcs) — avoids scanning entire EntityMap.
fn dc_slots(squad_state: &SquadState, entity_map: &EntityMap, is_dc: impl Fn(Entity) -> bool) -> Vec<usize> {
    let si = squad_state.selected;
    if si < 0 {
        return Vec::new();
    }
    let Some(squad) = squad_state.squads.get(si as usize) else {
        return Vec::new();
    };
    if !squad.is_player() {
        return Vec::new();
    }
    squad
        .members
        .iter()
        .filter_map(|uid| entity_map.slot_for_uid(*uid))
        .filter(|&slot| entity_map.get_npc(slot).is_some_and(|n| !n.dead))
        .filter(|&slot| {
            entity_map.get_npc(slot).is_some_and(|n| is_dc(n.entity))
        })
        .collect()
}

// ============================================================================
// INSPECTOR LINK HELPERS
// ============================================================================

/// Action returned by inspector UI when user clicks an entity link.
#[allow(dead_code)]
enum InspectorAction {
    SelectNpc(i32),
    SelectBuilding(usize),
}

/// Render an NPC name as a clickable link. Returns action if clicked.
fn npc_link(ui: &mut egui::Ui, meta_cache: &NpcMetaCache, slot: usize) -> Option<InspectorAction> {
    if slot < meta_cache.0.len() {
        let meta = &meta_cache.0[slot];
        if ui
            .link(format!("{} (Lv.{})", meta.name, meta.level))
            .clicked()
        {
            return Some(InspectorAction::SelectNpc(slot as i32));
        }
    }
    None
}

/// Render a building name as a clickable link. Returns action if clicked.
fn building_link(ui: &mut egui::Ui, label: &str, slot: usize) -> Option<InspectorAction> {
    if ui.link(label).clicked() {
        Some(InspectorAction::SelectBuilding(slot))
    } else {
        None
    }
}

/// Apply an inspector action: select entity, deselect the other, jump camera.
fn apply_inspector_action(
    action: InspectorAction,
    selected_npc: &mut SelectedNpc,
    selected_building: &mut SelectedBuilding,
    gpu_state: &GpuReadState,
    entity_map: &EntityMap,
    grid: &WorldGrid,
    camera_query: &mut Query<&mut Transform, With<MainCamera>>,
) {
    match action {
        InspectorAction::SelectNpc(slot) => {
            selected_npc.0 = slot;
            selected_building.active = false;
            selected_building.slot = None;
            selected_building.kind = None;
            let idx = slot as usize;
            if idx * 2 + 1 < gpu_state.positions.len() {
                let x = gpu_state.positions[idx * 2];
                let y = gpu_state.positions[idx * 2 + 1];
                if let Ok(mut t) = camera_query.single_mut() {
                    t.translation.x = x;
                    t.translation.y = y;
                }
            }
        }
        InspectorAction::SelectBuilding(slot) => {
            selected_npc.0 = -1;
            if let Some(inst) = entity_map.get_instance(slot) {
                let (col, row) = grid.world_to_grid(inst.position);
                *selected_building = SelectedBuilding {
                    col,
                    row,
                    active: true,
                    slot: Some(slot),
                    kind: Some(inst.kind),
                };
                if let Ok(mut t) = camera_query.single_mut() {
                    t.translation.x = inst.position.x;
                    t.translation.y = inst.position.y;
                }
            }
        }
    }
}

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
    entity_map: Res<EntityMap>,
    catalog: Res<HelpCatalog>,
    time: Res<Time>,
    mut avg_fps: Local<f32>,
    mut ups: ResMut<UpsCounter>,
    mut ups_elapsed: Local<f32>,
    settings: Res<crate::settings::UserSettings>,
    mut camera_query: Query<&mut Transform, With<MainCamera>>,
    llm_state: Option<Res<crate::systems::llm_player::LlmPlayerState>>,
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
                if ui
                    .selectable_label(
                        ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Roster,
                        "Roster",
                    )
                    .clicked()
                {
                    ui_state.toggle_left_tab(LeftPanelTab::Roster);
                }
                if ui
                    .selectable_label(
                        ui_state.tech_tree_open,
                        "Tech Tree",
                    )
                    .clicked()
                {
                    ui_state.tech_tree_open = !ui_state.tech_tree_open;
                }
                if ui
                    .selectable_label(
                        ui_state.left_panel_open
                            && ui_state.left_panel_tab == LeftPanelTab::Policies,
                        "Policies",
                    )
                    .clicked()
                {
                    ui_state.toggle_left_tab(LeftPanelTab::Policies);
                }
                if ui
                    .selectable_label(
                        ui_state.left_panel_open
                            && ui_state.left_panel_tab == LeftPanelTab::Patrols,
                        "Patrols",
                    )
                    .clicked()
                {
                    ui_state.toggle_left_tab(LeftPanelTab::Patrols);
                }
                if ui
                    .selectable_label(
                        ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Squads,
                        "Squads",
                    )
                    .clicked()
                {
                    ui_state.toggle_left_tab(LeftPanelTab::Squads);
                }
                if ui
                    .selectable_label(
                        ui_state.left_panel_open
                            && ui_state.left_panel_tab == LeftPanelTab::Inventory,
                        "Armory",
                    )
                    .clicked()
                {
                    ui_state.toggle_left_tab(LeftPanelTab::Inventory);
                }
                if ui
                    .selectable_label(
                        ui_state.left_panel_open
                            && ui_state.left_panel_tab == LeftPanelTab::Factions,
                        "Factions",
                    )
                    .clicked()
                {
                    ui_state.toggle_left_tab(LeftPanelTab::Factions);
                }
                if ui
                    .selectable_label(
                        ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Help,
                        "Help",
                    )
                    .clicked()
                {
                    ui_state.toggle_left_tab(LeftPanelTab::Help);
                }
                if settings.debug_profiler {
                    if ui
                        .selectable_label(
                            ui_state.left_panel_open
                                && ui_state.left_panel_tab == LeftPanelTab::Profiler,
                            "Profiler",
                        )
                        .clicked()
                    {
                        ui_state.toggle_left_tab(LeftPanelTab::Profiler);
                    }
                }

                // CENTER: town name + time (painted at true center of bar)
                let town_name = world_data
                    .towns
                    .first()
                    .map(|t| t.name.as_str())
                    .unwrap_or("Unknown");
                let period = if game_time.is_daytime() {
                    "Day"
                } else {
                    "Night"
                };
                let center_text = format!(
                    "{}  -  Day {} {:02}:{:02} ({}) {:.0}x{}",
                    town_name,
                    game_time.day(),
                    game_time.hour(),
                    game_time.minute(),
                    period,
                    game_time.time_scale,
                    if game_time.is_paused() {
                        " [PAUSED]"
                    } else {
                        ""
                    }
                );
                let galley = ui.painter().layout_no_wrap(
                    center_text.clone(),
                    egui::FontId::default(),
                    ui.style().visuals.text_color(),
                );
                let center = ui.max_rect().center();
                let text_rect = egui::Rect::from_center_size(center, galley.size());
                let center_id = ui.make_persistent_id("top_bar_center_town_name");
                let center_resp = ui.interact(
                    text_rect.expand2(egui::vec2(6.0, 4.0)),
                    center_id,
                    egui::Sense::click(),
                );
                if center_resp.double_clicked() {
                    if let (Some(town), Ok(mut cam)) =
                        (world_data.towns.first(), camera_query.single_mut())
                    {
                        cam.translation.x = town.center.x;
                        cam.translation.y = town.center.y;
                    }
                }
                ui.painter().galley(
                    text_rect.left_top(),
                    galley,
                    ui.style().visuals.text_color(),
                );

                // RIGHT: stats pushed to the right edge
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // FPS + UPS (far right)
                    let dt = time.delta_secs();
                    if dt > 0.0 {
                        let fps = 1.0 / dt;
                        *avg_fps = if *avg_fps == 0.0 {
                            fps
                        } else {
                            *avg_fps * 0.95 + fps * 0.05
                        };
                    }
                    // Sample UPS once per wall-clock second
                    *ups_elapsed += dt;
                    if *ups_elapsed >= 1.0 {
                        ups.display_ups = ups.ticks_this_second;
                        ups.ticks_this_second = 0;
                        *ups_elapsed -= 1.0;
                    }
                    ui.label(
                        egui::RichText::new(format!("FPS: {:.0}", *avg_fps))
                            .size(12.0)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(format!("UPS: {}", ups.display_ups))
                            .size(12.0)
                            .strong(),
                    );

                    // LLM status indicator — painted circle with color coding
                    if let Some(ref llm) = llm_state {
                        use crate::systems::llm_player::LlmStatus;
                        let (color, tip) = match llm.status {
                            LlmStatus::Idle => (
                                egui::Color32::from_rgb(80, 80, 80),
                                "LLM idle",
                            ),
                            LlmStatus::Sending => (
                                egui::Color32::from_rgb(80, 180, 255),
                                "LLM sending state",
                            ),
                            LlmStatus::Thinking => (
                                egui::Color32::from_rgb(255, 200, 50),
                                "LLM thinking...",
                            ),
                            LlmStatus::Done(n) => (
                                egui::Color32::from_rgb(80, 220, 120),
                                if n > 0 { "LLM executed actions" } else { "LLM no actions" },
                            ),
                        };
                        let size = 8.0;
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(size + 4.0, size + 4.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().circle_filled(rect.center(), size * 0.5, color);
                        resp.on_hover_text(tip);
                    }

                    ui.separator();

                    // Player stats (right-aligned) — player's town is index 0
                    let town_food = food_storage.food.first().copied().unwrap_or(0);
                    let town_gold = gold_storage.gold.first().copied().unwrap_or(0);
                    tipped(
                        ui,
                        egui::RichText::new(format!("Gold: {}", town_gold))
                            .color(egui::Color32::from_rgb(220, 190, 50)),
                        catalog.0.get("gold").unwrap_or(&""),
                    );
                    tipped(
                        ui,
                        format!("Food: {}", town_food),
                        catalog.0.get("food").unwrap_or(&""),
                    );

                    let farmers = pop_stats.0.get(&(0, 0)).map(|s| s.alive).unwrap_or(0);
                    let guards = pop_stats.0.get(&(1, 0)).map(|s| s.alive).unwrap_or(0);
                    let crossbows = pop_stats.0.get(&(5, 0)).map(|s| s.alive).unwrap_or(0);
                    let houses = entity_map.count_for_town(BuildingKind::FarmerHome, 0);
                    let barracks = entity_map.count_for_town(BuildingKind::ArcherHome, 0);
                    let xbow_homes = entity_map.count_for_town(BuildingKind::CrossbowHome, 0);
                    tipped(
                        ui,
                        format!("Archers: {}/{}", guards, barracks),
                        catalog.0.get("archers").unwrap_or(&""),
                    );
                    tipped(
                        ui,
                        format!("Crossbow: {}/{}", crossbows, xbow_homes),
                        catalog.0.get("crossbow").unwrap_or(&""),
                    );
                    tipped(
                        ui,
                        format!("Farmers: {}/{}", farmers, houses),
                        catalog.0.get("farmers").unwrap_or(&""),
                    );
                    let total_alive: i32 = pop_stats.0.values().map(|s| s.alive).sum();
                    let total_spawners: usize = entity_map
                        .iter_instances()
                        .filter(|i| crate::constants::building_def(i.kind).spawner.is_some())
                        .count();
                    tipped(
                        ui,
                        format!("Pop: {}/{}", total_alive, total_spawners),
                        catalog.0.get("pop").unwrap_or(&""),
                    );
                });
            });
        });

    Ok(())
}

// ============================================================================
// BOTTOM PANEL (INSPECTOR + COMBAT LOG)
// ============================================================================

/// Bundled readonly resources for bottom panel.
#[derive(SystemParam)]
pub struct BottomPanelData<'w> {
    game_time: Res<'w, GameTime>,
    npc_logs: Res<'w, NpcLogCache>,
    selected: ResMut<'w, SelectedNpc>,
    combat_log: ResMut<'w, CombatLog>,
}

/// Bundled resources for building inspector.
#[derive(SystemParam)]
pub struct BuildingInspectorData<'w, 's> {
    selected_building: ResMut<'w, SelectedBuilding>,
    grid: Res<'w, WorldGrid>,
    food_storage: ResMut<'w, FoodStorage>,
    gold_storage: ResMut<'w, GoldStorage>,
    combat_config: Res<'w, CombatConfig>,
    town_upgrades: Res<'w, TownUpgrades>,
    entity_map: ResMut<'w, EntityMap>,
    building_health: Query<'w, 's, &'static mut Health, With<Building>>,
    pub npc_flags_q: Query<'w, 's, &'static mut NpcFlags>,
    pub squad_id_q: Query<'w, 's, &'static SquadId>,
    pub manual_target_q: Query<'w, 's, &'static ManualTarget>,
    pub activity_q: Query<'w, 's, &'static Activity>,
    pub npc_health_q: Query<'w, 's, &'static Health, Without<Building>>,
    pub cached_stats_q: Query<'w, 's, &'static CachedStats>,
    pub combat_state_q: Query<'w, 's, &'static CombatState>,
    pub energy_q: Query<'w, 's, &'static Energy>,
    pub personality_q: Query<'w, 's, &'static Personality>,
    pub home_q: Query<'w, 's, &'static Home>,
    pub work_state_q: Query<'w, 's, &'static NpcWorkState>,
    pub equipment_q: Query<'w, 's, &'static NpcEquipment>,
    pub carried_loot_q: Query<'w, 's, &'static CarriedLoot>,
    pub patrol_route_q: Query<'w, 's, &'static PatrolRoute>,
    pub last_hit_by_q: Query<'w, 's, &'static LastHitBy>,
    pub merchant_inv: ResMut<'w, MerchantInventory>,
    pub town_inventory: ResMut<'w, TownInventory>,
    pub next_loot_id: ResMut<'w, NextLootItemId>,
    pub tower_bld_q: Query<'w, 's, &'static mut TowerBuildingState, With<Building>>,
    pub miner_cfg_q: Query<'w, 's, &'static mut MinerHomeConfig>,
    pub production_q: Query<'w, 's, &'static ProductionState, With<Building>>,
    pub construction_q: Query<'w, 's, &'static ConstructionProgress, With<Building>>,
    pub spawner_q: Query<'w, 's, &'static SpawnerState, With<Building>>,
    pub wall_level_q: Query<'w, 's, &'static mut WallLevel, With<Building>>,
    pub waypoint_order_q: Query<'w, 's, &'static WaypointOrder, With<Building>>,
}

#[derive(SystemParam)]
pub struct BottomPanelUiState<'w> {
    destroy_request: MessageWriter<'w, crate::messages::DestroyBuildingMsg>,
    faction_select: MessageWriter<'w, crate::messages::SelectFactionMsg>,
    unequip_writer: MessageWriter<'w, UnequipItemMsg>,
    ui_state: ResMut<'w, UiState>,
    mining_policy: ResMut<'w, MiningPolicy>,
    dirty_writers: crate::messages::DirtyWriters<'w>,
    squad_state: ResMut<'w, SquadState>,
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
    pub show_loot: bool,
    pub show_llm: bool,
    pub show_chat: bool,
    /// -1 = all factions, 0 = my faction only
    pub faction_filter: i32,
    pub initialized: bool,
    pub chat_input: String,
    // Cached merged log entries — skip rebuild when sources unchanged
    cached_selected_npc: i32,
    cached_filters: (bool, bool, bool, bool, bool, bool, bool, bool, bool, bool, bool, i32),
    cached_entries: Vec<(i64, egui::Color32, String, String, Option<bevy::math::Vec2>)>,
}

#[derive(Default)]
pub struct InspectorRenameState {
    slot: i32,
    text: String,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum InspectorNpcTab {
    #[default]
    Overview,
    Loadout,
    Economy,
    Log,
}

#[derive(Default)]
pub struct InspectorTabState {
    /// true = NPC tab, false = Building tab
    show_npc: bool,
    npc_tab: InspectorNpcTab,
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
    mut data: BottomPanelData,
    mut meta_cache: ResMut<NpcMetaCache>,
    mut bld_data: BuildingInspectorData,
    mut world_data: ResMut<WorldData>,
    gpu_state: Res<GpuReadState>,
    _buffer_writes: Res<EntityGpuState>,
    _visual_upload: Res<crate::gpu::NpcVisualUpload>,
    mut follow: ResMut<FollowSelected>,
    settings: Res<UserSettings>,
    catalog: Res<HelpCatalog>,
    mut panel_state: BottomPanelUiState,
    mut inspector_state: Local<InspectorUiState>,
    mut camera_query: Query<&mut Transform, With<MainCamera>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    let mut copy_text: Option<String> = None;

    // Only show inspector when something is selected (or DC group active)
    let has_npc = data.selected.0 >= 0;
    let has_building = bld_data.selected_building.active;
    let dc_count = dc_slots(&panel_state.squad_state, &bld_data.entity_map, |e| {
        bld_data.npc_flags_q.get(e).is_ok_and(|f| f.direct_control)
    }).len();
    panel_state.ui_state.inspector_visible = has_npc || has_building || dc_count > 0;
    if panel_state.ui_state.inspector_visible {
        if has_npc && !has_building {
            inspector_state.tabs.show_npc = true;
        } else if has_building && !has_npc {
            inspector_state.tabs.show_npc = false;
        } else if has_npc
            && has_building
            && inspector_state.last_click_seq != panel_state.ui_state.inspector_click_seq
        {
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
                    let npc_label = if data.selected.0 >= 0
                        && (data.selected.0 as usize) < meta_cache.0.len()
                    {
                        format!("NPC: {}", meta_cache.0[data.selected.0 as usize].name)
                    } else {
                        "NPC".to_string()
                    };
                    let bld_label = selected_building_info(
                        &bld_data.selected_building,
                        &bld_data.grid,
                        &bld_data.entity_map,
                    )
                    .map(|(k, _, _, _, _)| format!("Building: {}", building_def(k).label))
                    .unwrap_or_else(|| "Building".to_string());

                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(inspector_state.tabs.show_npc, npc_label)
                            .clicked()
                        {
                            inspector_state.tabs.show_npc = true;
                        }
                        if ui
                            .selectable_label(!inspector_state.tabs.show_npc, bld_label)
                            .clicked()
                        {
                            inspector_state.tabs.show_npc = false;
                        }
                    });
                    ui.separator();
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let show_npc = has_npc && (!has_building || inspector_state.tabs.show_npc);
                        let InspectorUiState { rename, tabs, .. } = &mut *inspector_state;
                        let action = inspector_content(
                            ui,
                            &mut data,
                            &mut meta_cache,
                            rename,
                            &mut tabs.npc_tab,
                            &mut bld_data,
                            &mut world_data,
                            &gpu_state,
                            &mut follow,
                            &settings,
                            &catalog,
                            &mut copy_text,
                            &mut panel_state.ui_state,
                            &mut panel_state.mining_policy,
                            &mut panel_state.dirty_writers,
                            show_npc,
                            &mut panel_state.squad_state,
                            &mut panel_state.faction_select,
                            &mut panel_state.unequip_writer,
                            dc_count,
                        );
                        if let Some(action) = action {
                            apply_inspector_action(
                                action,
                                &mut data.selected,
                                &mut bld_data.selected_building,
                                &gpu_state,
                                &bld_data.entity_map,
                                &bld_data.grid,
                                &mut camera_query,
                            );
                        }
                        // Destroy button for selected player-owned buildings (not fountains/mines)
                        let show_building = has_building && (!has_npc || !show_npc);
                        if show_building {
                            let selected_info = selected_building_info(
                                &bld_data.selected_building,
                                &bld_data.grid,
                                &bld_data.entity_map,
                            );
                            let is_destructible = selected_info
                                .as_ref()
                                .map(|(k, ti, _, _, _)| {
                                    !matches!(k, BuildingKind::Fountain | BuildingKind::GoldMine)
                                        && world_data
                                            .towns
                                            .get(*ti as usize)
                                            .is_some_and(|t| t.faction == crate::constants::FACTION_PLAYER)
                                })
                                .unwrap_or(false);
                            if is_destructible {
                                ui.separator();
                                if ui
                                    .button(
                                        egui::RichText::new("Destroy")
                                            .color(egui::Color32::from_rgb(220, 80, 80)),
                                    )
                                    .clicked()
                                {
                                    if let Some((_, _, _, col, row)) = selected_info {
                                        panel_state
                                            .destroy_request
                                            .write(crate::messages::DestroyBuildingMsg(col, row));
                                    }
                                }
                            }
                        }
                    });
            });
    }

    // Tower upgrade popup window
    tower_upgrade_window(ctx, &mut bld_data, &mut panel_state.ui_state);

    // Handle clipboard copy (must be outside egui closure)
    if let Some(text) = copy_text {
        info!("Copy button clicked, {} bytes", text.len());
        match arboard::Clipboard::new() {
            Ok(mut cb) => match cb.set_text(text) {
                Ok(_) => info!("Clipboard: text copied successfully"),
                Err(e) => error!("Clipboard: set_text failed: {e}"),
            },
            Err(e) => error!("Clipboard: failed to open: {e}"),
        }
    }

    Ok(())
}

/// Tower upgrade popup — big buttons, explanations, per-stat auto-buy.
fn tower_upgrade_window(
    ctx: &egui::Context,
    bld: &mut BuildingInspectorData,
    ui_state: &mut UiState,
) {
    let Some(slot) = ui_state.tower_upgrade_slot else {
        return;
    };

    // Auto-close if selected building changed or tower no longer exists
    let inst_exists = bld.entity_map.get_instance(slot)
        .is_some_and(|i| i.kind == BuildingKind::Tower);
    if !inst_exists {
        ui_state.tower_upgrade_slot = None;
        return;
    }

    // Read tower data from ECS TowerBuildingState + EntityMap for town_idx
    let Some(inst) = bld.entity_map.get_instance(slot) else { return; };
    let town_idx = inst.town_idx as usize;
    let Some(&tower_entity) = bld.entity_map.entities.get(&slot) else { return; };
    let Ok(tbs) = bld.tower_bld_q.get(tower_entity) else { return; };
    let level = level_from_xp(tbs.xp);
    let upgrade_levels = tbs.upgrade_levels.clone();
    let auto_flags = tbs.auto_upgrade_flags.clone();

    let food = bld.food_storage.food.get(town_idx).copied().unwrap_or(0);
    let gold = bld.gold_storage.gold.get(town_idx).copied().unwrap_or(0);
    let tower_upgrades = crate::constants::TOWER_UPGRADES;

    let mut open = true;
    egui::Window::new("Tower Upgrades")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(400.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(format!("Level {} Tower", level))
                    .heading()
                    .strong(),
            );
            ui.add_space(4.0);

            for (i, upg) in tower_upgrades.iter().enumerate() {
                let lv = upgrade_levels.get(i).copied().unwrap_or(0);
                let cost_mult = crate::systems::stats::upgrade_cost(lv);

                let can_afford = upg.cost.iter().all(|(res, base)| {
                    let total = base * cost_mult;
                    match res {
                        ResourceKind::Food => food >= total,
                        ResourceKind::Gold => gold >= total,
                    }
                });

                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(40, 40, 50, 200))
                    .inner_margin(egui::Margin::same(8))
                    .corner_radius(4.0)
                    .show(ui, |ui| {
                        // Row 1: Auto checkbox + label + level
                        ui.horizontal(|ui| {
                            let mut auto = auto_flags.get(i).copied().unwrap_or(false);
                            if ui
                                .checkbox(&mut auto, "Auto")
                                .on_hover_text("Auto-buy this upgrade each game-hour")
                                .changed()
                            {
                                if let Ok(mut tbs) = bld.tower_bld_q.get_mut(tower_entity) {
                                    while tbs.auto_upgrade_flags.len() <= i {
                                        tbs.auto_upgrade_flags.push(false);
                                    }
                                    tbs.auto_upgrade_flags[i] = auto;
                                }
                            }
                            ui.label(egui::RichText::new(upg.label).heading());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("Lv {}", lv))
                                            .strong()
                                            .size(16.0),
                                    );
                                },
                            );
                        });

                        // Row 2: Description
                        ui.label(egui::RichText::new(upg.tooltip).weak());

                        // Row 3: Current → next effect
                        let current = tower_upgrade_effect(upg, lv);
                        let next = tower_upgrade_effect(upg, lv + 1);
                        ui.label(format!("Current: {}    Next: {}", current, next));

                        // Row 4: Buy button (full width)
                        let cost_parts: Vec<String> = upg
                            .cost
                            .iter()
                            .map(|(res, base)| {
                                let total = base * cost_mult;
                                match res {
                                    ResourceKind::Food => format!("{} food", total),
                                    ResourceKind::Gold => format!("{} gold", total),
                                }
                            })
                            .collect();
                        let cost_text = format!("Buy: {}", cost_parts.join(", "));

                        let btn = egui::Button::new(
                            egui::RichText::new(&cost_text).size(14.0),
                        )
                        .min_size(egui::vec2(ui.available_width(), 28.0));

                        if ui.add_enabled(can_afford, btn).clicked() {
                            // Deduct resources
                            for (res, base) in upg.cost {
                                let total = base * cost_mult;
                                match res {
                                    ResourceKind::Food => {
                                        if let Some(f) = bld.food_storage.food.get_mut(town_idx) {
                                            *f -= total;
                                        }
                                    }
                                    ResourceKind::Gold => {
                                        if let Some(g) = bld.gold_storage.gold.get_mut(town_idx) {
                                            *g -= total;
                                        }
                                    }
                                }
                            }
                            // Increment upgrade level
                            if let Ok(mut tbs) = bld.tower_bld_q.get_mut(tower_entity) {
                                while tbs.upgrade_levels.len() <= i {
                                    tbs.upgrade_levels.push(0);
                                }
                                tbs.upgrade_levels[i] += 1;
                            }
                        }
                    });
                ui.add_space(2.0);
            }
        });

    if !open {
        ui_state.tower_upgrade_slot = None;
    }
}

/// Format a tower upgrade effect string for a given level.
fn tower_upgrade_effect(upg: &crate::constants::UpgradeStatDef, lv: u8) -> String {
    match upg.display {
        EffectDisplay::Percentage => format!("+{}%", (lv as f32 * upg.pct * 100.0) as i32),
        EffectDisplay::CooldownReduction => {
            if lv == 0 {
                "0%".to_string()
            } else {
                let boost = (1.0 - 1.0 / (1.0 + lv as f32 * upg.pct)) * 100.0;
                format!("+{:.0}%", boost)
            }
        }
        EffectDisplay::Discrete => {
            if upg.kind == UpgradeStatKind::HpRegen {
                format!("+{:.1}/s", lv as f32 * 2.0)
            } else {
                format!("+{}", lv)
            }
        }
        _ => format!("Lv{}", lv),
    }
}

/// Combat log window anchored at bottom-right.
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
                    if filter_state.faction_filter == 0 && entry.faction != crate::constants::FACTION_PLAYER && entry.faction != crate::constants::FACTION_NEUTRAL
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
                        let other_town = if msg.from_town == 0 { msg.to_town } else { msg.from_town };
                        let town_name = world_data.towns.get(other_town)
                            .map(|t| t.name.as_str())
                            .unwrap_or("?");
                        let label = if msg.from_town == 0 {
                            format!("[chat to {}] {}", town_name, msg.text)
                        } else {
                            format!("[chat from {}] {}", town_name, msg.text)
                        };
                        let key = (msg.day as i64) * 10000
                            + (msg.hour as i64) * 100
                            + msg.minute as i64;
                        let ts = format!("[D{} {:02}:{:02}]", msg.day, msg.hour, msg.minute);
                        filter_state.cached_entries.push((key, chat_color, ts, label, None));
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
                day, hour, minute,
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

/// DirectControl group summary — shown when DC units exist but no single NPC selected.
fn dc_group_inspector(
    ui: &mut egui::Ui,
    entity_map: &crate::resources::EntityMap,
    squad_state: &mut SquadState,
    health_q: &Query<&Health, Without<Building>>,
    cached_stats_q: &Query<&CachedStats>,
    npc_flags_q: &Query<&mut NpcFlags>,
) {
    let mut total_hp = 0.0f32;
    let mut total_max_hp = 0.0f32;
    let mut job_counts: Vec<(&str, usize)> = Vec::new();

    let slots = dc_slots(squad_state, entity_map, |e| {
        npc_flags_q.get(e).is_ok_and(|f| f.direct_control)
    });
    let count = slots.len();

    for slot in &slots {
        let Some(npc) = entity_map.get_npc(*slot) else {
            continue;
        };
        total_hp += health_q.get(npc.entity).map(|h| h.0).unwrap_or(0.0);
        total_max_hp += cached_stats_q
            .get(npc.entity)
            .map(|s| s.max_health)
            .unwrap_or(100.0);
        let name = crate::job_name(npc.job as i32);
        if let Some(entry) = job_counts.iter_mut().find(|(n, _)| *n == name) {
            entry.1 += 1;
        } else {
            job_counts.push((name, 1));
        }
    }

    ui.heading(format!("Direct Control — {} units", count));
    ui.separator();

    let hp_frac = if total_max_hp > 0.0 {
        total_hp / total_max_hp
    } else {
        1.0
    };
    ui.label(format!(
        "HP: {:.0} / {:.0} ({:.0}%)",
        total_hp,
        total_max_hp,
        hp_frac * 100.0
    ));

    let parts: Vec<String> = job_counts
        .iter()
        .map(|(j, c)| format!("{} {}", c, j))
        .collect();
    if !parts.is_empty() {
        ui.label(parts.join(", "));
    }

    ui.separator();
    ui.checkbox(&mut squad_state.dc_no_return, "Keep fighting after loot");
}

/// Render inspector content into a ui region (left side of bottom panel).
fn inspector_content(
    ui: &mut egui::Ui,
    data: &mut BottomPanelData,
    meta_cache: &mut NpcMetaCache,
    rename_state: &mut InspectorRenameState,
    npc_tab: &mut InspectorNpcTab,
    bld_data: &mut BuildingInspectorData,
    world_data: &mut WorldData,
    gpu_state: &GpuReadState,
    follow: &mut FollowSelected,
    settings: &UserSettings,
    catalog: &HelpCatalog,
    copy_text: &mut Option<String>,
    ui_state: &mut UiState,
    mining_policy: &mut MiningPolicy,
    dirty_writers: &mut crate::messages::DirtyWriters,
    show_npc: bool,
    squad_state: &mut SquadState,
    faction_select: &mut MessageWriter<crate::messages::SelectFactionMsg>,
    unequip_writer: &mut MessageWriter<UnequipItemMsg>,
    dc_count: usize,
) -> Option<InspectorAction> {
    if !show_npc {
        rename_state.slot = -1;
        rename_state.text.clear();
        if bld_data.selected_building.active {
            return building_inspector_content(
                ui,
                bld_data,
                world_data,
                mining_policy,
                dirty_writers,
                meta_cache,
                ui_state,
                settings,
                gpu_state,
                copy_text,
                faction_select,
            );
        }
        if dc_count > 0 {
            dc_group_inspector(
                ui,
                &bld_data.entity_map,
                squad_state,
                &bld_data.npc_health_q,
                &bld_data.cached_stats_q,
                &bld_data.npc_flags_q,
            );
            return None;
        }
        ui.label("Click an NPC or building to inspect");
        return None;
    }

    let sel = data.selected.0;
    if sel < 0 {
        rename_state.slot = -1;
        rename_state.text.clear();
        if bld_data.selected_building.active {
            return building_inspector_content(
                ui,
                bld_data,
                world_data,
                mining_policy,
                dirty_writers,
                meta_cache,
                ui_state,
                settings,
                gpu_state,
                copy_text,
                faction_select,
            );
        }
        if dc_count > 0 {
            dc_group_inspector(
                ui,
                &bld_data.entity_map,
                squad_state,
                &bld_data.npc_health_q,
                &bld_data.cached_stats_q,
                &bld_data.npc_flags_q,
            );
            return None;
        }
        ui.label("Click an NPC or building to inspect");
        return None;
    }
    let idx = sel as usize;
    if idx >= meta_cache.0.len() {
        return None;
    }
    if bld_data.entity_map.get_npc(idx).is_none() {
        rename_state.slot = -1;
        rename_state.text.clear();
        if bld_data.selected_building.active {
            return building_inspector_content(
                ui,
                bld_data,
                world_data,
                mining_policy,
                dirty_writers,
                meta_cache,
                ui_state,
                settings,
                gpu_state,
                copy_text,
                faction_select,
            );
        } else {
            ui.label("Click an NPC or building to inspect");
        }
        return None;
    }

    if rename_state.slot != sel {
        rename_state.slot = sel;
        rename_state.text = meta_cache.0[idx].name.clone();
    }

    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(npc_tab, InspectorNpcTab::Overview, "Overview");
        ui.selectable_value(npc_tab, InspectorNpcTab::Loadout, "Loadout");
        ui.selectable_value(npc_tab, InspectorNpcTab::Economy, "Economy");
        ui.selectable_value(npc_tab, InspectorNpcTab::Log, "Log");
    });
    ui.separator();
    let show_overview = *npc_tab == InspectorNpcTab::Overview;
    let show_loadout = *npc_tab == InspectorNpcTab::Loadout;
    let show_economy = *npc_tab == InspectorNpcTab::Economy;
    let show_log = *npc_tab == InspectorNpcTab::Log;

    if show_overview {
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
    }

    let meta = &meta_cache.0[idx];

    if show_overview {
        tipped(
            ui,
            format!(
                "{} Lv.{}  XP: {}/{}",
                crate::job_name(meta.job),
                meta.level,
                meta.xp,
                (meta.level + 1) * (meta.level + 1) * 100
            ),
            catalog.0.get("npc_level").unwrap_or(&""),
        );
    }

    if show_overview {
        if let Some(npc) = bld_data.entity_map.get_npc(idx) {
            if let Ok(pers) = bld_data.personality_q.get(npc.entity) {
                let trait_str = pers.trait_summary();
                if !trait_str.is_empty() {
                    tipped(
                        ui,
                        format!("Trait: {}", trait_str),
                        catalog.0.get("npc_trait").unwrap_or(&""),
                    );
                }
            }
        }
    }

    // Find HP, energy, combat stats from ECS queries
    let (hp, max_hp, energy, cached_stats) = if let Some(npc) = bld_data.entity_map.get_npc(idx) {
        let hp = bld_data
            .npc_health_q
            .get(npc.entity)
            .map(|h| h.0)
            .unwrap_or(0.0);
        let cs = bld_data.cached_stats_q.get(npc.entity).cloned().ok();
        let max_hp = cs.as_ref().map(|s| s.max_health).unwrap_or(100.0f32);
        let energy = bld_data
            .energy_q
            .get(npc.entity)
            .map(|e| e.0)
            .unwrap_or(0.0);
        (hp, max_hp, energy, cs)
    } else {
        (0.0f32, 100.0f32, 0.0f32, None)
    };

    if show_overview {
        // HP bar
        let hp_frac = if max_hp > 0.0 {
            (hp / max_hp).clamp(0.0, 1.0)
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
        ui.horizontal(|ui| {
            ui.label("HP:");
            ui.add(
                egui::ProgressBar::new(hp_frac)
                    .text(
                        egui::RichText::new(format!("{:.0}/{:.0}", hp, max_hp))
                            .color(egui::Color32::BLACK),
                    )
                    .fill(hp_color),
            );
        });

        // Energy bar
        let energy_frac = (energy / 100.0).clamp(0.0, 1.0);
        ui.horizontal(|ui| {
            tipped(ui, "EN:", catalog.0.get("npc_energy").unwrap_or(&""));
            ui.add(
                egui::ProgressBar::new(energy_frac)
                    .text(egui::RichText::new(format!("{:.0}", energy)).color(egui::Color32::BLACK))
                    .fill(egui::Color32::from_rgb(60, 120, 200)),
            );
        });

        // Combat stats
        if let Some(ref stats) = cached_stats {
            ui.label(format!(
                "Dmg: {:.0}  Rng: {:.0}  CD: {:.1}s  Spd: {:.0}",
                stats.damage, stats.range, stats.cooldown, stats.speed
            ));
        }
    }

    // Equipment + status from EntityMap + ECS
    if show_loadout {
        ui.separator();
        ui.label(egui::RichText::new("Loadout").strong());
    }
    let mut squad_id: Option<i32> = None;
    let job = Job::from_i32(meta.job);
    let can_equip = !npc_def(job).equip_slots.is_empty();
    if let Some(npc) = bld_data.entity_map.get_npc(idx) {
        if show_loadout {
            if let Ok(eq) = bld_data.equipment_q.get(npc.entity) {
                use crate::constants::EquipmentSlot;
                let slots: &[(&str, &Option<crate::constants::LootItem>, EquipmentSlot, u8)] = &[
                    ("Weapon", &eq.weapon, EquipmentSlot::Weapon, 0),
                    ("Helm", &eq.helm, EquipmentSlot::Helm, 0),
                    ("Armor", &eq.armor, EquipmentSlot::Armor, 0),
                    ("Shield", &eq.shield, EquipmentSlot::Shield, 0),
                    ("Gloves", &eq.gloves, EquipmentSlot::Gloves, 0),
                    ("Boots", &eq.boots, EquipmentSlot::Boots, 0),
                    ("Belt", &eq.belt, EquipmentSlot::Belt, 0),
                    ("Amulet", &eq.amulet, EquipmentSlot::Amulet, 0),
                    ("Ring 1", &eq.ring1, EquipmentSlot::Ring, 0),
                    ("Ring 2", &eq.ring2, EquipmentSlot::Ring, 1),
                ];
                let mut any = false;
                for &(label, item_opt, slot, ring_index) in slots {
                    if let Some(item) = item_opt {
                        any = true;
                        ui.horizontal(|ui| {
                            let (r, g, b) = item.rarity.color();
                            ui.label(format!("{}:", label));
                            ui.label(egui::RichText::new(&item.name).color(egui::Color32::from_rgb(r, g, b)));
                            ui.label(format!("(+{:.0}%)", item.stat_bonus * 100.0));
                            if ui.small_button("Unequip").clicked() {
                                unequip_writer.write(UnequipItemMsg {
                                    npc_entity: npc.entity,
                                    slot,
                                    ring_index,
                                });
                            }
                        });
                    }
                }
                if !any && can_equip {
                    ui.label("No equipment");
                }
            }
            if can_equip {
                if ui.small_button("Open Armory >").clicked() {
                    ui_state.left_panel_open = true;
                    ui_state.left_panel_tab = LeftPanelTab::Inventory;
                }
            }
        }

        // Status markers
        if show_overview {
            if bld_data
                .npc_flags_q
                .get(npc.entity)
                .is_ok_and(|f| f.starving)
            {
                ui.colored_label(egui::Color32::from_rgb(200, 60, 60), "Starving");
            }
        }
        squad_id = bld_data.squad_id_q.get(npc.entity).ok().map(|s| s.0);
    }

    // Town name
    if show_overview && meta.town_id >= 0 {
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
    let mut home_slot: Option<usize> = None;
    let mut is_mining_at_mine = false;

    let mut carried_food = 0i32;
    let mut carried_gold = 0i32;
    let mut carried_equip_count = 0usize;
    let mut carried_equip_preview: Vec<String> = Vec::new();
    let mut carried_equip_more = 0usize;
    let mut activity_debug = String::new();
    if let Some(npc) = bld_data.entity_map.get_npc(idx) {
        let npc_home = bld_data
            .home_q
            .get(npc.entity)
            .map(|h| h.0)
            .unwrap_or(Vec2::ZERO);
        home_pos = Some(npc_home);
        let home_valid = npc_home.x >= 0.0 && npc_home.y >= 0.0;
        if home_valid {
            home_slot = bld_data.entity_map.find_by_position(npc_home).map(|i| i.slot);
            home_str = format!("({:.0}, {:.0})", npc_home.x, npc_home.y);
        } else {
            home_str = "Homeless".to_string();
        }
        faction_str = if let Some(town) = world_data.towns.get(npc.town_idx as usize) {
            format!("{} (F{})", town.name, town.faction)
        } else {
            format!("F{}", npc.faction)
        };
        faction_id = Some(npc.faction);
        let npc_act = bld_data.activity_q.get(npc.entity).ok();
        is_mining_at_mine = npc_act.is_some_and(|a| matches!(a.kind, ActivityKind::Mine { .. }));
        activity_debug = npc_act.map(|a| format!("{:?}", a)).unwrap_or_default();

        if let Ok(cl) = bld_data.carried_loot_q.get(npc.entity) {
            carried_food = cl.food;
            carried_gold = cl.gold;
            carried_equip_count = cl.equipment.len();
            carried_equip_preview = cl
                .equipment
                .iter()
                .take(4)
                .map(|it| format!("{} ({:?} +{:.0}%)", it.name, it.slot, it.stat_bonus * 100.0))
                .collect();
            carried_equip_more = cl.equipment.len().saturating_sub(carried_equip_preview.len());
        }

        let mut parts: Vec<&str> = Vec::new();
        let combat_name = bld_data
            .combat_state_q
            .get(npc.entity)
            .map(|cs| cs.name())
            .unwrap_or("");
        if !combat_name.is_empty() {
            parts.push(combat_name);
        }
        parts.push(npc_act.map(|a| a.name()).unwrap_or("Unknown"));
        state_str = parts.join(", ");
    }

    if show_overview {
        tipped(
            ui,
            format!("State: {}", state_str),
            catalog.0.get("npc_state").unwrap_or(&""),
        );
        let home_action = ui.horizontal(|ui| {
            if let Some(fid) = faction_id {
                if ui.link(format!("Faction: {}", faction_str)).clicked() {
                    ui_state.left_panel_open = true;
                    ui_state.left_panel_tab = LeftPanelTab::Factions;
                    faction_select.write(crate::messages::SelectFactionMsg(fid));
                }
            } else {
                ui.label(format!("Faction: {}", faction_str));
            }
            if let Some(slot) = home_slot {
                building_link(ui, &format!("Home: {}", home_str), slot)
            } else {
                ui.label(format!("Home: {}", home_str));
                None
            }
        }).inner;
        if let Some(action) = home_action {
            return Some(action);
        }
        if let Some(sq) = squad_id {
            ui.label(format!("Squad: {}", sq));
        }
    }

    if show_economy {
        // Carried loot
        ui.separator();
        ui.label(egui::RichText::new("Personal Economy").strong());
        {
            let mut parts: Vec<String> = Vec::new();
            if carried_food > 0 {
                parts.push(format!("{} food", carried_food));
            }
            if carried_gold > 0 {
                parts.push(format!("{} gold", carried_gold));
            }
            if carried_equip_count > 0 {
                parts.push(format!("{} item(s)", carried_equip_count));
            }
            let loot_str = if parts.is_empty() {
                "none".to_string()
            } else {
                parts.join(", ")
            };
            ui.label(format!("Carrying: {}", loot_str));
            if !carried_equip_preview.is_empty() {
                for line in carried_equip_preview {
                    ui.small(format!("Carried item: {}", line));
                }
                if carried_equip_more > 0 {
                    ui.small(format!("...and {} more item(s)", carried_equip_more));
                }
            }
        }
    }

    if show_log {
        ui.separator();
        ui.label(egui::RichText::new("Personal Log").strong());
        if idx < data.npc_logs.logs.len() {
            let log = &data.npc_logs.logs[idx];
            if log.is_empty() {
                ui.small("No activity entries recorded for this NPC yet.");
            } else {
                for entry in log.iter().rev().take(12) {
                    ui.small(format!(
                        "[D{} {:02}:{:02}] {}",
                        entry.day, entry.hour, entry.minute, entry.message
                    ));
                }
                if log.len() > 12 {
                    ui.small(format!("...{} older entries", log.len() - 12));
                }
            }
        }
    }

    // Mine assignment for miners (same UI as MinerHome building inspector)
    if show_overview && meta.job == 4 {
        if let Some(hp) = home_pos {
            let mh_slot = bld_data
                .entity_map
                .find_by_position(hp)
                .filter(|i| i.kind == BuildingKind::MinerHome)
                .map(|i| i.slot);
            if let Some(mh_slot) = mh_slot {
                ui.separator();
                if let Some(action) = mine_assignment_ui(
                    ui,
                    world_data,
                    &bld_data.entity_map,
                    mh_slot,
                    hp,
                    dirty_writers,
                    ui_state,
                    &mut bld_data.miner_cfg_q,
                ) {
                    return Some(action);
                }
                // Show mine productivity when actively mining
                if is_mining_at_mine {
                    let mine_pos = bld_data.entity_map.entities.get(&mh_slot)
                        .and_then(|&e| bld_data.miner_cfg_q.get(e).ok())
                        .and_then(|mc| mc.assigned_mine);
                    if let Some(mine_pos) = mine_pos {
                        let occupants = bld_data
                            .entity_map
                            .slot_at_position(mine_pos)
                            .map(|s| bld_data.entity_map.occupant_count(s))
                            .unwrap_or(0);
                        if occupants > 0 {
                            let mult = crate::constants::mine_productivity_mult(occupants);
                            ui.label(format!(
                                "Mine productivity: {:.0}% ({} miners)",
                                mult * 100.0,
                                occupants
                            ));
                        }
                    }
                }
            }
        }
    }

    // Controls: Follow + Direct Control (near bottom)
    if show_overview {
        ui.separator();
        ui.horizontal(|ui| {
            if ui.selectable_label(follow.0, "Follow (F)").clicked() {
                follow.0 = !follow.0;
            }
        });
        {
            let entity = bld_data.entity_map.get_npc(idx).map(|n| n.entity);
            let is_dc = entity
                .and_then(|e| bld_data.npc_flags_q.get(e).ok())
                .is_some_and(|f| f.direct_control);
            ui.horizontal(|ui| {
                let label = if is_dc {
                    "Direct Control: ON"
                } else {
                    "Direct Control: OFF"
                };
                let color = if is_dc {
                    egui::Color32::from_rgb(80, 220, 80)
                } else {
                    egui::Color32::GRAY
                };
                if ui.button(egui::RichText::new(label).color(color)).clicked() {
                    if let Some(e) = entity {
                        if let Ok(mut flags) = bld_data.npc_flags_q.get_mut(e) {
                            flags.direct_control = !is_dc;
                        }
                    }
                }
            });
        }
    }

    // Debug IDs: show slot + UID + world coords for BRP queries, plus copy button
    if show_overview && settings.debug_ids {
        ui.separator();
        let uid = bld_data.entity_map.uid_for_slot(idx);
        let uid_str = uid.map_or("?".to_string(), |u| u.0.to_string());
        let world_pos_str = if idx * 2 + 1 < gpu_state.positions.len() {
            format!("({:.0}, {:.0})", gpu_state.positions[idx * 2], gpu_state.positions[idx * 2 + 1])
        } else {
            "?".into()
        };
        ui.label(format!("Slot: {}  UID: {}  Pos: {}", idx, uid_str, world_pos_str));

        if ui.button("Copy Debug Info").clicked() {
            let positions = &gpu_state.positions;
            let pos = if idx * 2 + 1 < positions.len() {
                format!("({:.0}, {:.0})", positions[idx * 2], positions[idx * 2 + 1])
            } else {
                "?".into()
            };
            let xp_next = (meta.level + 1) * (meta.level + 1) * 100;
            let mut info = format!(
                "NPC #{idx} \"{name}\" {job} Lv.{level}  XP: {xp}/{xp_next}\n\
                 Slot: {idx}  UID: {uid}\n\
                 HP: {hp:.0}/{max_hp:.0}  EN: {energy:.0}\n\
                 Pos: {pos}\n\
                 Home: {home}  Faction: {faction}\n\
                 State: {state}\n\
                 Activity: {activity}\n",
                idx = idx,
                name = meta.name,
                job = crate::job_name(meta.job),
                level = meta.level,
                xp = meta.xp,
                xp_next = xp_next,
                uid = uid_str,
                hp = hp,
                max_hp = max_hp,
                energy = energy,
                pos = pos,
                home = home_str,
                faction = faction_str,
                state = state_str,
                activity = activity_debug,
            );
            if let Some(npc) = bld_data.entity_map.get_npc(idx) {
                if let Ok(pers) = bld_data.personality_q.get(npc.entity) {
                    let trait_str = pers.trait_summary();
                    if !trait_str.is_empty() {
                        info.push_str(&format!("Trait: {}\n", trait_str));
                    }
                }
            }
            if let Some(ref stats) = cached_stats {
                info.push_str(&format!(
                    "Dmg: {:.0}  Rng: {:.0}  CD: {:.1}s  Spd: {:.0}\n",
                    stats.damage, stats.range, stats.cooldown, stats.speed
                ));
            }
            if let Some(npc) = bld_data.entity_map.get_npc(idx) {
                if let Ok(eq) = bld_data.equipment_q.get(npc.entity) {
                    let slots: &[(&str, &Option<crate::constants::LootItem>)] = &[
                        ("Weapon", &eq.weapon), ("Helm", &eq.helm), ("Armor", &eq.armor),
                        ("Shield", &eq.shield), ("Gloves", &eq.gloves), ("Boots", &eq.boots),
                        ("Belt", &eq.belt), ("Amulet", &eq.amulet),
                        ("Ring 1", &eq.ring1), ("Ring 2", &eq.ring2),
                    ];
                    for &(label, item_opt) in slots {
                        if let Some(item) = item_opt {
                            info.push_str(&format!("{}: {} ({} +{:.0}%)\n",
                                label, item.name, item.rarity.label(), item.stat_bonus * 100.0));
                        }
                    }
                }
                if let Ok(flags) = bld_data.npc_flags_q.get(npc.entity) {
                    let mut fp: Vec<&str> = Vec::new();
                    if flags.healing { fp.push("healing"); }
                    if flags.starving { fp.push("starving"); }
                    if flags.direct_control { fp.push("direct_control"); }
                    if flags.migrating { fp.push("migrating"); }
                    if flags.at_destination { fp.push("at_dest"); }
                    info.push_str(&format!("Flags: [{}]\n", fp.join(", ")));
                }
                let combat_state_name = bld_data
                    .combat_state_q
                    .get(npc.entity)
                    .map(|cs| cs.name())
                    .unwrap_or("Unknown");
                info.push_str(&format!("CombatState: {}\n", combat_state_name));
                info.push_str(&format!("CarriedLoot: food={} gold={}\n", carried_food, carried_gold));
            }
            info.push_str(&format!(
                "Day {day} {hour:02}:{min:02}\n",
                day = data.game_time.day(),
                hour = data.game_time.hour(),
                min = data.game_time.minute(),
            ));
            *copy_text = Some(info);
        }
    }
    None
}

// ============================================================================
// BUILDING INSPECTOR
// ============================================================================

fn selected_building_info(
    selected: &SelectedBuilding,
    grid: &WorldGrid,
    entity_map: &EntityMap,
) -> Option<(BuildingKind, u32, Vec2, usize, usize)> {
    if !selected.active {
        return None;
    }

    if let (Some(kind), Some(slot)) = (selected.kind, selected.slot) {
        if let Some(inst) = entity_map.get_instance(slot) {
            let (col, row) = grid.world_to_grid(inst.position);
            return Some((kind, inst.town_idx, inst.position, col, row));
        }
    }

    let col = selected.col;
    let row = selected.row;
    let inst = entity_map.get_at_grid(col as i32, row as i32)?;
    let pos = grid.grid_to_world(col, row);
    Some((inst.kind, inst.town_idx, pos, col, row))
}

/// Mine assignment UI: show assigned mine, "Set Mine" / "Clear" buttons.
/// Shared by building inspector (MinerHome) and NPC inspector (Miner).
fn mine_assignment_ui(
    ui: &mut egui::Ui,
    _world_data: &mut WorldData,
    entity_map: &EntityMap,
    mh_slot: usize,
    ref_pos: Vec2,
    dirty_writers: &mut crate::messages::DirtyWriters,
    ui_state: &mut UiState,
    miner_cfg_q: &mut Query<&mut MinerHomeConfig>,
) -> Option<InspectorAction> {
    let mh_entity = entity_map.entities.get(&mh_slot).copied();
    let (assigned, manual) = mh_entity
        .and_then(|e| miner_cfg_q.get(e).ok().map(|mc| (mc.assigned_mine, mc.manual_mine)))
        .unwrap_or((None, false));
    let mut action = None;
    if let Some(mine_pos) = assigned {
        let dist = mine_pos.distance(ref_pos);
        let mine_slot = entity_map.slot_at_position(mine_pos);
        let label = if let Some(mine_idx) = entity_map.gold_mine_index(mine_pos) {
            format!("Mine: {} - {:.0}px", crate::ui::gold_mine_name(mine_idx), dist)
        } else {
            format!("Mine: ({:.0}, {:.0}) - {:.0}px", mine_pos.x, mine_pos.y, dist)
        };
        if let Some(slot) = mine_slot {
            action = building_link(ui, &label, slot);
        } else {
            ui.label(label);
        }
    } else {
        ui.label("Mine: Auto (nearest)");
    }
    ui.small(if manual {
        "Mode: Manual"
    } else {
        "Mode: Auto-policy"
    });
    ui.horizontal(|ui| {
        if ui.button("Set Mine").clicked() {
            if let Some(e) = mh_entity {
                if let Ok(mut mc) = miner_cfg_q.get_mut(e) {
                    mc.manual_mine = true;
                }
            }
            dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
            ui_state.assigning_mine = Some(mh_slot);
        }
        if assigned.is_some() || manual {
            if ui.button("Clear").clicked() {
                if let Some(e) = mh_entity {
                    if let Ok(mut mc) = miner_cfg_q.get_mut(e) {
                        mc.manual_mine = false;
                        mc.assigned_mine = None;
                    }
                }
                dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
            }
        }
    });
    action
}

/// Render building inspector content when a building cell is selected.
fn building_inspector_content(
    ui: &mut egui::Ui,
    bld: &mut BuildingInspectorData,
    world_data: &mut WorldData,
    mining_policy: &mut MiningPolicy,
    dirty_writers: &mut crate::messages::DirtyWriters,
    meta_cache: &NpcMetaCache,
    ui_state: &mut UiState,
    settings: &UserSettings,
    gpu_state: &GpuReadState,
    copy_text: &mut Option<String>,
    faction_select: &mut MessageWriter<crate::messages::SelectFactionMsg>,
) -> Option<InspectorAction> {
    let (kind, bld_town_idx, world_pos, col, row) =
        selected_building_info(&bld.selected_building, &bld.grid, &bld.entity_map)?;

    let def = building_def(kind);
    let town_idx = bld_town_idx as usize;

    // Header
    ui.strong(def.label);

    // Town + faction
    if let Some(town) = world_data.towns.get(town_idx) {
        ui.label(format!("Town: {}", town.name));
        if ui.link(format!("Faction: {} (F{})", town.name, town.faction)).clicked() {
            ui_state.left_panel_open = true;
            ui_state.left_panel_tab = LeftPanelTab::Factions;
            faction_select.write(crate::messages::SelectFactionMsg(town.faction));
        }
    } else if kind == BuildingKind::GoldMine {
        ui.label("Faction: Unowned");
    }

    // Construction status from ECS ConstructionProgress
    let bld_slot = bld.entity_map.slot_at_position(world_pos);
    let bld_entity = bld_slot.and_then(|s| bld.entity_map.entities.get(&s).copied());
    let construction_remaining = bld_entity
        .and_then(|e| bld.construction_q.get(e).ok())
        .map(|cp| cp.0)
        .unwrap_or(0.0);
    let is_constructing = construction_remaining > 0.0;
    if is_constructing {
        let total = crate::constants::BUILDING_CONSTRUCT_SECS;
        let progress = ((total - construction_remaining) / total).clamp(0.0, 1.0);
        ui.colored_label(egui::Color32::from_rgb(200, 200, 40), "Under Construction");
        ui.horizontal(|ui| {
            ui.label("Progress:");
            ui.add(
                egui::ProgressBar::new(progress)
                    .text(format!(
                        "{:.0}% ({:.1}s)",
                        progress * 100.0,
                        construction_remaining
                    ))
                    .fill(egui::Color32::from_rgb(200, 160, 40)),
            );
        });
    }

    // Per-type details (hidden during construction)
    if !is_constructing { match kind {
        BuildingKind::Farm => {
            if let Some(ps) = bld_entity.and_then(|e| bld.production_q.get(e).ok()) {
                let state_name = if ps.ready { "Ready to harvest" } else { "Growing" };
                ui.label(format!("Status: {}", state_name));

                let color = if ps.ready {
                    egui::Color32::from_rgb(200, 200, 60)
                } else {
                    egui::Color32::from_rgb(80, 180, 80)
                };
                ui.horizontal(|ui| {
                    ui.label("Growth:");
                    ui.add(
                        egui::ProgressBar::new(ps.progress)
                            .text(format!("{:.0}%", ps.progress * 100.0))
                            .fill(color),
                    );
                });

                let occupants = bld_slot
                    .map(|s| bld.entity_map.occupant_count(s))
                    .unwrap_or(0);
                ui.label(format!("Farmers: {}", occupants));
            }
        }

        BuildingKind::Waypoint => {
            let order = bld_entity
                .and_then(|e| bld.waypoint_order_q.get(e).ok())
                .map(|w| w.0)
                .unwrap_or(0);
            ui.label(format!("Patrol order: {}", order));
        }

        BuildingKind::Fountain => {
            // Healing + tower info
            let base_radius = bld.combat_config.heal_radius;
            let levels = bld.town_upgrades.town_levels(town_idx);
            let upgrade_bonus =
                UPGRADES.stat_level(&levels, "Town", UpgradeStatKind::FountainRange) as f32 * 24.0;
            let tower = resolve_town_tower_stats(&levels);
            ui.label(format!("Heal radius: {:.0}px", base_radius + upgrade_bonus));
            ui.label(format!("Heal rate: {:.0}/s", bld.combat_config.heal_rate));
            ui.separator();
            ui.label(format!("Tower range: {:.0}px", tower.range));
            ui.label(format!("Tower damage: {:.1}", tower.damage));
            ui.label(format!("Tower cooldown: {:.2}s", tower.cooldown));
            ui.label(format!(
                "Tower projectile life: {:.2}s",
                tower.proj_lifetime
            ));

            // Kills / XP / Level from TowerBuildingState
            if let Some(tbs) = bld_entity.and_then(|e| bld.tower_bld_q.get(e).ok()) {
                let level = level_from_xp(tbs.xp);
                let xp_next = (level + 1) * (level + 1) * 100;
                ui.label(format!(
                    "Kills: {}  Lv.{}  XP: {}/{}",
                    tbs.kills, level, tbs.xp, xp_next
                ));
            }

            // Town food — town_idx is direct index into food_storage
            if let Some(&food) = bld.food_storage.food.get(town_idx) {
                ui.label(format!("Food: {}", food));
            }
        }

        BuildingKind::Bed => {
            ui.label("Rest point");
        }

        BuildingKind::GoldMine => {
            if let Some(mine_inst) = bld.entity_map.find_by_position(world_pos) {
                let mine_label = if let Some(idx) = bld.entity_map.gold_mine_index(world_pos) {
                    crate::ui::gold_mine_name(idx)
                } else {
                    format!("Gold Mine (slot {})", mine_inst.slot)
                };
                ui.label(format!("Name: {}", mine_label));
                let enabled = *mining_policy
                    .mine_enabled
                    .get(&mine_inst.slot)
                    .unwrap_or(&true);
                let label = if enabled {
                    "Auto-mining: ON"
                } else {
                    "Auto-mining: OFF"
                };
                if ui.button(label).clicked() {
                    mining_policy.mine_enabled.insert(mine_inst.slot, !enabled);
                    dirty_writers.mining.write(crate::messages::MiningDirtyMsg);
                }
            }
            if let Some(ps) = bld_entity.and_then(|e| bld.production_q.get(e).ok()) {
                let label = if ps.ready {
                    "Ready to harvest".to_string()
                } else {
                    format!("Growing: {:.0}%", ps.progress * 100.0)
                };
                ui.label(&label);
                let color = if ps.ready {
                    egui::Color32::from_rgb(200, 180, 40)
                } else if ps.progress > 0.0 {
                    egui::Color32::from_rgb(160, 140, 40)
                } else {
                    egui::Color32::from_rgb(100, 100, 100)
                };
                ui.add(
                    egui::ProgressBar::new(ps.progress)
                        .text(format!("{:.0}%", ps.progress * 100.0))
                        .fill(color),
                );
                let occupants = bld_slot
                    .map(|s| bld.entity_map.occupant_count(s))
                    .unwrap_or(0);
                if occupants > 0 {
                    let mult = crate::constants::mine_productivity_mult(occupants);
                    ui.label(format!(
                        "Miners: {} ({:.0}% speed)",
                        occupants,
                        mult * 100.0
                    ));
                }
            }
        }

        BuildingKind::Wall => {
            // Wall tier info + upgrade button
            if let Some(_wall_inst) = bld.entity_map.find_by_position(world_pos) {
                let wall_lv = bld_entity.and_then(|e| bld.wall_level_q.get(e).ok()).map(|w| w.0).unwrap_or(1);
                let level = wall_lv.max(1) as usize;
                let tier_name = WALL_TIER_NAMES.get(level - 1).unwrap_or(&"Wall");
                let tier_hp = WALL_TIER_HP.get(level - 1).copied().unwrap_or(80.0);
                ui.label(format!("Tier: {} (Lv.{})", tier_name, level));
                ui.label(format!("Max HP: {:.0}", tier_hp));

                // Show current HP from building entity
                {
                    let hp = bld_entity
                        .and_then(|e| bld.building_health.get(e).ok())
                        .map(|h| h.0);
                    if let Some(hp) = hp {
                        let color = if hp > tier_hp * 0.5 {
                            egui::Color32::from_rgb(80, 200, 80)
                        } else {
                            egui::Color32::from_rgb(200, 80, 80)
                        };
                        ui.horizontal(|ui| {
                            ui.label("HP:");
                            ui.add(
                                egui::ProgressBar::new(hp / tier_hp)
                                    .text(format!("{:.0}/{:.0}", hp, tier_hp))
                                    .fill(color),
                            );
                        });
                    }
                }

                // Upgrade button (if not max tier)
                if level < 3 {
                    let costs = WALL_UPGRADE_COSTS[level - 1];
                    let cost_str: Vec<String> = costs
                        .iter()
                        .map(|(r, amt)| match r {
                            ResourceKind::Food => format!("{} food", amt),
                            ResourceKind::Gold => format!("{} gold", amt),
                        })
                        .collect();
                    let next_name = WALL_TIER_NAMES[level];
                    let can_afford = costs.iter().all(|(r, amt)| match r {
                        ResourceKind::Food => {
                            bld.food_storage.food.get(town_idx).copied().unwrap_or(0) >= *amt
                        }
                        ResourceKind::Gold => {
                            bld.gold_storage.gold.get(town_idx).copied().unwrap_or(0) >= *amt
                        }
                    });

                    ui.separator();
                    let btn_text = format!("Upgrade to {} ({})", next_name, cost_str.join(", "));
                    let btn = ui.add_enabled(
                        can_afford,
                        egui::Button::new(egui::RichText::new(btn_text).color(if can_afford {
                            egui::Color32::from_rgb(80, 200, 200)
                        } else {
                            egui::Color32::from_rgb(120, 120, 120)
                        })),
                    );
                    if btn.clicked() && can_afford {
                        // Deduct costs
                        for (r, amt) in costs {
                            match r {
                                ResourceKind::Food => {
                                    if let Some(f) = bld.food_storage.food.get_mut(town_idx) {
                                        *f -= amt;
                                    }
                                }
                                ResourceKind::Gold => {
                                    if let Some(g) = bld.gold_storage.gold.get_mut(town_idx) {
                                        *g -= amt;
                                    }
                                }
                            }
                        }
                        // Upgrade wall level + HP via ECS
                        let new_level = (level + 1) as u8;
                        let new_hp = WALL_TIER_HP[level]; // level is 0-indexed for next tier
                        if let Some(e) = bld_entity {
                            if let Ok(mut wl) = bld.wall_level_q.get_mut(e) {
                                wl.0 = new_level;
                            }
                            if let Ok(mut health) = bld.building_health.get_mut(e) {
                                health.0 = new_hp;
                            }
                        }
                        dirty_writers
                            .building_grid
                            .write(crate::messages::BuildingGridDirtyMsg);
                    }
                } else {
                    ui.colored_label(egui::Color32::from_rgb(200, 180, 40), "Max tier reached");
                }
            }
        }

        BuildingKind::Tower => {
            // Resolve per-instance stats from ECS TowerBuildingState
            let slot = bld_slot.unwrap_or(usize::MAX);
            let (level, upgrade_levels_clone) = bld_entity
                .and_then(|e| bld.tower_bld_q.get(e).ok())
                .map(|tbs| (level_from_xp(tbs.xp), tbs.upgrade_levels.clone()))
                .unwrap_or((0, Vec::new()));
            let stats = resolve_tower_instance_stats(level, &upgrade_levels_clone);

            // Tower combat stats (resolved)
            ui.label(format!("Range: {:.0}px", stats.range));
            ui.label(format!("Damage: {:.1}", stats.damage));
            ui.label(format!("Cooldown: {:.2}s", stats.cooldown));
            if stats.hp_regen > 0.0 {
                ui.label(format!("HP Regen: {:.1}/s", stats.hp_regen));
            }

            // HP bar
            if let Some(&entity) = bld.entity_map.entities.get(&slot) {
                if let Ok(health) = bld.building_health.get(entity) {
                    let max_hp = stats.max_hp;
                    let pct = health.0 / max_hp;
                    let color = if pct > 0.5 {
                        egui::Color32::from_rgb(80, 200, 80)
                    } else {
                        egui::Color32::from_rgb(200, 80, 80)
                    };
                    ui.horizontal(|ui| {
                        ui.label("HP:");
                        ui.add(
                            egui::ProgressBar::new(pct)
                                .text(format!("{:.0}/{:.0}", health.0, max_hp))
                                .fill(color),
                        );
                    });
                }
            }

            // Kills / XP / Level
            if let Some(tbs) = bld_entity.and_then(|e| bld.tower_bld_q.get(e).ok()) {
                let xp_next = (level + 1) * (level + 1) * 100;
                ui.label(format!(
                    "Kills: {}  Lv.{}  XP: {}/{}",
                    tbs.kills, level, tbs.xp, xp_next
                ));
            }

            // Upgrade button — opens popup window
            if ui.button(egui::RichText::new("Upgrades").strong()).clicked() {
                ui_state.tower_upgrade_slot = Some(slot);
            }
        }

        BuildingKind::Merchant => {
            let tidx = town_idx;
            let stock = bld.merchant_inv.stocks.get(tidx);
            let stock_count = stock.map(|s| s.items.len()).unwrap_or(0);
            let timer = stock.map(|s| s.refresh_timer).unwrap_or(0.0);
            ui.label(format!("Stock ({} items) — refresh in {:.1}h", stock_count, timer));
            ui.separator();

            // List stock items with Buy buttons
            let mut buy_id: Option<u64> = None;
            if let Some(stock) = bld.merchant_inv.stocks.get(tidx) {
                for item in &stock.items {
                    let (r, g, b) = item.rarity.color();
                    let cost = item.rarity.gold_cost();
                    let gold = bld.gold_storage.gold.get(tidx).copied().unwrap_or(0);
                    let can_afford = gold >= cost;
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&item.name)
                                .color(egui::Color32::from_rgb(r, g, b)),
                        );
                        ui.label(format!("{:?} +{:.0}%", item.slot, item.stat_bonus * 100.0));
                        let btn = ui.add_enabled(
                            can_afford,
                            egui::Button::new(format!("Buy {}g", cost)),
                        );
                        if btn.clicked() && can_afford {
                            buy_id = Some(item.id);
                        }
                    });
                }
            }
            // Process buy
            if let Some(id) = buy_id {
                if let Some(item) = bld.merchant_inv.remove(tidx, id) {
                    let cost = item.rarity.gold_cost();
                    if let Some(g) = bld.gold_storage.gold.get_mut(tidx) {
                        *g -= cost;
                    }
                    bld.town_inventory.add(tidx, item);
                }
            }

            // Sell section — items from TownInventory
            ui.separator();
            ui.label("Sell from inventory:");
            let mut sell_id: Option<u64> = None;
            if let Some(items) = bld.town_inventory.items.get(tidx) {
                for item in items {
                    let (r, g, b) = item.rarity.color();
                    let sell_price = item.rarity.gold_cost() / 2;
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&item.name)
                                .color(egui::Color32::from_rgb(r, g, b)),
                        );
                        ui.label(format!("{:?}", item.slot));
                        if ui.button(format!("Sell {}g", sell_price)).clicked() {
                            sell_id = Some(item.id);
                        }
                    });
                }
            }
            // Process sell
            if let Some(id) = sell_id {
                if let Some(item) = bld.town_inventory.remove(tidx, id) {
                    let sell_price = item.rarity.gold_cost() / 2;
                    if let Some(g) = bld.gold_storage.gold.get_mut(tidx) {
                        *g += sell_price;
                    }
                }
            }

            // Reroll button
            ui.separator();
            let reroll_cost = 50;
            let gold = bld.gold_storage.gold.get(tidx).copied().unwrap_or(0);
            let can_reroll = gold >= reroll_cost;
            let btn = ui.add_enabled(
                can_reroll,
                egui::Button::new(format!("Reroll Stock ({}g)", reroll_cost)),
            );
            if btn.clicked() && can_reroll {
                if let Some(g) = bld.gold_storage.gold.get_mut(tidx) {
                    *g -= reroll_cost;
                }
                bld.merchant_inv.refresh(tidx, &mut bld.next_loot_id);
            }
        }

        BuildingKind::Casino => {
            if ui.button(egui::RichText::new("Open Casino").size(16.0).strong()).clicked() {
                ui_state.casino_open = true;
            }
        }

        _ => {
            if let Some(spawner) = def.spawner {
                let spawns_label = npc_def(Job::from_i32(spawner.job)).label;
                if let Some(inst) = bld
                    .entity_map
                    .find_by_position(world_pos)
                    .filter(|i| crate::constants::building_def(i.kind).spawner.is_some())
                {
                    ui.label(format!("Spawns: {}", spawns_label));
                    let spawner_state = bld.entity_map.entities.get(&inst.slot)
                        .and_then(|&e| bld.spawner_q.get(e).ok());
                    let npc_uid_opt = spawner_state.and_then(|s| s.npc_uid);
                    let respawn_timer = spawner_state.map(|s| s.respawn_timer).unwrap_or(0.0);
                    if let Some(npc_uid) = npc_uid_opt {
                        if let Some(slot) = bld.entity_map.slot_for_uid(npc_uid) {
                            if let Some(action) = npc_link(ui, meta_cache, slot) {
                                return Some(action);
                            }
                            ui.colored_label(egui::Color32::from_rgb(80, 200, 80), "Alive");
                            if let Some(npc) = bld.entity_map.get_npc(slot) {
                                let mut parts: Vec<&str> = Vec::new();
                                let combat_name = bld
                                    .combat_state_q
                                    .get(npc.entity)
                                    .map(|cs| cs.name())
                                    .unwrap_or("");
                                if !combat_name.is_empty() {
                                    parts.push(combat_name);
                                }
                                parts.push(
                                    bld.activity_q
                                        .get(npc.entity)
                                        .map(|a| a.name())
                                        .unwrap_or("Unknown"),
                                );
                                ui.label(format!("State: {}", parts.join(", ")));
                                if let Some(sq) = bld.squad_id_q.get(npc.entity).ok().map(|s| s.0) {
                                    ui.label(format!("Squad: {}", sq + 1));
                                }
                                let has_patrol = bld
                                    .patrol_route_q
                                    .get(npc.entity)
                                    .is_ok_and(|r| !r.posts.is_empty());
                                ui.label(format!(
                                    "Patrol route: {}",
                                    if has_patrol { "yes" } else { "none" }
                                ));
                                if slot * 2 + 1 < gpu_state.positions.len() {
                                    let px = gpu_state.positions[slot * 2];
                                    let py = gpu_state.positions[slot * 2 + 1];
                                    if px > -9000.0 {
                                        ui.label(format!("GPU pos: ({:.0}, {:.0})", px, py));
                                    }
                                }
                                ui.label(format!(
                                    "Home: ({:.0}, {:.0})",
                                    bld.home_q.get(npc.entity).map(|h| h.0.x).unwrap_or(0.0),
                                    bld.home_q.get(npc.entity).map(|h| h.0.y).unwrap_or(0.0)
                                ));
                            }
                        }
                    } else if respawn_timer > 0.0 {
                        ui.colored_label(
                            egui::Color32::from_rgb(200, 200, 40),
                            format!("Respawning in {:.0}h", respawn_timer),
                        );
                    } else {
                        ui.colored_label(egui::Color32::from_rgb(200, 200, 40), "Spawning...");
                    }
                }
                if def.kind == BuildingKind::MinerHome {
                    ui.separator();
                    let mh_slot = bld
                        .entity_map
                        .find_by_position(world_pos)
                        .filter(|i| i.kind == BuildingKind::MinerHome)
                        .map(|i| i.slot);
                    if let Some(mh_slot) = mh_slot {
                        if let Some(action) = mine_assignment_ui(
                            ui,
                            world_data,
                            &bld.entity_map,
                            mh_slot,
                            world_pos,
                            dirty_writers,
                            ui_state,
                            &mut bld.miner_cfg_q,
                        ) {
                            return Some(action);
                        }
                    }
                }
            }
        }
    } } // end if !is_constructing + match

    // Debug IDs: show slot + UID + world coords for BRP queries, plus copy button
    if settings.debug_ids {
        ui.separator();
        let selected_slot = bld.selected_building.slot.or_else(|| {
            bld.entity_map
                .find_by_position(world_pos)
                .map(|inst| inst.slot)
        });
        if let Some(slot) = selected_slot {
            let uid = bld.entity_map.uid_for_slot(slot);
            let uid_str = uid.map_or("?".to_string(), |u| u.0.to_string());
            ui.label(format!("Slot: {}  UID: {}  Pos: ({:.0}, {:.0})", slot, uid_str, world_pos.x, world_pos.y));

            if ui.button("Copy Debug Info").clicked() {
                let max_hp = crate::constants::building_def(kind).hp;
                let hp = bld.entity_map.entities.get(&slot)
                    .and_then(|&e| bld.building_health.get(e).ok())
                    .map(|h| h.0)
                    .unwrap_or(0.0);
                let town_name = world_data.towns.get(town_idx)
                    .map(|t| t.name.as_str()).unwrap_or("?");
                let faction_text = world_data.towns.get(town_idx)
                    .map(|t| format!("{} (F{})", t.name, t.faction))
                    .unwrap_or_else(|| "?".to_string());
                let mut info = format!(
                    "{name} [{kind:?}]\n\
                     Slot: {slot}  UID: {uid}\n\
                     Town: {town}  Faction: {faction}\n\
                     Pos: ({px:.0}, {py:.0})  Grid: ({col}, {row})\n\
                     HP: {hp:.0}/{max:.0}\n",
                    name = def.label,
                    kind = kind,
                    slot = slot,
                    uid = uid_str,
                    town = town_name,
                    faction = faction_text,
                    px = world_pos.x,
                    py = world_pos.y,
                    col = col,
                    row = row,
                    hp = hp,
                    max = max_hp,
                );
                // Spawner NPC state
                if let Some(spawner) = def.spawner {
                    let spawns_label = npc_def(Job::from_i32(spawner.job)).label;
                    info.push_str(&format!("Spawns: {}\n", spawns_label));
                    if let Some(ss) = bld_entity.and_then(|e| bld.spawner_q.get(e).ok()) {
                        if let Some(npc_uid) = ss.npc_uid {
                            if let Some(npc_slot) = bld.entity_map.slot_for_uid(npc_uid) {
                                if npc_slot < meta_cache.0.len() {
                                    let meta = &meta_cache.0[npc_slot];
                                    info.push_str(&format!(
                                        "NPC: {} (Lv.{}) slot={} uid={}\n",
                                        meta.name, meta.level, npc_slot, npc_uid.0
                                    ));
                                }
                            }
                        } else if ss.respawn_timer > 0.0 {
                            info.push_str(&format!("Respawning in {:.0}h\n", ss.respawn_timer));
                        }
                    }
                }
                *copy_text = Some(info);
            }
        }
    }
    None
}

// ============================================================================
// TARGET OVERLAY
// ============================================================================

/// Draw a target indicator line from selected NPC to its movement target.
/// Shows full A* path through all remaining waypoints when pathfinding is active.
/// Uses egui painter on the background layer so it renders over the game viewport.
pub fn target_overlay_system(
    mut contexts: EguiContexts,
    selected: Res<SelectedNpc>,
    gpu_state: Res<GpuReadState>,
    buffer_writes: Res<EntityGpuState>,
    entity_map: Res<crate::entity_map::EntityMap>,
    grid: Res<WorldGrid>,
    path_q: Query<&NpcPath>,
    camera_query: Query<(&Transform, &Projection), With<crate::render::MainCamera>>,
    windows: Query<&Window>,
) -> Result {
    if selected.0 < 0 {
        return Ok(());
    }
    let idx = selected.0 as usize;

    let positions = &gpu_state.positions;
    let targets = &buffer_writes.targets;
    if idx * 2 + 1 >= positions.len() || idx * 2 + 1 >= targets.len() {
        return Ok(());
    }

    let npc_x = positions[idx * 2];
    let npc_y = positions[idx * 2 + 1];
    if npc_x < -9000.0 {
        return Ok(());
    }

    let tgt_x = targets[idx * 2];
    let tgt_y = targets[idx * 2 + 1];

    // Skip if target == position (stationary)
    let dx = tgt_x - npc_x;
    let dy = tgt_y - npc_y;
    if dx * dx + dy * dy < 4.0 {
        return Ok(());
    }

    let Ok(window) = windows.single() else {
        return Ok(());
    };
    let Ok((transform, projection)) = camera_query.single() else {
        return Ok(());
    };

    let zoom = match projection {
        Projection::Orthographic(ortho) => 1.0 / ortho.scale,
        _ => 1.0,
    };
    let cam = transform.translation.truncate();
    let viewport = egui::Vec2::new(window.width(), window.height());
    let center = viewport * 0.5;

    let to_screen = |wx: f32, wy: f32| -> egui::Pos2 {
        egui::Pos2::new(
            center.x + (wx - cam.x) * zoom,
            center.y - (wy - cam.y) * zoom,
        )
    };

    let npc_screen = to_screen(npc_x, npc_y);

    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::background());
    let line_color = egui::Color32::from_rgba_unmultiplied(255, 220, 50, 200);
    let stroke = egui::Stroke::new(2.5, line_color);
    let dot_color = egui::Color32::from_rgba_unmultiplied(255, 220, 50, 160);

    // Try to get the full A* path for this NPC
    let npc_path = entity_map
        .entities
        .get(&idx)
        .and_then(|&entity| path_q.get(entity).ok());

    let final_screen = if let Some(path) = npc_path.filter(|p| p.current < p.waypoints.len()) {
        // Draw full path: NPC → remaining waypoints → goal_world
        let mut prev = npc_screen;
        for wp in &path.waypoints[path.current..] {
            let world_pos = grid.grid_to_world(wp.x as usize, wp.y as usize);
            let wp_screen = to_screen(world_pos.x, world_pos.y);
            painter.line_segment([prev, wp_screen], stroke);
            // Small dot at intermediate waypoint
            painter.circle_filled(wp_screen, 3.0, dot_color);
            prev = wp_screen;
        }
        // Final segment from last waypoint to goal_world
        let goal_screen = to_screen(path.goal_world.x, path.goal_world.y);
        painter.line_segment([prev, goal_screen], stroke);
        goal_screen
    } else {
        // No A* path — single line to GPU target (direct movement)
        let tgt_screen = to_screen(tgt_x, tgt_y);
        painter.line_segment([npc_screen, tgt_screen], stroke);
        tgt_screen
    };

    // Diamond marker at final destination
    let s = 7.0;
    let diamond = [
        egui::Pos2::new(final_screen.x, final_screen.y - s),
        egui::Pos2::new(final_screen.x + s, final_screen.y),
        egui::Pos2::new(final_screen.x, final_screen.y + s),
        egui::Pos2::new(final_screen.x - s, final_screen.y),
    ];
    let fill = egui::Color32::from_rgba_unmultiplied(255, 220, 50, 240);
    painter.add(egui::Shape::convex_polygon(
        diamond.to_vec(),
        fill,
        egui::Stroke::NONE,
    ));

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
    gpu_state: Res<GpuReadState>,
    camera_query: Query<(&Transform, &Projection), With<crate::render::MainCamera>>,
    windows: Query<&Window>,
    entity_map: Res<EntityMap>,
    manual_target_q: Query<&ManualTarget>,
    npc_flags_q: Query<&NpcFlags>,
) -> Result {
    let Ok(window) = windows.single() else {
        return Ok(());
    };
    let Ok((transform, projection)) = camera_query.single() else {
        return Ok(());
    };

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
        egui::Color32::from_rgb(255, 80, 80),   // red
        egui::Color32::from_rgb(80, 180, 255),  // blue
        egui::Color32::from_rgb(80, 220, 80),   // green
        egui::Color32::from_rgb(255, 200, 40),  // yellow
        egui::Color32::from_rgb(200, 80, 255),  // purple
        egui::Color32::from_rgb(255, 140, 40),  // orange
        egui::Color32::from_rgb(40, 220, 200),  // teal
        egui::Color32::from_rgb(255, 100, 180), // pink
        egui::Color32::from_rgb(180, 180, 80),  // olive
        egui::Color32::from_rgb(140, 140, 255), // light blue
    ];

    if !squad_state.box_selecting {
        for (i, squad) in squad_state.squads.iter().enumerate() {
            if !squad.is_player() {
                continue;
            }
            let Some(target) = squad.target else { continue };
            if squad.members.is_empty() {
                continue;
            }

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
    }

    // Crosshair on DirectControl attack targets
    let positions = &gpu_state.positions;
    let xh_color = egui::Color32::from_rgba_unmultiplied(80, 220, 80, 200);
    // Collect unique target positions to avoid drawing multiple crosshairs on same spot
    let mut drawn_targets: Vec<egui::Pos2> = Vec::new();
    let dc_targets: Vec<ManualTarget> = dc_slots(&squad_state, &entity_map, |e| {
        npc_flags_q.get(e).is_ok_and(|f| f.direct_control)
    })
        .iter()
        .filter_map(|&slot| entity_map.get_npc(slot))
        .filter_map(|npc| manual_target_q.get(npc.entity).ok().cloned())
        .collect();
    for mt in &dc_targets {
        let world_pos = match mt {
            ManualTarget::Npc(slot) => {
                if *slot * 2 + 1 >= positions.len() {
                    continue;
                }
                let px = positions[*slot * 2];
                let py = positions[*slot * 2 + 1];
                if px < -9000.0 {
                    continue;
                }
                Vec2::new(px, py)
            }
            ManualTarget::Building(pos) => *pos,
            ManualTarget::Position(pos) => *pos,
        };
        let sp = egui::Pos2::new(
            center.x + (world_pos.x - cam.x) * zoom,
            center.y - (world_pos.y - cam.y) * zoom,
        );
        // Skip if we already drew a crosshair near this screen position
        if drawn_targets
            .iter()
            .any(|p| (p.x - sp.x).abs() < 2.0 && (p.y - sp.y).abs() < 2.0)
        {
            continue;
        }
        drawn_targets.push(sp);
        let r = 10.0_f32;
        let gap = 3.0_f32;
        painter.line_segment(
            [
                egui::Pos2::new(sp.x - r, sp.y),
                egui::Pos2::new(sp.x - gap, sp.y),
            ],
            egui::Stroke::new(2.0, xh_color),
        );
        painter.line_segment(
            [
                egui::Pos2::new(sp.x + gap, sp.y),
                egui::Pos2::new(sp.x + r, sp.y),
            ],
            egui::Stroke::new(2.0, xh_color),
        );
        painter.line_segment(
            [
                egui::Pos2::new(sp.x, sp.y - r),
                egui::Pos2::new(sp.x, sp.y - gap),
            ],
            egui::Stroke::new(2.0, xh_color),
        );
        painter.line_segment(
            [
                egui::Pos2::new(sp.x, sp.y + gap),
                egui::Pos2::new(sp.x, sp.y + r),
            ],
            egui::Stroke::new(2.0, xh_color),
        );
        painter.circle_stroke(sp, r, egui::Stroke::new(1.5, xh_color));
    }

    // Placement mode cursor hint
    if squad_state.placing_target && squad_state.selected >= 0 {
        if let Some(cursor_pos) = window.cursor_position() {
            let cursor_egui = egui::Pos2::new(cursor_pos.x, cursor_pos.y);
            let hint_color = egui::Color32::from_rgba_unmultiplied(255, 255, 100, 160);
            painter.circle_stroke(cursor_egui, 12.0, egui::Stroke::new(2.0, hint_color));
        }
    }

    // Box-select drag rectangle
    if squad_state.box_selecting {
        if let Some(start) = squad_state.drag_start {
            if let Some(cursor_pos) = window.cursor_position() {
                let start_screen = egui::Pos2::new(
                    center.x + (start.x - cam.x) * zoom,
                    center.y - (start.y - cam.y) * zoom,
                );
                let end_screen = egui::Pos2::new(cursor_pos.x, cursor_pos.y);
                let rect = egui::Rect::from_two_pos(start_screen, end_screen);
                let fill = egui::Color32::from_rgba_unmultiplied(80, 220, 80, 30);
                let stroke =
                    egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(80, 220, 80, 180));
                painter.rect_filled(rect, 0.0, fill);
                painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Outside);
            }
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
    let selected_faction =
        if ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Factions {
            ui_state.factions_overlay_faction
        } else {
            None
        };
    if !show_all && selected_faction.is_none() {
        return Ok(());
    }

    let Ok(window) = windows.single() else {
        return Ok(());
    };
    let Ok((transform, projection)) = camera_query.single() else {
        return Ok(());
    };

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
        if !show_all && selected_faction != Some(faction) {
            continue;
        }
        let Some(target_world) = squad.target else {
            continue;
        };
        if squad.members.is_empty() {
            continue;
        }
        let Some(start_world) = start_by_faction.get(&faction).copied() else {
            continue;
        };

        let color_idx = color_idx_by_faction.entry(faction).or_insert(0usize);
        let color = palette[*color_idx % palette.len()];
        *color_idx += 1;
        let start = to_screen(start_world);
        let end = to_screen(target_world);
        let line = end - start;
        let len = line.length();
        if len < 6.0 {
            continue;
        }
        let dir = line / len;
        let perp = egui::vec2(-dir.y, dir.x);

        painter.line_segment([start, end], egui::Stroke::new(2.0, color));

        let head_len = 12.0;
        let head_w = 6.0;
        let base = end - dir * head_len;
        let p1 = base + perp * head_w;
        let p2 = base - perp * head_w;
        painter.add(egui::Shape::convex_polygon(
            vec![end, p1, p2],
            color,
            egui::Stroke::NONE,
        ));

        let label_pos = end + perp * 10.0;
        let label = if show_all {
            let town_label = world_data.towns.iter()
                .find(|t| t.faction == faction)
                .map(|t| t.name.as_str())
                .unwrap_or("?");
            format!("{} Squad {}", town_label, si + 1)
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
    let Some(track_idx) = audio.last_track else {
        return Ok(());
    };
    // Persist track when auto-advance changes it
    if settings.jukebox_track != Some(track_idx) {
        settings.jukebox_track = Some(track_idx);
        crate::settings::save_settings(&settings);
    }

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
                                ui.selectable_value(
                                    &mut selected,
                                    i,
                                    crate::systems::audio::track_display_name(i),
                                );
                            }
                        });
                    // Switch track if user picked a different one
                    if selected != track_idx {
                        audio.play_next = Some(selected);
                        settings.jukebox_track = Some(selected);
                        crate::settings::save_settings(&settings);
                        if let Ok((entity, _)) = music_query.single() {
                            commands.entity(entity).despawn();
                        }
                    }

                    if let Ok((entity, sink)) = music_query.single() {
                        let paused = sink.is_paused();
                        if ui.small_button(if paused { "▶" } else { "⏸" }).clicked() {
                            if paused { sink.play() } else { sink.pause() }
                            settings.jukebox_paused = !paused;
                            crate::settings::save_settings(&settings);
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
                                ui.selectable_value(
                                    &mut audio.music_speed,
                                    val,
                                    format!("{}%", pct),
                                );
                            }
                            // 150% to 500% in 50% steps
                            for pct in (150..=500).step_by(50) {
                                let val = pct as f32 / 100.0;
                                ui.selectable_value(
                                    &mut audio.music_speed,
                                    val,
                                    format!("{}%", pct),
                                );
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
                        settings.jukebox_loop = audio.loop_current;
                        crate::settings::save_settings(&settings);
                    }
                    if resp.hovered() {
                        resp.clone().show_tooltip_text(if audio.loop_current {
                            "Loop: ON"
                        } else {
                            "Loop: OFF"
                        });
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
pub fn save_toast_system(mut contexts: EguiContexts, toast: Res<crate::save::SaveToast>) -> Result {
    if toast.timer <= 0.0 {
        return Ok(());
    }
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
                    ui.label(
                        egui::RichText::new(&toast.message)
                            .size(18.0)
                            .strong()
                            .color(egui::Color32::from_rgba_unmultiplied(255, 255, 200, alpha)),
                    );
                });
        });

    Ok(())
}
