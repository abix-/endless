# Endless

You start with one town, a handful of farmers, and a fence that won't hold. Raiders are already coming.

Grow food. Train guards. Build walls. Research technology. Forge alliances — or break them. Raise an army, march across the map, and conquer every settlement on the continent. Then do it again on the next one.

A real-time kingdom builder inspired by **Lords of the Realm 2**, built to scale to tens of thousands of NPCs. Every villager has a job, a home, energy, personality, and opinions about whether they'd rather fight or flee. Every decision you make — who farms, who fights, where the walls go — ripples through the simulation.

*Early development. Core loop works: farming, combat, upgrades, building, policies. Conquest arc in progress.*

<!-- TODO: Add screenshot/GIF here -->

## The Struggle

1. **Produce** — Farmers work fields. Food feeds your people, fuels upgrades, and funds expansion. Without it, everything collapses.
2. **Defend** — Raiders steal your food and kill your people. Guards patrol, walls channel, turrets fire. Layout matters.
3. **Upgrade** — Spend food to research tech, improve stats, unlock new buildings. Every investment is food your people aren't eating.
4. **Expand** — Build more houses and barracks. Grow your economy. Claim territory beyond your walls.
5. **Conquer** — Form armies from your guards. March them to enemy camps. Siege, capture, and rule. One town becomes two. Two becomes a county. A county becomes the world.
6. **Endure** — Entropy never stops. Raiders escalate. Allies betray. The frontier always needs more than you have. Can you build something that lasts?

## Inspirations

- **Lords of the Realm 2** — assign villagers to roles, manage production, raise armies, conquer rival towns
- **Factorio** — scale to thousands of entities, the satisfaction of watching systems hum
- **RimWorld** — colonist needs, emergent chaos, stories that write themselves
- **Asimov's "The Last Question"** — entropy as the ultimate antagonist

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
| Q | Squads panel |
| ESC | Settings menu |

## Architecture

Built with [Bevy 0.18](https://bevyengine.org/) ECS and GPU compute shaders (WGSL). Data-oriented design with Factorio-style optimizations — 50K NPC buffer capacity via GPU instanced rendering in a single draw call.

See [docs/](docs/README.md) for architecture documentation — system maps, file tree, data flow, GPU buffers, combat pipeline, behavior state machines, and known issues with ratings.

## Credits

- Engine: [Bevy 0.18](https://bevyengine.org/)
- Sprites: [Kenney Roguelike RPG Pack](https://kenney.nl/assets/roguelike-rpg-pack)
