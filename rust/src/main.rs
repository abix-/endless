//! Endless - Colony sim with Bevy ECS and GPU compute.

use bevy::prelude::*;

fn main() {
    let mut app = App::new();

    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Endless".into(),
            resolution: (1280, 720).into(),
            ..default()
        }),
        ..default()
    }));

    // Wire up ECS systems
    endless::build_app(&mut app);

    app.run();
}
