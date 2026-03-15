# Audio

## Overview

Endless uses Bevy audio for two separate jobs: a jukebox-style music player and fire-and-forget sound effects. Runtime state lives in `GameAudio`; playback logic lives in `rust/src/systems/audio.rs`.

## Runtime State

`GameAudio` owns:

- `music_volume`
- `sfx_volume`
- `tracks`
- `last_track`
- `loop_current`
- `play_next`
- `music_speed`
- `sfx_handles`
- `sfx_shoot_enabled`

User-facing defaults come from `UserSettings`:

- music volume `0.3`
- SFX volume `0.15`
- arrow shoot SFX off by default
- jukebox loop off by default

## Music

`load_music()` loads every path in `MUSIC_TRACKS` at startup. The current soundtrack contains 22 CC0 tracks under `assets/sounds/music/not-jam-music/`.

`start_music()` runs on entry to `Playing`:

- syncs volume, speed, loop mode, and selected track from `UserSettings`
- chooses either the saved track or a random non-repeat track
- spawns a `MusicTrack` entity with `PlaybackSettings::DESPAWN`

`jukebox_system()` starts the next track when the current music entity despawns.

- `play_next` forces an explicit next track from the UI
- `loop_current` repeats the last track instead of rolling a new random choice
- otherwise the picker avoids immediate repeats when more than one track exists

`stop_music()` despawns all `MusicTrack` entities when leaving `Playing`.

## Sound Effects

`PlaySfxMsg` is the fire-and-forget trigger message. Each message carries:

- `kind: SfxKind`
- `position: Option<Vec2>`

Current `SfxKind` enum values are:

- `ArrowShoot`
- `Death`
- `Build`
- `Click`
- `Upgrade`

Currently loaded asset banks are:

- `ArrowShoot`: one variant
- `Death`: 24 groan variants

`Build`, `Click`, and `Upgrade` are defined in the enum but do not currently load asset handles.

## SFX Playback Rules

`play_sfx_system()` applies three filters before playing a sound:

1. global SFX volume must be above zero
2. arrow shots require `sfx_shoot_enabled = true`
3. positioned sounds are spatially culled against the camera

Spatial culling uses the current camera center and orthographic scale.

- off-screen positioned sounds are skipped
- sounds are also skipped when zoomed too far out (`scale > 2.0`)
- after culling, the system deduplicates by `SfxKind`, so only one on-screen event per kind plays each frame

When a sound passes the filters, the system chooses a random variant for that kind and spawns an `AudioPlayer` with `PlaybackSettings::DESPAWN`.

## Settings and UI

Music and SFX values are driven from `UserSettings` and synced into `GameAudio` by the main menu and gameplay UI.

Current exposed controls include:

- music volume
- SFX volume
- music speed
- jukebox loop toggle
- explicit jukebox track selection
- jukebox paused state
- arrow shoot SFX toggle

## Current Coverage

Live audio behavior currently covers:

- background jukebox playback
- arrow shoot SFX
- NPC death SFX with 24 variants
- spatial culling
- one-per-kind-per-frame dedup

Planned but not fully wired audio remains in the roadmap, including building placement, wall hits, loot pickup, and later wave or element sounds.

## Related Docs

- [combat.md](combat.md): projectile fire path that emits `ArrowShoot`
- [resources.md](resources.md): `GameAudio` and audio-related resources
- [ui.md](ui.md): where the jukebox and settings surface to the player
- [history.md](history.md): delivery notes for the sound rollout
