//! In-game HUD — top resource bar, bottom panel (inspector + combat log), target overlay.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::render::MainCamera;
use crate::resources::*;
use crate::ui::tipped;
use crate::world::{BuildingKind, WorldData};

// ============================================================================
// DIRECT-CONTROL HELPERS
// ============================================================================

/// Collect alive direct-control NPC slots from the selected player squad.
/// Only includes NPCs with `NpcFlags.direct_control == true` (set by box-select).
/// O(squad_size) instead of O(all_npcs) — avoids scanning entire EntityMap.
fn dc_slots(
    squad_state: &SquadState,
    entity_map: &EntityMap,
    is_dc: impl Fn(Entity) -> bool,
) -> Vec<usize> {
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
        .filter_map(|e| entity_map.slot_for_entity(*e))
        .filter(|&slot| entity_map.get_npc(slot).is_some_and(|n| !n.dead))
        .filter(|&slot| entity_map.get_npc(slot).is_some_and(|n| is_dc(n.entity)))
        .collect()
}

pub mod build_ghost;
pub mod building_inspector;
pub mod combat_log;
pub mod inspector;
pub mod npc_inspector;
pub mod squad_overlay;

pub use combat_log::combat_log_system;
pub use inspector::{
    BottomPanelUiState, BuildingInspectorData, InspectorNpcTab, InspectorRenameState,
    InspectorTabState, InspectorUiState, bottom_panel_system,
};
pub use squad_overlay::{
    faction_squad_overlay_system, squad_overlay_system, target_overlay_system,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResourceIconAtlas {
    World,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HudResourceIcon {
    Food,
    Gold,
    Wood,
    Stone,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResourceIconSpec {
    atlas: ResourceIconAtlas,
    col: u32,
    row: u32,
}

fn resource_icon_spec(icon: HudResourceIcon) -> ResourceIconSpec {
    match icon {
        HudResourceIcon::Food => ResourceIconSpec {
            atlas: ResourceIconAtlas::World,
            col: 24,
            row: 9,
        },
        HudResourceIcon::Gold => ResourceIconSpec {
            atlas: ResourceIconAtlas::World,
            col: 41,
            row: 11,
        },
        HudResourceIcon::Wood => ResourceIconSpec {
            atlas: ResourceIconAtlas::World,
            col: 13,
            row: 9,
        },
        HudResourceIcon::Stone => ResourceIconSpec {
            atlas: ResourceIconAtlas::World,
            col: 7,
            row: 15,
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResourceIconPart {
    Icon,
    Value,
}

fn resource_icon_parts(prefer_right_to_left: bool) -> [ResourceIconPart; 2] {
    if prefer_right_to_left {
        [ResourceIconPart::Value, ResourceIconPart::Icon]
    } else {
        [ResourceIconPart::Icon, ResourceIconPart::Value]
    }
}

fn top_bar_resource_display_order() -> [HudResourceIcon; 4] {
    [
        HudResourceIcon::Wood,
        HudResourceIcon::Stone,
        HudResourceIcon::Food,
        HudResourceIcon::Gold,
    ]
}

fn top_bar_resource_widget_order(prefer_right_to_left: bool) -> [HudResourceIcon; 4] {
    let [a, b, c, d] = top_bar_resource_display_order();
    if prefer_right_to_left {
        [d, c, b, a]
    } else {
        [a, b, c, d]
    }
}

/// Cached egui texture IDs for resource icons extracted from atlas sprites.
#[derive(Resource, Default)]
pub struct ResourceIconCache {
    pub initialized: bool,
    pub food: Option<egui::TextureHandle>,
    pub gold: Option<egui::TextureHandle>,
    pub wood: Option<egui::TextureHandle>,
    pub stone: Option<egui::TextureHandle>,
}

/// Extract a single 16x16 sprite from a 17px-cell atlas at (col, row).
fn extract_atlas_cell(
    atlas: &Image,
    col: u32,
    row: u32,
    cell_size: u32,
) -> Option<egui::ColorImage> {
    let sprite_size = cell_size - 1;
    let src_w = atlas.width();
    let src_data = atlas.data.as_ref()?;
    let px = col * cell_size;
    let py = row * cell_size;
    let mut data = vec![0u8; (sprite_size * sprite_size * 4) as usize];
    for y in 0..sprite_size {
        for x in 0..sprite_size {
            let sx = (px + x) as usize;
            let sy = (py + y) as usize;
            let src_idx = (sy * src_w as usize + sx) * 4;
            let dst_idx = (y * sprite_size + x) as usize * 4;
            if src_idx + 3 < src_data.len() {
                data[dst_idx..dst_idx + 4].copy_from_slice(&src_data[src_idx..src_idx + 4]);
            }
        }
    }
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [sprite_size as usize, sprite_size as usize],
        &data,
    ))
}

/// Extract world-atlas cells into standalone icon textures so alpha stays intact in egui.
pub fn init_resource_icons(
    mut cache: ResMut<ResourceIconCache>,
    mut contexts: EguiContexts,
    sprites: Res<crate::render::SpriteAssets>,
    images: Res<Assets<Image>>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let cell = 17u32;
    let mut pending: Vec<(HudResourceIcon, egui::ColorImage)> = Vec::new();
    if let Some(atlas) = images.get(&sprites.world_texture) {
        for icon in [
            HudResourceIcon::Food,
            HudResourceIcon::Gold,
            HudResourceIcon::Wood,
            HudResourceIcon::Stone,
        ] {
            let missing = match icon {
                HudResourceIcon::Food => cache.food.is_none(),
                HudResourceIcon::Gold => cache.gold.is_none(),
                HudResourceIcon::Wood => cache.wood.is_none(),
                HudResourceIcon::Stone => cache.stone.is_none(),
            };
            if !missing {
                continue;
            }

            let spec = resource_icon_spec(icon);
            if let Some(img) = extract_atlas_cell(atlas, spec.col, spec.row, cell) {
                pending.push((icon, img));
            }
        }
    }
    for (icon, img) in pending {
        let tex = ctx.load_texture(
            format!("resource-icon-{icon:?}"),
            img,
            egui::TextureOptions::NEAREST,
        );
        match icon {
            HudResourceIcon::Food => cache.food = Some(tex),
            HudResourceIcon::Gold => cache.gold = Some(tex),
            HudResourceIcon::Wood => cache.wood = Some(tex),
            HudResourceIcon::Stone => cache.stone = Some(tex),
        }
    }
    cache.initialized = cache.food.is_some()
        && cache.gold.is_some()
        && cache.wood.is_some()
        && cache.stone.is_some();
}

/// Render a resource sprite icon + amount with tooltip.
/// Uses atlas texture + UV rect for the icon, falls back to colored square.
fn resource_icon(
    ui: &mut egui::Ui,
    amount: i32,
    tex: Option<&egui::TextureHandle>,
    color: egui::Color32,
    tip: &str,
) {
    let icon_size = 16.0;
    let spacing = 2.0;
    let parts = resource_icon_parts(ui.layout().prefer_right_to_left());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spacing;
        for part in parts {
            match part {
                ResourceIconPart::Icon => {
                    if let Some(tex) = tex {
                        ui.add(
                            egui::Image::new(egui::load::SizedTexture::new(
                                tex.id(),
                                [icon_size, icon_size],
                            ))
                            .tint(egui::Color32::WHITE),
                        );
                    } else {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(icon_size, icon_size),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(rect, 2.0, color);
                    }
                }
                ResourceIconPart::Value => {
                    ui.label(egui::RichText::new(amount.to_string()).color(color));
                }
            }
        }
    })
    .response
    .on_hover_text(tip);
}

// ============================================================================
// TOP RESOURCE BAR
// ============================================================================

/// Full-width opaque top bar (WC3 style): buttons left, town name center, stats right.
pub fn top_bar_system(
    mut contexts: EguiContexts,
    game_time: Res<GameTime>,
    pop_stats: Res<PopulationStats>,
    town_access: crate::systemparams::TownAccess,
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
    icon_cache: Res<ResourceIconCache>,
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
                        ui_state.left_panel_open && ui_state.left_panel_tab == LeftPanelTab::Stats,
                        "Stats",
                    )
                    .clicked()
                {
                    ui_state.toggle_left_tab(LeftPanelTab::Stats);
                }
                if ui
                    .selectable_label(ui_state.tech_tree_open, "Tech Tree")
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
                    .selectable_label(ui_state.armory_open, "Armory")
                    .clicked()
                {
                    ui_state.toggle_armory();
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
                            LlmStatus::Idle => (egui::Color32::from_rgb(80, 80, 80), "LLM idle"),
                            LlmStatus::Sending => {
                                (egui::Color32::from_rgb(80, 180, 255), "LLM sending state")
                            }
                            LlmStatus::Thinking => {
                                (egui::Color32::from_rgb(255, 200, 50), "LLM thinking...")
                            }
                            LlmStatus::Done(n) => (
                                egui::Color32::from_rgb(80, 220, 120),
                                if n > 0 {
                                    "LLM executed actions"
                                } else {
                                    "LLM no actions"
                                },
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

                    // Player stats (right-aligned) — look up player town by faction
                    let player_town_idx = world_data
                        .towns
                        .iter()
                        .position(|t| t.faction == crate::constants::FACTION_PLAYER)
                        .unwrap_or(0) as i32;
                    let town_food = town_access.food(player_town_idx);
                    let town_gold = town_access.gold(player_town_idx);
                    let town_wood = town_access.wood(player_town_idx);
                    let town_stone = town_access.stone(player_town_idx);
                    for icon in top_bar_resource_widget_order(ui.layout().prefer_right_to_left()) {
                        match icon {
                            HudResourceIcon::Wood => resource_icon(
                                ui,
                                town_wood,
                                icon_cache.wood.as_ref(),
                                egui::Color32::from_rgb(150, 110, 70),
                                catalog.0.get("wood").unwrap_or(&""),
                            ),
                            HudResourceIcon::Stone => resource_icon(
                                ui,
                                town_stone,
                                icon_cache.stone.as_ref(),
                                egui::Color32::from_rgb(170, 170, 180),
                                catalog.0.get("stone").unwrap_or(&""),
                            ),
                            HudResourceIcon::Food => resource_icon(
                                ui,
                                town_food,
                                icon_cache.food.as_ref(),
                                egui::Color32::from_rgb(120, 200, 80),
                                catalog.0.get("food").unwrap_or(&""),
                            ),
                            HudResourceIcon::Gold => resource_icon(
                                ui,
                                town_gold,
                                icon_cache.gold.as_ref(),
                                egui::Color32::from_rgb(220, 190, 50),
                                catalog.0.get("gold").unwrap_or(&""),
                            ),
                        }
                    }

                    let ptidx_u = player_town_idx as u32;
                    let farmers = pop_stats
                        .0
                        .get(&(crate::components::Job::Farmer as i32, player_town_idx))
                        .map(|s| s.alive)
                        .unwrap_or(0);
                    let guards = pop_stats
                        .0
                        .get(&(crate::components::Job::Archer as i32, player_town_idx))
                        .map(|s| s.alive)
                        .unwrap_or(0);
                    let crossbows = pop_stats
                        .0
                        .get(&(crate::components::Job::Crossbow as i32, player_town_idx))
                        .map(|s| s.alive)
                        .unwrap_or(0);
                    let houses = entity_map.count_for_town(BuildingKind::FarmerHome, ptidx_u);
                    let barracks = entity_map.count_for_town(BuildingKind::ArcherHome, ptidx_u);
                    let xbow_homes = entity_map.count_for_town(BuildingKind::CrossbowHome, ptidx_u);
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

/// Bundled readonly resources for bottom panel.
#[derive(SystemParam)]
pub struct BottomPanelData<'w> {
    game_time: Res<'w, GameTime>,
    npc_logs: Res<'w, NpcLogCache>,
    selected: ResMut<'w, SelectedNpc>,
    combat_log: ResMut<'w, CombatLog>,
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
                            if paused {
                                sink.play()
                            } else {
                                sink.pause()
                            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_icon_specs_use_world_atlas_cells() {
        assert_eq!(
            resource_icon_spec(HudResourceIcon::Food),
            ResourceIconSpec {
                atlas: ResourceIconAtlas::World,
                col: 24,
                row: 9,
            }
        );
        assert_eq!(
            resource_icon_spec(HudResourceIcon::Gold),
            ResourceIconSpec {
                atlas: ResourceIconAtlas::World,
                col: 41,
                row: 11,
            }
        );
        assert_eq!(
            resource_icon_spec(HudResourceIcon::Wood),
            ResourceIconSpec {
                atlas: ResourceIconAtlas::World,
                col: 13,
                row: 9,
            }
        );
        assert_eq!(
            resource_icon_spec(HudResourceIcon::Stone),
            ResourceIconSpec {
                atlas: ResourceIconAtlas::World,
                col: 7,
                row: 15,
            }
        );
    }

    #[test]
    fn resource_icon_parts_flip_for_rtl_parents() {
        assert_eq!(
            resource_icon_parts(false),
            [ResourceIconPart::Icon, ResourceIconPart::Value]
        );
        assert_eq!(
            resource_icon_parts(true),
            [ResourceIconPart::Value, ResourceIconPart::Icon]
        );
    }

    #[test]
    fn top_bar_resource_order_stays_wood_stone_food_gold_on_screen() {
        assert_eq!(
            top_bar_resource_display_order(),
            [
                HudResourceIcon::Wood,
                HudResourceIcon::Stone,
                HudResourceIcon::Food,
                HudResourceIcon::Gold,
            ]
        );
        assert_eq!(
            top_bar_resource_widget_order(false),
            [
                HudResourceIcon::Wood,
                HudResourceIcon::Stone,
                HudResourceIcon::Food,
                HudResourceIcon::Gold,
            ]
        );
        assert_eq!(
            top_bar_resource_widget_order(true),
            [
                HudResourceIcon::Gold,
                HudResourceIcon::Food,
                HudResourceIcon::Stone,
                HudResourceIcon::Wood,
            ]
        );
    }

    #[test]
    fn extract_atlas_cell_preserves_alpha() {
        let cell_size = 17u32;
        let mut data = vec![0u8; (cell_size * cell_size * 4) as usize];
        data[0..4].copy_from_slice(&[10, 20, 30, 0]);
        data[4..8].copy_from_slice(&[40, 50, 60, 128]);
        let atlas = Image::new(
            bevy::render::render_resource::Extent3d {
                width: cell_size,
                height: cell_size,
                depth_or_array_layers: 1,
            },
            bevy::render::render_resource::TextureDimension::D2,
            data,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            Default::default(),
        );

        let icon = extract_atlas_cell(&atlas, 0, 0, cell_size).expect("cell should extract");
        assert_eq!(icon.size, [16, 16]);
        assert_eq!(icon.pixels[0].to_srgba_unmultiplied()[3], 0);
        assert_eq!(icon.pixels[1].to_srgba_unmultiplied(), [40, 50, 60, 128]);
    }
}
