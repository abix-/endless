//! In-game HUD — population stats, time, food, selected NPC inspector.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::components::*;
use crate::resources::*;
use crate::world::WorldData;

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
) -> Result {
    let ctx = contexts.ctx_mut()?;

    egui::SidePanel::left("game_hud").default_width(220.0).show(ctx, |ui| {
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
        let idx = selected.0;
        if idx >= 0 {
            let idx = idx as usize;
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
            }
        } else {
            ui.label("Click an NPC to inspect");
        }

        ui.separator();
        ui.small("ESC = back to menu");
    });

    Ok(())
}
