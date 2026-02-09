//! Render Module - Bevy 2D sprite rendering for NPCs and world.
//!
//! Replaces Godot MultiMesh with bevy_sprite TextureAtlas.

use bevy::prelude::*;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};

use crate::gpu::NpcSpriteTexture;
use crate::resources::SelectedNpc;

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

/// Camera state — extracted to render world for shader upload.
#[derive(Resource, Clone, ExtractResource)]
pub struct CameraState {
    pub position: Vec2,
    pub zoom: f32,
    pub viewport: Vec2,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            position: Vec2::new(400.0, 300.0),
            zoom: 1.0,
            viewport: Vec2::new(1280.0, 720.0),
        }
    }
}

const CAMERA_PAN_SPEED: f32 = 400.0;
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
            .init_resource::<CameraState>()
            .add_plugins(ExtractResourcePlugin::<CameraState>::default())
            .add_systems(Startup, (setup_camera, load_sprites))
            .add_systems(Update, (
                camera_pan_system,
                camera_zoom_system,
                camera_viewport_sync,
                camera_transform_sync,
                click_to_select_system,
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

    // Share texture handle with GPU module for instanced rendering
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

/// WASD camera pan. Speed scales inversely with zoom for consistent screen-space feel.
fn camera_pan_system(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut camera_state: ResMut<CameraState>,
) {
    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) { dir.y += 1.0; }
    if keys.pressed(KeyCode::KeyS) { dir.y -= 1.0; }
    if keys.pressed(KeyCode::KeyA) { dir.x -= 1.0; }
    if keys.pressed(KeyCode::KeyD) { dir.x += 1.0; }

    if dir != Vec2::ZERO {
        let speed = CAMERA_PAN_SPEED / camera_state.zoom;
        camera_state.position += dir.normalize() * speed * time.delta_secs();
    }
}

/// Scroll wheel zoom toward mouse cursor (same math as Godot player.gd).
fn camera_zoom_system(
    accumulated_scroll: Res<AccumulatedMouseScroll>,
    windows: Query<&Window>,
    mut camera_state: ResMut<CameraState>,
) {
    let scroll = accumulated_scroll.delta.y;
    if scroll == 0.0 { return; }

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };

    let viewport = Vec2::new(window.width(), window.height());
    let screen_center = viewport / 2.0;

    // Mouse offset from screen center (flip Y: screen Y-down → world Y-up)
    let mouse_offset = Vec2::new(
        cursor_pos.x - screen_center.x,
        screen_center.y - cursor_pos.y,
    );

    // World position under mouse before zoom
    let world_pos = camera_state.position + mouse_offset / camera_state.zoom;

    // Apply zoom
    let factor = if scroll > 0.0 { 1.0 + CAMERA_ZOOM_SPEED } else { 1.0 - CAMERA_ZOOM_SPEED };
    camera_state.zoom = (camera_state.zoom * factor).clamp(CAMERA_MIN_ZOOM, CAMERA_MAX_ZOOM);

    // Move camera so world_pos stays under mouse
    camera_state.position = world_pos - mouse_offset / camera_state.zoom;
}

/// Keep viewport size in sync with window.
fn camera_viewport_sync(
    windows: Query<&Window>,
    mut camera_state: ResMut<CameraState>,
) {
    if let Ok(window) = windows.single() {
        camera_state.viewport = Vec2::new(window.width(), window.height());
    }
}

/// Sync CameraState to Bevy Camera2d Transform (for coordinate queries).
fn camera_transform_sync(
    camera_state: Res<CameraState>,
    mut query: Query<&mut Transform, With<MainCamera>>,
) {
    let Ok(mut transform) = query.single_mut() else { return };
    transform.translation.x = camera_state.position.x;
    transform.translation.y = camera_state.position.y;
}

/// Left click to select nearest NPC within 20px.
fn click_to_select_system(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_state: Res<CameraState>,
    mut selected: ResMut<SelectedNpc>,
) {
    if !mouse.just_pressed(MouseButton::Left) { return; }

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };

    let viewport = Vec2::new(window.width(), window.height());
    let screen_center = viewport / 2.0;
    let mouse_offset = Vec2::new(
        cursor_pos.x - screen_center.x,
        screen_center.y - cursor_pos.y,
    );
    let world_pos = camera_state.position + mouse_offset / camera_state.zoom;

    // Find nearest NPC within 20px radius using GPU readback positions
    let positions = crate::messages::GPU_READ_STATE
        .lock()
        .ok()
        .map(|s| s.positions.clone())
        .unwrap_or_default();

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

    selected.0 = best_idx;
}

