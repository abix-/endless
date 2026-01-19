# Endless

A village siege simulation built in Godot 4.5 using Data-Oriented Design (DOD) for high-performance NPC management. Inspired by DOTA-style gameplay where guards and raiders act as "creeps" fighting over resources.

## Overview

The world consists of 7 scattered towns, each with:
- **Town center** with farmers and guards
- **Farms** that generate food when worked
- **Raider camp** that sends raiders to steal food

The player observes (and can eventually influence) the ongoing conflict between towns and raiders.

## Current Features

### World Generation
- 7 procedurally placed towns with minimum distance constraints
- Each town has: 10 farmers, 5 guards, 2 farms, 1 raider camp (5 raiders)
- Towns have unique names (Millbrook, Ashford, Willowdale, etc.)
- World size: 4000x3000

### NPC System (Data-Oriented Design)
- Supports 3000+ NPCs at 60 FPS
- All NPC data stored in parallel PackedArrays for cache efficiency
- MultiMesh rendering with shader-based sprite atlas sampling
- Spatial grid for O(1) neighbor queries
- Staggered updates to distribute CPU load

### Combat
- Guards and raiders fight on sight
- Ranged projectile combat with blue (guard) and red (raider) projectiles
- Level scaling: sqrt-based stats (100x at level 9999), 50x max size
- XP granted on kills, with level-up notifications in combat log
- Farmers flee from combat

### AI Behaviors
- **Farmers**: Work farms during day, sleep at night, flee from enemies
- **Guards**: Patrol near town, engage raiders, day/night shifts
- **Raiders**: Steal food from farms, return to camp, retreat when wounded

### Food Economy
- Farmers generate 1 food/hour when working
- Raiders steal food from farms and deliver to camps
- Per-town and per-camp food tracking displayed in HUD

### UI
- HUD showing: unit counts, kills, time, FPS, zoom level
- Per-town food breakdown with town names
- Combat log for level-up events
- Settings menu (ESC) with HP bar visibility option
- Click NPCs to see detailed stats

### Time System
- Day/night cycle affecting NPC schedules
- Time controls: +/- to speed up/slow down, SPACE to pause
- 12-hour respawn timer for dead NPCs

## Architecture

```
main.gd                 # World generation, food tracking, game setup
autoloads/
  config.gd             # All tunable constants
  world_clock.gd        # Day/night cycle, time signals
  user_settings.gd      # Persistent user preferences
systems/
  npc_manager.gd        # Core NPC orchestration, data arrays
  npc_state.gd          # State machine logic
  npc_navigation.gd     # Movement, pathfinding, separation
  npc_combat.gd         # Scanning, targeting, damage
  npc_needs.gd          # Energy, schedules, raider AI
  npc_grid.gd           # Spatial partitioning
  npc_renderer.gd       # MultiMesh rendering, culling
  npc_sprite.gdshader   # Sprite atlas sampling, HP bars, tinting
  projectile_manager.gd # Projectile pooling and collision
entities/
  player.gd             # Camera controls, movement
world/
  location.gd           # Town/farm/camp markers
ui/
  hud.gd                # Stats display, combat log
  settings_menu.gd      # ESC menu
```

## Controls

| Key | Action |
|-----|--------|
| WASD / Arrows | Move camera |
| Mouse Wheel | Zoom in/out |
| Left Click | Select NPC |
| +/- | Speed up/slow down time |
| SPACE | Pause/unpause |
| ESC | Settings menu |

## Performance Notes

- Separation forces calculated as velocities, applied smoothly every frame
- Enemy scanning staggered across 8 frames
- Renderer tracks visible NPCs instead of scanning all 3000
- Combat log batches level-up messages to prevent frame freezes
- Grid cell size tuned for typical combat ranges

---

## TODO / Roadmap

### High Priority
- [ ] Win/lose conditions based on food totals
- [ ] Player combat abilities (tip the balance)
- [ ] Minimap showing town/camp locations
- [ ] Sound effects for combat, level-ups

### Gameplay
- [ ] Food consumption (NPCs eat from their faction's supply)
- [ ] Starvation effects when food runs out
- [ ] Raider confidence system (attack in groups, retreat when outnumbered)
- [ ] Guard alert system (call reinforcements)
- [ ] Farmer defense (hide in homes during raids)

### World
- [ ] Different town sizes (small/medium/large)
- [ ] Roads connecting towns
- [ ] Terrain features (forests, rivers, mountains)
- [ ] Day/night visual changes

### NPCs
- [ ] More NPC types (merchants, healers, archers)
- [ ] Equipment/weapons affecting stats
- [ ] NPC names and personalities
- [ ] Formation movement for groups

### UI/UX
- [ ] Town info panel on click
- [ ] Battle notifications ("Millbrook is under attack!")
- [ ] Statistics screen (graphs over time)
- [ ] Tutorial/help overlay

### Technical
- [ ] Save/load game state
- [ ] Performance profiling tools
- [ ] Unit tests for combat math
- [ ] Config file for easy tuning

### Polish
- [ ] Particle effects (dust, blood, magic)
- [ ] Death animations
- [ ] Building sprites for towns
- [ ] Weather effects

---

## Configuration

All tunable values are in `autoloads/config.gd`:

```gdscript
# NPC counts per town
FARMERS_PER_TOWN := 10
GUARDS_PER_TOWN := 5
RAIDERS_PER_CAMP := 5

# Combat stats
GUARD_HP := 150.0
GUARD_DAMAGE := 15.0
GUARD_RANGE := 150.0
RAIDER_HP := 120.0
RAIDER_DAMAGE := 15.0
RAIDER_RANGE := 150.0

# World
WORLD_WIDTH := 4000
WORLD_HEIGHT := 3000
NUM_TOWNS := 7
```

## Credits

- Sprite assets: [Kenney Roguelike RPG Pack](https://kenney.nl/assets/roguelike-rpg-pack)
- Engine: Godot 4.5
