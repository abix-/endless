//! Build bar - bottom-center horizontal bar for building placement + destroy mode.

use std::collections::HashMap;
use bevy::prelude::*;
use bevy::image::Image;
use bevy_egui::{EguiContexts, EguiTextureHandle, egui};

use crate::render::SpriteAssets;
use crate::resources::*;
use crate::settings::UserSettings;
use crate::world::{self, BuildingKind, SPRITE_SIZE, CELL};

struct BuildOption {
    kind: BuildingKind,
    label: &'static str,
    help: &'static str,
}

const PLAYER_BUILD_OPTIONS: &[BuildOption] = &[
    BuildOption { kind: BuildingKind::Farm, label: "Farm", help: "Grows food over time" },
    BuildOption { kind: BuildingKind::FarmerHome, label: "Farmer Home", help: "Spawns 1 farmer" },
    BuildOption { kind: BuildingKind::MinerHome, label: "Miner Home", help: "Spawns 1 miner" },
    BuildOption { kind: BuildingKind::ArcherHome, label: "Archer Home", help: "Spawns 1 archer" },
    BuildOption { kind: BuildingKind::CrossbowHome, label: "Crossbow Home", help: "Spawns 1 crossbow" },
    BuildOption { kind: BuildingKind::Waypoint, label: "Waypoint", help: "Patrol waypoint" },
];

const CAMP_BUILD_OPTIONS: &[BuildOption] = &[
    BuildOption { kind: BuildingKind::Tent, label: "Tent", help: "Spawns 1 raider" },
];


/// Extract a single 32x32 image from the world atlas for a Quad tile spec.
fn extract_quad_tile(atlas: &Image, quad: [(u32, u32); 4]) -> Image {
    let sprite = SPRITE_SIZE as u32; // 16
    let out_size = sprite * 2;       // 32
    let cell_size = CELL as u32;     // 17
    let atlas_width = atlas.width();
    let atlas_data = atlas.data.as_ref().expect("atlas image has no data");

    let mut data = vec![0u8; (out_size * out_size * 4) as usize];

    let blit = |data: &mut [u8], col: u32, row: u32, dx: u32, dy: u32| {
        let src_x = col * cell_size;
        let src_y = row * cell_size;
        for ty in 0..sprite {
            for tx in 0..sprite {
                let si = ((src_y + ty) * atlas_width + (src_x + tx)) as usize * 4;
                let di = ((dy + ty) * out_size + (dx + tx)) as usize * 4;
                if si + 4 <= atlas_data.len() && di + 4 <= data.len() {
                    data[di..di + 4].copy_from_slice(&atlas_data[si..si + 4]);
                }
            }
        }
    };

    blit(&mut data, quad[0].0, quad[0].1, 0, 0);           // TL
    blit(&mut data, quad[1].0, quad[1].1, sprite, 0);       // TR
    blit(&mut data, quad[2].0, quad[2].1, 0, sprite);       // BL
    blit(&mut data, quad[3].0, quad[3].1, sprite, sprite);  // BR

    Image::new(
        bevy::render::render_resource::Extent3d {
            width: out_size,
            height: out_size,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    )
}

/// Cached egui texture IDs for building sprites. Initialized once.
#[derive(Default)]
pub(crate) struct BuildSpriteCache {
    initialized: bool,
    textures: HashMap<BuildingKind, egui::TextureId>,
    _handles: Vec<Handle<Image>>, // prevent GC of extracted images
}

/// Initialize sprite cache: extract atlas tiles, register all handles with egui.
fn init_sprite_cache(
    cache: &mut BuildSpriteCache,
    contexts: &mut EguiContexts,
    sprites: &SpriteAssets,
    images: &mut Assets<Image>,
    build_ctx: &mut BuildMenuContext,
) {
    if cache.initialized { return; }

    // Farm: BUILDING_TILES[3] = Quad([(2,15),(4,15),(2,17),(4,17)])
    // Tent: BUILDING_TILES[7] = Quad([(48,10),(49,10),(48,11),(49,11)])
    let Some(atlas) = images.get(&sprites.world_texture).cloned() else { return };
    if images.get(&sprites.house_texture).is_none() { return; }
    if images.get(&sprites.barracks_texture).is_none() { return; }
    if images.get(&sprites.waypoint_texture).is_none() { return; }
    if images.get(&sprites.miner_house_texture).is_none() { return; }

    let farm_img = extract_quad_tile(&atlas, [(2, 15), (4, 15), (2, 17), (4, 17)]);
    let tent_img = extract_quad_tile(&atlas, [(48, 10), (49, 10), (48, 11), (49, 11)]);

    let farm_handle = images.add(farm_img);
    let tent_handle = images.add(tent_img);

    // Register all 6 with egui
    let registrations: [(BuildingKind, &Handle<Image>); 7] = [
        (BuildingKind::Farm, &farm_handle),
        (BuildingKind::FarmerHome, &sprites.house_texture),
        (BuildingKind::ArcherHome, &sprites.barracks_texture),
        (BuildingKind::CrossbowHome, &sprites.barracks_texture),
        (BuildingKind::Waypoint, &sprites.waypoint_texture),
        (BuildingKind::Tent, &tent_handle),
        (BuildingKind::MinerHome, &sprites.miner_house_texture),
    ];

    for (kind, handle) in registrations {
        let tex_id = contexts.add_image(EguiTextureHandle::Weak(handle.id()));
        cache.textures.insert(kind, tex_id);
    }

    // Store Bevy handles for world-space ghost preview
    build_ctx.ghost_sprites.insert(BuildingKind::Farm, farm_handle.clone());
    build_ctx.ghost_sprites.insert(BuildingKind::FarmerHome, sprites.house_texture.clone());
    build_ctx.ghost_sprites.insert(BuildingKind::ArcherHome, sprites.barracks_texture.clone());
    build_ctx.ghost_sprites.insert(BuildingKind::CrossbowHome, sprites.barracks_texture.clone());
    build_ctx.ghost_sprites.insert(BuildingKind::Waypoint, sprites.waypoint_texture.clone());
    build_ctx.ghost_sprites.insert(BuildingKind::Tent, tent_handle.clone());
    build_ctx.ghost_sprites.insert(BuildingKind::MinerHome, sprites.miner_house_texture.clone());

    cache._handles.push(farm_handle);
    cache._handles.push(tent_handle);
    cache.initialized = true;
}

pub(crate) fn build_menu_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut build_ctx: ResMut<BuildMenuContext>,
    world_data: Res<world::WorldData>,
    food_storage: Res<FoodStorage>,
    user_settings: Res<UserSettings>,
    _difficulty: Res<Difficulty>,
    sprites: Res<SpriteAssets>,
    mut images: ResMut<Assets<Image>>,
    mut cache: Local<BuildSpriteCache>,
    timings: Res<SystemTimings>,
) -> Result {
    let _t = timings.scope("ui_build_menu");
    // Initialize sprite cache (one-time, before borrowing egui context)
    init_sprite_cache(&mut cache, &mut contexts, &sprites, &mut images, &mut build_ctx);

    let ctx = contexts.ctx_mut()?;

    // Bottom-center Build toggle button (always visible)
    let btn_offset = if ui_state.build_menu_open { -100.0 } else { 0.0 };
    egui::Area::new(egui::Id::new("build_toggle_btn"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, btn_offset])
        .show(ctx, |ui| {
            let label = "Build";
            let btn = egui::Button::new(egui::RichText::new(label).size(14.0))
                .fill(egui::Color32::from_rgb(50, 50, 60));
            if ui.add(btn).clicked() {
                ui_state.build_menu_open = !ui_state.build_menu_open;
                if ui_state.build_menu_open {
                    build_ctx.town_data_idx = world_data.towns.iter().position(|t| t.faction == 0);
                } else {
                    build_ctx.selected_build = None;
                }
            }
        });

    if !ui_state.build_menu_open { return Ok(()); }

    if build_ctx.town_data_idx.is_none() {
        build_ctx.town_data_idx = world_data.towns.iter().position(|t| t.faction == 0);
    }

    let Some(town_data_idx) = build_ctx.town_data_idx else {
        return Ok(());
    };
    let Some(town) = world_data.towns.get(town_data_idx) else {
        return Ok(());
    };
    let is_camp = town.faction > 0;
    let food = food_storage.food.get(town_data_idx).copied().unwrap_or(0);
    let options = if is_camp { CAMP_BUILD_OPTIONS } else { PLAYER_BUILD_OPTIONS };
    let text_scale = user_settings.build_menu_text_scale.clamp(0.7, 2.0);
    let label_size = 13.0 * text_scale;
    let help_size = 11.0 * text_scale;
    let destroy_size = 12.0 * text_scale;

    let frame = egui::Frame::new()
        .fill(egui::Color32::from_rgba_unmultiplied(30, 30, 35, 230))
        .inner_margin(egui::Margin::same(6));

    let mut open = true;
    egui::Window::new("Build")
        .open(&mut open)
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -2.0])
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .title_bar(false)
        .frame(frame)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for option in options {
                    let cost = crate::constants::building_cost(option.kind);
                    let can_afford = food >= cost;
                    let selected = build_ctx.selected_build == Some(option.kind);

                    let resp = ui.vertical(|ui| {
                        // Sprite image
                        if let Some(&tex_id) = cache.textures.get(&option.kind) {
                            let tint = if !can_afford {
                                egui::Color32::from_rgba_unmultiplied(100, 100, 100, 150)
                            } else if selected {
                                egui::Color32::from_rgb(120, 255, 120)
                            } else {
                                egui::Color32::WHITE
                            };
                            ui.add(egui::Image::new(egui::load::SizedTexture::new(tex_id, [48.0, 48.0])).tint(tint));
                        }

                        // Label + cost
                        let label_color = if selected {
                            egui::Color32::from_rgb(120, 220, 120)
                        } else if !can_afford {
                            egui::Color32::from_rgb(120, 120, 120)
                        } else {
                            egui::Color32::from_rgb(200, 200, 200)
                        };
                        ui.label(
                            egui::RichText::new(format!("{} ({})", option.label, cost))
                                .color(label_color)
                                .size(label_size),
                        );
                        ui.label(
                            egui::RichText::new(option.help)
                                .color(egui::Color32::from_rgb(140, 140, 140))
                                .size(help_size),
                        );
                    });

                    // Make the whole column clickable
                    if can_afford && resp.response.interact(egui::Sense::click()).clicked() {
                        if selected {
                            build_ctx.selected_build = None;
                            build_ctx.clear_drag();
                        } else {
                            build_ctx.selected_build = Some(option.kind);
                            build_ctx.destroy_mode = false;
                            build_ctx.clear_drag();
                        }
                    }

                    ui.separator();
                }

                // Destroy button
                let destroy_selected = build_ctx.destroy_mode;
                let destroy_resp = ui.allocate_ui_with_layout(
                    egui::vec2(84.0, 78.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let icon_color = if destroy_selected {
                            egui::Color32::from_rgb(255, 80, 80)
                        } else {
                            egui::Color32::from_rgb(200, 100, 100)
                        };
                        ui.label(egui::RichText::new("X").color(icon_color).size(24.0).strong());
                        let label_color = if destroy_selected {
                            egui::Color32::from_rgb(220, 80, 80)
                        } else {
                            egui::Color32::from_rgb(200, 200, 200)
                        };
                        ui.label(
                            egui::RichText::new("Destroy")
                                .color(label_color)
                                .size(destroy_size),
                        );
                    },
                );
                if ui.interact(destroy_resp.response.rect, egui::Id::new("destroy_btn"), egui::Sense::click()).clicked() {
                    if destroy_selected {
                        build_ctx.destroy_mode = false;
                        build_ctx.clear_drag();
                    } else {
                        build_ctx.destroy_mode = true;
                        build_ctx.selected_build = None;
                        build_ctx.clear_drag();
                    }
                }
            });
        });

    if !open {
        ui_state.build_menu_open = false;
        build_ctx.selected_build = None;
        build_ctx.destroy_mode = false;
        build_ctx.clear_drag();
    }

    // Cursor ghost sprite when placing / red X when destroying
    if build_ctx.destroy_mode || build_ctx.selected_build.is_some() {
        if let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) {
            let show_hint = build_ctx.destroy_mode || build_ctx.show_cursor_hint;
            if show_hint {
                egui::Area::new(egui::Id::new("build_cursor_hint"))
                    .fixed_pos(pos + egui::vec2(12.0, 12.0))
                    .interactable(false)
                    .show(ctx, |ui| {
                        if build_ctx.destroy_mode {
                            ui.label(egui::RichText::new("X").size(32.0)
                                .color(egui::Color32::from_rgb(220, 50, 50)));
                        } else if let Some(selected) = build_ctx.selected_build {
                            if let Some(&tex_id) = cache.textures.get(&selected) {
                                let img = egui::Image::new(egui::load::SizedTexture::new(tex_id, [48.0, 48.0]))
                                    .tint(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 180));
                                ui.add(img);
                            }
                        }
                    });
            }
        }
    }

    Ok(())
}
