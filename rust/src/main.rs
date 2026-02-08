//! Endless - Colony sim with Bevy ECS and GPU compute.

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
            file_path: "../assets".to_string(),
            ..default()
        })
    );

    // Wire up ECS systems
    endless::build_app(&mut app);

    app.run();
}
