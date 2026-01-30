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
  - The satisfaction of watching your creation work — farmers farming, guards patrolling, systems humming
  - Content with where you are, but always knowing there's more to build
- **Animal Crossing** - existence is the game, not winning
  - NPCs have their own lives, schedules, personalities
  - The world is worth inhabiting, not just optimizing

## The Struggle

1. **Produce** - Farmers generate food. Without food, nothing else matters.
2. **Defend** - Guards protect what you've built. Raiders want it.
3. **Upgrade** - Invest food to make guards stronger. Trade present resources for future survival.
4. **Expand** - Claim neutral towns. More territory, more production, more to defend.
5. **Endure** - Entropy never stops. Neither can you.

## Architecture

See [docs/](docs/README.md) for architecture documentation — system maps, file tree, data flow, GPU buffers, combat pipeline, behavior state machines, and known issues with ratings.

## Controls

| Key | Action |
|-----|--------|
| WASD / Arrows | Move camera |
| Mouse Wheel | Zoom (centers on cursor) |
| Left Click (NPC) | Select and inspect |
| Left Click (Farm) | Show farm occupancy |
| Right Click (slot) | Build menu / unlock slot |
| + / = | Speed up time (2x) |
| - | Slow down time (0.5x) |
| SPACE | Pause/unpause |
| R | Roster panel (view all guards/farmers) |
| P | Policies panel (faction settings) |
| ESC | Settings menu |

## Configuration

Key values in `autoloads/config.gd`:

| Setting | Value | Notes |
|---------|-------|-------|
| FARMERS_PER_TOWN | 2 | Starting farmers per town (configurable via start menu) |
| GUARDS_PER_TOWN | 4 | Starting guards per town (configurable via start menu) |
| MAX_FARMERS_PER_TOWN | 2 | Population cap (upgradeable +2/level) |
| MAX_GUARDS_PER_TOWN | 4 | Population cap (upgradeable +10/level) |
| RAIDERS_PER_CAMP | 6 | Raiders per camp (configurable via start menu) |
| GUARD_POSTS_PER_TOWN | 4 | Patrol points (clockwise corners) |
| WORLD_SIZE | 8000x8000 | Play area |
| MAX_NPC_COUNT | 3000 | Engine limit |
| ENERGY_STARVING | 10 | Eat food threshold |
| ENERGY_HUNGRY | 50 | Go home threshold |

## Credits

- Engine: [Godot 4.5](https://godotengine.org/)
- ECS: [Bevy](https://bevyengine.org/)
- Integration: [godot-bevy](https://github.com/bytemeadow/godot-bevy)
- Sprites: [Kenney Roguelike RPG Pack](https://kenney.nl/assets/roguelike-rpg-pack)
