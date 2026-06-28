# Water And Translucency Roadmap

This folder tracks the high-level plan for adding water, ice, glass, and future translucent/animated block rendering.

The first target is intentionally modest: simple full-block water visuals, persisted fluid state, chunk-sorted translucency, and a rendering architecture that can later support tinted glass and better transparency ordering.

## Goals

- Add water as a visual-first feature without blocking on full gameplay physics.
- Add a `Translucent` render path that supports water, ice, and future tinted glass.
- Add a `Fluid` concept with levels and persistence, even if the first renderer displays full blocks.
- Keep opaque mesh generation fast and avoid regressing the common opaque path.
- Make room for future better transparency sorting beyond chunk-level ordering.
- Keep shader/render validation in step with new resources and render layers.

## Non-Goals For First Pass

- Perfect transparency sorting.
- Full Minecraft-equivalent fluid behavior.
- Swimming, drag, drowning, item flow, boats, or audio.
- Order-independent transparency.
- Animated water textures or UV flow, unless cheap to add after the static version works.

## Proposed Phases

1. Data model and block definitions.
2. Rendering layer split and face culling rules.
3. Static full-block water rendering.
4. Persisted fluid levels and propagation jobs.
5. Chunk-boundary fluid propagation.
6. Ice and glass using the same translucent infrastructure.
7. Animated texture metadata and shader support.
8. Better translucent ordering if chunk sorting is too visibly wrong.

## Documents

- `rendering.md`: render layers, pipeline variants, culling, sorting, and validation.
- `fluids.md`: fluid state, levels, propagation, persistence, and chunk boundaries.
- `blocks_and_assets.md`: block definitions, textures, gameplay hooks, and tinted glass readiness.
- `water_flow_rendering_research.md`: Minecraft water propagation/rendering research and rewrite notes.

## First Implementation Slice

The smallest useful vertical slice is:

1. Add `BlockMaterialLayer::Translucent` and `BlockRenderLayer::Translucent`.
2. Add a water block with a static texture and simple full-cube mesh output.
3. Add a translucent pipeline with blending and depth write disabled.
4. Queue translucent chunk batches after opaque and cutout batches with chunk-distance sorting.
5. Add validation tests for any new WGSL resources.
6. Store fluid level data in chunk persistence, even if the first renderer treats all water as visually full.

This gets water visible early while preserving seams for later simulation and sorting improvements.
