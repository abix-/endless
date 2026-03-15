//! Endless - Colony sim with Bevy ECS and GPU compute.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::collapsible_if
)]

use bevy::{app::PluginGroupBuilder, prelude::*};

const SURFACE_CRASH_MARKERS: &[&str] = &[
    "Surface is not configured for presentation",
    "Invalid surface",
];

#[cfg(target_os = "windows")]
fn crash_log_path_near_exe() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("crash.log")))
        .unwrap_or_else(|| std::path::PathBuf::from("crash.log"))
}

/// If the previous run crashed with the known DX12/wgpu surface-presentation issue,
/// apply a safer startup profile so users can recover without editing files manually.
#[allow(unused_variables)]
fn apply_surface_crash_recovery(settings: &mut endless::settings::UserSettings) {
    #[cfg(target_os = "windows")]
    {
        // Only intervene when backend is Auto or DX12.
        if settings.gpu_backend != 0 && settings.gpu_backend != 2 {
            return;
        }

        let crash_log = crash_log_path_near_exe();
        let Ok(content) = std::fs::read_to_string(&crash_log) else {
            return;
        };
        if !SURFACE_CRASH_MARKERS.iter().any(|m| content.contains(m)) {
            return;
        }

        settings.gpu_backend = 1; // Vulkan
        settings.fullscreen = false;
        settings.window_maximized = true;
        endless::settings::save_settings(settings);

        // Rename the consumed crash log so we don't keep forcing fallback forever.
        let recovered = crash_log.with_file_name("crash.recovered.log");
        let _ = std::fs::rename(&crash_log, recovered);

        eprintln!(
            "Detected prior surface presentation crash. Auto-switched backend to Vulkan and windowed mode."
        );
    }
}

/// Install a panic hook that shows a native crash dialog, copies details to
/// clipboard, and writes a crash.log file. Must be called before anything else.
fn install_crash_handler() {
    std::panic::set_hook(Box::new(|info| {
        // Capture backtrace before doing anything else
        let backtrace = std::backtrace::Backtrace::force_capture();

        // Build crash message
        let message = match info.payload().downcast_ref::<&str>() {
            Some(s) => s.to_string(),
            None => match info.payload().downcast_ref::<String>() {
                Some(s) => s.clone(),
                None => "unknown panic".to_string(),
            },
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let report = format!(
            "ENDLESS CRASH REPORT\n\
             ====================\n\
             Version: {}\n\
             Time: {:?}\n\n\
             Panic: {}\n\
             Location: {}\n\n\
             Backtrace:\n{}",
            env!("CARGO_PKG_VERSION"),
            std::time::SystemTime::now(),
            message,
            location,
            backtrace,
        );

        // Try to copy to clipboard
        let clipboard_ok = arboard::Clipboard::new()
            .and_then(|mut cb| cb.set_text(&report))
            .is_ok();

        // Try to write crash.log next to executable
        let log_path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("crash.log")))
            .unwrap_or_else(|| std::path::PathBuf::from("crash.log"));
        let file_ok = std::fs::write(&log_path, &report).is_ok();

        // Build dialog text
        let mut dialog = format!(
            "Endless has crashed!\n\n\
             {message}\n\
             at {location}\n\n"
        );
        if SURFACE_CRASH_MARKERS.iter().any(|m| message.contains(m)) {
            dialog.push_str(
                "Likely graphics-backend instability.\n\
                 Relaunch and use Graphics Backend = Vulkan if needed.\n\n",
            );
        }
        if clipboard_ok {
            dialog.push_str("Crash details copied to clipboard.\n");
        }
        if file_ok {
            dialog.push_str(&format!("Also saved to: {}\n", log_path.display()));
        }
        dialog.push_str("\n(Ctrl+C copies this dialog text too)");

        // Always print to stderr so terminal users see the error
        eprintln!("\n{report}");

        // Show native Windows message box
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::ffi::OsStrExt;
            let title: Vec<u16> = std::ffi::OsStr::new("Endless — Crash")
                .encode_wide()
                .chain(Some(0))
                .collect();
            let body: Vec<u16> = std::ffi::OsStr::new(&dialog)
                .encode_wide()
                .chain(Some(0))
                .collect();
            const MB_OK: u32 = 0x0000;
            const MB_ICONERROR: u32 = 0x0010;
            unsafe extern "system" {
                fn MessageBoxW(
                    hwnd: *mut std::ffi::c_void,
                    text: *const u16,
                    caption: *const u16,
                    utype: u32,
                ) -> i32;
            }
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    body.as_ptr(),
                    title.as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }

        // Fallback for non-Windows: print to stderr
        #[cfg(not(target_os = "windows"))]
        {
            eprintln!("{report}");
        }
    }));
}

fn default_engine_plugins(initial_window: Window) -> PluginGroupBuilder {
    DefaultPlugins
        // Endless is a 2D renderer. Disabling Bevy's PBR plugin avoids unused
        // 3D SSR setup that has been crashing the visible frame.
        .set(WindowPlugin {
            primary_window: Some(initial_window),
            ..default()
        })
        .set(bevy::log::LogPlugin {
            custom_layer: |_app: &mut App| {
                Some(Box::new(endless::tracing_layer::SystemTimingLayer))
            },
            ..default()
        })
        .disable::<bevy::pbr::PbrPlugin>()
}

fn add_engine_plugins(app: &mut App, initial_window: Window) {
    app.add_plugins(default_engine_plugins(initial_window));
}

fn main() {
    install_crash_handler();

    // Apply GPU backend preference before wgpu initializes.
    // Safety: called in main() before any threads are spawned.
    let mut saved_settings = endless::settings::load_settings();
    apply_surface_crash_recovery(&mut saved_settings);
    match saved_settings.gpu_backend {
        1 => unsafe { std::env::set_var("WGPU_BACKEND", "vulkan") },
        2 => unsafe { std::env::set_var("WGPU_BACKEND", "dx12") },
        _ => {} // 0 = Auto, let wgpu decide
    }

    let mut app = App::new();

    // Release: embed assets in binary, fallback to disk for modding
    #[cfg(not(debug_assertions))]
    app.add_plugins(bevy_embedded_assets::EmbeddedAssetPlugin {
        mode: bevy_embedded_assets::PluginMode::ReplaceAndFallback {
            path: "assets".to_string(),
        },
    });

    // Build window from saved settings to avoid surface race condition.
    // Previously created at 1280×720 then mutated in Startup, causing wgpu
    // "Invalid surface" crashes during the OS window-state transition.
    let initial_window = {
        let mut w = Window {
            title: "Endless".into(),
            ..default()
        };
        endless::settings::apply_video_settings_to_window(&mut w, &saved_settings);
        w
    };

    add_engine_plugins(&mut app, initial_window);

    // Parse CLI flags
    if std::env::args().any(|a| a == "--autostart") {
        app.insert_resource(endless::resources::AutoStart(true));
    }
    if let Some(pos) = std::env::args().position(|a| a == "--test") {
        let filter = std::env::args().nth(pos + 1);
        app.insert_resource(endless::resources::CliTestMode {
            active: true,
            filter,
        });
    }

    // Wire up ECS systems
    endless::build_app(&mut app);

    // Apply saved display settings on startup
    app.add_systems(
        Startup,
        |mut windows: Query<&mut Window>,
         settings: Res<endless::settings::UserSettings>,
         mut winit_settings: ResMut<bevy::winit::WinitSettings>,
         mut framepace: ResMut<bevy_framepace::FramepaceSettings>| {
            if let Ok(mut window) = windows.single_mut() {
                endless::settings::apply_video_settings_to_window(&mut window, &settings);
            }
            winit_settings.focused_mode = bevy::winit::UpdateMode::Continuous;
            endless::settings::apply_fps_cap(settings.fps_cap, &mut framepace);
            if settings.background_fps {
                winit_settings.unfocused_mode = bevy::winit::UpdateMode::Continuous;
            }
        },
    );

    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_plugins_disable_pbr_for_2d_runtime() {
        let plugins = default_engine_plugins(Window::default());

        assert!(!plugins.enabled::<bevy::pbr::PbrPlugin>());
    }
}
