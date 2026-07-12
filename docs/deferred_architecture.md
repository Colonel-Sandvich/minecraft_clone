# Deferred Architecture Work

The current architecture work is focused on making an XZ chunk column the
dimension-owned unit of streaming residency. The items below are deliberately
deferred until that ownership boundary is stable.

## Fluid simulation

Fluid stepping still builds copied multi-chunk snapshots, carries raw world
coordinates through its solver API, and selects work with a soft,
position-biased budget. Replace this with a borrowed, typed fluid region that
separates simulated chunks from readable context and returns deterministic
per-chunk edits. Schedule active regions through a fair, bounded frontier and
apply every result through `ChunkEditor`.

## Derived chunk work

Mesh, collider, lighting, render-light, and fluid work is represented by a
family of ECS marker components. Consumers repeatedly scan broad queries and
reconstruct dimension membership. Once residency is dimension-owned, move
one-shot derived work into coalescing queues owned by that dimension. Queue
entries must retain the expected chunk entity so stale work cannot affect a
replacement at the same position. Save state remains a durable content
revision, not another transient work bit.

## Render ownership

Chunk origin and padded light data are currently duplicated across material
layer children, while GPU sharing is inferred from allocation identity. Move
shared render context to the chunk and leave material identity plus immutable
face geometry on each layer. The render world should prepare one chunk context
and independent layer buffers, use extracted transforms as the origin, and
eventually make camera uniforms and pipeline specialization view-specific.

## World generation

Generation still combines metadata, terrain noise, feature placement, and a
development landmark in one module. Introduce a `WorldGenerator` facade and
separate terrain, noise, and features after column streaming is established.
Column generation should compute shared surface and feature inputs once while
preserving generator-version output exactly.

## Persistence

Save tickets are bound to runtime dimension entities, but stored chunk keys do
not yet include a stable dimension identifier. Per-subchunk writes also share
one column heightmap, and backend column reads are not one transaction. Add a
stable `DimensionId`, then replace per-subchunk persistence with atomic column
load/save operations. Track the last durable content revision per column so
eviction can verify persisted state directly instead of trusting only dirty
markers.

## Intended order

1. Finish column residency, owner-bound loading, and revision-aware persistence.
2. Replace derived-work markers with dimension-owned queues.
3. Rebuild fluids around typed borrowed regions and fair scheduling.
4. Consolidate chunk render ownership and multi-view preparation.
5. Split generation behind stable, versioned output tests.
