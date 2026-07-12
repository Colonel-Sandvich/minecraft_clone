# Deferred Architecture Work

XZ chunk columns are now the dimension-owned unit of streaming residency.
Visible columns are published only after dependency-complete lighting, while a
resident support halo remains available to derived systems. The items below are
the next architecture targets rather than requirements for that foundation.

## Lighting follow-ups

Initial lighting now runs world-aligned compact patches off-thread with separate
calculation and commit sets. Jobs validate dimension ownership, column
incarnations, chunk content revisions, light revisions, and exact ticket
authority before committing. Cancelled work retains its worker slot until the
result is collected, so one dimension cannot accidentally overlap patches.

A slow column can still hold the other ready members of its initial-light tile.
Add a delayed and jittered loading profile, then introduce a per-tile age escape
if real storage shows head-of-line blocking. Capture a full-client trace with
meshing, colliders, and rendering enabled before changing the conservative
column budgets or adding lighting worker concurrency. Runtime block edits can
later use changed light boundaries instead of conservative connected relights.

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
reconstruct dimension membership. Move one-shot derived work into coalescing
queues owned by that dimension. Each entry must retain the expected chunk
entity so stale work cannot affect a replacement at the same position. Save
state remains a durable content revision, not another transient work bit.

## Render ownership

Authoritative padded light data is now prepared on the chunk, but material
layer children still mirror shared render state and GPU sharing is inferred
from allocation identity. Make chunk render context an explicit extracted
resource and leave material identity plus immutable face geometry on each
layer. Use extracted transforms as the origin and eventually make camera
uniforms and pipeline specialization view-specific.

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

## Dimension activation

Production currently assumes one active dimension. If runtime switching is
introduced, treat activation as an exposure transition: hide published roots
and disable colliders for the old dimension, refresh the new owner's desired
view, and reveal only its already-published columns. Inactive dimensions now
cancel lighting authority and drain their worker, but the complete activation
lifecycle still needs tests before runtime switching is supported.

## Intended order

1. Profile moving-view and delayed-load lighting in the full client.
2. Replace derived-work markers with dimension-owned queues.
3. Rebuild fluids around typed borrowed regions and fair scheduling.
4. Consolidate chunk render ownership and multi-view preparation.
5. Split generation behind stable, versioned output tests.
6. Add stable dimension identity, atomic column persistence, and activation
   lifecycle support.
