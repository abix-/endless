# Endless

A game about fighting entropy. Raiders steal your food. Guards die in combat. Farms lie fallow. Everything tends toward chaos and collapse. Can you build something that lasts?

Built with Bevy 0.18 ECS and GPU compute shaders (WGSL). Data-oriented design with Factorio-style optimizations — 16K NPCs via GPU instanced rendering in a single draw call.

## Getting Started

**Prerequisites:** [Rust 1.93+](https://rustup.rs/), GPU with Vulkan or DX12 support

```bash
git clone https://github.com/abix-/endless.git
cd endless/rust
cargo run --release
```

## The Struggle

1. **Produce** - Farmers generate food. Without food, nothing else matters.
2. **Defend** - Guards protect what you've built. Raiders want it.
3. **Upgrade** - Invest food to make guards stronger. Trade present resources for future survival.
4. **Expand** - Claim neutral towns. More territory, more production, more to defend.
5. **Endure** - Entropy never stops. Neither can you.

## Controls (Planned)

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

## Architecture

See [docs/](docs/README.md) for architecture documentation — system maps, file tree, data flow, GPU buffers, combat pipeline, behavior state machines, and known issues with ratings.

## Inspirations

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

## Credits

- Engine: [Bevy 0.18](https://bevyengine.org/)
- Sprites: [Kenney Roguelike RPG Pack](https://kenney.nl/assets/roguelike-rpg-pack)
