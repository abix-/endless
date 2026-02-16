//! User settings persistence — save/load config to JSON file.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::resources::PolicySet;

const SETTINGS_VERSION: u32 = 2;

/// Persisted user settings. Saved to `Documents\Endless\settings.json`.
#[derive(Resource, Serialize, Deserialize, Clone)]
pub struct UserSettings {
    #[serde(default)]
    pub version: u32,
    // World gen (main menu sliders)
    pub world_size: f32,
    pub towns: usize,
    #[serde(default = "default_farms")]
    pub farms: usize,
    pub farmers: usize,
    #[serde(alias = "guards")]
    pub archers: usize,
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
    pub debug_coordinates: bool,
    #[serde(default)]
    pub debug_all_npcs: bool,
    // Debug logging (formerly F-key toggles)
    #[serde(default)]
    pub debug_readback: bool,
    #[serde(default)]
    pub debug_combat: bool,
    #[serde(default)]
    pub debug_spawns: bool,
    #[serde(default)]
    pub debug_behavior: bool,
    #[serde(default)]
    pub debug_profiler: bool,
    // Town policies
    #[serde(default)]
    pub policy: PolicySet,
    // Display
    #[serde(default)]
    pub background_fps: bool,
    // World gen style (0=Classic, 1=Continents)
    #[serde(default)]
    pub gen_style: u8,
    // AI players
    #[serde(default = "default_one")]
    pub ai_towns: usize,
    #[serde(default = "default_one")]
    pub raider_camps: usize,
    #[serde(default = "default_ai_interval")]
    pub ai_interval: f32,
    #[serde(default = "default_gold_mines")]
    pub gold_mines_per_town: usize,
    #[serde(default = "default_npc_interval")]
    pub npc_interval: f32,
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
    #[serde(default = "default_help_text_size")]
    pub help_text_size: f32,
    #[serde(default = "default_build_menu_text_scale")]
    pub build_menu_text_scale: f32,
    #[serde(default)]
    pub raider_passive_forage: bool,
    // Per-upgrade auto-buy flags (player town only)
    #[serde(default)]
    pub auto_upgrades: Vec<bool>,
}

fn default_gold_mines() -> usize { 2 }

fn default_true() -> bool { true }
fn default_farms() -> usize { 2 }
fn default_one() -> usize { 1 }
fn default_ai_interval() -> f32 { 5.0 }
fn default_npc_interval() -> f32 { 2.0 }
fn default_ui_scale() -> f32 { 1.0 }
fn default_help_text_size() -> f32 { 14.0 }
fn default_build_menu_text_scale() -> f32 { 1.2 }

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            world_size: 8000.0,
            towns: 1,
            farms: 2,
            farmers: 2,
            archers: 2,
            raiders: 1,
            scroll_speed: 400.0,
            log_kills: true,
            log_spawns: true,
            log_raids: true,
            log_harvests: true,
            log_levelups: true,
            log_npc_activity: true,
            log_ai: true,
            gen_style: 0,
            background_fps: false,
            debug_coordinates: false,
            debug_all_npcs: false,
            debug_readback: false,
            debug_combat: false,
            debug_spawns: false,
            debug_behavior: false,
            debug_profiler: false,
            policy: PolicySet::default(),
            ai_towns: 1,
            raider_camps: 1,
            ai_interval: 5.0,
            gold_mines_per_town: 2,
            npc_interval: 2.0,
            ui_scale: 1.0,
            help_text_size: 14.0,
            build_menu_text_scale: 1.2,
            raider_passive_forage: false,
            auto_upgrades: Vec::new(),
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
        Ok(json) => {
            let mut settings: UserSettings = serde_json::from_str(&json).unwrap_or_default();
            if settings.version < SETTINGS_VERSION {
                info!(
                    "Settings version {} → {}, new fields use defaults",
                    settings.version, SETTINGS_VERSION
                );
                settings.version = SETTINGS_VERSION;
                save_settings(&settings);
            }
            settings
        }
        Err(_) => UserSettings::default(),
    }
}
