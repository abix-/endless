# Endless

One town. A few farmers. Raiders at the gate.

Grow food. Train guards. Build walls. Raise an army. Take the county. Take the world.

A real-time kingdom builder inspired by [Lords of the Realm 2](https://en.wikipedia.org/wiki/Lords_of_the_Realm_II) meets [Factorio](https://factorio.com)-scale simulation — tens of thousands of NPCs, each with a job, a home, and a personality.

<!-- TODO: Add screenshot/GIF here -->

> *Early development — core loop works: farming, combat, upgrades, building, policies. [Roadmap](docs/roadmap.md)*

## Download (v0.1)

**Windows build:** https://github.com/abix-/endless/releases/tag/v0.1

1. Download the `.zip` from the release page.
2. Extract it.
3. Run `Endless.exe`

## Getting Started (Build from Source)

Requires [Rust](https://rustup.rs/) and a GPU with Vulkan, DX12, or Metal support. First build takes 2-5 minutes. Subsequent builds ~15 seconds.

### Windows

1. Install [Rust](https://rustup.rs/) + [Visual Studio C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/).

```bash
git clone https://github.com/abix-/endless.git
cd endless/rust
cargo run --release
````

### macOS

1. Install [Rust](https://rustup.rs/). Xcode Command Line Tools install automatically if missing.

```bash
git clone https://github.com/abix-/endless.git
cd endless/rust
cargo run --release
```

### Linux

1. Install [Rust](https://rustup.rs/) + Bevy dependencies:

   ```bash
   # Ubuntu/Debian
   sudo apt install g++ pkg-config libx11-dev libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev
   # Fedora
   sudo dnf install gcc-c++ libX11-devel alsa-lib-devel systemd-devel wayland-devel libxkbcommon-devel
   ```

```bash
git clone https://github.com/abix-/endless.git
cd endless/rust
cargo run --release
```

## Controls

| Key               | Action                                          |
| ----------------- | ----------------------------------------------- |
| WASD              | Move camera                                     |
| Scroll            | Zoom                                            |
| Left Click        | Select NPC / building                           |
| Right Click       | Build menu                                      |
| Space             | Pause                                           |
| +/-               | Time speed                                      |
| R / U / P / T / Q | Roster / Upgrades / Policies / Patrols / Squads |
| F                 | Follow selected NPC                             |
| ESC               | Settings                                        |

## Inspirations

- **Lords of the Realm 2** — assign villagers to roles, manage production, raise armies, conquer rival towns
- **Factorio** — scale to thousands of entities, the satisfaction of watching systems hum
- **RimWorld** — colonist needs, emergent chaos, stories that write themselves
- **Asimov's "The Last Question"** — entropy as the ultimate antagonist

## Architecture

Built with [Bevy 0.18](https://bevyengine.org/) ECS + GPU compute shaders (WGSL). 50K NPC capacity via instanced rendering in a single draw call. See [docs/](docs/README.md) for details.

## Credits

- Engine: [Bevy 0.18](https://bevyengine.org/)
- Sprites: [Kenney Roguelike RPG Pack](https://kenney.nl/assets/roguelike-rpg-pack)
