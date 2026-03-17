//! NPC inspector panel -- DC group summary and NPC detail view.

use super::building_inspector::{building_inspector_content, mine_assignment_ui};
use super::inspector::{
    BuildingInspectorData, InspectorAction, InspectorNpcTab, InspectorRenameState, building_link,
};
use super::{BottomPanelData, dc_slots};
use crate::components::*;
use crate::constants::npc_def;
use crate::resources::*;
use crate::settings::UserSettings;
use crate::systems::stats::UnequipItemMsg;
use crate::ui::tipped;
use crate::world::{BuildingKind, WorldData};
use bevy::prelude::*;
use bevy_egui::egui;

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
pub(crate) fn inspector_content(
    ui: &mut egui::Ui,
    data: &mut BottomPanelData,
    npc_stats_q: &mut Query<&mut NpcStats>,
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
                npc_stats_q,
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
                npc_stats_q,
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
                npc_stats_q,
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

    let npc_entity = bld_data.entity_map.get_npc(idx).map(|n| n.entity);
    if rename_state.slot != sel {
        rename_state.slot = sel;
        rename_state.text = npc_entity
            .and_then(|e| npc_stats_q.get(e).ok())
            .map(|s| s.name.clone())
            .unwrap_or_default();
    }

    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(npc_tab, InspectorNpcTab::Overview, "Overview");
        ui.selectable_value(npc_tab, InspectorNpcTab::Loadout, "Loadout");
        ui.selectable_value(npc_tab, InspectorNpcTab::Skills, "Skills");
        ui.selectable_value(npc_tab, InspectorNpcTab::Economy, "Economy");
        ui.selectable_value(npc_tab, InspectorNpcTab::Log, "Log");
    });
    ui.separator();
    let show_overview = *npc_tab == InspectorNpcTab::Overview;
    let show_loadout = *npc_tab == InspectorNpcTab::Loadout;
    let show_skills = *npc_tab == InspectorNpcTab::Skills;
    let show_economy = *npc_tab == InspectorNpcTab::Economy;
    let show_log = *npc_tab == InspectorNpcTab::Log;

    if show_overview {
        ui.horizontal(|ui| {
            ui.label("Name:");
            let edit = ui.text_edit_singleline(&mut rename_state.text);
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Rename").clicked() || enter) && !rename_state.text.trim().is_empty() {
                let new_name = rename_state.text.trim().to_string();
                if let Some(e) = npc_entity {
                    if let Ok(mut s) = npc_stats_q.get_mut(e) {
                        s.name = new_name.clone();
                    }
                }
                rename_state.text = new_name;
            }
        });
    }

    let npc_stats = npc_entity.and_then(|e| npc_stats_q.get(e).ok());
    let npc_xp = npc_stats.map(|s| s.xp).unwrap_or(0);
    let npc_level = crate::systems::stats::level_from_xp(npc_xp);
    let npc_job = bld_data.entity_map.get_npc(idx).map(|n| n.job);

    if show_overview {
        tipped(
            ui,
            format!(
                "{} Lv.{}  XP: {}/{}",
                npc_job.map(|j| j.label()).unwrap_or("?"),
                npc_level,
                npc_xp,
                (npc_level + 1) * (npc_level + 1) * 100
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

    // Skills tab -- dedicated proficiency display with bars and effect descriptions
    if show_skills {
        if let Some(npc) = bld_data.entity_map.get_npc(idx) {
            let skills = bld_data
                .skills_q
                .get(npc.entity)
                .cloned()
                .unwrap_or_default();
            let max = crate::constants::MAX_PROFICIENCY;
            let bar_w = 180.0f32;
            let bar_h = 12.0f32;

            // Build all possible entries
            let all_entries: Vec<(&str, f32, String)> = vec![
                (
                    "farming",
                    skills.farming,
                    format!(
                        "+{:.2}/hr tending. {:.2}x growth rate.",
                        crate::constants::FARMING_SKILL_RATE,
                        crate::systems::stats::proficiency_mult(skills.farming),
                    ),
                ),
                (
                    "combat",
                    skills.combat,
                    format!(
                        "+{:.1}/kill. {:.2}x damage, {:.2}x cooldown.",
                        crate::constants::COMBAT_SKILL_RATE,
                        crate::systems::stats::proficiency_mult(skills.combat),
                        1.0 / crate::systems::stats::proficiency_mult(skills.combat),
                    ),
                ),
                ("dodge", skills.dodge, {
                    let dodge_mult = crate::systems::stats::proficiency_mult(skills.dodge);
                    let dodge_pct = (1.0 - 1.0 / dodge_mult) * 100.0;
                    format!(
                        "+{:.1}/dodge. {:.1}% miss chance.",
                        crate::constants::DODGE_SKILL_RATE,
                        dodge_pct,
                    )
                }),
            ];

            // Filter to job-relevant skills only
            let relevant: &[&str] = match npc_job.unwrap_or(Job::Farmer) {
                Job::Farmer => &["farming"],
                Job::Archer | Job::Crossbow | Job::Fighter | Job::Raider => &["combat", "dodge"],
                _ => &[],
            };
            let skill_entries: Vec<_> = all_entries
                .into_iter()
                .filter(|(name, _, _)| relevant.contains(name))
                .collect();

            if skill_entries.is_empty() {
                ui.label(
                    egui::RichText::new("No skills for this job yet.")
                        .color(egui::Color32::from_rgb(120, 120, 120)),
                );
            }

            for (name, value, desc) in &skill_entries {
                let value = *value;
                let color = crate::ui::skill_prof_color(value);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{:<8}", name))
                            .color(egui::Color32::WHITE)
                            .strong(),
                    );
                    // Progress bar
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(bar_w, bar_h), egui::Sense::hover());
                    let painter = ui.painter();
                    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(40, 40, 45));
                    let fill_w = (value / max).clamp(0.0, 1.0) * bar_w;
                    if fill_w > 0.5 {
                        let fill_rect =
                            egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, bar_h));
                        painter.rect_filled(fill_rect, 2.0, color);
                    }
                    ui.label(
                        egui::RichText::new(format!("{} / {}", value as i32, max as i32))
                            .color(color),
                    );
                });
                ui.label(
                    egui::RichText::new(format!("  {}", desc))
                        .color(egui::Color32::from_rgb(160, 160, 160))
                        .small(),
                );
                ui.add_space(4.0);
            }
        }
    }

    // Equipment + status from EntityMap + ECS
    if show_loadout {
        ui.separator();
        ui.label(egui::RichText::new("Loadout").strong());
    }
    let mut squad_id: Option<i32> = None;
    let job = npc_job.unwrap_or(Job::Farmer);
    let can_equip = !npc_def(job).equip_slots.is_empty();
    if let Some(npc) = bld_data.entity_map.get_npc(idx) {
        if show_loadout {
            if let Ok(eq) = bld_data.equipment_q.get(npc.entity) {
                use crate::constants::ItemKind;
                let slots: &[(&str, &Option<crate::constants::LootItem>, ItemKind, u8)] = &[
                    ("Weapon", &eq.weapon, ItemKind::Weapon, 0),
                    ("Helm", &eq.helm, ItemKind::Helm, 0),
                    ("Armor", &eq.armor, ItemKind::Armor, 0),
                    ("Shield", &eq.shield, ItemKind::Shield, 0),
                    ("Gloves", &eq.gloves, ItemKind::Gloves, 0),
                    ("Boots", &eq.boots, ItemKind::Boots, 0),
                    ("Belt", &eq.belt, ItemKind::Belt, 0),
                    ("Amulet", &eq.amulet, ItemKind::Amulet, 0),
                    ("Ring 1", &eq.ring1, ItemKind::Ring, 0),
                    ("Ring 2", &eq.ring2, ItemKind::Ring, 1),
                ];
                let mut any = false;
                for &(label, item_opt, slot, ring_index) in slots {
                    if let Some(item) = item_opt {
                        any = true;
                        ui.horizontal(|ui| {
                            let (r, g, b) = item.rarity.color();
                            ui.label(format!("{}:", label));
                            ui.label(
                                egui::RichText::new(&item.name)
                                    .color(egui::Color32::from_rgb(r, g, b)),
                            );
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
                    ui_state.open_armory();
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
    let npc_town_idx = bld_data
        .entity_map
        .get_npc(idx)
        .map(|n| n.town_idx)
        .unwrap_or(-1);
    if show_overview && npc_town_idx >= 0 {
        if let Some(town) = world_data.towns.get(npc_town_idx as usize) {
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
            home_slot = bld_data
                .entity_map
                .find_by_position(npc_home)
                .map(|i| i.slot);
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
        is_mining_at_mine = npc_act.is_some_and(|a| a.kind == ActivityKind::Mine);
        activity_debug = npc_act.map(|a| format!("{:?}", a)).unwrap_or_default();

        if let Ok(cl) = bld_data.carried_loot_q.get(npc.entity) {
            carried_food = cl.food;
            carried_gold = cl.gold;
            carried_equip_count = cl.equipment.len();
            carried_equip_preview = cl
                .equipment
                .iter()
                .take(4)
                .map(|it| format!("{} ({:?} +{:.0}%)", it.name, it.kind, it.stat_bonus * 100.0))
                .collect();
            carried_equip_more = cl
                .equipment
                .len()
                .saturating_sub(carried_equip_preview.len());
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
        let home_action = ui
            .horizontal(|ui| {
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
            })
            .inner;
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
    if show_overview && npc_job == Some(Job::Miner) {
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
                    let mine_pos = bld_data
                        .entity_map
                        .entities
                        .get(&mh_slot)
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
        let entity = bld_data.entity_map.entities.get(&idx);
        let entity_str = entity.map_or("?".to_string(), |e| format!("{:?}", e));
        let world_pos_str = if idx * 2 + 1 < gpu_state.positions.len() {
            format!(
                "({:.0}, {:.0})",
                gpu_state.positions[idx * 2],
                gpu_state.positions[idx * 2 + 1]
            )
        } else {
            "?".into()
        };
        ui.label(format!(
            "Slot: {}  Entity: {}  Pos: {}",
            idx, entity_str, world_pos_str
        ));

        if ui.button("Copy Debug Info").clicked() {
            let positions = &gpu_state.positions;
            let pos = if idx * 2 + 1 < positions.len() {
                format!("({:.0}, {:.0})", positions[idx * 2], positions[idx * 2 + 1])
            } else {
                "?".into()
            };
            let npc_name = npc_stats.map(|s| s.name.as_str()).unwrap_or("?");
            let xp_next = (npc_level + 1) * (npc_level + 1) * 100;
            let mut info = format!(
                "NPC #{idx} \"{name}\" {job} Lv.{level}  XP: {xp}/{xp_next}\n\
                 Slot: {idx}  Entity: {entity}\n\
                 HP: {hp:.0}/{max_hp:.0}  EN: {energy:.0}\n\
                 Pos: {pos}\n\
                 Home: {home}  Faction: {faction}\n\
                 State: {state}\n\
                 Activity: {activity}\n",
                idx = idx,
                name = npc_name,
                job = npc_job.map(|j| j.label()).unwrap_or("?"),
                level = npc_level,
                xp = npc_xp,
                xp_next = xp_next,
                entity = entity_str,
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
                        ("Weapon", &eq.weapon),
                        ("Helm", &eq.helm),
                        ("Armor", &eq.armor),
                        ("Shield", &eq.shield),
                        ("Gloves", &eq.gloves),
                        ("Boots", &eq.boots),
                        ("Belt", &eq.belt),
                        ("Amulet", &eq.amulet),
                        ("Ring 1", &eq.ring1),
                        ("Ring 2", &eq.ring2),
                    ];
                    for &(label, item_opt) in slots {
                        if let Some(item) = item_opt {
                            info.push_str(&format!(
                                "{}: {} ({} +{:.0}%)\n",
                                label,
                                item.name,
                                item.rarity.label(),
                                item.stat_bonus * 100.0
                            ));
                        }
                    }
                }
                if let Ok(flags) = bld_data.npc_flags_q.get(npc.entity) {
                    let mut fp: Vec<&str> = Vec::new();
                    if flags.healing {
                        fp.push("healing");
                    }
                    if flags.starving {
                        fp.push("starving");
                    }
                    if flags.direct_control {
                        fp.push("direct_control");
                    }
                    if flags.migrating {
                        fp.push("migrating");
                    }
                    if flags.at_destination {
                        fp.push("at_dest");
                    }
                    info.push_str(&format!("Flags: [{}]\n", fp.join(", ")));
                }
                let combat_state_name = bld_data
                    .combat_state_q
                    .get(npc.entity)
                    .map(|cs| cs.name())
                    .unwrap_or("Unknown");
                info.push_str(&format!("CombatState: {}\n", combat_state_name));
                info.push_str(&format!(
                    "CarriedLoot: food={} gold={}\n",
                    carried_food, carried_gold
                ));
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
