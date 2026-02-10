//! In-game HUD — population stats, time, food, selected NPC debug inspector.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::components::*;
use crate::gpu::NpcBufferWrites;
use crate::resources::*;
use crate::world::WorldData;

/// Query bundle for NPC state marker components.
#[derive(SystemParam)]
pub struct NpcStateQuery<'w, 's> {
    states: Query<'w, 's, (
        &'static NpcIndex,
        &'static Home,
        &'static Faction,
        &'static TownId,
        Option<&'static InCombat>,
        Option<&'static Working>,
        Option<&'static OnDuty>,
        Option<&'static Resting>,
        Option<&'static Raiding>,
        Option<&'static Returning>,
        Option<&'static Patrolling>,
        Option<&'static GoingToWork>,
        Option<&'static GoingToRest>,
        Option<&'static Wandering>,
        Option<&'static HasTarget>,
    ), Without<Dead>>,
    extra: Query<'w, 's, (
        &'static NpcIndex,
        Option<&'static AtDestination>,
        Option<&'static CarryingFood>,
        Option<&'static Starving>,
        Option<&'static Healing>,
        Option<&'static CombatOrigin>,
    ), Without<Dead>>,
}

pub fn game_hud_system(
    mut contexts: EguiContexts,
    game_time: Res<GameTime>,
    npc_count: Res<NpcCount>,
    kill_stats: Res<KillStats>,
    food_storage: Res<FoodStorage>,
    faction_stats: Res<FactionStats>,
    selected: Res<SelectedNpc>,
    meta_cache: Res<NpcMetaCache>,
    energy_cache: Res<NpcEnergyCache>,
    world_data: Res<WorldData>,
    health_query: Query<(&NpcIndex, &Health, &MaxHealth), Without<Dead>>,
    npc_states: NpcStateQuery,
    gpu_state: Res<GpuReadState>,
    buffer_writes: Res<NpcBufferWrites>,
    npc_logs: Res<NpcLogCache>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let mut copy_text: Option<String> = None;

    egui::SidePanel::left("game_hud").default_width(260.0).show(ctx, |ui| {
        ui.heading("Endless");
        ui.separator();

        // Time
        let period = if game_time.is_daytime() { "Day" } else { "Night" };
        ui.label(format!("Day {} - {:02}:{:02} ({})",
            game_time.day(), game_time.hour(), game_time.minute(), period));
        ui.label(format!("Speed: {:.0}x{}", game_time.time_scale,
            if game_time.paused { " [PAUSED]" } else { "" }));
        ui.small("Space=pause  +/-=speed");
        ui.separator();

        // Population
        ui.label(format!("NPCs alive: {}", npc_count.0));
        if let Some(villagers) = faction_stats.stats.first() {
            ui.label(format!("Villagers: {} alive, {} dead", villagers.alive, villagers.dead));
        }
        let raider_alive: i32 = faction_stats.stats.iter().skip(1).map(|s| s.alive).sum();
        let raider_dead: i32 = faction_stats.stats.iter().skip(1).map(|s| s.dead).sum();
        ui.label(format!("Raiders: {} alive, {} dead", raider_alive, raider_dead));
        ui.label(format!("Kills: guard {} | raider {}",
            kill_stats.guard_kills, kill_stats.villager_kills));
        ui.separator();

        // Food
        let num_villager_towns = world_data.towns.len() / 2;
        let town_food: i32 = food_storage.food.iter().take(num_villager_towns).sum();
        let camp_food: i32 = food_storage.food.iter().skip(num_villager_towns).sum();
        ui.label(format!("Food: town {} | camp {}", town_food, camp_food));
        ui.separator();

        // Selected NPC inspector
        let sel = selected.0;
        if sel >= 0 {
            let idx = sel as usize;
            if idx < meta_cache.0.len() {
                let meta = &meta_cache.0[idx];
                let energy = energy_cache.0.get(idx).copied().unwrap_or(0.0);

                ui.heading(format!("{}", meta.name));
                ui.label(format!("{} Lv.{}", crate::job_name(meta.job), meta.level));

                let trait_str = crate::trait_name(meta.trait_id);
                if !trait_str.is_empty() {
                    ui.label(format!("Trait: {}", trait_str));
                }

                // Find HP from query
                let mut hp = 0.0f32;
                let mut max_hp = 100.0f32;
                for (npc_idx, health, max_health) in health_query.iter() {
                    if npc_idx.0 == idx {
                        hp = health.0;
                        max_hp = max_health.0;
                        break;
                    }
                }

                // HP bar
                let hp_frac = if max_hp > 0.0 { (hp / max_hp).clamp(0.0, 1.0) } else { 0.0 };
                let hp_color = if hp_frac > 0.6 {
                    egui::Color32::from_rgb(80, 200, 80)
                } else if hp_frac > 0.3 {
                    egui::Color32::from_rgb(200, 200, 40)
                } else {
                    egui::Color32::from_rgb(200, 60, 60)
                };
                ui.horizontal(|ui| {
                    ui.label("HP:");
                    ui.add(egui::ProgressBar::new(hp_frac)
                        .text(format!("{:.0}/{:.0}", hp, max_hp))
                        .fill(hp_color));
                });

                // Energy bar
                let energy_frac = (energy / 100.0).clamp(0.0, 1.0);
                ui.horizontal(|ui| {
                    ui.label("EN:");
                    ui.add(egui::ProgressBar::new(energy_frac)
                        .text(format!("{:.0}", energy))
                        .fill(egui::Color32::from_rgb(60, 120, 200)));
                });

                // Town name
                if meta.town_id >= 0 {
                    if let Some(town) = world_data.towns.get(meta.town_id as usize) {
                        ui.label(format!("Town: {}", town.name));
                    }
                }

                ui.separator();

                // Debug: position, target, home, faction, state components
                let positions = &gpu_state.positions;
                let targets = &buffer_writes.targets;

                let pos = if idx * 2 + 1 < positions.len() {
                    format!("({:.0}, {:.0})", positions[idx * 2], positions[idx * 2 + 1])
                } else {
                    "?".into()
                };
                let target = if idx * 2 + 1 < targets.len() {
                    format!("({:.0}, {:.0})", targets[idx * 2], targets[idx * 2 + 1])
                } else {
                    "?".into()
                };

                // Collect state flags from ECS queries
                let mut flags: Vec<&str> = Vec::new();
                let mut home_str = String::new();
                let mut faction_str = String::new();

                if let Some((_, home, faction, town_id, in_combat, working, on_duty,
                    resting, raiding, returning, patrolling, going_work, going_rest,
                    wandering, has_target))
                    = npc_states.states.iter().find(|(ni, ..)| ni.0 == idx)
                {
                    home_str = format!("({:.0}, {:.0})", home.0.x, home.0.y);
                    faction_str = format!("{} (town {})", faction.0, town_id.0);

                    if in_combat.is_some() { flags.push("InCombat"); }
                    if working.is_some() { flags.push("Working"); }
                    if on_duty.is_some() { flags.push("OnDuty"); }
                    if resting.is_some() { flags.push("Resting"); }
                    if raiding.is_some() { flags.push("Raiding"); }
                    if returning.is_some() { flags.push("Returning"); }
                    if patrolling.is_some() { flags.push("Patrolling"); }
                    if going_work.is_some() { flags.push("GoingToWork"); }
                    if going_rest.is_some() { flags.push("GoingToRest"); }
                    if wandering.is_some() { flags.push("Wandering"); }
                    if has_target.is_some() { flags.push("HasTarget"); }
                }

                if let Some((_, at_dest, carrying, starving, healing, combat_origin))
                    = npc_states.extra.iter().find(|(ni, ..)| ni.0 == idx)
                {
                    if at_dest.is_some() { flags.push("AtDest"); }
                    if carrying.is_some() { flags.push("CarryingFood"); }
                    if starving.is_some() { flags.push("Starving"); }
                    if healing.is_some() { flags.push("Healing"); }
                    if combat_origin.is_some() { flags.push("CombatOrigin"); }
                }

                let state_str = if flags.is_empty() { "Idle".into() } else { flags.join(", ") };

                ui.label(format!("Pos: {}", pos));
                ui.label(format!("Target: {}", target));
                ui.label(format!("Home: {}", home_str));
                ui.label(format!("Faction: {}", faction_str));
                ui.label(format!("State: {}", state_str));

                // Recent log entries
                ui.separator();
                ui.label("Log:");
                if idx < npc_logs.0.len() {
                    let log = &npc_logs.0[idx];
                    let start = if log.len() > 8 { log.len() - 8 } else { 0 };
                    for entry in log.iter().skip(start) {
                        ui.small(format!("D{}:{:02}:{:02} {}",
                            entry.day, entry.hour, entry.minute, entry.message));
                    }
                }

                // Copy debug info button
                ui.separator();
                if ui.button("Copy Debug Info").clicked() {
                    let mut info = format!(
                        "NPC #{idx} \"{name}\" {job} Lv.{level}\n\
                         HP: {hp:.0}/{max_hp:.0}  EN: {energy:.0}\n\
                         Pos: {pos}  Target: {target}\n\
                         Home: {home}  Faction: {faction}\n\
                         State: {state}\n\
                         Day {day} {hour:02}:{min:02}\n\
                         ---\n",
                        idx = idx,
                        name = meta.name,
                        job = crate::job_name(meta.job),
                        level = meta.level,
                        hp = hp,
                        max_hp = max_hp,
                        energy = energy,
                        pos = pos,
                        target = target,
                        home = home_str,
                        faction = faction_str,
                        state = state_str,
                        day = game_time.day(),
                        hour = game_time.hour(),
                        min = game_time.minute(),
                    );
                    // Append recent log
                    if idx < npc_logs.0.len() {
                        for entry in npc_logs.0[idx].iter() {
                            info.push_str(&format!("D{}:{:02}:{:02} {}\n",
                                entry.day, entry.hour, entry.minute, entry.message));
                        }
                    }
                    copy_text = Some(info);
                }
            }
        } else {
            ui.label("Click an NPC to inspect");
        }

        ui.separator();
        ui.small("ESC = back to menu");
    });

    if let Some(text) = copy_text {
        info!("Copy button clicked, {} bytes", text.len());
        match arboard::Clipboard::new() {
            Ok(mut cb) => {
                match cb.set_text(text) {
                    Ok(_) => info!("Clipboard: text copied successfully"),
                    Err(e) => error!("Clipboard: set_text failed: {e}"),
                }
            }
            Err(e) => error!("Clipboard: failed to open: {e}"),
        }
    }

    Ok(())
}
