# Endless

A game about fighting entropy. Raiders steal your food. Guards die in combat. Farms lie fallow. Everything tends toward chaos and collapse. Can you build something that lasts?

Built in Godot 4.5 using Data-Oriented Design (DOD) for high-performance NPC management.

**Inspirations:**
- **Asimov's "The Last Question"** - entropy as the ultimate antagonist
- **Lords of the Realm 2** - assign villagers to roles, manage production, raise armies, conquer rival towns
  - Farming: villagers work fields → grain → rations, weather affects yield, starvation causes unrest
  - Balance farming vs other jobs (woodcutting, mining, blacksmithing, army)
- **RimWorld** - colonist needs, AI storytelling, emergent chaos
- **Factorio** - scale to thousands of entities, optimize production chains

## The Struggle

1. **Produce** - Farmers generate food. Without food, nothing else matters.
2. **Defend** - Guards protect what you've built. Raiders want it.
3. **Upgrade** - Invest food to make guards stronger. Trade present resources for future survival.
4. **Expand** - Claim neutral towns. More territory, more production, more to defend.
5. **Endure** - Entropy never stops. Neither can you.

## Features

### World
- [x] Procedural town generation (7 towns, 1200px minimum spacing)
- [x] Named towns from pool of 15 Florida cities (Miami, Orlando, Tampa, etc.)
- [x] Farms (2 per town, 200-300px from center)
- [x] Homes for farmers (ring 350-450px from center)
- [x] Guard posts (6 per town, 500-600px perimeter)
- [x] Raider camps (positioned away from all towns)
- [x] Visible world border with corner markers
- [ ] Destructible buildings
- [ ] Build new structures
- [ ] Structure upgrades (increase output, capacity, defense)

### Economy
- [x] Food production (farmers generate 1 food/hour when working)
- [x] Food theft (radius derived from sprite size)
- [x] Food delivery (raiders return loot to camp)
- [x] Loot icon above raiders carrying food
- [x] Per-town and per-camp food tracking in HUD
- [ ] Food consumption (NPCs eat from faction supply)
- [ ] Starvation effects (HP drain, desertion)
- [ ] Multiple resources (wood, iron, gold)
- [ ] Production buildings (lumber mill, mine, blacksmith)

### Combat
- [x] Faction-based auto-targeting (villagers vs raiders)
- [x] Ranged projectile combat (guards and raiders have equal stats: 120 HP, 15 dmg, 150 range)
- [x] Melee combat (farmers)
- [x] Level/XP system (sqrt scaling, level 9999 = 100x stats)
- [x] Damage flash effect
- [x] Leash system (farmers/raiders return home if combat drags 400px+)
- [x] Guards have no leash - fight anywhere
- [x] Alert nearby allies when combat starts
- [x] Target switching (stop chasing fleeing enemies if closer threat exists)
- [x] 500 projectile pool with faction coloring
- [ ] Player combat abilities
- [ ] Army units (peasant levy, archers, knights)

### AI Behaviors
- [x] Farmers: day/night work schedule, always flee to town center
- [x] Guards: patrol between all 6 posts (30min each), day/night shifts, flee to town center below 33% health
- [x] Raiders: priority system (wounded → exhausted → deliver loot → steal), flee to camp below 50% health
- [x] Energy system (sleep +12/hr, rest +5/hr, activity -6/hr)
- [x] HP regen (2/hr awake, 6/hr sleeping, 10x at fountain/camp)
- [x] Recovery system (fleeing NPCs heal until 75% before resuming)
- [x] 15-minute decision cycles
- [x] Building arrival based on sprite size (not pixel coordinates)
- [ ] AI lords that expand and compete

### NPC States
Activity-specific states (no translation layer):

| State | Jobs | Description |
|-------|------|-------------|
| IDLE | All | Between decisions |
| SLEEPING | All | At home/camp, asleep |
| OFF_DUTY | All | At home/camp, awake |
| FIGHTING | Guard, Raider | In combat |
| FLEEING | All | Running from combat |
| WALKING | Farmer, Guard | Moving (to farm/home) |
| FARMING | Farmer | Working at farm |
| ON_DUTY | Guard | Stationed at post |
| PATROLLING | Guard | Moving between posts |
| RAIDING | Raider | Going to/at farm |
| RETURNING | Raider | Heading back to camp |

### Player Controls
- [x] WASD camera movement (configurable speed 100-2000)
- [x] Mouse wheel zoom (0.1x - 4.0x, centers on cursor)
- [ ] Click to select and inspect NPCs
- [x] Time controls (+/- speed, SPACE pause)
- [x] Settings menu (ESC) with HP bar modes, scroll speed
- [x] First town is player-controlled (click fountain for upgrades)
- [x] Guard upgrades: health, attack, range, size (10 levels each, costs food)
- [ ] Villager role assignment UI
- [ ] Build/upgrade buildings
- [ ] Train guards from population
- [ ] Equipment crafting
- [ ] Army recruitment and movement
- [ ] Attack and capture enemy towns

### Victory Condition
There is no victory. Only the endless struggle against entropy.

### Performance (supports 3000+ NPCs at 60 FPS)
- [x] Data-Oriented Design with PackedArrays
- [x] MultiMesh rendering (single draw call)
- [x] Spatial grid for O(1) neighbor queries
- [x] LOD system (distant NPCs update less often)
- [x] Camera culling (only render visible NPCs)
- [x] Staggered scanning (1/8 NPCs per frame)
- [x] Combat log batching
- [x] TCP-like collision avoidance (head-on, crossing, overtaking - index-based symmetry breaking)
- [x] Golden halo shader effect for NPCs receiving healing bonus

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
  npc_state.gd          # Activity-specific states, validation per job
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
  upgrade_menu.gd       # Town upgrade UI
```

## Controls

| Key | Action |
|-----|--------|
| WASD / Arrows | Move camera |
| Mouse Wheel | Zoom (centers on cursor) |
| Left Click (fountain) | Open upgrade menu |
| + / = | Speed up time (2x) |
| - | Slow down time (0.5x) |
| SPACE | Pause/unpause |
| ESC | Settings menu |

## Configuration

Key values in `autoloads/config.gd`:

| Setting | Value | Notes |
|---------|-------|-------|
| FARMERS_PER_TOWN | 10 | Food producers |
| GUARDS_PER_TOWN | 60 | Town defense |
| RAIDERS_PER_CAMP | 30 | Enemy forces |
| GUARD_POSTS_PER_TOWN | 6 | Patrol points |
| WORLD_SIZE | 8000x8000 | Play area |
| MAX_NPC_COUNT | 3000 | Engine limit |

## Credits

- Sprite assets: [Kenney Roguelike RPG Pack](https://kenney.nl/assets/roguelike-rpg-pack)
- Engine: Godot 4.5
