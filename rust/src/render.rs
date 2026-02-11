//! Render Module - Bevy 2D sprite rendering for NPCs and world.
//!
//! Replaces Godot MultiMesh with bevy_sprite TextureAtlas.

use bevy::prelude::*;
use bevy::input::mouse::AccumulatedMouseScroll;

use bevy::sprite_render::{AlphaMode2d, TilemapChunk, TileData, TilemapChunkTileData};

use crate::gpu::NpcSpriteTexture;
use crate::resources::{SelectedNpc, SelectedBuilding};
use crate::settings::UserSettings;
use crate::world::{WorldGrid, build_tileset, TERRAIN_TILES, BUILDING_TILES};

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
                camera_zoom_system,
                camera_follow_system,
                click_to_select_system,
                spawn_world_tilemap,
                sync_building_tilemap,
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
    assets.char_texture = asset_server.load("assets/roguelikeChar_transparent.png");

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
    assets.world_texture = asset_server.load("assets/roguelikeSheet_transparent.png");
    npc_sprite_tex.world_handle = Some(assets.world_texture.clone());

    // Load heal halo sprite (single 16x16 texture)
    npc_sprite_tex.heal_handle = Some(asset_server.load("assets/heal.png"));

    // Load sleep icon sprite (single 16x16 texture)
    npc_sprite_tex.sleep_handle = Some(asset_server.load("assets/sleep.png"));

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

/// Left click to select nearest NPC within 20px.
/// Skips when egui wants the pointer (clicking UI buttons).
fn click_to_select_system(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_query: Query<(&Transform, &Projection), With<MainCamera>>,
    mut selected: ResMut<SelectedNpc>,
    mut selected_building: ResMut<SelectedBuilding>,
    mut egui_contexts: bevy_egui::EguiContexts,
    gpu_state: Res<crate::resources::GpuReadState>,
    grid: Res<WorldGrid>,
) {
    if !mouse.just_pressed(MouseButton::Left) { return; }

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

        let dx = world_pos.x - px;
        let dy = world_pos.y - py;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < best_dist {
            best_dist = dist;
            best_idx = i as i32;
        }
    }

    if best_idx >= 0 {
        // NPC found — select it, clear building selection
        selected.0 = best_idx;
        selected_building.active = false;
    } else {
        // No NPC — check for building under cursor
        selected.0 = -1;
        let (col, row) = grid.world_to_grid(world_pos);
        if let Some(cell) = grid.cell(col, row) {
            if cell.building.is_some() {
                *selected_building = SelectedBuilding { col, row, active: true };
            } else {
                selected_building.active = false;
            }
        } else {
            selected_building.active = false;
        }
    }
}

// =============================================================================
// WORLD TILEMAP (TERRAIN + BUILDINGS)
// =============================================================================

/// Spawn a TilemapChunk entity with the given tile data, z-depth, and alpha mode.
fn spawn_chunk(
    commands: &mut Commands,
    grid: &WorldGrid,
    tileset: Handle<Image>,
    tile_data: Vec<Option<TileData>>,
    z: f32,
    alpha: AlphaMode2d,
) {
    let world_w = grid.width as f32 * grid.cell_size;
    let world_h = grid.height as f32 * grid.cell_size;

    commands.spawn((
        TilemapChunk {
            chunk_size: UVec2::new(grid.width as u32, grid.height as u32),
            tile_display_size: UVec2::new(grid.cell_size as u32, grid.cell_size as u32),
            tileset,
            alpha_mode: alpha,
        },
        TilemapChunkTileData(tile_data),
        Transform::from_xyz(world_w / 2.0, world_h / 2.0, z),
    ));
}

/// Tracks whether the tilemap has been spawned. Resource (not Local) so cleanup can reset it.
#[derive(Resource, Default)]
pub struct TilemapSpawned(pub bool);

/// Marker component on the building TilemapChunk layer for runtime tile updates.
#[derive(Component)]
pub struct BuildingChunk;

/// Spawn terrain + building TilemapChunk layers. Runs once when WorldGrid is populated
/// and the world atlas image is loaded.
fn spawn_world_tilemap(
    mut commands: Commands,
    grid: Res<WorldGrid>,
    assets: Res<SpriteAssets>,
    mut images: ResMut<Assets<Image>>,
    mut spawned: ResMut<TilemapSpawned>,
) {
    if spawned.0 || grid.width == 0 { return; }
    let Some(atlas) = images.get(&assets.world_texture).cloned() else { return; };

    // Terrain layer: every cell filled, opaque
    let terrain_tileset = build_tileset(&atlas, &TERRAIN_TILES, &mut images);
    let terrain_tiles: Vec<Option<TileData>> = grid.cells.iter().enumerate()
        .map(|(i, cell)| Some(TileData::from_tileset_index(cell.terrain.tileset_index(i))))
        .collect();
    spawn_chunk(&mut commands, &grid, terrain_tileset, terrain_tiles, -1.0, AlphaMode2d::Blend);

    // Building layer: None for empty cells, building tile where placed.
    // Spawned with BuildingChunk marker for runtime tile updates.
    let building_tileset = build_tileset(&atlas, &BUILDING_TILES, &mut images);
    let building_tiles: Vec<Option<TileData>> = grid.cells.iter()
        .map(|cell| cell.building.as_ref().map(|b| TileData::from_tileset_index(b.tileset_index())))
        .collect();
    let building_count = building_tiles.iter().filter(|t| t.is_some()).count();
    let world_w = grid.width as f32 * grid.cell_size;
    let world_h = grid.height as f32 * grid.cell_size;
    commands.spawn((
        TilemapChunk {
            chunk_size: UVec2::new(grid.width as u32, grid.height as u32),
            tile_display_size: UVec2::new(grid.cell_size as u32, grid.cell_size as u32),
            tileset: building_tileset,
            alpha_mode: AlphaMode2d::Blend,
        },
        TilemapChunkTileData(building_tiles),
        Transform::from_xyz(world_w / 2.0, world_h / 2.0, -0.5),
        BuildingChunk,
    ));

    info!("World tilemap spawned: {}x{} grid, {} buildings", grid.width, grid.height, building_count);
    spawned.0 = true;
}

/// Sync building tilemap tiles when WorldGrid changes at runtime (building placed/destroyed).
fn sync_building_tilemap(
    grid: Res<WorldGrid>,
    mut chunks: Query<&mut TilemapChunkTileData, With<BuildingChunk>>,
) {
    if !grid.is_changed() || grid.width == 0 { return; }

    for mut tile_data in chunks.iter_mut() {
        // Rebuild tile data from current grid cells
        for (i, cell) in grid.cells.iter().enumerate() {
            if i >= tile_data.0.len() { break; }
            tile_data.0[i] = cell.building.as_ref()
                .map(|b| TileData::from_tileset_index(b.tileset_index()));
        }
    }
}
