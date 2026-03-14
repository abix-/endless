use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::components::*;
use crate::constants::npc_def;
use crate::resources::*;
use super::left_panel::InventoryParams;

// Armory visual theme -- dark steel with olive accents
const STEEL_BG: egui::Color32 = egui::Color32::from_rgb(35, 38, 42);
const STEEL_BORDER: egui::Color32 = egui::Color32::from_rgb(90, 95, 100);
const OLIVE_ACCENT: egui::Color32 = egui::Color32::from_rgb(120, 140, 80);
const BRASS_HIGHLIGHT: egui::Color32 = egui::Color32::from_rgb(190, 165, 100);
const SLOT_BG: egui::Color32 = egui::Color32::from_rgb(45, 48, 55);
const SLOT_EMPTY: egui::Color32 = egui::Color32::from_rgb(60, 63, 68);
const ROSTER_SELECTED: egui::Color32 = egui::Color32::from_rgb(55, 70, 50);
const ROSTER_HOVER: egui::Color32 = egui::Color32::from_rgb(50, 55, 60);

fn rarity_color(rarity: crate::constants::Rarity) -> egui::Color32 {
    let (r, g, b) = rarity.color();
    egui::Color32::from_rgb(r, g, b)
}

pub fn armory_window_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut selected_npc: ResMut<SelectedNpc>,
    entity_map: Res<EntityMap>,
    npc_stats_q: Query<&NpcStats>,
    equipment_q: Query<(Entity, &NpcEquipment, &Job, &TownId, &GpuSlot)>,
    mut inv: InventoryParams,
    town_access: crate::systemparams::TownAccess,
) -> Result {
    if !ui_state.armory_open {
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;

    let frame = egui::Frame::new()
        .fill(STEEL_BG)
        .stroke(egui::Stroke::new(2.0, STEEL_BORDER))
        .inner_margin(egui::Margin::same(16))
        .corner_radius(egui::CornerRadius::same(6));

    let mut open = true;
    egui::Window::new("Armory")
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .default_width(720.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(frame)
        .show(ctx, |ui| {
            armory_content(
                ui,
                &mut selected_npc,
                &entity_map,
                &npc_stats_q,
                &equipment_q,
                &mut inv,
                &town_access,
                &mut ui_state,
            );
        });

    if !open {
        ui_state.armory_open = false;
    }

    Ok(())
}

fn armory_content(
    ui: &mut egui::Ui,
    selected_npc: &mut SelectedNpc,
    entity_map: &EntityMap,
    npc_stats_q: &Query<&NpcStats>,
    equipment_q: &Query<(Entity, &NpcEquipment, &Job, &TownId, &GpuSlot)>,
    inv: &mut InventoryParams,
    town_access: &crate::systemparams::TownAccess,
    ui_state: &mut UiState,
) {
    // Determine active town from selected NPC
    let town_idx: usize = if selected_npc.0 >= 0 {
        entity_map
            .get_npc(selected_npc.0 as usize)
            .and_then(|npc| {
                equipment_q
                    .get(npc.entity)
                    .ok()
                    .map(|(_, _, _, tid, _)| tid.0.max(0) as usize)
            })
            .unwrap_or(0)
    } else {
        0
    };

    // Collect equippable NPCs for this town
    let mut roster: Vec<(Entity, usize, String, Job)> = Vec::new();
    for (entity, _equip, job, tid, gpu_slot) in equipment_q.iter() {
        if tid.0 != town_idx as i32 { continue; }
        let def = npc_def(*job);
        if def.equip_slots.is_empty() { continue; }
        let name = npc_stats_q.get(entity)
            .map(|s| s.name.clone())
            .unwrap_or_else(|_| "NPC".into());
        roster.push((entity, gpu_slot.0, name, *job));
    }
    roster.sort_by_key(|(_, slot, _, _)| *slot);

    // Header
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("Town {} Armory", town_idx + 1))
                .strong()
                .size(18.0)
                .color(BRASS_HIGHLIGHT),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!("{} units", roster.len()))
                    .size(12.0)
                    .color(egui::Color32::from_rgb(150, 150, 150)),
            );
        });
    });
    ui.add_space(8.0);

    // Three-column layout
    ui.columns(3, |cols| {
        // LEFT: Roster rail
        roster_panel(&mut cols[0], &roster, selected_npc, entity_map);

        // CENTER: Equipment board
        equipment_panel(
            &mut cols[1],
            selected_npc,
            entity_map,
            npc_stats_q,
            equipment_q,
            inv,
            town_idx,
        );

        // RIGHT: Town inventory
        inventory_panel(
            &mut cols[2],
            selected_npc,
            entity_map,
            equipment_q,
            npc_stats_q,
            inv,
            town_access,
            ui_state,
            town_idx,
        );
    });
}

fn roster_panel(
    ui: &mut egui::Ui,
    roster: &[(Entity, usize, String, Job)],
    selected_npc: &mut SelectedNpc,
    entity_map: &EntityMap,
) {
    ui.label(
        egui::RichText::new("Roster")
            .strong()
            .size(13.0)
            .color(OLIVE_ACCENT),
    );
    ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .max_height(400.0)
        .show(ui, |ui| {
            for (entity, slot, name, job) in roster {
                let is_selected = entity_map
                    .get_npc(selected_npc.0.max(0) as usize)
                    .is_some_and(|npc| npc.entity == *entity);

                let bg = if is_selected { ROSTER_SELECTED } else { SLOT_BG };

                let resp = egui::Frame::new()
                    .fill(bg)
                    .corner_radius(egui::CornerRadius::same(3))
                    .inner_margin(egui::Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let def = npc_def(*job);
                            let (r, g, b) = def.ui_color;
                            ui.label(
                                egui::RichText::new(format!("{:?}", job))
                                    .size(10.0)
                                    .color(egui::Color32::from_rgb(r, g, b)),
                            );
                            ui.label(
                                egui::RichText::new(name)
                                    .size(11.0)
                                    .color(if is_selected {
                                        egui::Color32::WHITE
                                    } else {
                                        egui::Color32::from_rgb(200, 200, 200)
                                    }),
                            );
                        });
                    })
                    .response;

                if resp.interact(egui::Sense::click()).clicked() {
                    selected_npc.0 = *slot as i32;
                }
                if resp.hovered() && !is_selected {
                    ui.painter().rect_stroke(
                        resp.rect,
                        egui::CornerRadius::same(3),
                        egui::Stroke::new(1.0, ROSTER_HOVER),
                        egui::StrokeKind::Outside,
                    );
                }
            }
        });
}

fn equipment_panel(
    ui: &mut egui::Ui,
    selected_npc: &SelectedNpc,
    entity_map: &EntityMap,
    npc_stats_q: &Query<&NpcStats>,
    equipment_q: &Query<(Entity, &NpcEquipment, &Job, &TownId, &GpuSlot)>,
    inv: &mut InventoryParams,
    town_idx: usize,
) {
    use crate::constants::{ALL_EQUIP_KINDS, ItemKind};

    ui.label(
        egui::RichText::new("Equipment")
            .strong()
            .size(13.0)
            .color(OLIVE_ACCENT),
    );
    ui.add_space(4.0);

    let sel = selected_npc.0;
    if sel < 0 {
        ui.label(
            egui::RichText::new("Select a unit from the roster")
                .size(12.0)
                .color(egui::Color32::from_rgb(120, 120, 120)),
        );
        return;
    }

    let Some(npc) = entity_map.get_npc(sel as usize) else {
        ui.label("No NPC selected");
        return;
    };

    let Ok((_, equip, job, _tid, _)) = equipment_q.get(npc.entity) else {
        ui.label("NPC has no equipment");
        return;
    };

    let def = npc_def(*job);
    let name = npc_stats_q.get(npc.entity)
        .map(|s| s.name.as_str())
        .unwrap_or("NPC");

    // NPC header
    let (r, g, b) = def.ui_color;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(name)
                .strong()
                .size(14.0)
                .color(egui::Color32::WHITE),
        );
        ui.label(
            egui::RichText::new(format!("{:?}", job))
                .size(12.0)
                .color(egui::Color32::from_rgb(r, g, b)),
        );
    });
    ui.add_space(6.0);

    if def.equip_slots.is_empty() {
        ui.label("This unit cannot equip items.");
        return;
    }

    // Equipment slots as visual cards
    for &slot_kind in ALL_EQUIP_KINDS {
        if slot_kind == ItemKind::Ring {
            for (ring_idx, ring) in [&equip.ring1, &equip.ring2].iter().enumerate() {
                equipment_slot_card(
                    ui,
                    &format!("Ring {}", ring_idx + 1),
                    ring.as_ref(),
                    || {
                        inv.unequip_writer.write(
                            crate::systems::stats::UnequipItemMsg {
                                npc_entity: npc.entity,
                                slot: slot_kind,
                                ring_index: ring_idx as u8,
                            },
                        );
                    },
                );
            }
        } else {
            let item = equip.slot(slot_kind);
            equipment_slot_card(
                ui,
                slot_kind.label(),
                item.as_ref(),
                || {
                    inv.unequip_writer.write(
                        crate::systems::stats::UnequipItemMsg {
                            npc_entity: npc.entity,
                            slot: slot_kind,
                            ring_index: 0,
                        },
                    );
                },
            );
        }
    }

    // Auto-equip buttons
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("Auto-equip Selected").clicked() {
            inv.auto_equip_writer.write(
                crate::systems::stats::AutoEquipNowMsg {
                    town_idx,
                    npc_entity: Some(npc.entity),
                },
            );
        }
        if ui.button("Auto-equip Town").clicked() {
            inv.auto_equip_writer.write(
                crate::systems::stats::AutoEquipNowMsg {
                    town_idx,
                    npc_entity: None,
                },
            );
        }
    });
}

fn equipment_slot_card(
    ui: &mut egui::Ui,
    label: &str,
    item: Option<&crate::constants::LootItem>,
    mut on_unequip: impl FnMut(),
) {
    let bg = if item.is_some() { SLOT_BG } else { SLOT_EMPTY };
    egui::Frame::new()
        .fill(bg)
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(label)
                        .size(10.0)
                        .color(egui::Color32::from_rgb(140, 140, 140)),
                );
                if let Some(item) = item {
                    ui.label(
                        egui::RichText::new(&item.name)
                            .color(rarity_color(item.rarity)),
                    );
                    ui.label(
                        egui::RichText::new(format!("+{:.0}%", item.stat_bonus * 100.0))
                            .size(11.0)
                            .color(egui::Color32::from_rgb(180, 180, 180)),
                    );
                    if ui.small_button("x").clicked() {
                        on_unequip();
                    }
                } else {
                    ui.label(
                        egui::RichText::new("-- empty --")
                            .size(11.0)
                            .color(egui::Color32::from_rgb(80, 80, 80)),
                    );
                }
            });
        });
    ui.add_space(2.0);
}

fn inventory_panel(
    ui: &mut egui::Ui,
    selected_npc: &SelectedNpc,
    entity_map: &EntityMap,
    equipment_q: &Query<(Entity, &NpcEquipment, &Job, &TownId, &GpuSlot)>,
    _npc_stats_q: &Query<&NpcStats>,
    inv: &mut InventoryParams,
    town_access: &crate::systemparams::TownAccess,
    ui_state: &mut UiState,
    town_idx: usize,
) {
    use crate::constants::{ALL_EQUIP_KINDS, ItemKind};

    ui.label(
        egui::RichText::new("Town Inventory")
            .strong()
            .size(13.0)
            .color(OLIVE_ACCENT),
    );
    ui.add_space(4.0);

    let items = town_access.equipment(town_idx as i32).unwrap_or_default();

    // Slot filter buttons
    let filter = &mut ui_state.inv_slot_filter;
    ui.horizontal_wrapped(|ui| {
        for (i, &slot) in ALL_EQUIP_KINDS.iter().enumerate() {
            let bit = 1u16 << i;
            let enabled = *filter & bit != 0;
            let count = items.iter().filter(|it| it.kind == slot).count();
            let label = format!("{} ({})", slot.label(), count);
            if ui.selectable_label(enabled, egui::RichText::new(label).size(10.0)).clicked() {
                *filter ^= bit;
            }
        }
    });
    let filter_val = *filter;

    let slot_passes = |kind: ItemKind| -> bool {
        let idx = ALL_EQUIP_KINDS.iter().position(|&s| s == kind).unwrap_or(0);
        filter_val & (1u16 << idx) != 0
    };

    // Get selected NPC entity for equip button
    let npc_entity = if selected_npc.0 >= 0 {
        entity_map.get_npc(selected_npc.0 as usize).and_then(|npc| {
            equipment_q.get(npc.entity).ok().and_then(|(_, _, job, tid, _)| {
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

    // Get selected NPC equipment for comparison tooltip
    let selected_equip = if selected_npc.0 >= 0 {
        entity_map.get_npc(selected_npc.0 as usize).and_then(|npc| {
            equipment_q.get(npc.entity).ok().map(|(_, equip, _, _, _)| equip)
        })
    } else {
        None
    };

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!("{} items", items.len()))
            .size(11.0)
            .color(egui::Color32::from_rgb(150, 150, 150)),
    );

    egui::ScrollArea::vertical()
        .max_height(350.0)
        .show(ui, |ui| {
            let mut sorted: Vec<&crate::constants::LootItem> = items
                .iter()
                .filter(|it| slot_passes(it.kind))
                .collect();
            sorted.sort_by(|a, b| {
                let sa = ALL_EQUIP_KINDS.iter().position(|&s| s == a.kind).unwrap_or(0);
                let sb = ALL_EQUIP_KINDS.iter().position(|&s| s == b.kind).unwrap_or(0);
                sa.cmp(&sb)
                    .then(rarity_ord(b.rarity).cmp(&rarity_ord(a.rarity)))
                    .then(b.stat_bonus.partial_cmp(&a.stat_bonus).unwrap_or(std::cmp::Ordering::Equal))
            });

            for item in &sorted {
                let resp = egui::Frame::new()
                    .fill(SLOT_BG)
                    .corner_radius(egui::CornerRadius::same(2))
                    .inner_margin(egui::Margin::symmetric(6, 3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(&item.name)
                                    .color(rarity_color(item.rarity))
                                    .size(11.0),
                            );
                            ui.label(
                                egui::RichText::new(format!("{} +{:.0}%", item.kind.label(), item.stat_bonus * 100.0))
                                    .size(10.0)
                                    .color(egui::Color32::from_rgb(150, 150, 150)),
                            );
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
                    })
                    .response;

                // Comparison tooltip
                if let Some(equip) = selected_equip {
                    if resp.hovered() {
                        let current = match item.kind {
                            ItemKind::Ring => {
                                let b1 = equip.ring1.as_ref().map(|i| i.stat_bonus).unwrap_or(0.0);
                                let b2 = equip.ring2.as_ref().map(|i| i.stat_bonus).unwrap_or(0.0);
                                b1.min(b2)
                            }
                            _ => equip.slot(item.kind).as_ref().map(|i| i.stat_bonus).unwrap_or(0.0),
                        };
                        let diff = item.stat_bonus - current;
                        let arrow = if diff > 0.0 { "^" } else if diff < 0.0 { "v" } else { "=" };
                        resp.show_tooltip_text(format!(
                            "Current: +{:.0}% -> New: +{:.0}% ({}{:.0}%)",
                            current * 100.0,
                            item.stat_bonus * 100.0,
                            arrow,
                            diff * 100.0,
                        ));
                    }
                }

                ui.add_space(1.0);
            }

            if sorted.is_empty() {
                ui.label(
                    egui::RichText::new("No items matching filter")
                        .size(11.0)
                        .color(egui::Color32::from_rgb(100, 100, 100)),
                );
            }
        });
}

fn rarity_ord(r: crate::constants::Rarity) -> u8 {
    match r {
        crate::constants::Rarity::Common => 0,
        crate::constants::Rarity::Uncommon => 1,
        crate::constants::Rarity::Rare => 2,
        crate::constants::Rarity::Epic => 3,
    }
}
