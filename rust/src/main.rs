//! Endless - Colony sim with Bevy ECS and GPU compute.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bevy::prelude::*;
use bevy::asset::AssetPlugin;

fn main() {
    let mut app = App::new();

    // Configure asset path relative to Cargo.toml (rust/)
    // Assets are at ../assets/ from the rust/ directory
    app.add_plugins(DefaultPlugins
        .set(WindowPlugin {
            primary_window: Some(Window {
                title: "Endless".into(),
                resolution: (1280, 720).into(),
                ..default()
            }),
            ..default()
        })
        .set(AssetPlugin {
            file_path: if cfg!(debug_assertions) { ".." } else { "." }.to_string(),
            ..default()
        })
    );

    // Wire up ECS systems
    endless::build_app(&mut app);

    // Maximize window + apply saved display settings on startup
    app.add_systems(Startup, |
        mut windows: Query<&mut Window>,
        settings: Res<endless::settings::UserSettings>,
        mut winit_settings: ResMut<bevy::winit::WinitSettings>,
    | {
        if let Ok(mut window) = windows.single_mut() {
            window.set_maximized(true);
        }
        if settings.background_fps {
            winit_settings.unfocused_mode = bevy::winit::UpdateMode::Continuous;
        }
    });

    app.run();
}
