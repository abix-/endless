use super::*;

#[derive(SystemParam)]
pub struct InventoryParams<'w, 's> {
    pub equipment_q: Query<'w, 's, (Entity, &'static NpcEquipment, &'static Job, &'static TownId, &'static GpuSlot)>,
    pub equip_writer: MessageWriter<'w, crate::systems::stats::EquipItemMsg>,
    pub unequip_writer: MessageWriter<'w, crate::systems::stats::UnequipItemMsg>,
    pub auto_equip_writer: MessageWriter<'w, crate::systems::stats::AutoEquipNowMsg>,
    pub catalog: Res<'w, HelpCatalog>,
}

// ============================================================================
// INVENTORY TAB
// ============================================================================

fn rarity_color(rarity: Rarity) -> egui::Color32 {
    let (r, g, b) = rarity.color();
    egui::Color32::from_rgb(r, g, b)
}

pub(crate) fn inventory_content(
    ui: &mut egui::Ui,
    inv: &mut InventoryParams,
    selected_npc: &SelectedNpc,
    npc_stats_q: &Query<&mut NpcStats>,
    entity_map: &EntityMap,
    ui_state: &mut UiState,
    town_access: &mut crate::systemparams::TownAccess<'_, '_>,
) {
    // Derive town from selected NPC, fallback to player town 0
    let sel = selected_npc.0;
    let town_idx: usize = if sel >= 0 {
        entity_map
            .get_npc(sel as usize)
            .and_then(|npc| {
                inv.equipment_q
                    .get(npc.entity)
                    .ok()
                    .map(|(_, _, _, tid, _)| tid.0.max(0) as usize)
            })
            .unwrap_or(0)
    } else {
        0
    };
    ui.label(
        egui::RichText::new(format!("Town {} Armory", town_idx + 1))
            .strong()
            .size(15.0),
    );
    ui.small("Bulk equipment management for this town.");
    ui.separator();
    let selected_military_entity = if sel >= 0 {
        entity_map.get_npc(sel as usize).and_then(|npc| {
            inv.equipment_q.get(npc.entity).ok().and_then(|(_, _, job, tid, _)| {
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

    // --- Selected NPC equipment section ---
    let mut selected_equip: Option<&NpcEquipment> = None;
    if sel >= 0 {
        if let Some(npc) = entity_map.get_npc(sel as usize) {
            if let Ok((_, equip, job, _town_id, _)) = inv.equipment_q.get(npc.entity) {
                let def = npc_def(*job);
                let name = npc_stats_q.get(npc.entity)
                    .map(|s| s.name.as_str())
                    .unwrap_or("NPC");
                ui.label(
                    egui::RichText::new(format!("{} — {:?}", name, job))
                        .strong()
                        .size(14.0),
                );

                if def.equip_slots.is_empty() {
                    ui.label("(non-military — cannot equip)");
                } else {
                    selected_equip = Some(equip);
                    for &slot in ALL_EQUIP_KINDS {
                        if slot == ItemKind::Ring {
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
                                ui.label(format!("{}:", slot.label()));
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
        ui.label("Select a military NPC to preview equip/unequip.");
        ui.separator();
    }

    let town_items_pre = town_access.equipment(town_idx as i32).unwrap_or_default();
    let town_item_count = town_items_pre.len();
    ui.horizontal(|ui| {
        let can_town_auto = town_item_count > 0;
        let town_btn = ui.add_enabled(can_town_auto, egui::Button::new("Auto-equip Town Now"));
        if town_btn.clicked() {
            inv.auto_equip_writer
                .write(crate::systems::stats::AutoEquipNowMsg {
                    town_idx,
                    npc_entity: None,
                });
        }
        if let Some(ent) = selected_military_entity {
            let sel_btn = ui.add_enabled(can_town_auto, egui::Button::new("Auto-equip Selected"));
            if sel_btn.clicked() {
                inv.auto_equip_writer
                    .write(crate::systems::stats::AutoEquipNowMsg {
                        town_idx,
                        npc_entity: Some(ent),
                    });
            }
        }
    });
    ui.small("Uses the same upgrade rules as hourly auto-equip, but runs immediately.");
    ui.separator();

    // --- View mode toggle ---
    let view_mode = &mut ui_state.inv_view_mode;
    ui.horizontal(|ui| {
        if ui.selectable_label(*view_mode == 0, "Unequipped").clicked() { *view_mode = 0; }
        if ui.selectable_label(*view_mode == 1, "Equipped").clicked() { *view_mode = 1; }
        if ui.selectable_label(*view_mode == 2, "All").clicked() { *view_mode = 2; }
    });
    let view = *view_mode;

    // --- Collect equipped items across town NPCs (for Equipped/All views) ---
    struct EquippedEntry {
        owner: String,
        entity: Entity,
        item: crate::constants::LootItem,
        ring_index: u8,
    }
    let mut equipped_entries: Vec<EquippedEntry> = Vec::new();
    if view >= 1 {
        for (entity, equip, job, tid, _gpu_slot) in inv.equipment_q.iter() {
            if tid.0 != town_idx as i32 { continue; }
            let def = npc_def(*job);
            if def.equip_slots.is_empty() { continue; }
            let name = npc_stats_q.get(entity)
                .map(|s| s.name.as_str())
                .unwrap_or("NPC");
            let owner = format!("{} ({:?})", name, job);
            for item in equip.all_items() {
                let ring_index = if item.kind == ItemKind::Ring {
                    if equip.ring2.as_ref().map(|r| r.id) == Some(item.id) { 1 } else { 0 }
                } else { 0 };
                equipped_entries.push(EquippedEntry { owner: owner.clone(), entity, item, ring_index });
            }
        }
    }

    // --- Unequipped items ---
    let show_unequipped = view == 0 || view == 2;

    if show_unequipped {
        // Bulk sell Common
        let common_count = town_items_pre.iter().filter(|it| it.rarity == Rarity::Common).count();
        if common_count > 0 {
            let total_gold: i32 = common_count as i32 * (Rarity::Common.gold_cost() / 2);
            if ui
                .button(format!(
                    "Sell All Common ({} items, +{}g)",
                    common_count, total_gold
                ))
                .clicked()
            {
                let common_ids: Vec<u64> = town_items_pre
                    .iter()
                    .filter(|it| it.rarity == Rarity::Common)
                    .map(|it| it.id)
                    .collect();
                // Remove commons from equipment, then add gold
                let mut removed = 0i32;
                if let Some(mut eq) = town_access.equipment_mut(town_idx as i32) {
                    for id in common_ids {
                        if let Some(pos) = eq.0.iter().position(|it| it.id == id) {
                            eq.0.swap_remove(pos);
                            removed += 1;
                        }
                    }
                }
                if removed > 0 {
                    if let Some(mut g) = town_access.gold_mut(town_idx as i32) {
                        g.0 += removed * (Rarity::Common.gold_cost() / 2);
                    }
                }
            }
        }
    }

    // Get fresh items after potential sell
    let items = town_access.equipment(town_idx as i32).unwrap_or_default();

    // Header with counts
    let unequipped_count = items.len();
    let equipped_count = equipped_entries.len();
    match view {
        0 => ui.label(egui::RichText::new(format!("Unequipped ({} items)", unequipped_count)).strong().size(14.0)),
        1 => ui.label(egui::RichText::new(format!("Equipped ({} items)", equipped_count)).strong().size(14.0)),
        _ => ui.label(egui::RichText::new(format!("All ({} equipped, {} unequipped)", equipped_count, unequipped_count)).strong().size(14.0)),
    };

    // Slot filter buttons with counts (count from visible items)
    let filter = &mut ui_state.inv_slot_filter;
    ui.horizontal_wrapped(|ui| {
        for (i, &slot) in ALL_EQUIP_KINDS.iter().enumerate() {
            let bit = 1u16 << i;
            let enabled = *filter & bit != 0;
            let mut count = 0usize;
            if show_unequipped { count += items.iter().filter(|it| it.kind == slot).count(); }
            if view >= 1 { count += equipped_entries.iter().filter(|e| e.item.kind == slot).count(); }
            // Deduplicate for All mode — equipped already counted separately
            if view == 2 { count = items.iter().filter(|it| it.kind == slot).count() + equipped_entries.iter().filter(|e| e.item.kind == slot).count(); }
            let label = format!("{} ({})", slot.label(), count);
            if ui.selectable_label(enabled, label).clicked() {
                *filter ^= bit;
            }
        }
    });
    let filter_val = *filter;

    // Can equip if a valid military NPC from this town is selected
    let npc_entity = selected_military_entity;

    let slot_passes_filter = |kind: ItemKind| -> bool {
        let idx = ALL_EQUIP_KINDS.iter().position(|&s| s == kind).unwrap_or(0);
        filter_val & (1u16 << idx) != 0
    };

    // Check if there's anything to show
    let has_unequipped = show_unequipped && items.iter().any(|it| slot_passes_filter(it.kind));
    let has_equipped = view >= 1 && equipped_entries.iter().any(|e| slot_passes_filter(e.item.kind));
    if !has_unequipped && !has_equipped {
        ui.label("No items to show.");
        return;
    }

    egui::ScrollArea::vertical()
        .max_height(400.0)
        .show(ui, |ui| {
            // Show equipped items
            if view >= 1 {
                // Sort equipped entries
                equipped_entries.sort_by(|a, b| {
                    let sa = ALL_EQUIP_KINDS.iter().position(|&s| s == a.item.kind).unwrap_or(0);
                    let sb = ALL_EQUIP_KINDS.iter().position(|&s| s == b.item.kind).unwrap_or(0);
                    sa.cmp(&sb)
                        .then(rarity_ord(b.item.rarity).cmp(&rarity_ord(a.item.rarity)))
                        .then(b.item.stat_bonus.partial_cmp(&a.item.stat_bonus).unwrap_or(std::cmp::Ordering::Equal))
                });
                for entry in &equipped_entries {
                    if !slot_passes_filter(entry.item.kind) { continue; }
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&entry.item.name)
                                .color(rarity_color(entry.item.rarity)),
                        );
                        ui.label(format!(
                            "{} +{:.0}%",
                            entry.item.kind.label(),
                            entry.item.stat_bonus * 100.0,
                        ));
                        ui.label(
                            egui::RichText::new(&entry.owner)
                                .color(egui::Color32::from_rgb(150, 150, 150))
                                .small(),
                        );
                        if ui.small_button("Unequip").clicked() {
                            inv.unequip_writer.write(
                                crate::systems::stats::UnequipItemMsg {
                                    npc_entity: entry.entity,
                                    slot: entry.item.kind,
                                    ring_index: entry.ring_index,
                                },
                            );
                        }
                    });
                }
                if show_unequipped && has_unequipped && has_equipped {
                    ui.separator();
                }
            }

            // Show unequipped items
            if show_unequipped {
                let mut sorted: Vec<&crate::constants::LootItem> = items
                    .iter()
                    .filter(|it| slot_passes_filter(it.kind))
                    .collect();
                sorted.sort_by(|a, b| {
                    let slot_a = ALL_EQUIP_KINDS.iter().position(|&s| s == a.kind).unwrap_or(0);
                    let slot_b = ALL_EQUIP_KINDS.iter().position(|&s| s == b.kind).unwrap_or(0);
                    slot_a.cmp(&slot_b)
                        .then(rarity_ord(b.rarity).cmp(&rarity_ord(a.rarity)))
                        .then(b.stat_bonus.partial_cmp(&a.stat_bonus).unwrap_or(std::cmp::Ordering::Equal))
                });
                for item in &sorted {
                    let resp = ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(&item.name)
                                .color(rarity_color(item.rarity)),
                        );
                        ui.label(format!(
                            "{} +{:.0}%",
                            item.kind.label(),
                            item.stat_bonus * 100.0
                        ));
                        if view == 2 {
                            ui.label(
                                egui::RichText::new("[Unequipped]")
                                    .color(egui::Color32::from_rgb(120, 120, 120))
                                    .small(),
                            );
                        }
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
                    }).response;

                    // Comparison tooltip when NPC is selected
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
                            let arrow = if diff > 0.0 { "▲" } else if diff < 0.0 { "▼" } else { "=" };
                            resp.show_tooltip_text(format!(
                                "Current: +{:.0}% → New: +{:.0}% ({}{:.0}%)",
                                current * 100.0,
                                item.stat_bonus * 100.0,
                                arrow,
                                diff * 100.0,
                            ));
                        }
                    }
                }
            }
        });
}

/// Rarity sort order (higher = rarer).
fn rarity_ord(r: Rarity) -> u8 {
    match r {
        Rarity::Common => 0,
        Rarity::Uncommon => 1,
        Rarity::Rare => 2,
        Rarity::Epic => 3,
    }
}
