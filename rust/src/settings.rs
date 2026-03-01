//! User settings persistence — save/load config to JSON file.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::resources::PolicySet;

const SETTINGS_VERSION: u32 = 12;

/// Controls which NPCs have their activity logged in `NpcLogCache`.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum NpcLogMode {
    /// Log for all NPCs. High memory with large populations.
    All,
    /// Log only for the player's faction.
    Faction,
    /// Log only for the currently selected NPC. Best performance.
    SelectedOnly,
}

impl Default for NpcLogMode {
    fn default() -> Self {
        Self::SelectedOnly
    }
}

/// Groupings used by the Controls settings page.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ControlGroup {
    Camera,
    Panels,
    Time,
    SaveLoad,
    Squads,
}

impl ControlGroup {
    pub const ALL: [Self; 5] = [
        Self::Camera,
        Self::Panels,
        Self::Time,
        Self::SaveLoad,
        Self::Squads,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Camera => "Camera",
            Self::Panels => "Panels",
            Self::Time => "Time",
            Self::SaveLoad => "Save / Load",
            Self::Squads => "Squads",
        }
    }
}

/// Rebindable keyboard actions.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ControlAction {
    PanUp,
    PanDown,
    PanLeft,
    PanRight,
    ToggleRoster,
    ToggleBuildMenu,
    ToggleUpgrades,
    TogglePolicies,
    TogglePatrols,
    ToggleSquads,
    ToggleFactions,
    ToggleHelp,
    ToggleCombatLog,
    ToggleFollow,
    SquadTarget1,
    SquadTarget2,
    SquadTarget3,
    SquadTarget4,
    SquadTarget5,
    SquadTarget6,
    SquadTarget7,
    SquadTarget8,
    SquadTarget9,
    SquadTarget10,
    PauseMenu,
    TogglePause,
    SpeedUp,
    SpeedDown,
    QuickSave,
    QuickLoad,
}

impl ControlAction {
    pub const ALL: [Self; 30] = [
        Self::PanUp,
        Self::PanDown,
        Self::PanLeft,
        Self::PanRight,
        Self::ToggleRoster,
        Self::ToggleBuildMenu,
        Self::ToggleUpgrades,
        Self::TogglePolicies,
        Self::TogglePatrols,
        Self::ToggleSquads,
        Self::ToggleFactions,
        Self::ToggleHelp,
        Self::ToggleCombatLog,
        Self::ToggleFollow,
        Self::SquadTarget1,
        Self::SquadTarget2,
        Self::SquadTarget3,
        Self::SquadTarget4,
        Self::SquadTarget5,
        Self::SquadTarget6,
        Self::SquadTarget7,
        Self::SquadTarget8,
        Self::SquadTarget9,
        Self::SquadTarget10,
        Self::PauseMenu,
        Self::TogglePause,
        Self::SpeedUp,
        Self::SpeedDown,
        Self::QuickSave,
        Self::QuickLoad,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::PanUp => "pan_up",
            Self::PanDown => "pan_down",
            Self::PanLeft => "pan_left",
            Self::PanRight => "pan_right",
            Self::ToggleRoster => "toggle_roster",
            Self::ToggleBuildMenu => "toggle_build_menu",
            Self::ToggleUpgrades => "toggle_upgrades",
            Self::TogglePolicies => "toggle_policies",
            Self::TogglePatrols => "toggle_patrols",
            Self::ToggleSquads => "toggle_squads",
            Self::ToggleFactions => "toggle_factions",
            Self::ToggleHelp => "toggle_help",
            Self::ToggleCombatLog => "toggle_combat_log",
            Self::ToggleFollow => "toggle_follow",
            Self::SquadTarget1 => "squad_target_1",
            Self::SquadTarget2 => "squad_target_2",
            Self::SquadTarget3 => "squad_target_3",
            Self::SquadTarget4 => "squad_target_4",
            Self::SquadTarget5 => "squad_target_5",
            Self::SquadTarget6 => "squad_target_6",
            Self::SquadTarget7 => "squad_target_7",
            Self::SquadTarget8 => "squad_target_8",
            Self::SquadTarget9 => "squad_target_9",
            Self::SquadTarget10 => "squad_target_10",
            Self::PauseMenu => "pause_menu",
            Self::TogglePause => "toggle_pause",
            Self::SpeedUp => "speed_up",
            Self::SpeedDown => "speed_down",
            Self::QuickSave => "quick_save",
            Self::QuickLoad => "quick_load",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PanUp => "Pan Up",
            Self::PanDown => "Pan Down",
            Self::PanLeft => "Pan Left",
            Self::PanRight => "Pan Right",
            Self::ToggleRoster => "Roster Tab",
            Self::ToggleBuildMenu => "Build Menu",
            Self::ToggleUpgrades => "Upgrades Tab",
            Self::TogglePolicies => "Policies Tab",
            Self::TogglePatrols => "Patrols Tab",
            Self::ToggleSquads => "Squads Tab",
            Self::ToggleFactions => "Factions Tab",
            Self::ToggleHelp => "Help Tab",
            Self::ToggleCombatLog => "Combat Log",
            Self::ToggleFollow => "Follow Selected",
            Self::SquadTarget1 => "Squad 1 Target",
            Self::SquadTarget2 => "Squad 2 Target",
            Self::SquadTarget3 => "Squad 3 Target",
            Self::SquadTarget4 => "Squad 4 Target",
            Self::SquadTarget5 => "Squad 5 Target",
            Self::SquadTarget6 => "Squad 6 Target",
            Self::SquadTarget7 => "Squad 7 Target",
            Self::SquadTarget8 => "Squad 8 Target",
            Self::SquadTarget9 => "Squad 9 Target",
            Self::SquadTarget10 => "Squad 10 Target",
            Self::PauseMenu => "Pause Menu",
            Self::TogglePause => "Pause / Unpause",
            Self::SpeedUp => "Increase Time Speed",
            Self::SpeedDown => "Decrease Time Speed",
            Self::QuickSave => "Quick Save",
            Self::QuickLoad => "Quick Load",
        }
    }

    pub fn help_text(self) -> &'static str {
        match self {
            Self::PanUp | Self::PanDown | Self::PanLeft | Self::PanRight => {
                "Camera keyboard panning."
            }
            Self::ToggleRoster
            | Self::ToggleBuildMenu
            | Self::ToggleUpgrades
            | Self::TogglePolicies
            | Self::TogglePatrols
            | Self::ToggleSquads
            | Self::ToggleFactions
            | Self::ToggleHelp
            | Self::ToggleCombatLog
            | Self::ToggleFollow => "In-game panel and HUD shortcuts.",
            Self::SquadTarget1
            | Self::SquadTarget2
            | Self::SquadTarget3
            | Self::SquadTarget4
            | Self::SquadTarget5
            | Self::SquadTarget6
            | Self::SquadTarget7
            | Self::SquadTarget8
            | Self::SquadTarget9
            | Self::SquadTarget10 => "Select squad and enter target placement mode.",
            Self::PauseMenu | Self::TogglePause | Self::SpeedUp | Self::SpeedDown => {
                "Pause and time controls."
            }
            Self::QuickSave | Self::QuickLoad => "Save/load shortcuts while playing.",
        }
    }

    pub fn group(self) -> ControlGroup {
        match self {
            Self::PanUp | Self::PanDown | Self::PanLeft | Self::PanRight => ControlGroup::Camera,
            Self::ToggleRoster
            | Self::ToggleBuildMenu
            | Self::ToggleUpgrades
            | Self::TogglePolicies
            | Self::TogglePatrols
            | Self::ToggleSquads
            | Self::ToggleFactions
            | Self::ToggleHelp
            | Self::ToggleCombatLog
            | Self::ToggleFollow => ControlGroup::Panels,
            Self::SquadTarget1
            | Self::SquadTarget2
            | Self::SquadTarget3
            | Self::SquadTarget4
            | Self::SquadTarget5
            | Self::SquadTarget6
            | Self::SquadTarget7
            | Self::SquadTarget8
            | Self::SquadTarget9
            | Self::SquadTarget10 => ControlGroup::Squads,
            Self::PauseMenu | Self::TogglePause | Self::SpeedUp | Self::SpeedDown => {
                ControlGroup::Time
            }
            Self::QuickSave | Self::QuickLoad => ControlGroup::SaveLoad,
        }
    }

    pub fn default_key(self) -> KeyCode {
        match self {
            Self::PanUp => KeyCode::KeyW,
            Self::PanDown => KeyCode::KeyS,
            Self::PanLeft => KeyCode::KeyA,
            Self::PanRight => KeyCode::KeyD,
            Self::ToggleRoster => KeyCode::KeyR,
            Self::ToggleBuildMenu => KeyCode::KeyB,
            Self::ToggleUpgrades => KeyCode::KeyU,
            Self::TogglePolicies => KeyCode::KeyP,
            Self::TogglePatrols => KeyCode::KeyT,
            Self::ToggleSquads => KeyCode::KeyQ,
            Self::ToggleFactions => KeyCode::KeyI,
            Self::ToggleHelp => KeyCode::KeyH,
            Self::ToggleCombatLog => KeyCode::KeyL,
            Self::ToggleFollow => KeyCode::KeyF,
            Self::SquadTarget1 => KeyCode::Digit1,
            Self::SquadTarget2 => KeyCode::Digit2,
            Self::SquadTarget3 => KeyCode::Digit3,
            Self::SquadTarget4 => KeyCode::Digit4,
            Self::SquadTarget5 => KeyCode::Digit5,
            Self::SquadTarget6 => KeyCode::Digit6,
            Self::SquadTarget7 => KeyCode::Digit7,
            Self::SquadTarget8 => KeyCode::Digit8,
            Self::SquadTarget9 => KeyCode::Digit9,
            Self::SquadTarget10 => KeyCode::Digit0,
            Self::PauseMenu => KeyCode::Escape,
            Self::TogglePause => KeyCode::Space,
            Self::SpeedUp => KeyCode::Equal,
            Self::SpeedDown => KeyCode::Minus,
            Self::QuickSave => KeyCode::F5,
            Self::QuickLoad => KeyCode::F9,
        }
    }
}

pub const CAMERA_ACTIONS: [ControlAction; 4] = [
    ControlAction::PanUp,
    ControlAction::PanDown,
    ControlAction::PanLeft,
    ControlAction::PanRight,
];

pub const PANEL_ACTIONS: [ControlAction; 10] = [
    ControlAction::ToggleRoster,
    ControlAction::ToggleBuildMenu,
    ControlAction::ToggleUpgrades,
    ControlAction::TogglePolicies,
    ControlAction::TogglePatrols,
    ControlAction::ToggleSquads,
    ControlAction::ToggleFactions,
    ControlAction::ToggleHelp,
    ControlAction::ToggleCombatLog,
    ControlAction::ToggleFollow,
];

pub const SQUAD_TARGET_ACTIONS: [ControlAction; 10] = [
    ControlAction::SquadTarget1,
    ControlAction::SquadTarget2,
    ControlAction::SquadTarget3,
    ControlAction::SquadTarget4,
    ControlAction::SquadTarget5,
    ControlAction::SquadTarget6,
    ControlAction::SquadTarget7,
    ControlAction::SquadTarget8,
    ControlAction::SquadTarget9,
    ControlAction::SquadTarget10,
];

pub const TIME_ACTIONS: [ControlAction; 4] = [
    ControlAction::PauseMenu,
    ControlAction::TogglePause,
    ControlAction::SpeedUp,
    ControlAction::SpeedDown,
];

pub const SAVELOAD_ACTIONS: [ControlAction; 2] =
    [ControlAction::QuickSave, ControlAction::QuickLoad];

pub fn control_actions_for_group(group: ControlGroup) -> &'static [ControlAction] {
    match group {
        ControlGroup::Camera => &CAMERA_ACTIONS,
        ControlGroup::Panels => &PANEL_ACTIONS,
        ControlGroup::Time => &TIME_ACTIONS,
        ControlGroup::SaveLoad => &SAVELOAD_ACTIONS,
        ControlGroup::Squads => &SQUAD_TARGET_ACTIONS,
    }
}

fn parse_letter_key(token: &str) -> Option<KeyCode> {
    let raw = token.strip_prefix("Key").unwrap_or(token);
    if raw.len() != 1 {
        return None;
    }
    match raw.as_bytes()[0].to_ascii_uppercase() {
        b'A' => Some(KeyCode::KeyA),
        b'B' => Some(KeyCode::KeyB),
        b'C' => Some(KeyCode::KeyC),
        b'D' => Some(KeyCode::KeyD),
        b'E' => Some(KeyCode::KeyE),
        b'F' => Some(KeyCode::KeyF),
        b'G' => Some(KeyCode::KeyG),
        b'H' => Some(KeyCode::KeyH),
        b'I' => Some(KeyCode::KeyI),
        b'J' => Some(KeyCode::KeyJ),
        b'K' => Some(KeyCode::KeyK),
        b'L' => Some(KeyCode::KeyL),
        b'M' => Some(KeyCode::KeyM),
        b'N' => Some(KeyCode::KeyN),
        b'O' => Some(KeyCode::KeyO),
        b'P' => Some(KeyCode::KeyP),
        b'Q' => Some(KeyCode::KeyQ),
        b'R' => Some(KeyCode::KeyR),
        b'S' => Some(KeyCode::KeyS),
        b'T' => Some(KeyCode::KeyT),
        b'U' => Some(KeyCode::KeyU),
        b'V' => Some(KeyCode::KeyV),
        b'W' => Some(KeyCode::KeyW),
        b'X' => Some(KeyCode::KeyX),
        b'Y' => Some(KeyCode::KeyY),
        b'Z' => Some(KeyCode::KeyZ),
        _ => None,
    }
}

fn parse_digit_key(token: &str) -> Option<KeyCode> {
    let raw = token.strip_prefix("Digit").unwrap_or(token);
    if raw.len() != 1 {
        return None;
    }
    match raw.as_bytes()[0] {
        b'0' => Some(KeyCode::Digit0),
        b'1' => Some(KeyCode::Digit1),
        b'2' => Some(KeyCode::Digit2),
        b'3' => Some(KeyCode::Digit3),
        b'4' => Some(KeyCode::Digit4),
        b'5' => Some(KeyCode::Digit5),
        b'6' => Some(KeyCode::Digit6),
        b'7' => Some(KeyCode::Digit7),
        b'8' => Some(KeyCode::Digit8),
        b'9' => Some(KeyCode::Digit9),
        _ => None,
    }
}

fn parse_function_key(token: &str) -> Option<KeyCode> {
    let Some(raw) = token.strip_prefix('F').or_else(|| token.strip_prefix('f')) else {
        return None;
    };
    let n = raw.parse::<u8>().ok()?;
    match n {
        1 => Some(KeyCode::F1),
        2 => Some(KeyCode::F2),
        3 => Some(KeyCode::F3),
        4 => Some(KeyCode::F4),
        5 => Some(KeyCode::F5),
        6 => Some(KeyCode::F6),
        7 => Some(KeyCode::F7),
        8 => Some(KeyCode::F8),
        9 => Some(KeyCode::F9),
        10 => Some(KeyCode::F10),
        11 => Some(KeyCode::F11),
        12 => Some(KeyCode::F12),
        _ => None,
    }
}

fn parse_keycode_token(raw: &str) -> Option<KeyCode> {
    let token = raw.trim();
    if token.is_empty() {
        return None;
    }
    if let Some(key) = parse_letter_key(token) {
        return Some(key);
    }
    if let Some(key) = parse_digit_key(token) {
        return Some(key);
    }
    if let Some(key) = parse_function_key(token) {
        return Some(key);
    }
    if token.eq_ignore_ascii_case("space") || token.eq_ignore_ascii_case("spacebar") {
        return Some(KeyCode::Space);
    }
    if token.eq_ignore_ascii_case("escape") || token.eq_ignore_ascii_case("esc") {
        return Some(KeyCode::Escape);
    }
    if token == "-" || token.eq_ignore_ascii_case("minus") {
        return Some(KeyCode::Minus);
    }
    if token == "=" || token == "+" || token.eq_ignore_ascii_case("equal") {
        return Some(KeyCode::Equal);
    }
    if token.eq_ignore_ascii_case("arrowup") || token.eq_ignore_ascii_case("up") {
        return Some(KeyCode::ArrowUp);
    }
    if token.eq_ignore_ascii_case("arrowdown") || token.eq_ignore_ascii_case("down") {
        return Some(KeyCode::ArrowDown);
    }
    if token.eq_ignore_ascii_case("arrowleft") || token.eq_ignore_ascii_case("left") {
        return Some(KeyCode::ArrowLeft);
    }
    if token.eq_ignore_ascii_case("arrowright") || token.eq_ignore_ascii_case("right") {
        return Some(KeyCode::ArrowRight);
    }
    None
}

fn default_key_bindings() -> BTreeMap<String, String> {
    let mut bindings = BTreeMap::new();
    for action in ControlAction::ALL {
        bindings.insert(
            action.id().to_string(),
            format!("{:?}", action.default_key()),
        );
    }
    bindings
}

pub fn keycode_display_name(key: KeyCode) -> String {
    let raw = format!("{:?}", key);
    if let Some(letter) = raw.strip_prefix("Key") {
        return letter.to_string();
    }
    if let Some(digit) = raw.strip_prefix("Digit") {
        return digit.to_string();
    }
    match raw.as_str() {
        "Escape" => "Esc".to_string(),
        "Space" => "Space".to_string(),
        "Minus" => "-".to_string(),
        "Equal" => "=".to_string(),
        "ArrowUp" => "Up".to_string(),
        "ArrowDown" => "Down".to_string(),
        "ArrowLeft" => "Left".to_string(),
        "ArrowRight" => "Right".to_string(),
        _ => raw,
    }
}

pub fn is_rebindable_key(key: KeyCode) -> bool {
    parse_keycode_token(&format!("{:?}", key)).is_some()
}

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
    /// Keyboard bindings by action id (value is bevy KeyCode name, e.g. "KeyW").
    #[serde(default = "default_key_bindings")]
    pub key_bindings: BTreeMap<String, String>,
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
    #[serde(default)]
    pub debug_ai_decisions: bool,
    #[serde(default = "default_true")]
    pub show_terrain_sprites: bool,
    #[serde(default)]
    pub show_all_faction_squad_lines: bool,
    // Town policies
    #[serde(default)]
    pub policy: PolicySet,
    // Video / display
    #[serde(default = "default_window_width")]
    pub window_width: u32,
    #[serde(default = "default_window_height")]
    pub window_height: u32,
    #[serde(default = "default_true")]
    pub window_maximized: bool,
    #[serde(default = "default_true")]
    pub vsync: bool,
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default)]
    pub background_fps: bool,
    // World gen style (0=Classic, 1=Continents)
    #[serde(default)]
    pub gen_style: u8,
    // AI players
    #[serde(default = "default_five")]
    pub ai_towns: usize,
    #[serde(default = "default_five")]
    pub raider_towns: usize,
    #[serde(default = "default_ai_interval")]
    pub ai_interval: f32,
    #[serde(default = "default_gold_mines")]
    pub gold_mines_per_town: usize,
    #[serde(default = "default_npc_interval")]
    pub npc_interval: f32,
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
    #[serde(default = "default_interface_text_size")]
    pub interface_text_size: f32,
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
    pub jukebox_loop: bool,
    #[serde(default)]
    pub tutorial_completed: bool,
    // Endless mode
    #[serde(default)]
    pub endless_mode: bool,
    #[serde(default = "default_endless_strength")]
    pub endless_strength: f32,
    // Zoom & LOD
    #[serde(default = "default_zoom_speed")]
    pub zoom_speed: f32,
    #[serde(default = "default_zoom_min")]
    pub zoom_min: f32,
    #[serde(default = "default_zoom_max")]
    pub zoom_max: f32,
    #[serde(default = "default_lod_transition")]
    pub lod_transition: f32,
    /// Which NPCs get activity-logged (perf: fewer = less allocation in hot loop).
    #[serde(default)]
    pub npc_log_mode: NpcLogMode,
}

fn default_endless_strength() -> f32 {
    0.75
}
fn default_gold_mines() -> usize {
    2
}

fn default_true() -> bool {
    true
}
fn default_neg1() -> i32 {
    -1
}
fn default_farms() -> usize {
    2
}
fn default_five() -> usize {
    5
}
fn default_ai_interval() -> f32 {
    5.0
}
fn default_npc_interval() -> f32 {
    2.0
}
fn default_ui_scale() -> f32 {
    1.0
}
fn default_interface_text_size() -> f32 {
    16.0
}
fn default_help_text_size() -> f32 {
    14.0
}
fn default_build_menu_text_scale() -> f32 {
    1.2
}
fn default_window_width() -> u32 {
    1280
}
fn default_window_height() -> u32 {
    720
}
fn default_autosave_hours() -> i32 {
    12
}
fn default_music_volume() -> f32 {
    0.3
}
fn default_sfx_volume() -> f32 {
    0.5
}
fn default_music_speed() -> f32 {
    1.0
}
fn default_zoom_speed() -> f32 {
    0.1
}
fn default_zoom_min() -> f32 {
    0.02
}
fn default_zoom_max() -> f32 {
    4.0
}
fn default_lod_transition() -> f32 {
    0.25
}

const MIN_WINDOW_WIDTH: u32 = 800;
const MAX_WINDOW_WIDTH: u32 = 7680;
const MIN_WINDOW_HEIGHT: u32 = 600;
const MAX_WINDOW_HEIGHT: u32 = 4320;

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
            npc_counts: crate::constants::NPC_REGISTRY
                .iter()
                .map(|d| (format!("{:?}", d.job), d.default_count))
                .collect(),
            scroll_speed: 400.0,
            key_bindings: default_key_bindings(),
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
            window_width: default_window_width(),
            window_height: default_window_height(),
            window_maximized: true,
            vsync: true,
            fullscreen: false,
            background_fps: false,
            debug_coordinates: false,
            debug_all_npcs: false,
            debug_readback: false,
            debug_combat: false,
            debug_spawns: false,
            debug_behavior: false,
            debug_profiler: false,
            debug_ai_decisions: false,
            show_terrain_sprites: true,
            show_all_faction_squad_lines: false,
            policy: PolicySet::default(),
            ai_towns: 5,
            raider_towns: 5,
            ai_interval: 5.0,
            gold_mines_per_town: 2,
            npc_interval: 2.0,
            ui_scale: 1.0,
            interface_text_size: 16.0,
            help_text_size: 14.0,
            build_menu_text_scale: 1.2,
            raider_passive_forage: false,
            auto_upgrades: Vec::new(),
            difficulty: crate::resources::Difficulty::Normal,
            autosave_hours: 12,
            music_volume: 0.3,
            sfx_volume: 0.5,
            music_speed: 1.0,
            jukebox_loop: false,
            tutorial_completed: false,
            upgrade_expanded: Vec::new(),
            endless_mode: false,
            endless_strength: 0.75,
            zoom_speed: 0.1,
            zoom_min: 0.02,
            zoom_max: 4.0,
            lod_transition: 0.25,
            npc_log_mode: NpcLogMode::default(),
        }
    }
}

impl UserSettings {
    pub fn key_for_action(&self, action: ControlAction) -> KeyCode {
        self.key_bindings
            .get(action.id())
            .and_then(|raw| parse_keycode_token(raw))
            .unwrap_or_else(|| action.default_key())
    }

    pub fn key_label_for_action(&self, action: ControlAction) -> String {
        keycode_display_name(self.key_for_action(action))
    }

    pub fn set_key_for_action(&mut self, action: ControlAction, key: KeyCode) {
        self.key_bindings
            .insert(action.id().to_string(), format!("{:?}", key));
    }

    pub fn ensure_key_bindings(&mut self) {
        for action in ControlAction::ALL {
            self.key_bindings
                .entry(action.id().to_string())
                .or_insert_with(|| format!("{:?}", action.default_key()));
        }
    }

    pub fn reset_key_bindings(&mut self) {
        self.key_bindings = default_key_bindings();
    }

    pub fn clamp_video_settings(&mut self) {
        self.window_width = self
            .window_width
            .clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_WIDTH);
        self.window_height = self
            .window_height
            .clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_HEIGHT);
    }
}

pub fn apply_video_settings_to_window(window: &mut bevy::window::Window, settings: &UserSettings) {
    let width = settings
        .window_width
        .clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_WIDTH);
    let height = settings
        .window_height
        .clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_HEIGHT);
    window.resolution = (width, height).into();
    window.present_mode = if settings.vsync {
        bevy::window::PresentMode::AutoVsync
    } else {
        bevy::window::PresentMode::AutoNoVsync
    };
    if settings.fullscreen {
        window.mode = bevy::window::WindowMode::BorderlessFullscreen(
            bevy::window::MonitorSelection::Current,
        );
    } else {
        window.mode = bevy::window::WindowMode::Windowed;
        window.set_maximized(settings.window_maximized);
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
    let Some(path) = settings_path() else {
        return UserSettings::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(json) => {
            let mut settings: UserSettings = serde_json::from_str(&json).unwrap_or_default();
            // Migrate legacy per-type fields into npc_counts
            if settings.npc_counts.is_empty() {
                if settings.farmers > 0 {
                    settings
                        .npc_counts
                        .insert("Farmer".into(), settings.farmers);
                }
                if settings.archers > 0 {
                    settings
                        .npc_counts
                        .insert("Archer".into(), settings.archers);
                }
                if settings.raiders > 0 {
                    settings
                        .npc_counts
                        .insert("Raider".into(), settings.raiders);
                }
                // Fill missing jobs from registry defaults
                for def in crate::constants::NPC_REGISTRY {
                    let key = format!("{:?}", def.job);
                    settings.npc_counts.entry(key).or_insert(def.default_count);
                }
            }
            settings.ensure_key_bindings();
            settings.clamp_video_settings();
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
