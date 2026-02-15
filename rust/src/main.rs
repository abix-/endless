//! Endless - Colony sim with Bevy ECS and GPU compute.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bevy::prelude::*;

fn main() {
    let mut app = App::new();

    // Release: embed assets in binary, fallback to disk for modding
    #[cfg(not(debug_assertions))]
    app.add_plugins(bevy_embedded_assets::EmbeddedAssetPlugin {
        mode: bevy_embedded_assets::PluginMode::ReplaceAndFallback {
            path: "assets".to_string(),
        },
    });

    app.add_plugins(DefaultPlugins
        .set(WindowPlugin {
            primary_window: Some(Window {
                title: "Endless".into(),
                resolution: (1280, 720).into(),
                ..default()
            }),
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
