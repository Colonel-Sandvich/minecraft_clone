# Audio system

## Current data flow

Audio is downstream of gameplay state, never raw input:

1. A successful player break or placement returns a `CellDelta` from `ChunkEditor`.
2. The interaction system publishes `BlockEditCommitted` with the edit kind, exact block
   position, and old/new cells. Failed or no-op attempts publish nothing.
3. A completed dropped-item transfer publishes `ItemPickedUp` with the player, dropped entity,
   and copied stack data.
4. The audio adapters map these domain messages to a `SoundCue` and publish `PlaySound`.
5. The playback system resolves a variant from `SoundBank` and spawns a Bevy `AudioPlayer`
   with `PlaybackSettings::DESPAWN`.

World sounds are emitted at the changed block's center. A single `SpatialListener` follows the
player camera, so the same request API can support listener-relative UI sounds and spatial world
sounds. One block is treated as one metre for Bevy's default spatial scale.

`GameAudioSettings` currently persists linear master and sound-effect gains through the existing
`bevy-settings` setup. Gains are sanitized when a sound is spawned, including values from a
hand-edited settings file.

## Extension points

- **Material sound sets:** Map the changed `ChunkCell`/`Item` to a sound set containing
  break, place, hit, and footstep variants. The committed edit already carries both old and new
  material, so this does not require changing interaction code.
- **Variants:** Each cue is backed by a vector, uses a per-cue cursor, and defines its own base
  playback speed. Add files to a cue's path list first; optional variation around that base speed
  can be added later if repetition remains audible.
- **Categories:** Add music, ambience, voice, and UI gains beside sound effects when those systems
  exist. `SoundCue` should own its category so callers cannot accidentally route a cue through the
  wrong volume control.
- **Long-lived audio:** Music and ambience should use marker components plus `AudioSink`/
  `SpatialAudioSink` control for pause, resume, fades, and live volume changes. Short one-shots do
  not need that machinery.
- **Voice limiting:** Before adding dense footsteps, fluids, or multiplayer effects, cap concurrent
  voices per cue/category and coalesce repeated events in the same area and frame.
- **Pause policy:** Gameplay stops producing new interaction sounds while paused. When loops are
  introduced, explicitly pause world ambience and decide separately whether menu/UI audio remains
  active.

## Asset policy

Prefer mono Ogg Vorbis for spatial one-shots: it is compact and supported by Bevy's default audio
feature. Retain each asset's source page, direct download URL, digest, license or usage terms, and
any conversion history in the root README. Mojang assets must be identified separately from
redistributable assets and handled according to the Minecraft Usage Guidelines.

The built-in Bevy audio stack is sufficient for the current scope. Reassess a dedicated mixer or
audio plugin only when the game needs mixer buses, advanced attenuation, streaming control, or
large numbers of simultaneous voices that the simple entity-per-sound model cannot manage cleanly.
