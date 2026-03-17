//! Building and NPC inspector panel.

use super::building_inspector::selected_building_info;
use super::npc_inspector::inspector_content;
use super::{BottomPanelData, dc_slots};
use crate::components::*;
use crate::constants::{EffectDisplay, ResourceKind, UpgradeStatKind, building_def};
use crate::gpu::EntityGpuState;
use crate::render::MainCamera;
use crate::resources::*;
use crate::settings::UserSettings;
use crate::systems::stats::{CombatConfig, UnequipItemMsg, level_from_xp};
use crate::world::{BuildingKind, WorldData, WorldGrid};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

// ============================================================================
// INSPECTOR LINK HELPERS
// ============================================================================

/// Action returned by inspector UI when user clicks an entity link.
#[allow(dead_code)]
pub(crate) enum InspectorAction {
    SelectNpc(i32),
    SelectBuilding(usize),
}

/// Render an NPC name as a clickable link. Returns action if clicked.
pub(crate) fn npc_link(
    ui: &mut egui::Ui,
    npc_stats_q: &Query<&mut NpcStats>,
    entity_map: &EntityMap,
    slot: usize,
) -> Option<InspectorAction> {
    if let Some(npc) = entity_map.get_npc(slot) {
        if let Ok(stats) = npc_stats_q.get(npc.entity) {
            let level = crate::systems::stats::level_from_xp(stats.xp);
            if ui.link(format!("{} (Lv.{})", stats.name, level)).clicked() {
                return Some(InspectorAction::SelectNpc(slot as i32));
            }
        }
    }
    None
}

/// Render a building name as a clickable link. Returns action if clicked.
pub(crate) fn building_link(
    ui: &mut egui::Ui,
    label: &str,
    slot: usize,
) -> Option<InspectorAction> {
    if ui.link(label).clicked() {
        Some(InspectorAction::SelectBuilding(slot))
    } else {
        None
    }
}

/// Apply an inspector action: select entity, deselect the other, jump camera.
pub(crate) fn apply_inspector_action(
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

/// Bundled resources for building inspector.
#[derive(SystemParam)]
pub struct BuildingInspectorData<'w, 's> {
    pub selected_building: ResMut<'w, SelectedBuilding>,
    pub grid: Res<'w, WorldGrid>,
    pub town_access: crate::systemparams::TownAccess<'w, 's>,
    pub combat_config: Res<'w, CombatConfig>,
    pub entity_map: ResMut<'w, EntityMap>,
    pub building_health: Query<'w, 's, &'static mut Health, With<Building>>,
    pub npc_flags_q: Query<'w, 's, &'static mut NpcFlags>,
    pub squad_id_q: Query<'w, 's, &'static SquadId>,
    pub manual_target_q: Query<'w, 's, &'static ManualTarget>,
    pub activity_q: Query<'w, 's, &'static Activity>,
    pub npc_health_q: Query<'w, 's, &'static Health, Without<Building>>,
    pub cached_stats_q: Query<'w, 's, &'static CachedStats>,
    pub combat_state_q: Query<'w, 's, &'static CombatState>,
    pub energy_q: Query<'w, 's, &'static Energy>,
    pub personality_q: Query<'w, 's, &'static Personality>,
    pub skills_q: Query<'w, 's, &'static NpcSkills>,
    pub home_q: Query<'w, 's, &'static Home>,
    pub work_state_q: Query<'w, 's, &'static NpcWorkState>,
    pub equipment_q: Query<'w, 's, &'static NpcEquipment>,
    pub carried_loot_q: Query<'w, 's, &'static CarriedLoot>,
    pub patrol_route_q: Query<'w, 's, &'static PatrolRoute>,
    pub last_hit_by_q: Query<'w, 's, &'static LastHitBy>,
    pub merchant_inv: ResMut<'w, MerchantInventory>,
    pub next_loot_id: ResMut<'w, NextLootItemId>,
    pub tower_bld_q: Query<'w, 's, &'static mut TowerBuildingState, With<Building>>,
    pub miner_cfg_q: Query<'w, 's, &'static mut MinerHomeConfig>,
    pub production_q: Query<'w, 's, &'static ProductionState, With<Building>>,
    pub construction_q: Query<'w, 's, &'static ConstructionProgress, With<Building>>,
    pub spawner_q: Query<'w, 's, &'static SpawnerState, With<Building>>,
    pub wall_level_q: Query<'w, 's, &'static mut WallLevel, With<Building>>,
    pub waypoint_order_q: Query<'w, 's, &'static WaypointOrder, With<Building>>,
    pub farm_mode_q: Query<'w, 's, &'static mut FarmModeComp, With<Building>>,
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
pub struct InspectorRenameState {
    pub slot: i32,
    pub text: String,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum InspectorNpcTab {
    #[default]
    Overview,
    Loadout,
    Skills,
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
    mut npc_stats_q: Query<&mut NpcStats>,
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
    })
    .len();
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
                        && bld_data
                            .entity_map
                            .get_npc(data.selected.0 as usize)
                            .is_some()
                    {
                        let npc_name = bld_data
                            .entity_map
                            .get_npc(data.selected.0 as usize)
                            .and_then(|n| npc_stats_q.get(n.entity).ok())
                            .map(|s| s.name.clone())
                            .unwrap_or_default();
                        format!("NPC: {}", npc_name)
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
                            &mut npc_stats_q,
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
                                        && world_data.towns.get(*ti as usize).is_some_and(|t| {
                                            t.faction == crate::constants::FACTION_PLAYER
                                        })
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
    let inst_exists = bld.entity_map.get_instance(slot).is_some_and(|i| {
        let def = crate::constants::building_def(i.kind);
        def.is_tower && i.kind != BuildingKind::Fountain
    });
    if !inst_exists {
        ui_state.tower_upgrade_slot = None;
        return;
    }

    // Read tower data from ECS TowerBuildingState + EntityMap for town_idx
    let Some(inst) = bld.entity_map.get_instance(slot) else {
        return;
    };
    let town_idx = inst.town_idx as usize;
    let Some(&tower_entity) = bld.entity_map.entities.get(&slot) else {
        return;
    };
    let Ok(tbs) = bld.tower_bld_q.get(tower_entity) else {
        return;
    };
    let level = level_from_xp(tbs.xp);
    let upgrade_levels = tbs.upgrade_levels.clone();
    let auto_flags = tbs.auto_upgrade_flags.clone();

    let food = bld.town_access.food(town_idx as i32);
    let gold = bld.town_access.gold(town_idx as i32);
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
                        ResourceKind::Wood | ResourceKind::Stone => false,
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
                                    ResourceKind::Wood => format!("{} wood", total),
                                    ResourceKind::Stone => format!("{} stone", total),
                                }
                            })
                            .collect();
                        let cost_text = format!("Buy: {}", cost_parts.join(", "));

                        let btn = egui::Button::new(egui::RichText::new(&cost_text).size(14.0))
                            .min_size(egui::vec2(ui.available_width(), 28.0));

                        if ui.add_enabled(can_afford, btn).clicked() {
                            // Deduct resources
                            for (res, base) in upg.cost {
                                let total = base * cost_mult;
                                match res {
                                    ResourceKind::Food => {
                                        if let Some(mut f) =
                                            bld.town_access.food_mut(town_idx as i32)
                                        {
                                            f.0 -= total;
                                        }
                                    }
                                    ResourceKind::Gold => {
                                        if let Some(mut g) =
                                            bld.town_access.gold_mut(town_idx as i32)
                                        {
                                            g.0 -= total;
                                        }
                                    }
                                    ResourceKind::Wood | ResourceKind::Stone => {}
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
