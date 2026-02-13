# Endless

One town. A few farmers. Raiders at the gate.

Grow food. Train guards. Build walls. Raise an army. Conquer the map. Then conquer the next one.

A real-time kingdom builder inspired by [Lords of the Realm 2](https://en.wikipedia.org/wiki/Lords_of_the_Realm_II) meets [Factorio](https://factorio.com)-scale simulation — tens of thousands of NPCs, each with a job, a home, and a personality.

<!-- TODO: Add screenshot/GIF here -->

> *Early development — core loop works: farming, combat, upgrades, building, policies. [Roadmap](docs/roadmap.md)*

## Getting Started

Requires [Rust](https://rustup.rs/) and a GPU with Vulkan, DX12, or Metal support.

```bash
git clone https://github.com/abix-/endless.git
cd endless/rust
cargo run --release
```

<details>
<summary>Platform notes</summary>

**Windows:** Requires [Visual Studio C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/).

**Linux:** Install Bevy dependencies:
```bash
# Ubuntu/Debian
sudo apt install g++ pkg-config libx11-dev libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev
# Fedora
sudo dnf install gcc-c++ libX11-devel alsa-lib-devel systemd-devel wayland-devel libxkbcommon-devel
```

First build takes 2-5 minutes. Subsequent builds ~15 seconds.
</details>

## Controls

| Key | Action |
|-----|--------|
| WASD | Move camera |
| Scroll | Zoom |
| Left Click | Select NPC / building |
| Right Click | Build menu |
| Space | Pause |
| +/- | Time speed |
| R / U / P / T / Q | Roster / Upgrades / Policies / Patrols / Squads |
| F | Follow selected NPC |
| ESC | Settings |

## Architecture

Built with [Bevy 0.18](https://bevyengine.org/) ECS + GPU compute shaders (WGSL). 50K NPC capacity via instanced rendering in a single draw call. See [docs/](docs/README.md) for details.

## Credits

- Engine: [Bevy 0.18](https://bevyengine.org/)
- Sprites: [Kenney Roguelike RPG Pack](https://kenney.nl/assets/roguelike-rpg-pack)
