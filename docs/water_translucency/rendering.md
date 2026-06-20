# Rendering Plan

## Render Layers

Use explicit render layers so mesh generation and pipeline choice stay predictable:

- `Opaque`: solid blocks with no alpha blending.
- `Cutout`: alpha-tested blocks that discard pixels but still write depth.
- `Translucent`: blended blocks like water, ice, and tinted glass.
- `Fluid`: optional semantic category for simulation and water-specific mesh rules. It can initially render through `Translucent`.

Keep `Fluid` separate from `Translucent` conceptually: water is both fluid and translucent, but tinted glass is translucent and not fluid.

## Pipeline Behavior

Opaque pipeline:

- Depth test: on.
- Depth write: on.
- Blending: off.
- Sort: existing opaque batching.

Cutout pipeline:

- Depth test: on.
- Depth write: on.
- Blending: optional/off, with fragment discard.
- Sort: same rough path as opaque.

Translucent pipeline:

- Depth test: on.
- Depth write: off.
- Blending: alpha blending.
- Sort: chunk-level back-to-front for first pass.

Water can initially share the translucent pipeline. If water needs special shader behavior later, add a water-specific pipeline variant or shader def.

## Chunk Sorting First

Start with chunk-level translucent sorting because it is simple and gives immediate feedback.

Sort key options:

- Distance from camera to chunk center.
- Distance from camera to translucent mesh AABB center.
- Distance to nearest/farthest AABB corner depending on artifacts.

Start with chunk center. If artifacts are obvious, try translucent AABB center before moving to face sorting.

## Expected Sorting Bugs

Chunk sorting will not correctly handle:

- Water behind glass in the same chunk.
- Intersecting translucent surfaces across adjacent chunks.
- Large water surfaces where one chunk is partly in front and partly behind another.
- Multiple translucent materials requiring different internal face ordering.

These are acceptable for the first pass. The renderer should be structured so we can later add per-chunk face sorting or another transparency approach.

## Future Ordering Upgrades

Likely upgrade path:

1. Chunk-level sorting.
2. Per-chunk translucent face sorting when camera movement crosses a threshold.
3. Separate water surface sorting from glass/ice if needed.
4. Weighted blended order-independent transparency only if the game needs many overlapping translucent surfaces.

Do not start with OIT. It adds render targets, shader complexity, and platform constraints before we know if visual artifacts are unacceptable.

## Face Culling Rules

Opaque blocks:

- Hide faces against opaque full cubes.
- Emit faces against air, fluids, cutout, and translucent blocks unless the neighbor is proven to fully obscure the face.
- Keep this path fast. Avoid expensive translucent/fluid checks in the hot opaque path unless profiling says it is fine.

Cutout blocks:

- Hide internal faces against same cutout block only if the block shape/material makes that safe.
- Emit faces against translucent/fluid blocks.

Translucent blocks:

- Hide faces against identical adjacent translucent blocks if they represent a continuous volume, such as water-water or glass-glass full cubes.
- Emit faces against opaque blocks if visible from the translucent side.
- Emit faces against different translucent materials initially, because sorting is approximate and hiding can create obvious holes.

Water/fluid blocks:

- Hide water-water internal faces.
- Emit water-air surface faces.
- Emit water-solid boundary faces if visible.
- For first full-block visuals, treat water shape as a full cube.
- Later fluid-level visuals can lower top surfaces and alter side faces.

## Performance Strategy

- Preserve the opaque mesh generation fast path.
- Avoid making opaque culling depend on full fluid simulation state if a cheap block render profile bit is enough.
- Keep layer-specific descriptor vectors, as the current vertex-pulling renderer already does.
- Only rebuild translucent/fluid meshes for affected chunks when fluid state changes.
- Defer per-face translucent sorting until chunk-level sorting has been evaluated visually.

## Shader And Validation Updates

Every new shader resource should update WGSL validation tests.

Likely future resources:

- Time uniform for animated water/textures.
- Animation metadata storage buffer.
- Per-material alpha/tint metadata.
- Water visual settings uniform.

Keep render layout helpers and WGSL resource tests in sync.
