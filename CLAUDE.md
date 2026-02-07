# Endless - Project Conventions

## Sprite Definitions

All building sprites are defined in `world/location.gd`:
- `SPRITES` dict: sprite sheet position and cell size
- `LOCATION_SPRITES` dict: maps location types to sprite names
- `*_PIECES` arrays: multi-sprite compositions

When adding new buildings, add entries to both `SPRITES` and `LOCATION_SPRITES`.

## Location Types

Valid types for `location_type` export:
- `"field"` - farm (3x3)
- `"camp"` - raider camp (2x2 tent)
- `"home"` - bed (1x1)
- `"guard_post"` - guard post (1x1)
- `"fountain"` - town center marker (1x1)

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

## README vs Roadmap

- **README.md** - Introduction to the game (description, gameplay, controls, credits)
- **docs/roadmap.md** - Feature tracking with `[x]`/`[ ]` checkboxes, performance targets, game design reference

Don't add feature checkboxes to README. All development tracking goes in roadmap.

## Rust/Bevy ECS

All NPC data and logic lives in Rust. Performance: 10,000 NPCs @ 140fps (release build).

See [docs/](docs/README.md) for architecture, system maps, and known issues.

**Setup:**
1. Install Rust from https://rustup.rs/
2. `cd rust && cargo build`
3. Run `scenes/ecs_test.tscn` in Godot

**Key files:**
- `rust/src/lib.rs` - EcsNpcManager: GDScript API bridge, GPU dispatch, rendering
- `rust/src/gpu.rs` - GPU compute buffer management
- `rust/src/systems/` - Bevy systems (spawn, combat, health, behavior)
- `bevy_npc.gdextension` - library paths for Godot

## Lessons Learned

When a mistake is made during development, document it here so we don't repeat it:

- **PowerShell error suppression**: Don't use `2>$null` - it causes parse errors. Use `-ErrorAction SilentlyContinue` instead. Example: `Get-Process *godot* -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue`
