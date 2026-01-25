# Endless - Project Conventions

## Interaction Radii

Never hardcode pixel distances for building interactions. Use `Location.get_interaction_radius()`:

```gdscript
const Location = preload("res://world/location.gd")

var radius = Location.get_interaction_radius("field")       # farm (1.25x buffer)
var radius = Location.get_interaction_radius("camp", 1.5)   # custom buffer
```

The radius is calculated from sprite definitions (cell size × scale × diagonal) with configurable buffer.

Arrival radii (`_arrival_*`) are cached at load - edge-based, for entering sprite boundary.

Per-NPC `arrival_radii[i]` is set when walking to a target.

NPCs target building CENTERS (no offset). Arrival triggers when entering sprite boundary:
```gdscript
manager.targets[i] = manager.work_positions[i]  # building center
manager.arrival_radii[i] = manager._arrival_farm  # edge radius (72px for 3x3 farm)
```
Separation forces spread NPCs naturally once inside the building.

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
- `"home"` - bed (1x1)
- `"guard_post"` - guard post (1x1)
- `"fountain"` - town center marker (1x1)

## README Maintenance

The README serves as both documentation and a development roadmap.

**Structure:**
- Short description with inspirations (LOTR2, RimWorld, Factorio)
- Gameplay loop overview (6-step cycle)
- Features section with categorized checkboxes
- Architecture tree showing file purposes
- Controls and configuration tables

**Feature checkboxes:**
- `[x]` = implemented and working
- `[ ]` = planned but not started
- Update checkboxes as features are completed
- Keep items concise (one line each)
- Group by category: World, Economy, Combat, AI, Player Controls, etc.

**When changing config values** (world size, NPC counts, etc.), update the Configuration table to match.

**Don't over-document:** README shows what exists and what's planned. Implementation details go in CLAUDE.md or code comments.

## Rust/Bevy ECS (godot-bevy)

Performance target: 10,000 NPCs @ 140fps — **achieved** (release build).

**Status:** godot-bevy integrated. Bevy App running alongside Godot, ready for state machine migration.

**Key optimizations discovered:**
- Bulk `set_buffer()` vs per-instance calls: 55fps → 140fps (2.5x improvement)
- Pre-allocated transform buffer in Rust, single upload per frame
- Colors set once at init (not per-frame)

**Setup:**
1. Install Rust from https://rustup.rs/
2. `cd rust && cargo build`
3. Run `scenes/bevy_poc.tscn` in Godot

**Architecture:** GDExtension (`bevy_npc.gdextension`) loads `rust/target/debug/endless_ecs.dll`. The `NpcBenchmark` node owns a Bevy App internally, ticks ECS systems each frame, bulk uploads positions to MultiMesh via `set_buffer()`.

**Next steps:** See README "Rust Migration Roadmap" for phased plan to reach 20K+ NPCs:
1. GPU compute integration (port separation_compute.glsl)
2. Game logic migration (state machines, decisions)
3. Zero-copy rendering (compute shader writes directly to MultiMesh buffer)

**Files:**
- `rust/Cargo.toml` - bevy_ecs + godot-rust dependencies
- `rust/src/lib.rs` - NpcBenchmark class, ECS components + systems
- `bevy_npc.gdextension` - library paths for Godot
- `scenes/bevy_poc.tscn` - test scene
