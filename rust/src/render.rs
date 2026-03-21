//! Render Module - Bevy 2D sprite rendering for NPCs and world.
//!
//! Replaces Godot MultiMesh with bevy_sprite TextureAtlas.

use bevy::ecs::system::SystemParam;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;

use bevy::sprite_render::{AlphaMode2d, TileData, TilemapChunk, TilemapChunkTileData};

use crate::components::{
    Activity, ActivityKind, Building, Dead, Faction, GpuSlot, Job, ManualTarget, MinerHomeConfig,
    NpcFlags, SquadId,
};
use crate::gpu::RenderFrameConfig;
use crate::messages::{SelectFactionMsg, TerrainDirtyMsg};
use crate::resources::{EntityMap, LeftPanelTab, SelectedBuilding, SelectedNpc, UiState};
use crate::settings::{ControlAction, UserSettings};
use crate::world::{
    BuildingKind, TERRAIN_TILES, WorldData, WorldGrid, build_building_atlas, build_extras_atlas,
    build_tileset, building_tiles,
};

// =============================================================================
// CONSTANTS
// =============================================================================

/// Character sprite sheet: 16x16 sprites, 1px margin = 17px cells
/// roguelikeChar_transparent.png is 918x203 (54 cols x 12 rows approx)
pub const CHAR_CELL: f32 = 17.0;
pub const CHAR_SPRITE_SIZE: f32 = 16.0;
pub const CHAR_SHEET_COLS: u32 = 54;
pub const CHAR_SHEET_ROWS: u32 = 12;

/// World sprite sheet: 16x16 sprites, 1px margin = 17px cells
/// roguelikeSheet_transparent.png is 968x526 (57 cols x 31 rows)
pub const WORLD_CELL: f32 = 17.0;
pub const WORLD_SPRITE_SIZE: f32 = 16.0;
pub const WORLD_SHEET_COLS: u32 = 57;
pub const WORLD_SHEET_ROWS: u32 = 31;
pub const WORLD_SHEET_SIZE: (f32, f32) = (968.0, 526.0);

// =============================================================================
// RESOURCES
// =============================================================================

/// Handles to loaded texture atlases.
#[derive(Resource, Default)]
pub struct SpriteAssets {
    /// Character sprite sheet (NPCs)
    pub char_texture: Handle<Image>,
    pub char_atlas: Handle<TextureAtlasLayout>,
    /// World sprite sheet (terrain, buildings)
    pub world_texture: Handle<Image>,
    pub world_atlas: Handle<TextureAtlasLayout>,
    /// External building sprites loaded from BUILDING_REGISTRY paths.
    pub external_textures: Vec<Handle<Image>>,
    /// Individual sprites composited into the extras atlas (heal, sleep, arrow, boat).
    pub extras_sprites: Vec<Handle<Image>>,
    /// Whether assets are loaded
    pub loaded: bool,
}

/// Marker component for NPC sprites.
#[derive(Component)]
pub struct NpcSprite {
    /// ECS entity this sprite represents
    pub npc_entity: Entity,
}

/// Marker component for the main game camera.
#[derive(Component)]
pub struct MainCamera;

/// Camera state for the render world — extracted from Bevy camera each frame.
/// Not used in the main world; input systems write to Transform + Projection directly.
#[derive(Resource, Clone)]
pub struct CameraState {
    pub position: Vec2,
    pub zoom: f32,
    pub viewport: Vec2,
    pub lod_zoom: f32,
    /// HP bar display mode: 0=Off, 1=WhenDamaged, 2=Always.
    pub hp_bar_mode: u32,
}

const EDGE_PAN_MARGIN: f32 = 8.0;

// =============================================================================
// PLUGIN
// =============================================================================

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpriteAssets>()
            .init_resource::<TilemapSpawned>()
            .add_systems(Startup, (setup_camera, load_sprites))
            .add_systems(
                Update,
                (
                    camera_pan_system,
                    camera_mouse_pan_system,
                    camera_edge_pan_system,
                    camera_zoom_system,
                    camera_follow_system,
                    click_to_select_system,
                    box_select_system,
                    spawn_world_tilemap,
                    sync_terrain_tilemap,
                    sync_terrain_visibility,
                ),
            );
    }
}

/// Set up 2D camera.
fn setup_camera(mut commands: Commands) {
    commands.spawn((Camera2d, MainCamera, Transform::from_xyz(400.0, 300.0, 0.0)));
    info!("2D camera spawned at (400, 300)");
}

/// Load sprite sheets.
fn load_sprites(
    mut assets: ResMut<SpriteAssets>,
    mut config: ResMut<RenderFrameConfig>,
    asset_server: Res<AssetServer>,
    mut texture_atlases: ResMut<Assets<TextureAtlasLayout>>,
) {
    // Load character sprite sheet
    assets.char_texture = asset_server.load("sprites/roguelikeChar_transparent.png");

    // Share texture handles with instanced renderer
    config.textures.handle = Some(assets.char_texture.clone());

    // Create atlas layout for characters (16x16 with 1px padding)
    let char_layout = TextureAtlasLayout::from_grid(
        UVec2::new(CHAR_SPRITE_SIZE as u32, CHAR_SPRITE_SIZE as u32),
        CHAR_SHEET_COLS,
        CHAR_SHEET_ROWS,
        Some(UVec2::new(1, 1)), // 1px padding
        Some(UVec2::new(0, 0)), // no offset
    );
    assets.char_atlas = texture_atlases.add(char_layout);

    // Load world sprite sheet + external building sprites from registry
    assets.world_texture = asset_server.load("sprites/roguelikeSheet_transparent.png");
    assets.external_textures = crate::constants::BUILDING_REGISTRY
        .iter()
        .filter_map(|def| match def.tile {
            crate::constants::TileSpec::External(path) => Some(asset_server.load(path)),
            _ => None,
        })
        .collect();
    config.textures.world_handle = Some(assets.world_texture.clone());

    // Extras atlas sprites: composited into a single grid texture in spawn_world_tilemap
    // Order must match atlas_id mapping in npc_render.wgsl calc_uv:
    //   col 0 = heal (atlas 2), col 1 = sleep (atlas 3), col 2 = arrow (atlas 4), col 3 = boat (atlas 8)
    assets.extras_sprites = vec![
        asset_server.load("sprites/heal.png"),
        asset_server.load("sprites/sleep.png"),
        asset_server.load("sprites/arrow.png"),
        asset_server.load("sprites/boat.png"),
    ];

    // Create atlas layout for world sprites
    let world_layout = TextureAtlasLayout::from_grid(
        UVec2::new(WORLD_SPRITE_SIZE as u32, WORLD_SPRITE_SIZE as u32),
        WORLD_SHEET_COLS,
        WORLD_SHEET_ROWS,
        Some(UVec2::new(1, 1)),
        Some(UVec2::new(0, 0)),
    );
    assets.world_atlas = texture_atlases.add(world_layout);

    assets.loaded = true;
    info!(
        "Sprite sheets loaded: char ({}x{}), world ({}x{})",
        CHAR_SHEET_COLS, CHAR_SHEET_ROWS, WORLD_SHEET_COLS, WORLD_SHEET_ROWS
    );
}

// =============================================================================
// CAMERA SYSTEMS
// =============================================================================

/// Read zoom factor from Projection (1.0 / orthographic scale).
fn ortho_zoom(projection: &Projection) -> f32 {
    match projection {
        Projection::Orthographic(ortho) => 1.0 / ortho.scale,
        _ => 1.0,
    }
}

/// Keyboard camera pan. Speed scales inversely with zoom for consistent screen-space feel.
/// Uses wall-clock delta (not game-scaled DeltaTime) so camera speed is independent of game speed.
fn camera_pan_system(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut query: Query<(&mut Transform, &Projection), With<MainCamera>>,
    user_settings: Res<UserSettings>,
    mut contexts: bevy_egui::EguiContexts,
) {
    // Suppress camera pan when typing in a text field
    if contexts
        .ctx_mut()
        .is_ok_and(|ctx| ctx.wants_keyboard_input())
    {
        return;
    }
    let Ok((mut transform, projection)) = query.single_mut() else {
        return;
    };

    let up_key = user_settings.key_for_action(ControlAction::PanUp);
    let down_key = user_settings.key_for_action(ControlAction::PanDown);
    let left_key = user_settings.key_for_action(ControlAction::PanLeft);
    let right_key = user_settings.key_for_action(ControlAction::PanRight);

    let mut dir = Vec2::ZERO;
    if keys.pressed(up_key) {
        dir.y += 1.0;
    }
    if keys.pressed(down_key) {
        dir.y -= 1.0;
    }
    if keys.pressed(left_key) {
        dir.x -= 1.0;
    }
    if keys.pressed(right_key) {
        dir.x += 1.0;
    }

    if dir != Vec2::ZERO {
        let speed = user_settings.scroll_speed / ortho_zoom(projection);
        let delta = dir.normalize() * speed * time.delta_secs();
        transform.translation.x += delta.x;
        transform.translation.y += delta.y;
    }
}

/// Right-click drag to pan camera.
fn camera_mouse_pan_system(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    mut query: Query<(&mut Transform, &Projection), With<MainCamera>>,
    mut egui_contexts: bevy_egui::EguiContexts,
    mut last_pos: Local<Option<Vec2>>,
) {
    if let Ok(ctx) = egui_contexts.ctx_mut() {
        if ctx.wants_pointer_input() || ctx.is_pointer_over_area() {
            *last_pos = None;
            return;
        }
    }

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else {
        *last_pos = None;
        return;
    };

    if mouse.just_pressed(MouseButton::Right) {
        *last_pos = Some(cursor_pos);
        return;
    }

    if mouse.pressed(MouseButton::Right) {
        if let Some(prev) = *last_pos {
            let screen_delta = cursor_pos - prev;
            if screen_delta != Vec2::ZERO {
                let Ok((mut transform, projection)) = query.single_mut() else {
                    return;
                };
                let zoom = ortho_zoom(projection);
                // Screen-space to world-space: divide by zoom, flip Y
                transform.translation.x -= screen_delta.x / zoom;
                transform.translation.y += screen_delta.y / zoom;
            }
        }
        *last_pos = Some(cursor_pos);
    } else {
        *last_pos = None;
    }
}

/// Pan camera when cursor hovers near screen edges.
/// Uses wall-clock delta (not game-scaled DeltaTime) so camera speed is independent of game speed.
fn camera_edge_pan_system(
    windows: Query<&Window>,
    time: Res<Time>,
    mut query: Query<(&mut Transform, &Projection), With<MainCamera>>,
    user_settings: Res<UserSettings>,
) {
    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };
    let w = window.width();
    let h = window.height();

    let mut dir = Vec2::ZERO;
    if cursor_pos.x < EDGE_PAN_MARGIN {
        dir.x -= 1.0;
    }
    if cursor_pos.x > w - EDGE_PAN_MARGIN {
        dir.x += 1.0;
    }
    if cursor_pos.y < EDGE_PAN_MARGIN {
        dir.y += 1.0;
    } // top of screen = +Y world
    if cursor_pos.y > h - EDGE_PAN_MARGIN {
        dir.y -= 1.0;
    }

    if dir != Vec2::ZERO {
        let Ok((mut transform, projection)) = query.single_mut() else {
            return;
        };
        let speed = user_settings.scroll_speed / ortho_zoom(projection);
        let delta = dir.normalize() * speed * time.delta_secs();
        transform.translation.x += delta.x;
        transform.translation.y += delta.y;
    }
}

/// Scroll wheel zoom toward mouse cursor.
fn camera_zoom_system(
    accumulated_scroll: Res<AccumulatedMouseScroll>,
    windows: Query<&Window>,
    mut query: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
    mut egui_contexts: bevy_egui::EguiContexts,
    ui_state: Res<UiState>,
    user_settings: Res<UserSettings>,
) {
    // Tech tree owns mouse-wheel behavior while open.
    if ui_state.tech_tree_open {
        return;
    }

    // Don't zoom when scrolling over UI panels (combat log, etc.)
    if let Ok(ctx) = egui_contexts.ctx_mut() {
        if ctx.wants_pointer_input() || ctx.is_pointer_over_area() {
            return;
        }
    }
    let scroll = accumulated_scroll.delta.y;
    if scroll == 0.0 {
        return;
    }

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };
    let Ok((mut transform, mut projection)) = query.single_mut() else {
        return;
    };
    let Projection::Orthographic(ref mut ortho) = *projection else {
        return;
    };

    let zoom = 1.0 / ortho.scale;
    let position = transform.translation.truncate();
    let viewport = Vec2::new(window.width(), window.height());
    let screen_center = viewport / 2.0;

    // Mouse offset from screen center (flip Y: screen Y-down → world Y-up)
    let mouse_offset = Vec2::new(
        cursor_pos.x - screen_center.x,
        screen_center.y - cursor_pos.y,
    );

    // World position under mouse before zoom
    let world_pos = position + mouse_offset / zoom;

    // Apply zoom
    let factor = if scroll > 0.0 {
        1.0 + user_settings.zoom_speed
    } else {
        1.0 - user_settings.zoom_speed
    };
    let new_zoom = (zoom * factor).clamp(user_settings.zoom_min, user_settings.zoom_max);
    ortho.scale = 1.0 / new_zoom;

    // Move camera so world_pos stays under mouse
    let new_position = world_pos - mouse_offset / new_zoom;
    transform.translation.x = new_position.x;
    transform.translation.y = new_position.y;
}

/// Track the camera to the selected NPC when follow mode is active.
fn camera_follow_system(
    selected: Res<SelectedNpc>,
    follow: Res<crate::resources::FollowSelected>,
    gpu_state: Res<crate::resources::GpuReadState>,
    mut query: Query<&mut Transform, With<MainCamera>>,
) {
    if !follow.0 || selected.0 < 0 {
        return;
    }
    let idx = selected.0 as usize;
    let positions = &gpu_state.positions;
    if idx * 2 + 1 >= positions.len() {
        return;
    }
    let x = positions[idx * 2];
    let y = positions[idx * 2 + 1];
    if x < -9000.0 {
        return;
    } // dead/hidden
    if let Ok(mut transform) = query.single_mut() {
        transform.translation.x = x;
        transform.translation.y = y;
    }
}

/// Tracks last click for double-click detection.
#[derive(Default)]
struct DoubleClickState {
    last_time: f64,
    last_pos: Vec2,
}

#[derive(Clone, Copy)]
struct NpcHit {
    slot: usize,
    pos: Vec2,
    dist_sq: f32,
}

#[derive(Clone, Copy)]
struct BuildingHit {
    kind: BuildingKind,
    pos: Vec2,
    slot: usize,
    dist_sq: f32,
}

fn gpu_slot_position(positions: &[f32], slot: usize) -> Option<Vec2> {
    let base = slot.checked_mul(2)?;
    let px = *positions.get(base)?;
    let py = *positions.get(base + 1)?;
    if px < -9000.0 {
        return None;
    }
    Some(Vec2::new(px, py))
}

fn nearest_npc_hit<F>(
    entity_map: &EntityMap,
    positions: &[f32],
    world_pos: Vec2,
    max_radius: f32,
    mut predicate: F,
) -> Option<NpcHit>
where
    F: FnMut(&crate::resources::NpcEntry) -> bool,
{
    let mut best_dist_sq = max_radius * max_radius;
    let mut best = None;
    for npc in entity_map.iter_npcs() {
        if npc.dead || !predicate(npc) {
            continue;
        }
        let Some(pos) = gpu_slot_position(positions, npc.slot) else {
            continue;
        };
        let dist_sq = world_pos.distance_squared(pos);
        if dist_sq < best_dist_sq {
            best_dist_sq = dist_sq;
            best = Some(NpcHit {
                slot: npc.slot,
                pos,
                dist_sq,
            });
        }
    }
    best
}

fn nearest_building_hit<F>(
    entity_map: &EntityMap,
    world_pos: Vec2,
    max_radius: f32,
    mut predicate: F,
) -> Option<BuildingHit>
where
    F: FnMut(&crate::resources::BuildingInstance) -> bool,
{
    let mut best_dist_sq = max_radius * max_radius;
    let mut best = None;
    for inst in entity_map.iter_instances() {
        if inst.position.x < -9000.0 || !predicate(inst) {
            continue;
        }
        let dist_sq = world_pos.distance_squared(inst.position);
        if dist_sq < best_dist_sq {
            best_dist_sq = dist_sq;
            best = Some(BuildingHit {
                kind: inst.kind,
                pos: inst.position,
                slot: inst.slot,
                dist_sq,
            });
        }
    }
    best
}

#[derive(SystemParam)]
struct ClickSelectParams<'w> {
    selected: ResMut<'w, SelectedNpc>,
    selected_building: ResMut<'w, SelectedBuilding>,
    squad_state: ResMut<'w, crate::resources::SquadState>,
    build_ctx: Res<'w, crate::resources::BuildMenuContext>,
    entity_map: ResMut<'w, crate::resources::EntityMap>,
    ui_state: ResMut<'w, crate::resources::UiState>,
    world_data: ResMut<'w, WorldData>,
}

/// Left click to select nearest NPC within 20px.
/// Skips when egui wants the pointer (clicking UI buttons).
fn click_to_select_system(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_query: Query<(&Transform, &Projection), With<MainCamera>>,
    mut click: ClickSelectParams,
    mut egui_contexts: bevy_egui::EguiContexts,
    gpu_state: Res<crate::resources::GpuReadState>,
    grid: Res<WorldGrid>,
    time: Res<Time<Real>>,
    mut dbl_click: Local<DoubleClickState>,
    mut intents: ResMut<crate::resources::PathRequestQueue>,
    mut faction_select: MessageWriter<SelectFactionMsg>,
    mut commands: Commands,
    mut npc_flags_q: Query<&mut NpcFlags>,
    mut activity_q: Query<&mut Activity>,
    mut miner_cfg_q: Query<&mut MinerHomeConfig>,
) {
    // Right-click: squad target placement, DirectControl micro, or cancel mine assignment
    if mouse.just_pressed(MouseButton::Right) {
        if click.ui_state.assigning_mine.is_some() {
            click.ui_state.assigning_mine = None;
            return;
        }

        let Ok(window) = windows.single() else { return };
        let Some(cursor_pos) = window.cursor_position() else {
            return;
        };
        let Ok((transform, projection)) = camera_query.single() else {
            return;
        };
        let zoom = ortho_zoom(projection);
        let cam = transform.translation.truncate();
        let viewport = Vec2::new(window.width(), window.height());
        let screen_center = viewport / 2.0;
        let mouse_offset = Vec2::new(
            cursor_pos.x - screen_center.x,
            screen_center.y - cursor_pos.y,
        );
        let world_pos = cam + mouse_offset / zoom;

        // Squad target placement mode: right-click sets squad.target for whole squad
        if click.squad_state.placing_target {
            let si = click.squad_state.selected;
            if si >= 0 && (si as usize) < click.squad_state.squads.len() {
                click.squad_state.squads[si as usize].target = Some(world_pos);
            }
            click.squad_state.placing_target = false;
            return;
        }

        // DirectControl micro: right-click commands only box-selected (DirectControl) members
        let si = click.squad_state.selected;
        if si >= 0
            && (si as usize) < click.squad_state.squads.len()
            && click.squad_state.squads[si as usize].is_player()
        {
            let members: Vec<usize> = click.squad_state.squads[si as usize]
                .members
                .iter()
                .filter_map(|uid| click.entity_map.slot_for_entity(*uid))
                .filter(|&slot| {
                    click
                        .entity_map
                        .entities
                        .get(&slot)
                        .and_then(|&e| npc_flags_q.get(e).ok())
                        .is_some_and(|f| f.direct_control)
                })
                .collect();
            if members.is_empty() {
                return;
            }

            let positions = &gpu_state.positions;
            let best_enemy =
                nearest_npc_hit(&click.entity_map, positions, world_pos, 20.0, |npc| {
                    npc.faction != crate::constants::FACTION_PLAYER
                        && npc.faction != crate::constants::FACTION_NEUTRAL
                });

            if let Some(enemy) = best_enemy {
                // Attack NPC: set ManualTarget + move toward enemy
                for &slot in &members {
                    if let Some(npc) = click.entity_map.get_npc(slot) {
                        let entity = npc.entity;
                        commands
                            .entity(entity)
                            .insert(ManualTarget::Npc(enemy.slot));
                        // Wake resting NPCs on move command
                        if let Ok(mut act) = activity_q.get_mut(entity) {
                            if act.kind == ActivityKind::Rest {
                                *act = Activity::default();
                            }
                        }
                        intents.submit(
                            entity,
                            enemy.pos,
                            crate::resources::MovementPriority::DirectControl,
                            "dc:attack",
                        );
                    }
                }
            } else {
                // Determine ManualTarget variant + GPU move target
                let (mt, target_pos) = if let Some(building) =
                    nearest_building_hit(&click.entity_map, world_pos, 48.0, |inst| {
                        inst.faction != crate::constants::FACTION_PLAYER
                            && inst.faction != crate::constants::FACTION_NEUTRAL
                    }) {
                    (ManualTarget::Building(building.pos), building.pos)
                } else {
                    (ManualTarget::Position(world_pos), world_pos)
                };
                for &slot in &members {
                    if let Some(npc) = click.entity_map.get_npc(slot) {
                        let entity = npc.entity;
                        commands.entity(entity).insert(mt.clone());
                        if let Ok(mut act) = activity_q.get_mut(entity) {
                            if act.kind == ActivityKind::Rest {
                                *act = Activity::default();
                            }
                        }
                        intents.submit(
                            entity,
                            target_pos,
                            crate::resources::MovementPriority::DirectControl,
                            "dc:move",
                        );
                    }
                }
            }
            return;
        }
    }

    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    // Build placement owns left-click while a build is selected.
    if click.build_ctx.selected_build.is_some() {
        return;
    }

    // Don't steal clicks from egui UI
    if let Ok(ctx) = egui_contexts.ctx_mut() {
        if ctx.wants_pointer_input() || ctx.is_pointer_over_area() {
            return;
        }
    }

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };
    let Ok((transform, projection)) = camera_query.single() else {
        return;
    };

    let zoom = ortho_zoom(projection);
    let position = transform.translation.truncate();
    let viewport = Vec2::new(window.width(), window.height());
    let screen_center = viewport / 2.0;
    let mouse_offset = Vec2::new(
        cursor_pos.x - screen_center.x,
        screen_center.y - cursor_pos.y,
    );
    let world_pos = position + mouse_offset / zoom;

    // Mine assignment — snap to nearest gold mine within radius
    if let Some(mh_slot) = click.ui_state.assigning_mine {
        let snap_radius = 60.0;
        let best = click
            .entity_map
            .iter_kind(BuildingKind::GoldMine)
            .map(|inst| (inst.position.distance(world_pos), inst.position))
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((dist, mine_pos)) = best {
            if dist < snap_radius {
                if let Some(&entity) = click.entity_map.entities.get(&mh_slot) {
                    if let Ok(mut cfg) = miner_cfg_q.get_mut(entity) {
                        cfg.manual_mine = true;
                        cfg.assigned_mine = Some(mine_pos);
                    }
                }
            }
        }
        click.ui_state.assigning_mine = None;
        return;
    }

    // Selection scan:
    // - NPCs use GPU readback positions (movement is GPU-driven).
    // - Buildings use authoritative EntityMap positions (deterministic placement).
    let positions = &gpu_state.positions;
    let best_npc = nearest_npc_hit(&click.entity_map, positions, world_pos, 40.0, |_| true);
    let best_idx = best_npc.map(|hit| hit.slot as i32).unwrap_or(-1);
    let best_building = nearest_building_hit(&click.entity_map, world_pos, 48.0, |_| true);

    // Double-click detection
    let now = time.elapsed_secs_f64();
    let is_double =
        (now - dbl_click.last_time) < 0.4 && (world_pos - dbl_click.last_pos).length() < 5.0;
    dbl_click.last_time = now;
    dbl_click.last_pos = world_pos;

    // Keep up to one NPC and one building selected from the same click.
    click.selected.0 = best_idx;
    if let Some(building) = best_building {
        let (bcol, brow) = grid.world_to_grid(building.pos);
        *click.selected_building = SelectedBuilding {
            col: bcol,
            row: brow,
            active: true,
            slot: Some(building.slot),
            kind: Some(building.kind),
        };

        // Double-click fountain -> open Factions tab for that faction.
        if is_double && building.kind == crate::world::BuildingKind::Fountain {
            if let Some(inst) = click.entity_map.get_instance(building.slot) {
                if let Some(town) = click.world_data.towns.get(inst.town_idx as usize) {
                    click.ui_state.left_panel_open = true;
                    click.ui_state.left_panel_tab = LeftPanelTab::Factions;
                    faction_select.write(SelectFactionMsg(town.faction));
                }
            }
        }

        // Double-click casino -> open blackjack popup.
        if is_double && building.kind == crate::world::BuildingKind::Casino {
            click.ui_state.casino_open = true;
        }
    } else {
        click.selected_building.active = false;
        click.selected_building.slot = None;
        click.selected_building.kind = None;
    }

    // Default active inspector tab by click proximity.
    if let (Some(npc), Some(building)) = (best_npc, best_building) {
        click.ui_state.inspector_prefer_npc = npc.dist_sq <= building.dist_sq;
    } else if best_idx >= 0 {
        click.ui_state.inspector_prefer_npc = true;
    } else if best_building.is_some() {
        click.ui_state.inspector_prefer_npc = false;
    }
    click.ui_state.inspector_click_seq = click.ui_state.inspector_click_seq.saturating_add(1);

    // Click empty ground → clear DirectControl from all player squad members
    if best_idx < 0 && best_building.is_none() {
        for squad in click.squad_state.squads.iter() {
            if !squad.is_player() {
                continue;
            }
            for &uid in &squad.members {
                let Some(slot) = click.entity_map.slot_for_entity(uid) else {
                    continue;
                };
                if let Some(&entity) = click.entity_map.entities.get(&slot) {
                    if let Ok(mut flags) = npc_flags_q.get_mut(entity) {
                        flags.direct_control = false;
                    }
                }
            }
        }
    }
}

// =============================================================================
// BOX SELECT
// =============================================================================

/// Runs every frame to track box-select drag state.
/// Left-press starts drag, movement > 5px activates box mode,
/// release selects all player NPCs in the AABB and populates the active squad.
fn box_select_system(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_query: Query<(&Transform, &Projection), With<MainCamera>>,
    mut squad_state: ResMut<crate::resources::SquadState>,
    build_ctx: Res<crate::resources::BuildMenuContext>,
    mut egui_contexts: bevy_egui::EguiContexts,
    entity_map: Res<EntityMap>,
    mut selected_npc: ResMut<SelectedNpc>,
    mut selected_building: ResMut<crate::resources::SelectedBuilding>,
    mut commands: Commands,
    mut npc_flags_q: Query<&mut NpcFlags>,
    box_npc_q: Query<(&GpuSlot, &Job, &Faction), (Without<Building>, Without<Dead>)>,
    gpu_state: Res<crate::resources::GpuReadState>,
) {
    // Don't box-select while building or placing squad targets
    if build_ctx.selected_build.is_some() || squad_state.placing_target {
        return;
    }

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };
    let Ok((transform, projection)) = camera_query.single() else {
        return;
    };
    let zoom = ortho_zoom(projection);
    let cam = transform.translation.truncate();
    let viewport = Vec2::new(window.width(), window.height());
    let screen_center = viewport / 2.0;
    let mouse_offset = Vec2::new(
        cursor_pos.x - screen_center.x,
        screen_center.y - cursor_pos.y,
    );
    let world_pos = cam + mouse_offset / zoom;

    // Check egui wants pointer
    let egui_wants = if let Ok(ctx) = egui_contexts.ctx_mut() {
        ctx.wants_pointer_input() || ctx.is_pointer_over_area()
    } else {
        false
    };

    if mouse.just_pressed(MouseButton::Left) && !egui_wants {
        squad_state.drag_start = Some(world_pos);
        squad_state.box_selecting = false;
    }

    if mouse.pressed(MouseButton::Left) {
        if let Some(start) = squad_state.drag_start {
            let dist = (world_pos - start).length();
            if dist > 5.0 {
                squad_state.box_selecting = true;
            }
        }
    }

    if mouse.just_released(MouseButton::Left) {
        if squad_state.box_selecting {
            if let Some(start) = squad_state.drag_start {
                let min_x = start.x.min(world_pos.x);
                let max_x = start.x.max(world_pos.x);
                let min_y = start.y.min(world_pos.y);
                let max_y = start.y.max(world_pos.y);

                let positions = &gpu_state.positions;
                let mut selected_slots: Vec<usize> = Vec::new();
                for (slot, job, faction) in box_npc_q.iter() {
                    // Player faction is 1; 0 is neutral and should never box-select.
                    if faction.0 != crate::constants::FACTION_PLAYER {
                        continue;
                    }
                    if !job.is_military() {
                        continue;
                    }
                    let i = slot.0;
                    if i * 2 + 1 >= positions.len() {
                        continue;
                    }
                    let px = positions[i * 2];
                    let py = positions[i * 2 + 1];
                    if px < -9000.0 {
                        continue;
                    }
                    if px >= min_x && px <= max_x && py >= min_y && py <= max_y {
                        selected_slots.push(i);
                    }
                }

                if !selected_slots.is_empty() {
                    let selected_set: std::collections::HashSet<usize> =
                        selected_slots.iter().copied().collect();
                    // Auto-select squad 0 if none selected
                    let si = if squad_state.selected < 0 {
                        0
                    } else {
                        squad_state.selected as usize
                    };
                    if si < squad_state.squads.len() && squad_state.squads[si].is_player() {
                        // Remove DirectControl from old squad members being replaced
                        for &old_uid in &squad_state.squads[si].members {
                            let Some(old_slot) = entity_map.slot_for_entity(old_uid) else {
                                continue;
                            };
                            if !selected_set.contains(&old_slot) {
                                if let Some(&entity) = entity_map.entities.get(&old_slot) {
                                    if let Ok(mut flags) = npc_flags_q.get_mut(entity) {
                                        flags.direct_control = false;
                                    }
                                }
                            }
                        }
                        // Remove these slots from any other player squad first
                        for qi in 0..squad_state.squads.len() {
                            if qi == si {
                                continue;
                            }
                            if !squad_state.squads[qi].is_player() {
                                continue;
                            }
                            squad_state.squads[qi].members.retain(|uid| {
                                entity_map
                                    .slot_for_entity(*uid)
                                    .is_some_and(|s| !selected_set.contains(&s))
                            });
                        }
                        // Set as the squad's members (replace, not append) — convert slots to UIDs
                        squad_state.squads[si].members = selected_slots
                            .iter()
                            .filter_map(|&slot| entity_map.entities.get(&slot).copied())
                            .collect();
                        // Update SquadId + DirectControl on each selected NPC
                        for &slot in &selected_slots {
                            if let Some(&entity) = entity_map.entities.get(&slot) {
                                commands.entity(entity).insert(SquadId(si as i32));
                                if let Ok(mut flags) = npc_flags_q.get_mut(entity) {
                                    flags.direct_control = true;
                                }
                            }
                        }
                        squad_state.selected = si as i32;
                    }
                    // Clear individual selections so inspector shows DC group view
                    selected_npc.0 = -1;
                    selected_building.active = false;
                }
            }
        }
        squad_state.drag_start = None;
        squad_state.box_selecting = false;
    }
}

// =============================================================================
// WORLD TILEMAP (TERRAIN + BUILDINGS)
// =============================================================================

/// Tracks whether the tilemap has been spawned. Resource (not Local) so cleanup can reset it.
#[derive(Resource, Default)]
pub struct TilemapSpawned(pub bool);

/// Chunk size in tiles for terrain tilemap splitting (32x32 = 1024 tiles per chunk).
const CHUNK_SIZE: usize = 32;

/// Marker component on terrain TilemapChunk entities for runtime tile updates.
#[derive(Component)]
pub struct TerrainChunk;

/// Grid origin and size for a terrain chunk, used by sync to update only its sub-region.
#[derive(Component)]
pub struct TerrainChunkRegion {
    pub origin_x: usize,
    pub origin_y: usize,
    pub chunk_w: usize,
    pub chunk_h: usize,
}

/// Spawn terrain TilemapChunk + building atlas for instanced renderer.
/// Runs once when WorldGrid is populated and all images are loaded.
fn spawn_world_tilemap(
    mut commands: Commands,
    grid: Res<WorldGrid>,
    assets: Res<SpriteAssets>,
    mut images: ResMut<Assets<Image>>,
    mut spawned: ResMut<TilemapSpawned>,
    mut config: ResMut<RenderFrameConfig>,
) {
    if spawned.0 || grid.width == 0 {
        return;
    }
    let Some(atlas) = images.get(&assets.world_texture).cloned() else {
        return;
    };
    // Collect external building images from registry-driven handles
    let extra_imgs: Option<Vec<Image>> = assets
        .external_textures
        .iter()
        .map(|h| images.get(h).cloned())
        .collect();
    let Some(extra_imgs) = extra_imgs else {
        return;
    };
    let extra_refs: Vec<&Image> = extra_imgs.iter().collect();

    // Terrain layer — split into CHUNK_SIZE x CHUNK_SIZE chunks for frustum culling
    let terrain_tileset = build_tileset(&atlas, &TERRAIN_TILES, &[], &mut images);
    let tile_disp = UVec2::new(grid.cell_size as u32, grid.cell_size as u32);
    let mut chunk_count = 0u32;
    for cy in (0..grid.height).step_by(CHUNK_SIZE) {
        for cx in (0..grid.width).step_by(CHUNK_SIZE) {
            let cw = CHUNK_SIZE.min(grid.width - cx);
            let ch = CHUNK_SIZE.min(grid.height - cy);
            let mut tile_data = Vec::with_capacity(cw * ch);
            for ly in 0..ch {
                for lx in 0..cw {
                    let gi = (cy + ly) * grid.width + (cx + lx);
                    tile_data.push(Some(TileData::from_tileset_index(
                        grid.cells[gi].terrain.tileset_index(gi),
                    )));
                }
            }
            let center_x = (cx as f32 + cw as f32 / 2.0) * grid.cell_size;
            let center_y = (cy as f32 + ch as f32 / 2.0) * grid.cell_size;
            commands.spawn((
                TilemapChunk {
                    chunk_size: UVec2::new(cw as u32, ch as u32),
                    tile_display_size: tile_disp,
                    tileset: terrain_tileset.clone(),
                    alpha_mode: AlphaMode2d::Blend,
                },
                TilemapChunkTileData(tile_data),
                Transform::from_xyz(center_x, center_y, -1.0),
                TerrainChunk,
                TerrainChunkRegion {
                    origin_x: cx,
                    origin_y: cy,
                    chunk_w: cw,
                    chunk_h: ch,
                },
            ));
            chunk_count += 1;
        }
    }

    // Building atlas for NPC instanced renderer (replaces building TilemapChunk)
    let btiles = building_tiles();
    let building_atlas = build_building_atlas(&atlas, &btiles, &extra_refs, &mut images);
    if let Some(img) = images.get(&building_atlas) {
        assert_eq!(
            img.height(),
            crate::world::ATLAS_CELL
                * (btiles.len()
                    + crate::constants::autotile_total_extra_layers()
                    + crate::constants::pick_total_extra_layers()) as u32,
            "building atlas height mismatch"
        );
    }
    config.textures.building_handle = Some(building_atlas);

    // Extras atlas: composites heal, sleep, arrow, boat into a single grid texture
    let extras_imgs: Option<Vec<Image>> = assets
        .extras_sprites
        .iter()
        .map(|h| images.get(h).cloned())
        .collect();
    if let Some(extras_imgs) = extras_imgs {
        config.textures.extras_handle = Some(build_extras_atlas(&extras_imgs, &mut images));
    }

    info!(
        "World tilemap spawned: {}x{} grid ({} terrain chunks)",
        grid.width, grid.height, chunk_count
    );
    spawned.0 = true;
}

/// Sync terrain tilemap tiles when WorldGrid terrain changes (slot unlock → Dirt).
/// Each chunk only re-reads its own sub-region of the grid.
fn rebuild_chunk(
    grid: &WorldGrid,
    tile_data: &mut TilemapChunkTileData,
    region: &TerrainChunkRegion,
) {
    for ly in 0..region.chunk_h {
        for lx in 0..region.chunk_w {
            let gi = (region.origin_y + ly) * grid.width + (region.origin_x + lx);
            let li = ly * region.chunk_w + lx;
            tile_data.0[li] = Some(TileData::from_tileset_index(
                grid.cells[gi].terrain.tileset_index(gi),
            ));
        }
    }
}

fn sync_terrain_tilemap(
    grid: Res<WorldGrid>,
    mut chunks: Query<(&mut TilemapChunkTileData, &TerrainChunkRegion), With<TerrainChunk>>,
    mut terrain_dirty: MessageReader<TerrainDirtyMsg>,
) {
    if grid.width == 0 {
        return;
    }

    let messages: Vec<TerrainDirtyMsg> = terrain_dirty.read().cloned().collect();
    if messages.is_empty() {
        return;
    }

    // Full rebuild if any message carries no tile coords (init, load, large area change).
    let full_rebuild = messages.iter().any(|m| m.tile.is_none());
    if full_rebuild {
        for (mut tile_data, region) in chunks.iter_mut() {
            rebuild_chunk(&grid, &mut tile_data, region);
        }
        return;
    }

    // Partial rebuild: collect the set of chunk (col, row) indices that contain a dirty tile.
    let mut dirty: Vec<(usize, usize)> = Vec::with_capacity(messages.len());
    for msg in &messages {
        if let Some((col, row)) = msg.tile {
            let chunk_col = col as usize / CHUNK_SIZE;
            let chunk_row = row as usize / CHUNK_SIZE;
            let pair = (chunk_col, chunk_row);
            if !dirty.contains(&pair) {
                dirty.push(pair);
            }
        }
    }

    for (mut tile_data, region) in chunks.iter_mut() {
        let chunk_col = region.origin_x / CHUNK_SIZE;
        let chunk_row = region.origin_y / CHUNK_SIZE;
        if dirty.contains(&(chunk_col, chunk_row)) {
            rebuild_chunk(&grid, &mut tile_data, region);
        }
    }
}

/// Toggle terrain tile visibility from user debug setting.
fn sync_terrain_visibility(
    user_settings: Res<UserSettings>,
    mut chunks: Query<&mut Visibility, With<TerrainChunk>>,
) {
    let vis = if user_settings.show_terrain_sprites {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut v in chunks.iter_mut() {
        if *v != vis {
            *v = vis;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use bevy::ecs::system::RunSystemOnce;
    use bevy::time::TimeUpdateStrategy;
    use bevy_egui::EguiUserTextures;

    /// Helper: populate a flat WorldGrid with a given width/height (all Grass).
    fn flat_grid(width: usize, height: usize) -> crate::world::WorldGrid {
        let mut grid = crate::world::WorldGrid::default();
        grid.width = width;
        grid.height = height;
        grid.cell_size = 16.0;
        grid.cells = vec![
            crate::world::WorldCell {
                terrain: crate::world::Biome::Grass,
                original_terrain: crate::world::Biome::Grass,
            };
            width * height
        ];
        grid
    }

    /// Regression test: a single-tile dirty message must rebuild only the affected chunk.
    /// Without the optimization (full rebuild on every message), chunk 1 would be rebuilt
    /// from None -> Some, causing this test to fail.
    #[test]
    fn test_sync_terrain_tilemap_partial_chunk_rebuild() {
        use crate::world::Biome;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        // 64x32 grid: chunk 0 covers cols 0..31, chunk 1 covers cols 32..63.
        let width = 64;
        let height = 32;
        let mut grid = flat_grid(width, height);
        // Tile (5, 5) in chunk 0 gets Forest.
        grid.cells[5 * width + 5].terrain = Biome::Forest;
        app.insert_resource(grid);
        app.add_message::<TerrainDirtyMsg>();

        // Spawn chunk 0 with all tiles None.
        app.world_mut().spawn((
            TilemapChunkTileData(vec![None; CHUNK_SIZE * CHUNK_SIZE]),
            TerrainChunkRegion {
                origin_x: 0,
                origin_y: 0,
                chunk_w: CHUNK_SIZE,
                chunk_h: CHUNK_SIZE,
            },
            TerrainChunk,
        ));
        // Spawn chunk 1 with all tiles None (sentinel: should stay None).
        app.world_mut().spawn((
            TilemapChunkTileData(vec![None; CHUNK_SIZE * CHUNK_SIZE]),
            TerrainChunkRegion {
                origin_x: CHUNK_SIZE,
                origin_y: 0,
                chunk_w: CHUNK_SIZE,
                chunk_h: CHUNK_SIZE,
            },
            TerrainChunk,
        ));

        // Write a tile-specific dirty message for tile (5, 5) -- inside chunk 0.
        app.world_mut()
            .run_system_once(|mut w: MessageWriter<TerrainDirtyMsg>| {
                w.write(TerrainDirtyMsg { tile: Some((5, 5)) });
            })
            .unwrap();

        // Run sync_terrain_tilemap.
        app.world_mut()
            .run_system_once(sync_terrain_tilemap)
            .unwrap();

        // Verify chunk 0 was rebuilt and chunk 1 was not.
        let mut q = app
            .world_mut()
            .query::<(&TilemapChunkTileData, &TerrainChunkRegion)>();
        let results: Vec<_> = q
            .iter(app.world())
            .map(|(td, r)| (r.origin_x, td.0.iter().any(|t| t.is_some())))
            .collect();

        for (origin_x, any_some) in results {
            if origin_x == 0 {
                assert!(any_some, "chunk 0 (dirty) should be rebuilt");
            } else {
                assert!(!any_some, "chunk 1 (not dirty) should not be rebuilt");
            }
        }
    }

    /// Full rebuild (tile: None) must still rebuild all chunks.
    #[test]
    fn test_sync_terrain_tilemap_full_rebuild() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        let width = 64;
        let height = 32;
        app.insert_resource(flat_grid(width, height));
        app.add_message::<TerrainDirtyMsg>();

        for cx in [0, CHUNK_SIZE] {
            app.world_mut().spawn((
                TilemapChunkTileData(vec![None; CHUNK_SIZE * CHUNK_SIZE]),
                TerrainChunkRegion {
                    origin_x: cx,
                    origin_y: 0,
                    chunk_w: CHUNK_SIZE,
                    chunk_h: CHUNK_SIZE,
                },
                TerrainChunk,
            ));
        }

        app.world_mut()
            .run_system_once(|mut w: MessageWriter<TerrainDirtyMsg>| {
                w.write(TerrainDirtyMsg { tile: None });
            })
            .unwrap();

        app.world_mut()
            .run_system_once(sync_terrain_tilemap)
            .unwrap();

        let mut q = app.world_mut().query::<&TilemapChunkTileData>();
        for td in q.iter(app.world()) {
            assert!(
                td.0.iter().all(|t| t.is_some()),
                "full rebuild should populate all tile slots"
            );
        }
    }

    fn setup_click_select_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.insert_resource(ButtonInput::<MouseButton>::default());
        app.insert_resource(crate::resources::SquadState::default());
        app.insert_resource(crate::resources::BuildMenuContext::default());
        app.insert_resource(EntityMap::default());
        app.insert_resource(SelectedNpc::default());
        app.insert_resource(SelectedBuilding::default());
        app.insert_resource(crate::resources::UiState::default());
        app.insert_resource(WorldData { towns: Vec::new() });
        app.insert_resource(WorldGrid::default());
        app.insert_resource(crate::resources::GpuReadState::default());
        app.insert_resource(crate::resources::PathRequestQueue::default());
        app.init_resource::<EguiUserTextures>();
        app.add_message::<SelectFactionMsg>();
        app.add_systems(Update, click_to_select_system);

        let window = Window {
            resolution: (800, 600).into(),
            ..Default::default()
        };
        app.world_mut().spawn(window);
        app.world_mut().spawn((
            MainCamera,
            Transform::default(),
            Projection::Orthographic(OrthographicProjection::default_2d()),
        ));
        app
    }

    fn setup_box_select_app() -> (App, Entity, Entity, Entity, Entity, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.insert_resource(ButtonInput::<MouseButton>::default());
        app.insert_resource(crate::resources::SquadState::default());
        app.insert_resource(crate::resources::BuildMenuContext::default());
        app.insert_resource(EntityMap::default());
        app.insert_resource(SelectedNpc(77));
        app.insert_resource(SelectedBuilding {
            active: true,
            slot: Some(99),
            ..Default::default()
        });
        app.insert_resource(crate::resources::GpuReadState {
            positions: vec![
                0.0, 0.0, // selected player archer
                5.0, 5.0, // player farmer inside box (excluded)
                0.0, 0.0, // enemy archer inside box (excluded)
                50.0, 50.0, // player archer outside box
                100.0, 100.0, // previously selected direct-control archer
            ],
            npc_count: 5,
            ..Default::default()
        });
        app.init_resource::<EguiUserTextures>();
        app.add_systems(Update, box_select_system);

        let window = Window {
            resolution: (800, 600).into(),
            ..Default::default()
        };
        app.world_mut().spawn(window);
        app.world_mut().spawn((
            MainCamera,
            Transform::default(),
            Projection::Orthographic(OrthographicProjection::default_2d()),
        ));

        let selected_archer = spawn_test_npc(
            &mut app,
            0,
            Job::Archer,
            crate::constants::FACTION_PLAYER,
            false,
        );
        let player_farmer = spawn_test_npc(
            &mut app,
            1,
            Job::Farmer,
            crate::constants::FACTION_PLAYER,
            false,
        );
        let enemy_archer = spawn_test_npc(&mut app, 2, Job::Archer, 2, false);
        let outside_archer = spawn_test_npc(
            &mut app,
            3,
            Job::Archer,
            crate::constants::FACTION_PLAYER,
            false,
        );
        let old_dc_archer = spawn_test_npc(
            &mut app,
            4,
            Job::Archer,
            crate::constants::FACTION_PLAYER,
            true,
        );

        {
            let mut squad_state = app
                .world_mut()
                .resource_mut::<crate::resources::SquadState>();
            squad_state.squads[0].members = vec![old_dc_archer];
            squad_state.selected = 0;
        }

        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();

        (
            app,
            selected_archer,
            player_farmer,
            enemy_archer,
            outside_archer,
            old_dc_archer,
        )
    }

    fn spawn_test_npc(
        app: &mut App,
        slot: usize,
        job: Job,
        faction: i32,
        direct_control: bool,
    ) -> Entity {
        let entity = app
            .world_mut()
            .spawn((
                GpuSlot(slot),
                job,
                Faction(faction),
                NpcFlags {
                    direct_control,
                    ..Default::default()
                },
            ))
            .id();
        app.world_mut()
            .resource_mut::<EntityMap>()
            .register_npc(slot, entity, job, faction, 0);
        entity
    }

    fn mark_test_npc_dead(app: &mut App, slot: usize) {
        let mut entity_map = app.world_mut().resource_mut::<EntityMap>();
        entity_map
            .get_npc_mut(slot)
            .expect("test npc should exist")
            .dead = true;
    }

    fn add_test_building(
        app: &mut App,
        slot: usize,
        kind: BuildingKind,
        position: Vec2,
        faction: i32,
    ) {
        app.world_mut().resource_mut::<EntityMap>().add_instance(
            crate::resources::BuildingInstance {
                kind,
                position,
                town_idx: 0,
                slot,
                faction,
            },
        );
    }

    fn set_gpu_positions(app: &mut App, slot_count: usize, entries: &[(usize, Vec2)]) {
        let mut positions = vec![-10000.0; slot_count * 2];
        for &(slot, pos) in entries {
            positions[slot * 2] = pos.x;
            positions[slot * 2 + 1] = pos.y;
        }
        *app.world_mut()
            .resource_mut::<crate::resources::GpuReadState>() = crate::resources::GpuReadState {
            positions,
            npc_count: slot_count,
            ..Default::default()
        };
    }

    fn screen_pos_for_world(world_pos: Vec2) -> Vec2 {
        Vec2::new(400.0 + world_pos.x, 300.0 - world_pos.y)
    }

    fn set_cursor_position(app: &mut App, cursor: Vec2) {
        let mut windows = app.world_mut().query::<&mut Window>();
        let mut window = windows.single_mut(app.world_mut()).expect("single window");
        window.set_cursor_position(Some(cursor));
    }

    #[test]
    fn box_select_uses_gpu_positions_for_player_military_only() {
        let (mut app, selected_archer, player_farmer, enemy_archer, outside_archer, old_dc_archer) =
            setup_box_select_app();

        set_cursor_position(&mut app, Vec2::new(390.0, 310.0));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        set_cursor_position(&mut app, Vec2::new(410.0, 290.0));
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();

        let squad_state = app.world().resource::<crate::resources::SquadState>();
        assert_eq!(squad_state.selected, 0);
        assert_eq!(
            squad_state.squads[0].members,
            vec![selected_archer],
            "box select should replace the selected squad with in-box player military NPCs"
        );
        assert!(
            app.world()
                .get::<NpcFlags>(selected_archer)
                .is_some_and(|flags| flags.direct_control),
            "selected player military NPC should gain direct control"
        );
        assert!(
            app.world()
                .get::<NpcFlags>(player_farmer)
                .is_some_and(|flags| !flags.direct_control),
            "player civilians inside the box should be excluded"
        );
        assert!(
            app.world()
                .get::<NpcFlags>(enemy_archer)
                .is_some_and(|flags| !flags.direct_control),
            "enemy NPCs inside the box should be excluded"
        );
        assert!(
            app.world()
                .get::<NpcFlags>(outside_archer)
                .is_some_and(|flags| !flags.direct_control),
            "player military NPCs outside the box should be excluded"
        );
        assert!(
            app.world()
                .get::<NpcFlags>(old_dc_archer)
                .is_some_and(|flags| !flags.direct_control),
            "replaced direct-control members should be cleared"
        );
        assert_eq!(app.world().resource::<SelectedNpc>().0, -1);
        assert!(
            !app.world().resource::<SelectedBuilding>().active,
            "box select should clear building selection"
        );
        assert_eq!(
            app.world().get::<SquadId>(selected_archer).map(|id| id.0),
            Some(0),
            "selected NPC should be assigned to the active squad"
        );
        assert!(
            app.world().get::<SquadId>(player_farmer).is_none(),
            "excluded civilians should not receive SquadId"
        );
        assert!(
            app.world().get::<SquadId>(enemy_archer).is_none(),
            "excluded enemies should not receive SquadId"
        );
    }

    #[test]
    fn click_select_ignores_padded_readback_slots_for_live_npcs() {
        let mut app = setup_click_select_app();
        spawn_test_npc(
            &mut app,
            600,
            Job::Archer,
            crate::constants::FACTION_PLAYER,
            false,
        );
        spawn_test_npc(
            &mut app,
            601,
            Job::Archer,
            crate::constants::FACTION_PLAYER,
            false,
        );
        mark_test_npc_dead(&mut app, 601);
        add_test_building(
            &mut app,
            100,
            BuildingKind::Farm,
            Vec2::new(20.0, 0.0),
            crate::constants::FACTION_PLAYER,
        );
        set_gpu_positions(
            &mut app,
            1024,
            &[
                (3, Vec2::new(12.0, 0.0)),
                (600, Vec2::new(10.0, 0.0)),
                (601, Vec2::new(11.0, 0.0)),
            ],
        );

        set_cursor_position(&mut app, screen_pos_for_world(Vec2::new(12.0, 0.0)));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();

        assert_eq!(
            app.world().resource::<SelectedNpc>().0,
            600,
            "left-click should resolve against live NPCs, not dead or padded slots"
        );
        let selected_building = app.world().resource::<SelectedBuilding>();
        assert!(selected_building.active);
        assert_eq!(selected_building.slot, Some(100));
        assert_eq!(selected_building.kind, Some(BuildingKind::Farm));
        assert!(
            app.world()
                .resource::<crate::resources::UiState>()
                .inspector_prefer_npc,
            "closer NPC should keep the inspector focused on NPC details"
        );
    }

    #[test]
    fn right_click_direct_control_targets_live_enemy_with_padded_readback() {
        let mut app = setup_click_select_app();
        let player = spawn_test_npc(
            &mut app,
            10,
            Job::Archer,
            crate::constants::FACTION_PLAYER,
            true,
        );
        spawn_test_npc(&mut app, 700, Job::Raider, 2, false);
        spawn_test_npc(&mut app, 701, Job::Raider, 2, false);
        mark_test_npc_dead(&mut app, 701);
        {
            let mut squad_state = app
                .world_mut()
                .resource_mut::<crate::resources::SquadState>();
            squad_state.selected = 0;
            squad_state.squads[0].members = vec![player];
        }
        let live_enemy_pos = Vec2::new(30.0, 0.0);
        set_gpu_positions(
            &mut app,
            1024,
            &[
                (5, live_enemy_pos),
                (10, Vec2::ZERO),
                (700, live_enemy_pos),
                (701, Vec2::new(29.0, 0.0)),
            ],
        );

        set_cursor_position(&mut app, screen_pos_for_world(live_enemy_pos));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.update();

        assert!(
            matches!(
                app.world().get::<ManualTarget>(player),
                Some(ManualTarget::Npc(slot)) if *slot == 700
            ),
            "direct-control right-click should target the live enemy slot"
        );
        let intents: Vec<_> = app
            .world_mut()
            .resource_mut::<crate::resources::PathRequestQueue>()
            .drain_intents()
            .collect();
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].0, player);
        assert_eq!(intents[0].1.target, live_enemy_pos);
        assert_eq!(
            intents[0].1.priority,
            crate::resources::MovementPriority::DirectControl
        );
        assert_eq!(intents[0].1.source, "dc:attack");
    }

    // ── sync_terrain_tilemap signal tests ─────────────────────────────────

    #[derive(Resource, Default)]
    struct SendTerrainDirty(bool);

    fn maybe_send_terrain_dirty(
        mut writer: MessageWriter<TerrainDirtyMsg>,
        mut flag: ResMut<SendTerrainDirty>,
    ) {
        if flag.0 {
            writer.write(TerrainDirtyMsg { tile: None });
            flag.0 = false;
        }
    }

    fn setup_terrain_sync_app() -> App {
        use crate::world::{Biome, WorldCell};
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_message::<TerrainDirtyMsg>();
        app.insert_resource(SendTerrainDirty(false));
        // 4x4 grid with distinct biomes
        let mut grid = WorldGrid::default();
        grid.width = 4;
        grid.height = 4;
        grid.cell_size = 64.0;
        grid.cells = vec![
            WorldCell {
                terrain: Biome::Forest,
                original_terrain: Biome::Forest,
            };
            16
        ];
        // Set cell (0,0) to Water (tileset_index = 8)
        grid.cells[0] = WorldCell {
            terrain: Biome::Water,
            original_terrain: Biome::Water,
        };
        app.insert_resource(grid);
        app.add_systems(
            FixedUpdate,
            (maybe_send_terrain_dirty, sync_terrain_tilemap).chain(),
        );
        app.update();
        app.update();
        app
    }

    #[test]
    fn sync_terrain_noop_without_message() {
        let mut app = setup_terrain_sync_app();
        // Spawn a chunk with tile_data pre-filled with tileset index 99 (sentinel)
        let sentinel_tile = TileData::from_tileset_index(99);
        let tile_data = vec![Some(sentinel_tile); 4 * 4];
        app.world_mut().spawn((
            TerrainChunk,
            TilemapChunkTileData(tile_data),
            TerrainChunkRegion {
                origin_x: 0,
                origin_y: 0,
                chunk_w: 4,
                chunk_h: 4,
            },
        ));
        // Run WITHOUT sending TerrainDirtyMsg
        app.update();
        let mut query = app.world_mut().query::<&TilemapChunkTileData>();
        let tile_data = query.single(app.world()).unwrap().0.clone();
        assert_eq!(
            tile_data[0].as_ref().map(|t| t.tileset_index),
            Some(99),
            "tile_data should be unchanged when no TerrainDirtyMsg is sent"
        );
    }

    #[test]
    fn sync_terrain_updates_tiles_with_message() {
        use crate::world::Biome;
        let mut app = setup_terrain_sync_app();
        // Spawn a chunk with tile_data pre-filled with sentinel index 99
        let sentinel_tile = TileData::from_tileset_index(99);
        let tile_data = vec![Some(sentinel_tile); 4 * 4];
        app.world_mut().spawn((
            TerrainChunk,
            TilemapChunkTileData(tile_data),
            TerrainChunkRegion {
                origin_x: 0,
                origin_y: 0,
                chunk_w: 4,
                chunk_h: 4,
            },
        ));
        // Send TerrainDirtyMsg and run
        app.insert_resource(SendTerrainDirty(true));
        app.update();
        let mut query = app.world_mut().query::<&TilemapChunkTileData>();
        let tile_data = query.single(app.world()).unwrap().0.clone();
        // Cell (0,0) = Water => tileset_index should be 8 (not 99 sentinel)
        let actual = tile_data[0].as_ref().map(|t| t.tileset_index);
        assert_ne!(
            actual,
            Some(99),
            "tile_data should be updated after TerrainDirtyMsg"
        );
        assert_eq!(
            actual,
            Some(Biome::Water.tileset_index(0)),
            "cell (0,0) should have Water tileset index"
        );
    }

    #[test]
    fn sync_terrain_noop_on_subsequent_frame_without_new_message() {
        let mut app = setup_terrain_sync_app();
        let sentinel_tile = TileData::from_tileset_index(99);
        let tile_data = vec![Some(sentinel_tile); 4 * 4];
        app.world_mut().spawn((
            TerrainChunk,
            TilemapChunkTileData(tile_data),
            TerrainChunkRegion {
                origin_x: 0,
                origin_y: 0,
                chunk_w: 4,
                chunk_h: 4,
            },
        ));
        // First run WITH message
        app.insert_resource(SendTerrainDirty(true));
        app.update();
        // Manually reset tile_data back to sentinel to detect re-run
        {
            let mut query = app.world_mut().query::<&mut TilemapChunkTileData>();
            let mut tile_data = query.single_mut(app.world_mut()).unwrap();
            for t in tile_data.0.iter_mut() {
                *t = Some(TileData::from_tileset_index(99));
            }
        }
        // Second run WITHOUT message -- tile_data should remain sentinel
        app.update();
        let mut query = app.world_mut().query::<&TilemapChunkTileData>();
        let tile_data = query.single(app.world()).unwrap().0.clone();
        assert_eq!(
            tile_data[0].as_ref().map(|t| t.tileset_index),
            Some(99),
            "sync_terrain_tilemap should be no-op on subsequent frame without TerrainDirtyMsg"
        );
    }
}
