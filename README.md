# Endless

A game about fighting entropy. Raiders steal your food. Guards die in combat. Farms lie fallow. Everything tends toward chaos and collapse. Can you build something that lasts?

Built in Godot 4.5 using Data-Oriented Design (DOD) with Factorio-style optimizations for high-performance NPC management.

**Inspirations:**
- **Asimov's "The Last Question"** - entropy as the ultimate antagonist
- **Lords of the Realm 2** - assign villagers to roles, manage production, raise armies, conquer rival towns
  - Farming: villagers work fields → grain → rations, weather affects yield, starvation causes unrest
  - Balance farming vs other jobs (woodcutting, mining, blacksmithing, army)
- **RimWorld** - colonist needs, AI storytelling, emergent chaos
- **Factorio** - scale to thousands of entities, predicted rendering, dormant states, spatial partitioning

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
- [x] Guard posts (4 per town at corners of 6x6 grid)
- [x] Raider camps (positioned away from all towns)
- [x] Visible world border with corner markers
- [x] Destructible buildings (right-click slot → Destroy)
- [x] Build new structures (right-click empty slots - farms, beds, guard posts)
- [ ] Structure upgrades (increase output, capacity, defense)

### Economy
- [x] Food production (farmers generate 1 food/hour when working)
- [x] Food theft (radius derived from sprite size)
- [x] Food delivery (raiders return loot to camp)
- [x] Loot icon above raiders carrying food
- [x] Per-town and per-camp food tracking in HUD
- [x] Food consumption (NPCs eat only when energy < 10, rest otherwise)
- [x] Population caps per town (upgradeable)
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

### NPC Identity
- [x] Named NPCs (55 first × 100 last = 5,500 unique combinations)
- [x] Rename NPCs via inspector (✎ button)
- [x] Personality traits (40% chance, 9 types)
- [x] Trait effects:
  - **Brave**: Never flees
  - **Coward**: Flees at +20% higher HP
  - **Efficient**: +25% farm yield, 25% faster attacks
  - **Hardy**: +25% max HP
  - **Lazy**: -20% farm yield, 20% slower attacks
  - **Strong**: +25% damage
  - **Swift**: +25% move speed
  - **Sharpshot**: +25% attack range
  - **Berserker**: +50% damage below 50% HP
- [ ] Trait combinations

### AI Behaviors
- [x] Farmers: day/night work schedule, always flee to town center
- [x] Guards: patrol between all 6 posts (30min each), day/night shifts, flee to town center below 33% HP
- [x] Raiders: priority system (wounded → exhausted → deliver loot → steal), flee to camp below 50% HP
- [x] Energy system (sleep +12/hr, rest +5/hr, activity -6/hr)
- [x] HP regen (2/hr awake, 6/hr sleeping, 10x at fountain/camp with upgrade scaling)
- [x] Recovery system (fleeing NPCs heal until 75% before resuming)
- [x] 15-minute decision cycles (event-driven override on state changes)
- [x] Building arrival based on sprite size (not pixel coordinates)
- [x] Permadeath (dead NPCs free slots for new spawns)
- [x] Collision avoidance for all NPCs (stationary guards get pushed too)
- [ ] AI lords that expand and compete

### NPC States
Activity-specific states (no translation layer):

| State | Jobs | Description |
|-------|------|-------------|
| IDLE | All | Between decisions |
| RESTING | All | At home/camp, recovering |
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
- [x] Click to select and inspect NPCs (inspector with follow/copy)
- [x] Time controls (+/- speed, SPACE pause)
- [x] Settings menu (ESC) with HP bar modes, scroll speed, log filters
- [x] First town is player-controlled (click fountain for upgrades)
- [x] Guard upgrades: health, attack, range, size, speed (10 levels each)
- [x] Economy upgrades: farm yield, farmer HP, population caps
- [x] Utility upgrades: healing rate, food efficiency
- [x] Town management panel with population stats and spawn timer
- [x] Resizable combat log at bottom of screen
- [ ] Villager role assignment UI
- [x] Build structures via grid slots (farms 50, beds 10, guard posts 25 food)
- [ ] Train guards from population
- [ ] Equipment crafting
- [ ] Army recruitment and movement
- [ ] Attack and capture enemy towns

### Victory Condition
There is no victory. Only the endless struggle against entropy.

### Performance (supports 3000+ NPCs at 60 FPS)

**Factorio-inspired optimizations:**
- [x] Predicted movement rendering (logic every 2-30 frames, render interpolates)
- [x] Dormant states (stationary NPCs skip navigation entirely)
- [x] Spatial threat registration (skip enemy scans when no threats in cell)
- [x] Event-driven wake-ups (state changes force immediate logic update)
- [x] LOD-based intervals (combat 2f, moving 5f, idle 30f, distance multiplied)

**Core architecture:**
- [x] Data-Oriented Design with PackedArrays
- [x] MultiMesh rendering (single draw call per layer)
- [x] Spatial grid for O(1) neighbor queries (64x64 cells)
- [x] Camera culling (only render visible NPCs)
- [x] Staggered scanning (1/8 NPCs per frame for combat)
- [x] Independent separation stagger (1/4 NPCs per frame for collision)
- [x] TCP-like collision avoidance (head-on, crossing, overtaking)
- [x] Combat log batching

**Visual effects:**
- [x] Golden halo shader for fountain/camp healing
- [x] Sleep "z" indicator for resting NPCs
- [x] Loot icon for raiders carrying food

---

## Architecture

```
main.gd                 # World generation, food tracking, game setup
autoloads/
  config.gd             # All tunable constants
  world_clock.gd        # Day/night cycle, time signals
  user_settings.gd      # Persistent user preferences
systems/
  npc_manager.gd        # Core orchestration, 30+ parallel data arrays
  npc_state.gd          # Activity-specific states, validation per job
  npc_navigation.gd     # Predicted movement, LOD intervals, separation
  npc_combat.gd         # Threat detection, targeting, damage, leashing
  npc_needs.gd          # Energy, schedules, decision trees
  npc_grid.gd           # Spatial partitioning (64x64 cells)
  npc_renderer.gd       # MultiMesh rendering, culling, indicators
  projectile_manager.gd # Projectile pooling, collision
entities/
  player.gd             # Camera controls
world/
  location.gd           # Sprite definitions, interaction radii
ui/
  left_panel.gd         # Stats, performance, NPC inspector (collapsible)
  combat_log.gd         # Resizable event log at bottom
  settings_menu.gd      # Options menu with log filters
  upgrade_menu.gd       # Town management, upgrades, population caps
  build_menu.gd         # Grid slot building (farms, beds)
```

## Controls

| Key | Action |
|-----|--------|
| WASD / Arrows | Move camera |
| Mouse Wheel | Zoom (centers on cursor) |
| Left Click (NPC) | Select and inspect |
| Left Click (fountain) | Open upgrade menu |
| + / = | Speed up time (2x) |
| - | Slow down time (0.5x) |
| SPACE | Pause/unpause |
| R | Roster panel (view all guards/farmers) |
| B | Build menu (on empty grid slots) |
| ESC | Settings menu |

## Configuration

Key values in `autoloads/config.gd`:

| Setting | Value | Notes |
|---------|-------|-------|
| FARMERS_PER_TOWN | 10 | Starting farmers |
| GUARDS_PER_TOWN | 60 | Starting guards |
| MAX_FARMERS_PER_TOWN | 10 | Population cap (upgradeable +2/level) |
| MAX_GUARDS_PER_TOWN | 60 | Population cap (upgradeable +10/level) |
| RAIDERS_PER_CAMP | 15 | Enemy forces |
| GUARD_POSTS_PER_TOWN | 6 | Patrol points |
| WORLD_SIZE | 8000x8000 | Play area |
| MAX_NPC_COUNT | 3000 | Engine limit |
| ENERGY_STARVING | 10 | Eat food threshold |
| ENERGY_HUNGRY | 50 | Go home threshold |

## Credits

- Sprite assets: [Kenney Roguelike RPG Pack](https://kenney.nl/assets/roguelike-rpg-pack)
- Engine: Godot 4.5
