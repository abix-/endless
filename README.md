# Endless

A game about fighting entropy. Raiders steal your food. Guards die in combat. Farms lie fallow. Everything tends toward chaos and collapse. Can you build something that lasts?

Built with Bevy 0.18 ECS and GPU compute shaders (WGSL). Data-oriented design with Factorio-style optimizations — 50K NPCs via GPU instanced rendering in a single draw call.

## Getting Started

### Windows

1. Install [Rust](https://rustup.rs/) (includes `cargo`). Requires [Visual Studio C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/).
2. GPU with Vulkan or DX12 support (any dedicated GPU from the last 10 years, most integrated GPUs).

```bash
git clone https://github.com/abix-/endless.git
cd endless/rust
cargo run --release
```

### macOS

1. Install [Rust](https://rustup.rs/) (includes `cargo`). Xcode Command Line Tools will be installed automatically if missing.
2. Apple Silicon or Intel Mac with Metal support (2012+).

```bash
git clone https://github.com/abix-/endless.git
cd endless/rust
cargo run --release
```

### Linux

1. Install [Rust](https://rustup.rs/) and system dependencies for Bevy:
   ```bash
   # Ubuntu/Debian
   sudo apt install g++ pkg-config libx11-dev libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev
   # Fedora
   sudo dnf install gcc-c++ libX11-devel alsa-lib-devel systemd-devel wayland-devel libxkbcommon-devel
   ```
2. GPU with Vulkan support and up-to-date drivers.

```bash
git clone https://github.com/abix-/endless.git
cd endless/rust
cargo run --release
```

First build takes 2-5 minutes (compiles Bevy + dependencies). Subsequent builds are ~15 seconds.

## The Struggle

1. **Produce** - Farmers generate food. Without food, nothing else matters.
2. **Defend** - Guards protect what you've built. Raiders want it.
3. **Upgrade** - Invest food to make guards stronger. Trade present resources for future survival.
4. **Expand** - Claim neutral towns. More territory, more production, more to defend.
5. **Endure** - Entropy never stops. Neither can you.

## Controls

| Key | Action |
|-----|--------|
| WASD / Arrows | Move camera |
| Mouse Wheel | Zoom (centers on cursor) |
| Left Click (NPC) | Select and inspect NPC |
| Left Click (building) | Select and inspect building |
| Right Click (slot) | Build menu / unlock slot / turret toggle |
| + / = | Speed up time (2x) |
| - | Slow down time (0.5x) |
| SPACE | Pause/unpause |
| R | Roster panel (view all guards/farmers) |
| F | Follow selected NPC |
| T | Patrols panel (reorder guard post routes) |
| B | Build menu panel |
| U | Upgrade menu |
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
