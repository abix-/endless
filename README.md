# Endless

Build a town under siege. Farm, fortify, raise an army, conquer

Every NPC has a job, a home, and a personality. Set policies, build walls, lay roads, organize squads

Your AI rivals do the same — each with a distinct strategy

<!-- TODO: Add screenshot/GIF here -->

[Roadmap](docs/roadmap.md)

## Download

- **Windows:** [endless-windows-v0.1.5.zip](https://github.com/abix-/endless/releases/download/v0.1.5/endless-windows-v0.1.5.zip) — extract, run `endless.exe`
- **macOS:** [endless-macos-v0.1.5.zip](https://github.com/abix-/endless/releases/download/v0.1.5/endless-macos-v0.1.5.zip) — extract, run `endless`
- **Linux:** [endless-linux-v0.1.5.zip](https://github.com/abix-/endless/releases/download/v0.1.5/endless-linux-v0.1.5.zip) — extract, run `endless`

## Controls

| Default Key       | Action                                          |
| ----------------- | ----------------------------------------------- |
| WASD              | Move camera                                     |
| Scroll            | Zoom                                            |
| Left Click        | Select NPC / building (when not placing)        |
| B / Build button  | Open build palette                              |
| Left Click        | Place selected building                         |
| Right Click / ESC | Cancel current building placement               |
| Space             | Pause                                           |
| +/-               | Time speed (`0x`, `0.25x`-`128x`)               |
| R / U / P / T / Q | Roster / Upgrades / Policies / Patrols / Squads |
| 1-9 / 0           | Squad 1-10 target mode                          |
| F5                | Quicksave                                       |
| F9                | Quickload                                       |
| L                 | Toggle combat log                               |
| F                 | Follow selected NPC                             |
| ESC               | Pause / Settings                                |

All keyboard shortcuts are rebindable in `ESC > Settings > Controls`.

## Inspirations

- **Lords of the Realm 2** - assign villagers to roles, manage production, raise armies, conquer rival towns
- **Factorio** - scale to thousands of entities, the satisfaction of watching systems hum
- **RimWorld** - colonist needs, emergent chaos, stories that write themselves
- **Asimov's "The Last Question"** - entropy as the ultimate antagonist

## Architecture

Built with [Bevy 0.18](https://bevyengine.org/) ECS + GPU compute shaders (WGSL). 50K NPC capacity via instanced rendering in a single draw call. See [docs/](docs/README.md) for details.

### Debugging Target Flips

- Enable the Profiler tab to view `NPC Target Thrash (sink, 1s window)` for live per-second target-change diagnostics.
- Selected NPC debug info includes sink metrics (`SinkTargetChanges/s`, `SinkPingPong/s`, `SinkTargetWrites/s`) and raw `SinkPrevTarget -> SinkLastTarget`.

## Credits

- Engine: [Bevy 0.18](https://bevyengine.org/)
- Sprites: [Kenney Roguelike RPG Pack](https://kenney.nl/assets/roguelike-rpg-pack)
- Music: [Not Jam Music Pack](https://not-jam.itch.io/not-jam-music-pack)
- SFX: [Death Sounds (Male) Audio Pack](https://opengameart.org/content/death-sounds-male-audio-pack)
- SFX: [Bow / Arrow Shot](https://opengameart.org/content/bow-arrow-shot)
