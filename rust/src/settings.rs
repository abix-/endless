//! User settings persistence — save/load config to JSON file.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::resources::PolicySet;

/// Persisted user settings. Saved to `Documents\Endless\settings.json`.
#[derive(Resource, Serialize, Deserialize, Clone)]
pub struct UserSettings {
    // World gen (main menu sliders)
    pub world_size: f32,
    pub towns: usize,
    #[serde(default = "default_farms")]
    pub farms: usize,
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
    #[serde(default = "default_true")]
    pub log_npc_activity: bool,
    #[serde(default = "default_true")]
    pub log_ai: bool,
    // Debug visibility (pause menu settings)
    #[serde(default)]
    pub debug_enemy_info: bool,
    #[serde(default)]
    pub debug_coordinates: bool,
    #[serde(default)]
    pub debug_all_npcs: bool,
    // Town policies
    #[serde(default)]
    pub policy: PolicySet,
    // Display
    #[serde(default)]
    pub background_fps: bool,
    // AI players
    #[serde(default = "default_one")]
    pub ai_towns: usize,
    #[serde(default = "default_one")]
    pub raider_camps: usize,
    #[serde(default = "default_ai_interval")]
    pub ai_interval: f32,
}

fn default_true() -> bool { true }
fn default_farms() -> usize { 2 }
fn default_one() -> usize { 1 }
fn default_ai_interval() -> f32 { 5.0 }

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            world_size: 8000.0,
            towns: 2,
            farms: 2,
            farmers: 2,
            guards: 2,
            raiders: 1,
            scroll_speed: 400.0,
            log_kills: true,
            log_spawns: true,
            log_raids: true,
            log_harvests: true,
            log_levelups: true,
            log_npc_activity: true,
            log_ai: true,
            background_fps: false,
            debug_enemy_info: false,
            debug_coordinates: false,
            debug_all_npcs: false,
            policy: PolicySet::default(),
            ai_towns: 1,
            raider_camps: 1,
            ai_interval: 5.0,
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let dir = PathBuf::from(home).join("Documents").join("Endless");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("settings.json"))
}

pub fn save_settings(settings: &UserSettings) {
    let Some(path) = settings_path() else { return };
    match serde_json::to_string_pretty(settings) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!("Failed to save settings: {}", e);
            }
        }
        Err(e) => warn!("Failed to serialize settings: {}", e),
    }
}

pub fn load_settings() -> UserSettings {
    let Some(path) = settings_path() else { return UserSettings::default() };
    match std::fs::read_to_string(&path) {
        Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
        Err(_) => UserSettings::default(),
    }
}
