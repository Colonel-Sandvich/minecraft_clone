# Day/Night Lighting Notes

## Key Distinction

Minecraft-style sky lighting has two separate pieces:

- **Propagated sky light level**: per-block `0..15` value for sky access through terrain, caves, roofs, overhangs, and chunk boundaries.
- **Time-of-day sky intensity/color**: global visual multiplier/tint for noon, dusk, night, weather, or dimension ambience.

Day/night should not recompute skylight flood fill every frame or every time tick. Flood fill changes when geometry changes, chunks load, or blocks affecting opacity are placed/broken.

## Renderer Implication

Keep per-chunk light buffers for stable propagated values:

```text
sky_light: 0..15
block_light: 0..15
```

Apply day/night through a global shader uniform, likely in the existing group 0 terrain globals / visual settings:

```text
sky_light_strength: f32
sky_light_color: vec3<f32>
ambient_strength: f32
```

Shader shape:

```wgsl
let sky = sky_light_level * terrain_visuals.sky_light_strength;
let block = block_light_level;
let brightness = max(block, sky);
```

Or RGB/tinted:

```wgsl
let light_rgb =
    block_light_level * block_light_color +
    sky_light_level * terrain_visuals.sky_light_strength * terrain_visuals.sky_light_color +
    terrain_visuals.ambient_strength;
```

## What Should Trigger Per-Chunk Relighting

- Placing or breaking opaque blocks.
- Placing or breaking emissive blocks.
- Chunk load/generation.
- Cross-chunk light propagation changes.

## What Should Not Trigger Per-Chunk Relighting

- Sun angle/time-of-day changes.
- Dusk/night brightness changes.
- Weather darkening, unless weather also changes geometry/opacity.

## Relation To Split Light Bind Group

The split light bind group is still useful for block-light and geometry-driven skylight changes because light buffers can update without rebuilding mesh descriptor buffers.

For day/night specifically, prefer updating only global uniforms. Avoid uploading thousands of chunk light buffers for a global sky brightness change.
