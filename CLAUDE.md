# Endless - Project Conventions

## Interaction Radii

Never hardcode pixel distances for building interactions. Use `Location.get_interaction_radius()`:

```gdscript
const Location = preload("res://world/location.gd")

var radius = Location.get_interaction_radius("field")  # farm
var radius = Location.get_interaction_radius("camp")   # raider camp
var radius = Location.get_interaction_radius("home")   # house
```

The radius is calculated from sprite definitions (cell size × scale × diagonal) with a 1.25x buffer.

## Sprite Definitions

All building sprites are defined in `world/location.gd`:
- `SPRITES` dict: sprite sheet position and cell size
- `LOCATION_SPRITES` dict: maps location types to sprite names
- `*_PIECES` arrays: multi-sprite compositions

When adding new buildings, add entries to both `SPRITES` and `LOCATION_SPRITES`.

## NPC Data (DOD)

NPC data lives in parallel PackedArrays in `npc_manager.gd`. When adding new NPC properties:
1. Add array declaration and resize in `_init_arrays()`
2. Initialize value in `_spawn_npc_internal()`
3. Reset on death if needed

## State Machine

States defined in `npc_state.gd`. State transitions go through `manager._state.set_state()`.
Decision logic in `npc_needs.gd` via `decide_what_to_do()`.

## Settings

User preferences in `autoloads/user_settings.gd`. When adding settings:
1. Add variable with default
2. Add setter that emits `settings_changed`
3. Add to `_save()` and `_load()`
4. Connect listeners via `UserSettings.settings_changed.connect()`
