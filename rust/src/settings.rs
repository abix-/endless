//! User settings persistence — save/load config to JSON file.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Persisted user settings. Saved to `endless_settings.json` next to executable.
#[derive(Resource, Serialize, Deserialize, Clone)]
pub struct UserSettings {
    // World gen (main menu sliders)
    pub world_size: f32,
    pub towns: usize,
    pub farmers: usize,
    pub guards: usize,
    pub raiders: usize,
    // Camera
    pub scroll_speed: f32,
    // Combat log filters
    #[serde(default = "default_true")]
    pub log_kills: bool,
    #[serde(default = "default_true")]
    pub log_spawns: bool,
    #[serde(default = "default_true")]
    pub log_raids: bool,
    #[serde(default = "default_true")]
    pub log_harvests: bool,
    #[serde(default = "default_true")]
    pub log_levelups: bool,
}

fn default_true() -> bool { true }

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            world_size: 8000.0,
            towns: 2,
            farmers: 2,
            guards: 2,
            raiders: 0,
            scroll_speed: 400.0,
            log_kills: true,
            log_spawns: true,
            log_raids: true,
            log_harvests: true,
            log_levelups: true,
        }
    }
}

fn settings_path() -> PathBuf {
    // Save next to executable for simplicity
    let mut path = std::env::current_exe().unwrap_or_default();
    path.set_file_name("endless_settings.json");
    path
}

pub fn save_settings(settings: &UserSettings) {
    let path = settings_path();
    match serde_json::to_string_pretty(settings) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!("Failed to save settings to {}: {}", path.display(), e);
            }
        }
        Err(e) => warn!("Failed to serialize settings: {}", e),
    }
}

pub fn load_settings() -> UserSettings {
    let path = settings_path();
    match std::fs::read_to_string(&path) {
        Ok(json) => match serde_json::from_str(&json) {
            Ok(settings) => settings,
            Err(e) => {
                warn!("Failed to parse settings from {}: {}", path.display(), e);
                UserSettings::default()
            }
        },
        Err(_) => UserSettings::default(), // File doesn't exist yet, no warning needed
    }
}
