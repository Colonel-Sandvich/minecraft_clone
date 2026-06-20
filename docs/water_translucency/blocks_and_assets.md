# Blocks And Assets Plan

## First Blocks

Add these in order:

1. Water.
2. Ice.
3. Clear glass.
4. Tinted/stained glass.

Water proves the fluid and translucent render path. Ice/glass prove non-fluid translucent blocks.

## Block Metadata

Block render profiles should distinguish:

- Rendered vs non-rendered.
- Full cube vs custom shape.
- Opaque occluder vs non-opaque.
- Cutout vs translucent.
- Fluid vs non-fluid.
- Light blocking behavior.
- AO occlusion behavior.
- Tint color or tint index.
- Texture animation metadata index, later.

This is more important than the exact enum names. The mesh generator needs cheap flags for hot culling paths.

## Water Texture

Initial water can use one static texture layer.

Later animation options:

- Cycle texture array layers by time.
- Scroll UVs based on flow direction.
- Rotate/flip UVs for fake variation.
- Use a block/face animation metadata buffer.

Do not block first water rendering on animation.

## Ice And Glass

Support both cheap and blended styles:

- Cutout-style glass/ice if a texture wants hard alpha/discard.
- Blended translucent glass/ice for real transparency.

Tinted glass should use the same material metadata path as existing tints if possible.

For future tinted glass, keep per-block-face tint data generic rather than water-specific.

## Lighting And AO

Initial rules:

- Water should not behave like a full opaque light blocker.
- Ice/glass should not fully occlude AO unless deliberately styled that way.
- Opaque blocks should still render faces adjacent to water/glass/ice.
- Translucent blocks may hide faces against identical neighboring translucent blocks.

Future rules:

- Stained glass could tint transmitted light.
- Water could attenuate light by depth.
- Ice could partially affect skylight.

Those are not first-pass goals.

## Gameplay Integration

Initial target is visual-first.

Defer:

- Swimming.
- Buoyancy.
- Drag.
- Drowning.
- Fluid sounds.
- Item/entity flow.

Still expose enough block/fluid query APIs so gameplay systems can later ask:

- Is this cell fluid?
- What fluid type is here?
- What level is here?
- Is the player eye or feet inside fluid?

## Asset Pipeline

Add textures with stable texture-layer assignment.

When texture animation arrives, avoid encoding animation in block type alone. Prefer metadata such as:

- Base texture layer.
- Frame count.
- Frame duration.
- Animation mode.
- UV rotation/flip mode.

This keeps water, lava, animated blocks, and fake texture rotations on the same path.
