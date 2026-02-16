//! Audio systems — music jukebox and SFX playback.

use bevy::audio::Volume;
use bevy::prelude::*;
use rand::Rng;
use crate::resources::{GameAudio, MusicTrack, PlaySfxMsg};

/// All music track paths (embedded in release binary via bevy_embedded_assets).
/// Add/remove entries here when the soundtrack changes.
const MUSIC_TRACKS: &[&str] = &[
    // Not Jam Music Pack (CC0)
    "sounds/music/not-jam-music/BreakbeatChips.ogg",
    "sounds/music/not-jam-music/ChillMenu.ogg",
    "sounds/music/not-jam-music/CrashingOut.ogg",
    "sounds/music/not-jam-music/CriticalTheme.ogg",
    "sounds/music/not-jam-music/DarkCavern_PhoneHome.ogg",
    "sounds/music/not-jam-music/DescendGameplay.ogg",
    "sounds/music/not-jam-music/DragAndDreadTheme.ogg",
    "sounds/music/not-jam-music/HaroldParanormalInstigatorTheme.ogg",
    "sounds/music/not-jam-music/KleptoLindaCavernsA.ogg",
    "sounds/music/not-jam-music/KleptoLindaCavernsB.ogg",
    "sounds/music/not-jam-music/KleptoLindaCredits.ogg",
    "sounds/music/not-jam-music/KleptoLindaMountainA.ogg",
    "sounds/music/not-jam-music/KleptoLindaMountainB.ogg",
    "sounds/music/not-jam-music/KleptoLindaTitles.ogg",
    "sounds/music/not-jam-music/MeltdownTheme.ogg",
    "sounds/music/not-jam-music/PitcherPerfectTheme.ogg",
    "sounds/music/not-jam-music/SeeingDouble.ogg",
    "sounds/music/not-jam-music/SwitchWithMeTheme.ogg",
    "sounds/music/not-jam-music/TitleTheme_PhoneHome.ogg",
    "sounds/music/not-jam-music/TypeCastTheme.ogg",
    "sounds/music/not-jam-music/UntitledTrack01.ogg",
    "sounds/music/not-jam-music/VictoryLap.ogg",
];

/// Load all music track handles at startup.
pub fn load_music(mut audio: ResMut<GameAudio>, server: Res<AssetServer>) {
    for path in MUSIC_TRACKS {
        audio.tracks.push(server.load(*path));
    }
}

/// Pick a random track index, avoiding the last played track.
fn pick_track(audio: &GameAudio) -> usize {
    let len = audio.tracks.len();
    if len <= 1 { return 0; }
    let mut rng = rand::rng();
    loop {
        let idx = rng.random_range(0..len);
        if Some(idx) != audio.last_track { return idx; }
    }
}

/// Start music on entering Playing state. Syncs volume from UserSettings.
pub fn start_music(
    mut commands: Commands,
    mut audio: ResMut<GameAudio>,
    settings: Res<crate::settings::UserSettings>,
) {
    audio.music_volume = settings.music_volume;
    audio.sfx_volume = settings.sfx_volume;
    if audio.tracks.is_empty() { return; }
    let idx = pick_track(&audio);
    audio.last_track = Some(idx);
    commands.spawn((
        AudioPlayer::new(audio.tracks[idx].clone()),
        PlaybackSettings::DESPAWN.with_volume(Volume::Linear(audio.music_volume)),
        MusicTrack,
    ));
}

/// When the current track finishes (entity despawned), start the next random track.
pub fn jukebox_system(
    mut commands: Commands,
    query: Query<(), With<MusicTrack>>,
    mut audio: ResMut<GameAudio>,
) {
    if !query.is_empty() || audio.tracks.is_empty() { return; }
    let idx = pick_track(&audio);
    audio.last_track = Some(idx);
    commands.spawn((
        AudioPlayer::new(audio.tracks[idx].clone()),
        PlaybackSettings::DESPAWN.with_volume(Volume::Linear(audio.music_volume)),
        MusicTrack,
    ));
}

/// Stop music on leaving Playing state.
pub fn stop_music(mut commands: Commands, query: Query<Entity, With<MusicTrack>>) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}

/// Drain SFX messages (placeholder — no .ogg files wired yet).
pub fn play_sfx_system(mut events: MessageReader<PlaySfxMsg>) {
    // Drain to prevent unbounded accumulation. Actual playback added when SFX assets arrive.
    for _event in events.read() {}
}
