//! User settings persistence — save/load config to JSON file.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::resources::PolicySet;

const SETTINGS_VERSION: u32 = 7;

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
    // Legacy per-type fields — kept for backward compat loading, migrated to npc_counts
    #[serde(default)]
    pub farmers: usize,
    #[serde(default, alias = "guards")]
    pub archers: usize,
    #[serde(default)]
    pub raiders: usize,
    /// Per-job NPC counts (key = Job debug name, e.g. "Farmer"). Replaces farmers/archers/raiders.
    #[serde(default)]
    pub npc_counts: std::collections::BTreeMap<String, usize>,
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
    #[serde(default = "default_true")]
    pub log_building_damage: bool,
    #[serde(default = "default_true")]
    pub log_loot: bool,
    /// -1 = all factions, 0 = my faction only
    #[serde(default = "default_neg1")]
    pub log_faction_filter: i32,
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
    #[serde(default = "default_true")]
    pub show_terrain_sprites: bool,
    #[serde(default)]
    pub show_all_faction_squad_lines: bool,
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
    #[serde(default = "default_five")]
    pub ai_towns: usize,
    #[serde(default = "default_five")]
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
    // Upgrade branches the user has expanded (by label); empty = all collapsed
    #[serde(default)]
    pub upgrade_expanded: Vec<String>,
    // Difficulty
    #[serde(default)]
    pub difficulty: crate::resources::Difficulty,
    // Autosave interval in game-hours (0 = disabled)
    #[serde(default = "default_autosave_hours")]
    pub autosave_hours: i32,
    // Audio
    #[serde(default = "default_music_volume")]
    pub music_volume: f32,
    #[serde(default = "default_sfx_volume")]
    pub sfx_volume: f32,
    #[serde(default = "default_music_speed")]
    pub music_speed: f32,
    #[serde(default)]
    pub tutorial_completed: bool,
}

fn default_gold_mines() -> usize { 2 }

fn default_true() -> bool { true }
fn default_neg1() -> i32 { -1 }
fn default_farms() -> usize { 2 }
fn default_five() -> usize { 5 }
fn default_ai_interval() -> f32 { 5.0 }
fn default_npc_interval() -> f32 { 2.0 }
fn default_ui_scale() -> f32 { 1.0 }
fn default_help_text_size() -> f32 { 14.0 }
fn default_build_menu_text_scale() -> f32 { 1.2 }
fn default_autosave_hours() -> i32 { 12 }
fn default_music_volume() -> f32 { 0.3 }
fn default_sfx_volume() -> f32 { 0.5 }
fn default_music_speed() -> f32 { 1.0 }

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            world_size: 8000.0,
            towns: 1,
            farms: 2,
            farmers: 0,
            archers: 0,
            raiders: 0,
            npc_counts: crate::constants::NPC_REGISTRY.iter()
                .map(|d| (format!("{:?}", d.job), d.default_count))
                .collect(),
            scroll_speed: 400.0,
            log_kills: true,
            log_spawns: true,
            log_raids: true,
            log_harvests: true,
            log_levelups: true,
            log_npc_activity: true,
            log_ai: true,
            log_building_damage: true,
            log_loot: true,
            log_faction_filter: -1,
            gen_style: 1,
            background_fps: false,
            debug_coordinates: false,
            debug_all_npcs: false,
            debug_readback: false,
            debug_combat: false,
            debug_spawns: false,
            debug_behavior: false,
            debug_profiler: false,
            show_terrain_sprites: true,
            show_all_faction_squad_lines: false,
            policy: PolicySet::default(),
            ai_towns: 5,
            raider_camps: 5,
            ai_interval: 5.0,
            gold_mines_per_town: 2,
            npc_interval: 2.0,
            ui_scale: 1.0,
            help_text_size: 14.0,
            build_menu_text_scale: 1.2,
            raider_passive_forage: false,
            auto_upgrades: Vec::new(),
            difficulty: crate::resources::Difficulty::Normal,
            autosave_hours: 12,
            music_volume: 0.3,
            sfx_volume: 0.5,
            music_speed: 1.0,
            tutorial_completed: false,
            upgrade_expanded: Vec::new(),
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
            // Migrate legacy per-type fields into npc_counts
            if settings.npc_counts.is_empty() {
                if settings.farmers > 0 {
                    settings.npc_counts.insert("Farmer".into(), settings.farmers);
                }
                if settings.archers > 0 {
                    settings.npc_counts.insert("Archer".into(), settings.archers);
                }
                if settings.raiders > 0 {
                    settings.npc_counts.insert("Raider".into(), settings.raiders);
                }
                // Fill missing jobs from registry defaults
                for def in crate::constants::NPC_REGISTRY {
                    let key = format!("{:?}", def.job);
                    settings.npc_counts.entry(key).or_insert(def.default_count);
                }
            }
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
