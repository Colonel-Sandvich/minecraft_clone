# Fluid Simulation Plan

## Initial Scope

Water should be visual-first, but the data model should support persisted fluid levels from the beginning.

The first renderer may display water as full blocks. The simulation data should still represent levels so we do not need to migrate saved worlds immediately when level visuals arrive.

## Fluid Data Model

Represent fluid separately from solid block type if possible.

Recommended concepts:

- `FluidType`: `None`, `Water`, later `Lava`.
- `FluidLevel`: compact integer level, probably `0..=8` or `0..=15`.
- `FluidFlags`: source, falling, scheduled, or other state if needed.

Open implementation choice:

- Store fluid as part of block state.
- Store fluid in a separate per-chunk fluid array.

Separate fluid arrays are cleaner for waterlogged blocks and future rendering, but block-state storage may be simpler if the current world storage assumes one block value per cell.

## Persistence

The first correct target is persisted simulation.

Persist:

- Fluid type per occupied cell.
- Fluid level per occupied cell.
- Any minimal flags required to resume propagation.

Avoid requiring all scheduled updates to persist perfectly. It is acceptable to rebuild/reseed fluid update queues when chunks load by scanning fluid cells and boundaries.

## Propagation Model

Start with a simple cellular propagation model:

- Source water remains full.
- Water flows downward first.
- If downward flow is blocked, water spreads horizontally.
- Horizontal flow reduces level.
- Low-level water disappears if unsupported and not replenished.

Keep it deterministic and chunk-local where possible.

## Chunk Boundaries

Fluid propagation must eventually cross loaded chunk boundaries.

Target behavior:

- If neighboring chunk is loaded, propagation can enqueue updates in that chunk.
- If neighboring chunk is not loaded, record a boundary dirty marker.
- When the neighbor loads, reconcile boundary fluid cells and schedule propagation.
- Chunk persistence should include enough edge state to avoid losing water at unload boundaries.

Do not block water work on fully simulating unloaded chunks.

## Scheduling

Use queued fluid updates rather than scanning all fluid cells every frame.

Events that enqueue fluid work:

- Block placed/removed near fluid.
- Fluid cell changed.
- Chunk loaded near fluid boundary.
- Neighbor chunk loaded/unloaded.

Batch updates to avoid frame spikes.

## Mesh Invalidation

Fluid changes should mark affected chunks for mesh rebuild.

Likely affected chunks:

- The chunk containing the changed fluid cell.
- Neighbor chunks when the changed cell touches a chunk boundary.

For first full-block water visuals, this can be conservative. Later level visuals may need more precise invalidation.

## Performance Risks

- Fluid propagation can cascade across many chunks.
- Persisted updates can cause load-time spikes if all fluid cells are scanned naively.
- Mesh rebuilds from fluid changes can compete with opaque chunk rebuilds.

Mitigations:

- Cap fluid work per frame.
- Prioritize visible/nearby chunks.
- Coalesce mesh rebuild requests.
- Keep opaque mesh generation independent from fluid simulation where possible.

## Future Extensions

- Waterlogged blocks.
- Lava or other fluids.
- Flow direction metadata for animated UVs.
- Player/mob water interaction.
- Fluid source creation/removal rules.
