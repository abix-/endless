//! Endless - Colony sim with Bevy ECS and GPU compute.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bevy::prelude::*;

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
        if clipboard_ok {
            dialog.push_str("Crash details copied to clipboard.\n");
        }
        if file_ok {
            dialog.push_str(&format!("Also saved to: {}\n", log_path.display()));
        }
        dialog.push_str("\n(Ctrl+C copies this dialog text too)");

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

fn main() {
    install_crash_handler();

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
