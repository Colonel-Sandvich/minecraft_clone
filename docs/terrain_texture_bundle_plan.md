# Terrain Texture Bundle Plan

Status: shelved until the block palette grows enough to justify it.

## Problem

Terrain textures are selected from the registered block render profiles, not by
scanning the texture directory. The current palette resolves to 13 unique PNG
files, which expand to 75 texture-array layers after animation strips are split.

Expanding the palette toward the full asset set would make startup scale with
roughly 1,111 file opens and PNG decodes. Runtime setup would also split frames,
resize textures, and build mipmaps on the main thread. The final normalized
texture data is only about 2.25 MiB; per-file asset work is the real cost.

The full set expands to roughly 1,651 layers. That exceeds the portable limit of
one 256-layer texture array, so a single-array design is not a sound long-term
interface even where a particular desktop adapter supports it.

Hotbar icon generation independently reopens block PNGs on the main thread and
would retain the same scaling problem.

## Direction

Add an explicit offline terrain asset compiler rather than doing this work in a
Cargo build script or during game startup. It should consume the canonical block
render catalog and source PNGs, then emit one versioned terrain bundle containing:

- normalized 16x16 animation frames;
- prebuilt alpha-aware mip levels;
- texture-array pages capped at the chosen portable layer limit;
- a dense render-material address table with page, base layer, and frame count;
- a fingerprint covering compiler settings, the material catalog, and source
  bytes.

The runtime asset loader should read the bundle asynchronously and create the
prepared page images directly. Source PNGs should never become runtime assets,
and startup should perform no image decoding, resizing, or mip generation.

Use an explicit command such as `cargo xtask terrain-textures`. The generated
asset can remain ignored while source asset redistribution is unresolved. Do
not make ordinary `cargo check` or `cargo test` depend on the local asset pack.

## Related Work

- Allow CPU chunk meshing before terrain textures are GPU-ready; meshing emits
  render IDs and does not consume texture pixels.
- Generate hotbar icons into the bundle, or derive them from retained first-frame
  bundle data without reopening PNGs.
- Replace the current misleading source-entry-as-layer startup metric with
  separate PNG source, animation layer, packing, and GPU readiness measurements.
- Address the independent 8-bit render ID limit before the material palette
  exceeds 256 render profiles.

## Implementation Shape

1. Add a deterministic bundle schema and pure compiler tests.
2. Add the explicit compiler command with content-fingerprint/no-op behavior.
3. Add a runtime loader while retaining the PNG path for comparison.
4. Add paged texture addressing and renderer bindings.
5. Move hotbar icons onto compiled data and remove the runtime PNG path.
6. Profile both the current and expanded palettes before removing instrumentation.

