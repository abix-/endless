# UI and UX

## Overview

Endless uses Bevy plus egui for all player-facing UI. Runtime window and panel state lives in `UiState`, while durable preferences and per-panel persistence live in `UserSettings`.

Current implementation is split across `rust/src/ui/mod.rs`, `rust/src/ui/game_hud.rs`, `rust/src/ui/left_panel/`, `rust/src/ui/armory.rs`, and `rust/src/render.rs`.

## UI State Ownership

`UiState` owns open and closed runtime state for the main gameplay interface, including:

- build menu
- pause menu and active settings tab
- left panel open state and active tab
- combat log visibility
- tech tree, inspector, blackjack, and armory windows
- mine-assignment and faction-overlay helpers
- selected inventory filter and view state
- game-over screen

`UserSettings` persists the durable parts of the interface such as key bindings, panel state, volume, video, autosave interval, and other player preferences.

## Main Surfaces

### Top Bar and HUD

`game_hud.rs` owns the in-game HUD: population and resource summaries, inspector content, combat log, jukebox controls, build ghost status, squad overlay, and save toast rendering.

### Left Panel

The left panel hosts Roster, Upgrades, Policies, Patrols, Squads, Factions, Profiler, and Help content.

`UiState.left_panel_open` plus `UiState.left_panel_tab` are the live source of truth. When the panel closes, the code snapshots the current tab and tracked collapsible sections into `UserSettings`.

Tracked collapse persistence currently covers:

- roster and faction-analysis sections
- profiler sections
- help sections (`Quick Start`, `Economy`, `Military`, `Controls`, `Tips`)

### Armory

The armory is a centered modal window, not the legacy side-panel inventory view.

- `ControlAction::ToggleInventory` opens `UiState.armory_open`
- `UiState.open_armory()` closes the old `LeftPanelTab::Inventory` if it is still active
- `armory.rs` renders the modal with a roster rail, equipment board, and town inventory panel

### Pause, Settings, Save, and Load

`ui/mod.rs` renders the pause menu and its sub-tabs, including settings, save, and load flows. The main menu reuses the settings panel and exposes a separate load picker before entering the game.

## Help System

`HelpCatalog` is the shared in-memory help dictionary keyed by short topic ids. `help_tip()` renders the small `?` affordance used across the HUD and left panel.

Current help coverage includes:

- top-bar economy and population stats
- left-panel tab summaries
- build-menu explanations
- NPC inspector hints
- the dedicated Help tab with Quick Start, Economy, Military, Controls, and Tips sections

## Input Model

Keyboard toggles are centralized in `ui_toggle_system()`.

Current default hotkeys include:

- `B`: build menu
- `R`: roster
- `U`: tech tree and upgrades
- `P`: policies
- `T`: patrols
- `Q`: squads
- `I`: armory
- `G`: factions
- `H`: help
- `L`: combat log
- `F`: follow
- `1-0`: squad targeting

The UI layer also guards gameplay input when egui wants pointer or keyboard focus, so typing in fields and hovering panels suppresses gameplay clicks and camera motion.

## Escape Order

`game_escape_system()` closes or cancels UI in a fixed order before opening the pause menu:

1. active box-select or squad target placement
2. armory modal
3. tech tree
4. blackjack window
5. left panel
6. build placement or destroy mode
7. pause menu

This keeps `Esc` consistent across overlapping windows instead of opening the pause menu immediately.

## Toasts and Feedback

`SaveToast` is reused as the general lightweight toast channel for:

- save, load, and autosave results
- invalid building placement
- road upgrade and build rejection
- load failures from the main UI flow

`game_hud::save_toast_system()` renders the toast; `save_toast_tick_system()` expires it over time.

## Camera UX

Camera control lives in `render.rs`.

- keyboard pan uses wall-clock `Time::delta_secs()`, not game-scaled simulation time
- edge pan uses the same wall-clock model
- right-drag pans in screen space converted back to world space
- zoom is mouse-centered and suppressed while the tech tree or other egui surfaces own scrolling
- camera pan is suppressed while typing in text fields

The result is consistent camera speed regardless of game speed and fewer accidental camera moves while interacting with UI.

## Selection and Click Safety

`click_to_select_system()` and build-placement systems respect egui focus before consuming mouse input. This keeps UI widgets, drag operations, and modal windows from leaking clicks into world selection or building placement.

## Persistence

Current persistent UI state includes:

- left-panel tab
- tracked collapsed sections
- key bindings
- autosave interval
- audio and video settings
- AI Manager settings
- other options stored in `UserSettings`

The persistence boundary is deliberate: transient window state lives in `UiState`, while cross-session preferences live in `UserSettings`.

## Related Docs

- [armory-ui.md](armory-ui.md): armory interaction and presentation goals
- [save-load.md](save-load.md): save/load entry points and toast behavior
- [audio.md](audio.md): jukebox and sound-effect systems
- [rendering.md](rendering.md): camera extraction and world selection details
- [history.md](history.md): historical UI and UX delivery notes
