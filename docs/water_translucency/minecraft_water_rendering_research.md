# Minecraft Water Rendering Research

This is the concrete reference pass for water mesh and submerged-camera behavior. It is based on the official Minecraft Java 1.21.4 client jar decompiled locally with CFR, using Mojang/Yarn mappings to identify classes and methods.

## Vanilla Classes

- `LiquidBlockRenderer` / Yarn `FluidRenderer`: block water mesh construction.
- `FlowingFluid` / Yarn `FlowableFluid`: fluid amount, falling state, flow vector, source creation.
- `FogRenderer` / Yarn `BackgroundRenderer`: water fog color and fog distances.
- `ScreenEffectRenderer` / Yarn `InGameOverlayRenderer`: underwater screen overlay.

## Water Mesh Numbers

- `MAX_FLUID_HEIGHT = 0.8888889`, exactly `8.0 / 9.0`.
- A fluid cell with same fluid directly above reports height `1.0`.
- Otherwise its own height is `amount / 9.0`.
- Air/non-solid neighbors contribute height `0.0` to corner averaging.
- Solid non-water neighbors are ignored by the average.
- Corner heights use weighted averaging: heights `>= 0.8` are weighted by `10.0`, other non-negative heights by `1.0`.
- The diagonal corner sample is only included when at least one of the two adjacent side samples is positive.
- If either adjacent side sample or the diagonal sample reaches `1.0`, the corner is forced to `1.0`.
- Top water vertices are inset by `0.001` in vanilla to avoid z fighting. This clone does not currently subtract that in the shader because it can reintroduce visible cracks at vertical water columns unless we encode whether the top face was emitted.
- Vanilla renders water side faces double-sided and also emits a reversed top surface in some cases. This clone uses no culling for the translucent pipeline, which makes the surface visible from underwater without duplicating descriptors.

## Flowing/Falling Water Shape

- Source water amount is `8`, so its exposed top is `8/9` unless water exists above.
- Falling water also uses amount `8`, with the falling flag affecting flow vector and legacy block-state level.
- Because a water cell with water above is full-height (`9/9` for our packed mesh), vertical falling columns should connect without side gaps.
- Flow vector calculation in vanilla subtracts neighboring heights. If the fluid is falling and blocked on a horizontal side, vanilla adds a strong downward component before normalizing.

## Underwater View Numbers

- Vanilla water fog color comes from the biome water fog color. The default is `0x050533`, or RGB `(5, 5, 51) / 255`.
- Fog color transitions over `5000 ms` when the biome water fog color changes.
- Water fog start is `-8.0`.
- Water fog end is `96.0`.
- Local player water-vision effects can scale the `96.0` end distance, clamped to at least `25%`. Biome tags can apply an additional `0.85` multiplier.
- Lava fog is separate and much shorter; it is not relevant to water.
- The underwater overlay uses `textures/misc/underwater.png` with alpha `0.1`, and scrolls UVs using camera yaw/pitch divided by `64.0`.

## Clone Mapping

- Water corner heights now pack ninths (`0..9`) instead of eighths (`0..8`).
- Shader water vertex height divides by `9.0`.
- A water cell with water above returns all four corner heights as `9`.
- Corner averaging follows vanilla weighting and air/solid handling, quantized to nearest ninth.
- Camera submersion checks the camera cell, compares eye-local Y against `fluid_level / 9.0`, and treats water above as full height.
- Underwater terrain visuals use fog color `0x050533`, start `-8.0`, end `96.0`, strength `1.0`, and screen tint strength `0.1`.
- Clear color is also switched to `0x050533` while submerged so the background participates in the underwater tint.
- The previous always-on inactive fluid scan was replaced by active-chunk stepping plus adjacent loaded fluid-neighbor expansion. This keeps boundary equilibrium without simulating settled unrelated water forever.
- Fluid snapshots are now built only for active source chunks and their loaded neighbors, not every loaded chunk every fluid tick.

## Remaining Gaps

- The clone approximates the vanilla underwater overlay with a uniform tint rather than rendering the actual overlay texture.
- Biome-specific water fog colors and the 5-second transition are not implemented yet.
- Water-vision effects and biome fog multipliers are not implemented yet.
- Top-surface `0.001` inset is intentionally deferred until the descriptor can distinguish top-emitted side vertices without causing column cracks.
