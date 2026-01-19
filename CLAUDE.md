# Endless - Project Conventions

## Interaction Radii

Never hardcode pixel distances for building interactions. Use `Location.get_interaction_radius()`:

```gdscript
const Location = preload("res://world/location.gd")

var radius = Location.get_interaction_radius("field")       # farm (1.25x buffer)
var radius = Location.get_interaction_radius("camp", 1.5)   # custom buffer
```

The radius is calculated from sprite definitions (cell size × scale × diagonal) with configurable buffer.

When targeting a building, set `arrival_radii[i]` so NPCs "arrive" when on the sprite, not at the exact center:
```gdscript
manager.targets[i] = building_pos
manager.arrival_radii[i] = Location.get_interaction_radius("field")
```
Default arrival radius is 5.0 (for exact positions like work spots).

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

## Shaders

Shader uniforms for per-instance data use INSTANCE_CUSTOM (Color with r,g,b,a packed values).
Example from `npc_sprite.gdshader`:
- r = health percent
- g = flash intensity
- b = sprite frame X / 255
- a = sprite frame Y / 255

HP bar modes: 0=off, 1=when damaged, 2=always (uniform int, set via ShaderMaterial)

## MultiMesh Rendering

NPCs and overlays (loot icons) use separate MultiMesh instances in `npc_renderer.gd`.
Each MultiMesh needs: mesh, transform_format, instance_count, optional use_colors/use_custom_data.
Hide instances by setting transform position to (-9999, -9999).

## Location Types

Valid types for `location_type` export:
- `"field"` - farm (3x3)
- `"camp"` - raider camp (2x2 tent)
- `"home"` - house (2x2 composed)
- `"guard_post"` - guard tower (1x1)
