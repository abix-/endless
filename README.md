# Endless

A village siege simulation built in Godot 4.5 using Data-Oriented Design (DOD) for high-performance NPC management. Inspired by Lords of the Realm 2 - build your economy, raise armies, conquer the world.

## Features

### World
- [x] Procedural town generation (7 towns, 1200px minimum spacing)
- [x] Named towns from pool of 15 (Millbrook, Ashford, Willowdale, etc.)
- [x] Farms (2 per town, 200-300px from center)
- [x] Homes for NPCs (ring 350-450px from center)
- [x] Guard posts (6 per town, 500-600px perimeter)
- [x] Raider camps (900px from target town)
- [ ] Destructible buildings
- [ ] Build new structures

### Economy
- [x] Food production (farmers generate 1 food/hour when working)
- [x] Food theft (raiders steal within 100px of farm)
- [x] Food delivery (raiders return loot to camp)
- [x] Loot icon above raiders carrying food
- [x] Per-town and per-camp food tracking in HUD
- [ ] Food consumption (NPCs eat from faction supply)
- [ ] Starvation effects (HP drain, desertion)
- [ ] Multiple resources (wood, iron, gold)
- [ ] Production buildings (lumber mill, mine, blacksmith)

### Combat
- [x] Faction-based auto-targeting (villagers vs raiders)
- [x] Ranged projectile combat (guards, raiders)
- [x] Melee combat (farmers)
- [x] Level/XP system (sqrt scaling, level 9999 = 100x stats)
- [x] Damage flash effect
- [x] Leash system (farmers/raiders return home if combat drags 400px+)
- [x] Guards have no leash - fight anywhere
- [x] Alert nearby allies when combat starts
- [x] 500 projectile pool with faction coloring
- [ ] Player combat abilities
- [ ] Army units (peasant levy, archers, knights)

### AI Behaviors
- [x] Farmers: day/night work schedule, flee from enemies
- [x] Guards: patrol at guard posts, day/night shifts
- [x] Raiders: priority system (wounded → exhausted → deliver loot → steal)
- [x] Energy system (sleep +12/hr, rest +5/hr, activity -6/hr)
- [x] 15-minute decision cycles
- [ ] AI lords that expand and compete

### Player Controls
- [x] WASD camera movement
- [x] Mouse wheel zoom (0.1x - 4.0x, centers on cursor)
- [x] Click to select and inspect NPCs
- [x] Time controls (+/- speed, SPACE pause)
- [x] Settings menu (ESC)
- [ ] Claim a town as capital
- [ ] Villager role assignment UI
- [ ] Build/upgrade buildings
- [ ] Train guards from population
- [ ] Equipment crafting
- [ ] Army recruitment and movement
- [ ] Attack and capture enemy towns

### Victory Conditions
- [ ] Domination (conquer all towns)
- [ ] Economic (accumulate wealth threshold)
- [ ] Survival mode (endless waves)

### Performance (supports 3000+ NPCs at 60 FPS)
- [x] Data-Oriented Design with PackedArrays
- [x] MultiMesh rendering (single draw call)
- [x] Spatial grid for O(1) neighbor queries
- [x] LOD system (distant NPCs update less often)
- [x] Camera culling (only render visible NPCs)
- [x] Staggered scanning (1/8 NPCs per frame)
- [x] Combat log batching

---

## Architecture

```
main.gd                 # World generation, food tracking, game setup
autoloads/
  config.gd             # All tunable constants
  world_clock.gd        # Day/night cycle, time signals
  user_settings.gd      # Persistent user preferences
systems/
  npc_manager.gd        # Core NPC orchestration, data arrays
  npc_state.gd          # State machine, valid states per job
  npc_navigation.gd     # Movement, LOD, separation forces
  npc_combat.gd         # Scanning, targeting, damage, leashing
  npc_needs.gd          # Energy, schedules, raider AI
  npc_grid.gd           # Spatial partitioning (64x64)
  npc_renderer.gd       # MultiMesh rendering, culling
  projectile_manager.gd # Projectile pooling, collision
entities/
  player.gd             # Camera controls
world/
  location.gd           # Town/farm/camp markers
ui/
  hud.gd                # Stats, food tracking, combat log
  settings_menu.gd      # Options menu
```

## Controls

| Key | Action |
|-----|--------|
| WASD / Arrows | Move camera |
| Mouse Wheel | Zoom (centers on cursor) |
| Left Click | Select NPC |
| + / = | Speed up time (2x) |
| - | Slow down time (0.5x) |
| SPACE | Pause/unpause |
| ESC | Settings menu |

## Configuration

Key values in `autoloads/config.gd`:

| Setting | Value | Notes |
|---------|-------|-------|
| FARMERS_PER_TOWN | 10 | Food producers |
| GUARDS_PER_TOWN | 30 | Town defense |
| RAIDERS_PER_CAMP | 30 | Enemy forces |
| GUARD_POSTS_PER_TOWN | 6 | Patrol points |
| WORLD_SIZE | 6000x4500 | Play area |
| MAX_NPC_COUNT | 3000 | Engine limit |

## Credits

- Sprite assets: [Kenney Roguelike RPG Pack](https://kenney.nl/assets/roguelike-rpg-pack)
- Engine: Godot 4.5
