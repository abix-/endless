# Endless

Build a town under siege—farm, fortify, raise an army, and conquer

Every NPC has a job, a home, and a personality. Set town policies, plan your tech tree, organize squads — and watch AI rivals do the same

<!-- TODO: Add screenshot/GIF here -->

[Roadmap](docs/roadmap.md)*

## Download

- **Windows:** [endless-windows-v0.1.1.zip](https://github.com/abix-/endless/releases/download/v0.1.1/endless-windows-v0.1.1.zip) — extract, run `endless.exe`
- **macOS:** [endless-macos-v0.1.1.zip](https://github.com/abix-/endless/releases/download/v0.1.1/endless-macos-v0.1.1.zip) — extract, run `endless`
- **Linux:** [endless-linux-v0.1.1.zip](https://github.com/abix-/endless/releases/download/v0.1.1/endless-linux-v0.1.1.zip) — extract, run `endless`

## Controls

| Key               | Action                                          |
| ----------------- | ----------------------------------------------- |
| WASD              | Move camera                                     |
| Scroll            | Zoom                                            |
| Left Click        | Select NPC / building (when not placing)        |
| B / Build button  | Open build palette                              |
| Left Click        | Place selected building                         |
| Right Click / ESC | Cancel current building placement               |
| Space             | Pause                                           |
| +/-               | Time speed                                      |
| R / U / P / T / Q | Roster / Upgrades / Policies / Patrols / Squads |
| F5                | Quicksave                                       |
| F9                | Quickload                                       |
| L                 | Toggle combat log                               |
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
- Music: [Not Jam Music Pack](https://not-jam.itch.io/not-jam-music-pack)



