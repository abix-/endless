//! Render Module - Bevy 2D sprite rendering for NPCs and world.
//!
//! Replaces Godot MultiMesh with bevy_sprite TextureAtlas.

use bevy::prelude::*;

use crate::gpu::NpcSpriteTexture;

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

// =============================================================================
// PLUGIN
// =============================================================================

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpriteAssets>()
            .add_systems(Startup, (setup_camera, load_sprites));
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
    assets.char_texture = asset_server.load("roguelikeChar_transparent.png");

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
    assets.world_texture = asset_server.load("roguelikeSheet_transparent.png");

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

/// Spawn test sprites to verify rendering works.
fn spawn_test_sprites(
    mut commands: Commands,
    assets: Res<SpriteAssets>,
) {
    if !assets.loaded {
        return;
    }

    // Spawn test NPCs: first 2 columns x 4 rows (assuming 54 cols per row)
    // Row 0: 0, 1 | Row 1: 54, 55 | Row 2: 108, 109 | Row 3: 162, 163
    let cols = CHAR_SHEET_COLS as usize; // 54
    let sprite_indices = [
        0, 1,           // row 0
        cols, cols + 1, // row 1
        cols * 2, cols * 2 + 1, // row 2
        cols * 3, cols * 3 + 1, // row 3
    ];
    for (i, &idx) in sprite_indices.iter().enumerate() {
        let col = i % 2;
        let row = i / 2;
        let x = 300.0 + (col as f32) * 50.0;
        let y = 350.0 - (row as f32) * 50.0; // flip Y so row 0 is at top

        commands.spawn((
            Sprite::from_atlas_image(
                assets.char_texture.clone(),
                TextureAtlas {
                    layout: assets.char_atlas.clone(),
                    index: idx,
                },
            ),
            Transform::from_xyz(x, y, 0.0).with_scale(Vec3::splat(2.0)),
        ));
    }

    info!("Spawned {} test sprites", sprite_indices.len());
}
