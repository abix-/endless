# Save and Load

## Overview

Endless persists the live ECS world to JSON files in `Documents/Endless/saves`. The save system supports quicksave/quickload, named saves, rotating autosaves, versioned migration, and in-game toast feedback.

Current implementation lives in `rust/src/save.rs`, with menu entry points in `rust/src/ui/main_menu.rs` and `rust/src/ui/mod.rs`.

## Save Files

- `quicksave.json`: the default target for F5 and quick-load F9.
- `autosave_1.json` through `autosave_3.json`: rotating autosave slots.
- `<name>.json`: manual named saves created from the pause menu.

`named_save_path()` sanitizes names to ASCII letters, numbers, `-`, and `_`, replacing other characters with `_`.

## Entry Points

- `F5`: writes `quicksave.json` by sending `SaveGameMsg`.
- `F9`: loads `quicksave.json` by sending `LoadGameMsg`.
- Pause menu `Save Game`: quicksave.
- Pause menu `Save Game As...`: writes a named save.
- Pause menu `Load Game`: loads quicksave, named saves, or any discovered save file.
- Main menu `Load Game`: opens a save picker and enters `Playing` with `SaveLoadRequest.load_on_enter = true`.

## Runtime Control Model

`SaveLoadRequest` is the shared control resource:

- `load_on_enter`: tells startup to restore a save instead of generating a new world
- `save_path`: optional override for the next save request
- `load_path`: optional override for the next load request
- `autosave_hours`: autosave interval in game-hours
- `autosave_last_hour`: last hour that triggered an autosave
- `autosave_slot`: next rotating autosave slot index

`SaveGameMsg` and `LoadGameMsg` trigger the runtime systems; the request resource carries the optional path overrides and autosave state.

## What Persists

`collect_save_data()` serializes the full playable state, including:

- world terrain and original terrain restoration data
- placed buildings and per-building runtime state
- town area levels, food, gold, wood, and stone
- town upgrades, policies, auto-upgrade flags, and town equipment
- NPC positions, stats, activity state, health, energy, combat state, home/work state, carried loot, and equipment
- squad membership, targets, patrol/rest settings, and loot thresholds
- AI players, faction stats, reputation, migration state, endless-mode state, and merchant inventory
- loot item id counters and faction list data

The load path rebuilds the world through `restore_world_from_save()` and re-materializes ECS entities from the serialized save model instead of trying to resume transient runtime state.

## Save Flow

`save_game_system()`:

1. Collects NPC, building, town, AI, inventory, and faction state from ECS.
2. Builds a `SaveData` payload with `collect_save_data()`.
3. Writes either the explicit `save_path` or the default quicksave path.
4. Updates `SaveToast` with success or failure feedback.

## Load Flow

`load_game_system()`:

1. Reads either the explicit `load_path` or the default quicksave path.
2. Rejects unsupported future save versions and logs migrations for older saves.
3. Despawns live NPC entities and transient farm markers.
4. Calls `restore_world_from_save()` to rebuild towns, buildings, NPCs, inventories, squads, AI state, and GPU data.
5. Updates `SaveToast` with load feedback.

## Autosaves

`autosave_system()` runs on the game-hour tick.

- `autosave_hours <= 0` disables autosave.
- Autosaves only fire when the configured interval has elapsed since `autosave_last_hour`.
- The system rotates through `autosave_1.json`, `autosave_2.json`, and `autosave_3.json`.
- Autosaves use the same serialization path as manual saves.

The autosave interval is configured from `UserSettings.autosave_hours` and copied into `SaveLoadRequest` on startup and settings changes.

## User Feedback

`SaveToast` is the shared transient feedback resource for:

- save success / failure
- load success / failure
- autosave success / failure
- build-placement errors that reuse the same toast overlay

`save_toast_system()` renders the on-screen toast and `save_toast_tick_system()` expires it over time.

## Versioning

`read_save_from()` enforces `SAVE_VERSION`.

- newer-than-supported saves are rejected
- older saves are accepted and migrated by the current load code
- compatibility helpers pad or translate older data shapes where needed

## Related Docs

- [spawn.md](spawn.md): shared NPC materialization path used by startup and restore
- [resources.md](resources.md): persisted resources and settings fields
- [ui.md](ui.md): save/load UI entry points and toast behavior
- [history.md](history.md): historical delivery notes for the save/load rollout
