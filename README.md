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
- 7 procedurally placed towns with minimum 800px distance between centers
- Each town has: 10 farmers, 5 guards, 2 farms, 1 raider camp (5 raiders)
- Towns have unique names from pool of 15 (Millbrook, Ashford, Willowdale, etc.)
- World size: 4000x3000 with 300px margin from edges
- Farms placed 80-150px from town center, homes in ring 150-250px out
- Raider camps placed 500px from their target town

### NPC System (Data-Oriented Design)
- Supports 3000+ NPCs at 60 FPS
- All NPC data stored in parallel PackedArrays for cache efficiency
- MultiMesh rendering with shader-based sprite atlas sampling
- Spatial grid (64x64 cells, 100px each) for O(1) neighbor queries
- Staggered updates to distribute CPU load across frames

### Combat
- Guards and raiders fight on sight (scan every 0.2s, staggered across 8 frames)
- Ranged projectile combat: blue (guards), red (raiders) - 500 projectile pool
- Farmers do melee damage and flee when threatened
- Level scaling: sqrt-based stats (level 9999 = 100x damage/HP), 50x visual size
- XP system: killing grants XP equal to victim's level
- Level-up notifications batched in combat log
- Damage flash effect (white overlay, fades in ~0.12s)
- Leash system: farmers/raiders return home if combat drags too far (400px, raiders 1.5x)
- Guards have no leash - they fight anywhere

### AI Behaviors

**Farmers:**
- Work farms during day (6 AM - 8 PM), sleep at night
- Flee from enemies (until 150px away)
- Generate 1 food/hour when in WORKING state

**Guards:**
- Patrol near town center, engage raiders on sight
- Day/night shifts (randomly assigned)
- Alert nearby guards when combat starts

**Raiders:**
- Priority 1: Retreat to camp when wounded (<50% HP), drop any food
- Priority 2: Sleep when exhausted (<20 energy)
- Priority 3: Return to camp if carrying stolen food
- Priority 4: Go steal food from random farm
- Alert nearby raiders (200px) when one starts fighting

### Energy System
- Max energy: 100
- Sleeping: +12/hour
- Resting: +5/hour
- Active (walking, working, fighting): -6/hour
- Exhausted threshold: 20 (triggers sleep)
- Successful raid delivery: +30 energy

### Food Economy
- Farmers generate 1 food/hour when working at farms
- Raiders steal food when within 60px of farm
- Raiders deliver food to camp, credited to camp's total
- Per-town and per-camp food tracking displayed in HUD

### LOD System (Level of Detail)
NPCs update at different rates based on camera distance:
| Distance | Update Rate | Delta Multiplier |
|----------|-------------|------------------|
| < 400px | Every frame | 1.0x |
| 400-800px | Every 2 frames | 2.0x |
| 800-1200px | Every 4 frames | 4.0x |
| > 1200px | Every 8 frames | 8.0x |

### UI
- HUD showing: unit counts (alive/dead/kills), time, FPS, loop time, zoom
- Per-town food breakdown with town names and raider camp totals
- Combat log for level-up events (batched to prevent lag)
- Settings menu (ESC) with HP bar visibility option
- Click NPCs to see: job, level, HP, energy, current state
- Raiders carrying food show "Loot" status

### Time System
- Day: 6 AM - 8 PM, Night: 8 PM - 6 AM
- Default speed: 10 game minutes per real second
- Time controls: +/- to double/halve speed, SPACE to pause
- 12-hour (720 minute) respawn timer for dead NPCs
- NPC decisions reconsidered every 15 game minutes

### Rendering
- Sprite tinting: Farmers (green), Guards (blue), Raiders (red)
- HP bars in top 15% of sprite (green >50%, yellow >25%, red <25%)
- HP bars hidden at full health unless setting enabled
- Camera culling: only visible NPCs rendered (with 100px margin)

## Architecture

```
main.gd                 # World generation, food tracking, game setup
autoloads/
  config.gd             # All tunable constants (~50 values)
  world_clock.gd        # Day/night cycle, time signals
  user_settings.gd      # Persistent user preferences
systems/
  npc_manager.gd        # Core NPC orchestration, data arrays, signals
  npc_state.gd          # State machine logic, valid states per job
  npc_navigation.gd     # Movement, LOD updates, separation forces
  npc_combat.gd         # Scanning, targeting, damage, leashing
  npc_needs.gd          # Energy, schedules, raider AI priorities
  npc_grid.gd           # Spatial partitioning (64x64 grid)
  npc_renderer.gd       # MultiMesh rendering, culling, flash effects
  npc_sprite.gdshader   # Sprite atlas, HP bars, tinting, flash
  projectile_manager.gd # Projectile pooling (500), collision
entities/
  player.gd             # Camera controls, movement
world/
  location.gd           # Town/farm/camp markers with labels
ui/
  hud.gd                # Stats display, food tracking, combat log
  settings_menu.gd      # ESC menu, HP bar toggle
```

## Controls

| Key | Action |
|-----|--------|
| WASD / Arrows | Move camera |
| Mouse Wheel | Zoom in/out (0.1x - 4.0x) |
| Left Click | Select NPC |
| + / = | Speed up time (2x) |
| - | Slow down time (0.5x) |
| SPACE | Pause/unpause |
| ESC | Settings menu |

## NPC States

| State | Farmer | Guard | Raider |
|-------|--------|-------|--------|
| IDLE | Yes | Yes | Yes |
| WALKING | Yes | Yes | Yes |
| SLEEPING | Yes | Yes | Yes |
| WORKING | Yes | Yes | No |
| RESTING | Yes | Yes | Yes |
| WANDERING | No | Yes | Yes |
| FIGHTING | No | Yes | Yes |
| FLEEING | Yes | No | No |

## Performance Optimizations

- **MultiMesh**: All NPCs in single draw call
- **Spatial grid**: O(1) neighbor queries vs O(n) brute force
- **Staggered scanning**: 1/8 of NPCs check for enemies per frame
- **LOD updates**: Distant NPCs update less often with delta compensation
- **Velocity separation**: Calculate force every 4 frames, apply smoothly every frame
- **Camera culling**: Only render visible NPCs, track rendered set for efficient hiding
- **Combat log batching**: Accumulate level-ups, flush once per frame
- **Projectile pooling**: Reuse 500 projectiles instead of create/destroy

---

## TODO / Roadmap

- [ ] Player combat abilities (tip the balance like a DOTA hero)
- [ ] Food consumption (NPCs eat from their faction's supply)
- [ ] UI to show per-town food when clicking on town/camp markers
- [ ] Implement RAIDER_CONFIDENCE_THRESHOLD logic (config exists but unused)
- [ ] Implement RAIDER_HUNGRY_THRESHOLD logic (config exists but unused)

---

## Configuration

All tunable values are in `autoloads/config.gd`:

```gdscript
# NPC counts per town
FARMERS_PER_TOWN := 10
GUARDS_PER_TOWN := 5
RAIDERS_PER_CAMP := 5

# Combat stats
FARMER_HP := 50.0
FARMER_DAMAGE := 5.0
FARMER_RANGE := 30.0      # Melee
GUARD_HP := 150.0
GUARD_DAMAGE := 15.0
GUARD_RANGE := 150.0      # Ranged
RAIDER_HP := 120.0
RAIDER_DAMAGE := 15.0
RAIDER_RANGE := 150.0     # Ranged

# Combat distances
LEASH_DISTANCE := 400.0
FLEE_DISTANCE := 150.0
ALERT_RADIUS := 200.0

# Energy
ENERGY_MAX := 100.0
ENERGY_SLEEP_GAIN := 12.0
ENERGY_REST_GAIN := 5.0
ENERGY_ACTIVITY_DRAIN := 6.0
ENERGY_EXHAUSTED := 20.0

# World
WORLD_WIDTH := 4000
WORLD_HEIGHT := 3000
WORLD_MARGIN := 300
CAMP_DISTANCE := 500

# Performance
MAX_NPC_COUNT := 3000
MAX_PROJECTILES := 500
GRID_SIZE := 64
GRID_CELL_SIZE := 100.0
SCAN_STAGGER := 8
```

## Signals

| Signal | Source | Data |
|--------|--------|------|
| `time_tick` | WorldClock | hour, minute |
| `hour_changed` | WorldClock | hour |
| `day_changed` | WorldClock | day |
| `npc_leveled_up` | NPCManager | index, job, old_level, new_level |
| `raider_delivered_food` | NPCManager | town_idx |
| `arrived` | NPCNavigation | npc_index |
| `settings_changed` | UserSettings | (none) |

## Credits

- Sprite assets: [Kenney Roguelike RPG Pack](https://kenney.nl/assets/roguelike-rpg-pack)
- Engine: Godot 4.5
