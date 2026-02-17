//! Render Module - Bevy 2D sprite rendering for NPCs and world.
//!
//! Replaces Godot MultiMesh with bevy_sprite TextureAtlas.

use bevy::prelude::*;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::ecs::system::SystemParam;

use bevy::sprite_render::{AlphaMode2d, TilemapChunk, TileData, TilemapChunkTileData};

use crate::gpu::NpcSpriteTexture;
use crate::resources::{SelectedNpc, SelectedBuilding, LeftPanelTab, SystemTimings};
use crate::settings::UserSettings;
use crate::world::{WorldData, WorldGrid, BuildingKind, build_tileset, build_building_atlas, TERRAIN_TILES, BUILDING_TILES};

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
    /// External building sprites (32x32 PNGs)
    pub house_texture: Handle<Image>,
    pub barracks_texture: Handle<Image>,
    pub waypoint_texture: Handle<Image>,
    pub miner_house_texture: Handle<Image>,
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
}

const CAMERA_ZOOM_SPEED: f32 = 0.1;
const CAMERA_MIN_ZOOM: f32 = 0.1;
const CAMERA_MAX_ZOOM: f32 = 4.0;
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
            .add_systems(Update, (
                camera_pan_system,
                camera_mouse_pan_system,
                camera_edge_pan_system,
                camera_zoom_system,
                camera_follow_system,
                click_to_select_system,
                spawn_world_tilemap,
                sync_terrain_tilemap,
                sync_terrain_visibility,
            ));
    }
}

/// Set up 2D camera.
fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        MainCamera,
        Transform::from_xyz(400.0, 300.0, 0.0),
    ));
    info!("2D camera spawned at (400, 300)");
}

/// Load sprite sheets.
fn load_sprites(
    mut assets: ResMut<SpriteAssets>,
    mut npc_sprite_tex: ResMut<NpcSpriteTexture>,
    asset_server: Res<AssetServer>,
    mut texture_atlases: ResMut<Assets<TextureAtlasLayout>>,
) {
    // Load character sprite sheet
    assets.char_texture = asset_server.load("sprites/roguelikeChar_transparent.png");

    // Share texture handles with instanced renderer
    npc_sprite_tex.handle = Some(assets.char_texture.clone());

    // Create atlas layout for characters (16x16 with 1px padding)
    let char_layout = TextureAtlasLayout::from_grid(
        UVec2::new(CHAR_SPRITE_SIZE as u32, CHAR_SPRITE_SIZE as u32),
        CHAR_SHEET_COLS,
        CHAR_SHEET_ROWS,
        Some(UVec2::new(1, 1)), // 1px padding
        Some(UVec2::new(0, 0)), // no offset
    );
    assets.char_atlas = texture_atlases.add(char_layout);

    // Load world sprite sheet
    assets.world_texture = asset_server.load("sprites/roguelikeSheet_transparent.png");
    assets.house_texture = asset_server.load("sprites/house.png");
    assets.barracks_texture = asset_server.load("sprites/barracks.png");
    assets.waypoint_texture = asset_server.load("sprites/waypoint.png");
    assets.miner_house_texture = asset_server.load("sprites/miner_house.png");
    npc_sprite_tex.world_handle = Some(assets.world_texture.clone());

    // Load heal halo sprite (single 16x16 texture)
    npc_sprite_tex.heal_handle = Some(asset_server.load("sprites/heal.png"));

    // Load sleep icon sprite (single 16x16 texture)
    npc_sprite_tex.sleep_handle = Some(asset_server.load("sprites/sleep.png"));

    // Load arrow projectile sprite (single texture, white, points up)
    npc_sprite_tex.arrow_handle = Some(asset_server.load("sprites/arrow.png"));

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
    info!("Sprite sheets loaded: char ({}x{}), world ({}x{})",
          CHAR_SHEET_COLS, CHAR_SHEET_ROWS, WORLD_SHEET_COLS, WORLD_SHEET_ROWS);
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

/// WASD camera pan. Speed scales inversely with zoom for consistent screen-space feel.
fn camera_pan_system(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut query: Query<(&mut Transform, &Projection), With<MainCamera>>,
    user_settings: Res<UserSettings>,
) {
    let Ok((mut transform, projection)) = query.single_mut() else { return };

    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) { dir.y += 1.0; }
    if keys.pressed(KeyCode::KeyS) { dir.y -= 1.0; }
    if keys.pressed(KeyCode::KeyA) { dir.x -= 1.0; }
    if keys.pressed(KeyCode::KeyD) { dir.x += 1.0; }

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
                let Ok((mut transform, projection)) = query.single_mut() else { return };
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
fn camera_edge_pan_system(
    windows: Query<&Window>,
    time: Res<Time>,
    mut query: Query<(&mut Transform, &Projection), With<MainCamera>>,
    user_settings: Res<UserSettings>,
) {
    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let w = window.width();
    let h = window.height();

    let mut dir = Vec2::ZERO;
    if cursor_pos.x < EDGE_PAN_MARGIN { dir.x -= 1.0; }
    if cursor_pos.x > w - EDGE_PAN_MARGIN { dir.x += 1.0; }
    if cursor_pos.y < EDGE_PAN_MARGIN { dir.y += 1.0; } // top of screen = +Y world
    if cursor_pos.y > h - EDGE_PAN_MARGIN { dir.y -= 1.0; }

    if dir != Vec2::ZERO {
        let Ok((mut transform, projection)) = query.single_mut() else { return };
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
) {
    // Don't zoom when scrolling over UI panels (combat log, etc.)
    if let Ok(ctx) = egui_contexts.ctx_mut() {
        if ctx.wants_pointer_input() || ctx.is_pointer_over_area() { return; }
    }
    let scroll = accumulated_scroll.delta.y;
    if scroll == 0.0 { return; }

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((mut transform, mut projection)) = query.single_mut() else { return };
    let Projection::Orthographic(ref mut ortho) = *projection else { return };

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
    let factor = if scroll > 0.0 { 1.0 + CAMERA_ZOOM_SPEED } else { 1.0 - CAMERA_ZOOM_SPEED };
    let new_zoom = (zoom * factor).clamp(CAMERA_MIN_ZOOM, CAMERA_MAX_ZOOM);
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
    if !follow.0 || selected.0 < 0 { return; }
    let idx = selected.0 as usize;
    let positions = &gpu_state.positions;
    if idx * 2 + 1 >= positions.len() { return; }
    let x = positions[idx * 2];
    let y = positions[idx * 2 + 1];
    if x < -9000.0 { return; } // dead/hidden
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

#[derive(SystemParam)]
struct ClickSelectParams<'w> {
    selected: ResMut<'w, SelectedNpc>,
    selected_building: ResMut<'w, SelectedBuilding>,
    squad_state: ResMut<'w, crate::resources::SquadState>,
    build_ctx: Res<'w, crate::resources::BuildMenuContext>,
    building_slots: Res<'w, crate::resources::BuildingSlotMap>,
    ui_state: ResMut<'w, crate::resources::UiState>,
    world_data: ResMut<'w, WorldData>,
    npc_entity_map: Res<'w, crate::resources::NpcEntityMap>,
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
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("click_select");
    // Right-click cancels squad target placement or mine assignment
    if mouse.just_pressed(MouseButton::Right) {
        if click.squad_state.placing_target {
            click.squad_state.placing_target = false;
            return;
        }
        if click.ui_state.assigning_mine.is_some() {
            click.ui_state.assigning_mine = None;
            return;
        }
    }

    if !mouse.just_pressed(MouseButton::Left) { return; }

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
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((transform, projection)) = camera_query.single() else { return };

    let zoom = ortho_zoom(projection);
    let position = transform.translation.truncate();
    let viewport = Vec2::new(window.width(), window.height());
    let screen_center = viewport / 2.0;
    let mouse_offset = Vec2::new(
        cursor_pos.x - screen_center.x,
        screen_center.y - cursor_pos.y,
    );
    let world_pos = position + mouse_offset / zoom;

    // Squad target placement — intercept before NPC selection
    if click.squad_state.placing_target {
        let si = click.squad_state.selected;
        if si >= 0 && (si as usize) < click.squad_state.squads.len() {
            click.squad_state.squads[si as usize].target = Some(world_pos);
        }
        click.squad_state.placing_target = false;
        return;
    }

    // Mine assignment — snap to nearest gold mine within radius
    if let Some(mh_idx) = click.ui_state.assigning_mine {
        let snap_radius = 60.0;
        let best = click.world_data.gold_mines.iter()
            .map(|m| (m.position.distance(world_pos), m.position))
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((dist, mine_pos)) = best {
            if dist < snap_radius {
                if let Some(mh) = click.world_data.miner_homes.get_mut(mh_idx) {
                    mh.manual_mine = true;
                    mh.assigned_mine = Some(mine_pos);
                }
            }
        }
        click.ui_state.assigning_mine = None;
        return;
    }

    // Find nearest NPC within 20px radius using GPU readback positions
    let positions = &gpu_state.positions;

    let select_radius = 20.0_f32;
    let mut best_dist = select_radius;
    let mut best_idx: i32 = -1;

    let npc_count = positions.len() / 2;
    for i in 0..npc_count {
        let px = positions[i * 2];
        let py = positions[i * 2 + 1];
        if px < -9000.0 { continue; }
        if click.building_slots.is_building(i) { continue; }
        if !click.npc_entity_map.0.contains_key(&i) { continue; }

        let dx = world_pos.x - px;
        let dy = world_pos.y - py;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < best_dist {
            best_dist = dist;
            best_idx = i as i32;
        }
    }

    // Double-click detection
    let now = time.elapsed_secs_f64();
    let is_double = (now - dbl_click.last_time) < 0.4
        && (world_pos - dbl_click.last_pos).length() < 5.0;
    dbl_click.last_time = now;
    dbl_click.last_pos = world_pos;

    let (col, row) = grid.world_to_grid(world_pos);
    let building = grid.cell(col, row).and_then(|c| c.building.as_ref());

    // Find nearest building via the same distance-based hit-test style as NPC selection.
    let building_select_radius = 24.0_f32;
    let mut best_building_dist = building_select_radius;
    let mut best_building: Option<(BuildingKind, usize, Vec2, Option<usize>)> = None;
    for i in 0..npc_count {
        let Some((kind, bidx)) = click.building_slots.get_building(i) else { continue };
        let px = positions[i * 2];
        let py = positions[i * 2 + 1];
        if px < -9000.0 { continue; }

        let dx = world_pos.x - px;
        let dy = world_pos.y - py;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < best_building_dist {
            best_building_dist = dist;
            best_building = Some((kind, bidx, Vec2::new(px, py), Some(i)));
        }
    }
    // Fallback to clicked cell building when available.
    if let Some(b) = building {
        let bpos = grid.grid_to_world(col, row);
        let dx = world_pos.x - bpos.x;
        let dy = world_pos.y - bpos.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < best_building_dist {
            if let Some(bidx) = crate::world::find_building_data_index(&click.world_data, *b, bpos) {
                best_building = Some((b.kind(), bidx, bpos, None));
            }
        }
    }

    // Keep up to one NPC and one building selected from the same click.
    click.selected.0 = best_idx;
    if let Some((kind, bidx, bpos, bslot)) = best_building {
        let (bcol, brow) = grid.world_to_grid(bpos);
        *click.selected_building = SelectedBuilding {
            col: bcol,
            row: brow,
            active: true,
            slot: bslot,
            kind: Some(kind),
            index: Some(bidx),
        };

        // Double-click fountain -> open Factions tab for that faction.
        if is_double {
            if let Some(crate::world::Building::Fountain { town_idx }) = building {
                if let Some(town) = click.world_data.towns.get(*town_idx as usize) {
                    click.ui_state.left_panel_open = true;
                    click.ui_state.left_panel_tab = LeftPanelTab::Factions;
                    click.ui_state.pending_faction_select = Some(town.faction);
                }
            }
        }
    } else {
        click.selected_building.active = false;
        click.selected_building.slot = None;
        click.selected_building.kind = None;
        click.selected_building.index = None;
    }

    // Default active inspector tab by click proximity.
    if best_idx >= 0 && best_building.is_some() {
        let npc_x = positions[best_idx as usize * 2];
        let npc_y = positions[best_idx as usize * 2 + 1];
        let (_, _, bpos, _) = best_building.unwrap_or((BuildingKind::Farm, 0, grid.grid_to_world(col, row), None));
        let npc_dx = world_pos.x - npc_x;
        let npc_dy = world_pos.y - npc_y;
        let bld_dx = world_pos.x - bpos.x;
        let bld_dy = world_pos.y - bpos.y;
        let npc_d2 = npc_dx * npc_dx + npc_dy * npc_dy;
        let bld_d2 = bld_dx * bld_dx + bld_dy * bld_dy;
        click.ui_state.inspector_prefer_npc = npc_d2 <= bld_d2;
    } else if best_idx >= 0 {
        click.ui_state.inspector_prefer_npc = true;
    } else if best_building.is_some() {
        click.ui_state.inspector_prefer_npc = false;
    }
    click.ui_state.inspector_click_seq = click.ui_state.inspector_click_seq.saturating_add(1);
}

// =============================================================================
// WORLD TILEMAP (TERRAIN + BUILDINGS)
// =============================================================================

/// Tracks whether the tilemap has been spawned. Resource (not Local) so cleanup can reset it.
#[derive(Resource, Default)]
pub struct TilemapSpawned(pub bool);

/// Marker component on the terrain TilemapChunk layer for runtime tile updates.
#[derive(Component)]
pub struct TerrainChunk;

/// Spawn terrain TilemapChunk + building atlas for instanced renderer.
/// Runs once when WorldGrid is populated and all images are loaded.
fn spawn_world_tilemap(
    mut commands: Commands,
    grid: Res<WorldGrid>,
    assets: Res<SpriteAssets>,
    mut images: ResMut<Assets<Image>>,
    mut spawned: ResMut<TilemapSpawned>,
    mut npc_sprite_tex: ResMut<NpcSpriteTexture>,
) {
    if spawned.0 || grid.width == 0 { return; }
    let Some(atlas) = images.get(&assets.world_texture).cloned() else { return; };
    let Some(house_img) = images.get(&assets.house_texture).cloned() else { return; };
    let Some(barracks_img) = images.get(&assets.barracks_texture).cloned() else { return; };
    let Some(waypoint_img) = images.get(&assets.waypoint_texture).cloned() else { return; };
    let Some(miner_house_img) = images.get(&assets.miner_house_texture).cloned() else { return; };

    // Terrain layer
    let terrain_tileset = build_tileset(&atlas, &TERRAIN_TILES, &[], &mut images);
    let terrain_tiles: Vec<Option<TileData>> = grid.cells.iter().enumerate()
        .map(|(i, cell)| Some(TileData::from_tileset_index(cell.terrain.tileset_index(i))))
        .collect();
    let world_w = grid.width as f32 * grid.cell_size;
    let world_h = grid.height as f32 * grid.cell_size;
    commands.spawn((
        TilemapChunk {
            chunk_size: UVec2::new(grid.width as u32, grid.height as u32),
            tile_display_size: UVec2::new(grid.cell_size as u32, grid.cell_size as u32),
            tileset: terrain_tileset,
            alpha_mode: AlphaMode2d::Blend,
        },
        TilemapChunkTileData(terrain_tiles),
        Transform::from_xyz(world_w / 2.0, world_h / 2.0, -1.0),
        TerrainChunk,
    ));

    // Building atlas for NPC instanced renderer (replaces building TilemapChunk)
    let building_atlas = build_building_atlas(
        &atlas,
        &BUILDING_TILES,
        &[&house_img, &barracks_img, &waypoint_img, &miner_house_img],
        &mut images,
    );
    if let Some(img) = images.get(&building_atlas) {
        assert_eq!(img.height(), 32 * BUILDING_TILES.len() as u32,
            "building atlas height mismatch");
    }
    npc_sprite_tex.building_handle = Some(building_atlas);

    info!("World tilemap spawned: {}x{} grid", grid.width, grid.height);
    spawned.0 = true;
}

/// Sync terrain tilemap tiles when WorldGrid terrain changes (slot unlock → Dirt).
fn sync_terrain_tilemap(
    grid: Res<WorldGrid>,
    mut chunks: Query<&mut TilemapChunkTileData, With<TerrainChunk>>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("sync_terrain");
    if !grid.is_changed() || grid.width == 0 { return; }

    for mut tile_data in chunks.iter_mut() {
        for (i, cell) in grid.cells.iter().enumerate() {
            if i >= tile_data.0.len() { break; }
            tile_data.0[i] = Some(TileData::from_tileset_index(cell.terrain.tileset_index(i)));
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

