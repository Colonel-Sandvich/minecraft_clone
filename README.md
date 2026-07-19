## Textures

https://mcasset.cloud/1.21.11/assets/minecraft/textures

Copy them into the repo such that: 

/assets/textures/block/dirt.png

is a valid path.

```bash
find assets -type f -and \( -name '*.mcmeta' -or -name '_list.json' \) -delete
```

## Audio assets

The block interaction sounds come from sources explicitly published under
[CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/), which permits
copying, modification, and redistribution without requiring attribution. Credit and full
provenance are retained here for auditability.

| Repository asset | In-game use | Source, author, and license evidence |
| --- | --- | --- |
| `assets/audio/block/rock_break.ogg` | Successful block break | [`rock_break.ogg`](https://opengameart.org/sites/default/files/rock_break.ogg) from [Breaking Rock](https://opengameart.org/content/breaking-rock), a CC0 derivative by **themightyglider** of SoundCollectah's independently [CC0-licensed Freesound recording](https://freesound.org/people/SoundCollectah/sounds/109360/) |
| `assets/audio/block/small_rock_impact.ogg` | Successful block placement | [`small_rock_impact.wav`](https://opengameart.org/sites/default/files/small_rock_impact.wav) from Spring Spring's [Various Sound Effects](https://opengameart.org/content/various-sound-effects-0), published under CC0 |
| `assets/audio/item/pickup.ogg` | Item pickup | Mojang asset `minecraft/sounds/random/pop.ogg` from the Minecraft 26.2 asset index, downloaded from the [official asset CDN](https://resources.download.minecraft.net/d6/d6ae1c04d0a7376a33d1df12e1b8057cfbab6bc2), object SHA-1 `d6ae1c04d0a7376a33d1df12e1b8057cfbab6bc2` |

The Mojang item-pickup sound is not CC0 and is governed by the
[Minecraft Usage Guidelines](https://www.minecraft.net/usage-guidelines). It is listed separately
so that its origin and redistribution status are not confused with the two CC0 assets.

Both CC0 assets are mono, 48 kHz Ogg Vorbis conversions. `rock_break.ogg` was
downmixed and resampled from the linked 24 kHz stereo Ogg; `small_rock_impact.ogg` was
downmixed and resampled from the linked 96 kHz two-channel WAV.
See [`docs/audio_system.md`](docs/audio_system.md) for the runtime design and extension plan.
The unmodified source SHA-256 digests are:

- `rock_break.ogg`: `b645f35d21194e7855dd4a5405b1e8e6bec070731b92df6abcb202676b8e9f2e`
- `small_rock_impact.wav`: `913ac9b5f4bd2839eca00f84490a766e6907dd1e024f0745377d1b8838354c65`
- `pickup.ogg`: `4ddf491e1b736a9711cd76590ba70b203136d7ea1d5134a42088e58daccf3951`
