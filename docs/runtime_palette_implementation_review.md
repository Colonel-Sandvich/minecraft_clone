# Runtime Palette Implementation Review

## Verdict

The implementation moves in the right storage direction: chunks now have local
palette indices, palette entries cache hot metadata, and common chunks can stay
near one byte per cell.

However, the important architectural boundary is not clean yet. The code mostly
wraps the old `ChunkCell` and render-ID model instead of establishing a real
block-state/metadata layer. Before adding slabs, stairs, waterlogging, or other
finite states, the design should be tightened.

The highest-value fix is a structural one: make `Chunk` a thin owner of
`ChunkPalette + CellStorage`, move semantic state mapping into a real state
layer, move storage bytes into a codec module, and move fluid stepping into a
pure simulation module. That deletes incidental branching and gives mesh, light,
fluid, UI, and persistence one stable metadata boundary.

## Read This First

There are two different classes of work here.

- Correctness and commit-safety fixes should happen before relying on this branch
  for normal development.
- Architecture cleanup should happen before adding finite block states such as
  slabs, stairs, waterlogging, or shape-dependent occlusion.

Immediate must-fix items:

- Include referenced new modules in the implementation commit.
- Bump `CHUNK_FORMAT_VERSION` so old saved chunks are intentionally rejected or
  regenerated.
- Fix cross-chunk downward water so it follows the same rule as in-chunk falling
  water.
- Keep `ChunkBlockCounts` correct when boundary fluids enter neighbor chunks.
- Move or ignore the binary meshing perf microbench so normal tests stay normal.

Do not start slabs, stairs, or waterlogging until:

- `BlockStateId` no longer has to collapse through `ChunkCell` for normal hot-path
  metadata lookup.
- Mesh generation consumes state metadata directly instead of reconstructing
  behavior from render IDs.
- Water/fluid render behavior has one canonical profile source.
- `chunk/mod.rs` is split back into focused modules.

## Intended Direction

The target model from `runtime_block_state_palette_plan.md` is:

```text
Chunk cells      = compact local palette indices, usually u8
Chunk palette    = states used by this chunk + hot metadata cached beside them
Global registry  = canonical block-state definitions
Side storage     = sparse arbitrary block-entity data only
```

Hot paths should consume metadata directly. They should not reconstruct behavior
from `ChunkCell`, raw render IDs, or scattered water checks.

## Major Structural Findings

### 1. `chunk/mod.rs` Is Now A Monolith

Reference: `src/world/chunk/mod.rs:1-1341` (`Chunk`, `ChunkPlugin`,
`Chunk::step_fluids`, `Chunk::to_storage_bytes`, module tests)

The file grew from roughly 708 lines to 1341 lines and now owns plugin wiring,
cell semantics, block-state shim logic, palette storage, cell storage,
serialization, fluid simulation, coordinate indexing, iteration, and tests.

This crosses the 1000-line smell threshold hard. It will block future work by
making every block-state change touch the same oversized module.

Remedy:

```text
src/world/chunk/mod.rs        exports, plugin wiring, public API surface
src/world/chunk/cell.rs       ChunkCell compatibility, FluidState, CellDelta
src/world/chunk/palette.rs    ChunkPalette, PaletteEntry, CellStorage
src/world/chunk/codec.rs      to_storage_bytes / try_from_storage_bytes
src/world/chunk/fluid_core.rs pure in-chunk fluid stepping + boundary outflows
src/world/chunk/tests.rs      chunk/palette/codec tests
```

### 2. Storage Format Changed Without A Version Bump

References: `src/world/storage/mod.rs:112-119` (`WorldMetadata` entries),
`src/world/chunk/mod.rs:894-989` (`Chunk::to_storage_bytes`,
`Chunk::try_from_storage_bytes`)

Chunk storage semantics changed. Old worlds can be discarded, so avoid migration
complexity and intentionally reject/regenerate old chunks.

Remedy:

Update `CHUNK_FORMAT_VERSION` and make sure world metadata mismatch causes old
stored chunks to be ignored or the world to be regenerated through the existing
metadata path.

### 3. `BlockStateId` Boundary Is Fake

References: `src/block/mod.rs:44-55` (`BlockStateId`, `HotBlockStateMeta`),
`src/world/chunk/mod.rs:35-51` (`AIR_BLOCK_STATE_ID`, `BlockRegistry`),
`src/world/chunk/mod.rs:262-279` (`cell_from_state_id`)

`BlockStateId` lives in `block`, but the mapping/registry lives inside
`world::chunk`. `BlockRegistry` is currently a zero-sized shim around private
chunk functions. More importantly, `BlockStateId` immediately collapses back
through `ChunkCell`, which can only represent `Empty | Block(BlockType) |
Fluid(FluidState)`.

This means future finite states like `slab[type=top]`,
`stairs[facing=north,half=bottom]`, and `leaves[waterlogged=true]` cannot be
represented without another redesign.

Remedy:

Either keep `BlockStateId` internal until the registry is real, or move state-ID
mapping into a real `block::state` or `block::registry` module that owns:

```rust
BlockStateId -> BlockStateMeta
BlockStateId -> HotBlockStateMeta
BlockStateId -> storage name/properties
```

Do not require `BlockStateId -> ChunkCell` for normal operation.

## Fluid Findings

### 4. Fluid Simulation Is Split Awkwardly

References: `src/world/chunk/mod.rs:722-855` (`Chunk::step_fluids`),
`src/world/chunk/fluid.rs:38-187` (`step_chunk_fluids`)

The pure in-chunk algorithm lives inside `Chunk`, while cross-chunk boundary
propagation lives in the Bevy system. These two paths do not share one rule set,
which already caused a boundary bug.

Remedy:

Move fluid stepping into a pure module that takes a snapshot and returns:

```rust
struct FluidStepOutput {
    changed: bool,
    local_changes: Vec<(LocalBlockIndex, ChunkCell)>,
    boundary_outflows: Vec<BoundaryFluidOutflow>,
}
```

The Bevy system should only orchestrate queries, apply deltas, update counts,
and mark dirty components.

### 5. Downward Boundary Flow Uses The Wrong Fluid State

References: `src/world/chunk/fluid.rs:149-157` (`BoundaryFlow` creation),
`src/world/chunk/mod.rs:752-759` (`Chunk::step_fluids` downward flow)

In-chunk falling water writes max-level flowing water, but vertical boundary flow
writes `FluidState::water_source()` into the lower chunk. Today
`FluidState::is_source()` treats `MAX_LEVEL` as source, so the type model cannot
fully express the distinction. Conceptually, falling water and source water need
separate states before boundary behavior can be made unambiguous.

Remedy:

Boundary outflow generation must use the same operation as in-chunk downward
flow. Tighten the fluid model so non-source falling max-level water is
representable, then use that state for vertical downward outflow.

### 6. Boundary Outflow Can Be Lost When Local Cells Do Not Change

References: `src/world/chunk/fluid.rs:73-76` (`ChunkHasActiveFluids` removal),
`src/world/chunk/fluid.rs:86-162` (boundary outflow scan)

`ChunkHasActiveFluids` is removed when `chunk.step_fluids()` reports no local
change. But boundary outflow availability is separate from local mutation. A
stable source at a chunk edge can need to flow into a neighbor after that
neighbor loads or changes, even if the source chunk itself did not change.

Remedy:

Track boundary outflow/reactivation explicitly. Neighbor load/edit should be
able to reactivate adjacent fluid chunks, and the step result should distinguish
`local_changed` from `has_boundary_outflows`.

### 7. Boundary Writes Do Not Update Target Chunk Counts

Reference: `src/world/chunk/fluid.rs:167-181` (`step_chunk_fluids` boundary write pass)

The boundary write query mutates neighbor chunks but does not include
`&mut ChunkBlockCounts`. Rendered/translucent counts can go stale after water
enters a neighboring chunk.

Remedy:

Apply the returned `CellDelta` to target `ChunkBlockCounts`, or collect touched
target chunks and recompute counts once per target.

### 8. Zero-Level Fluids Remain Representable

References: `src/world/chunk/mod.rs:65-116` (`FluidState`, `FluidState::water_flow`,
`FluidState::from_name`)

`FluidState` has public fields and `water_flow(0)` is allowed, while
`from_name` rejects `water_0` and `ChunkCell::canonical` silently collapses
zero-level fluid later.

Remedy:

Make zero-level fluid unrepresentable. Prefer private fields plus validated
constructors, or use a non-zero level type. If zero must exist temporarily, keep
it entirely inside the fluid simulation module and never store it in chunks.

## Palette, Metadata, And Mesh Findings

### 9. Meshing Throws Away Cached Hot Metadata

References: `src/world/chunk/mod.rs:385-389` (`PaletteEntry`),
`src/world/chunk/mesh/blocks.rs:93-97` (`ChunkMeshBlocks::copy_center_chunk`),
`src/world/chunk/mesh/mod.rs:43-49` (`block_mesh_flags`)

Palette entries cache `HotBlockStateMeta`, but `ChunkMeshBlocks` stores only
`render_id` and `fluid_level`. Mesh flags are then recomputed from raw render ID
using `block_mesh_flags(render_id)`.

That reintroduces render ID as a behavior key. Future states may share a render
material but have different occlusion, shape, light, or fluid behavior.

Remedy:

Pass hot metadata through mesh preparation, at minimum:

```rust
blocks: [u16; PADDED_CHUNK_VOLUME]          // render/material ID for shader lookup
mesh_flags: [u8; PADDED_CHUNK_VOLUME]      // behavior from palette metadata
fluid_levels: [u8; PADDED_CHUNK_VOLUME]
```

Then delete or greatly narrow `block_mesh_flags(render_id)`.

### 10. Water Render Behavior Is Scattered

References: `src/block/mod.rs:76-84`, `src/block/mod.rs:318-388`,
`src/textures.rs:273-289`, `src/ui/hotbar.rs:221-256`,
`src/world/chunk/mesh/mod.rs:43-49` (`block_mesh_flags`),
`src/world/chunk/mesh/vertex_pulling.rs:131-187` (`FaceDescriptor` water decoration)

Water behavior is spread across block metadata, texture loading, tint lookup,
hotbar icon generation, mesh flags, descriptor decoration, and shader-facing
bits.

Remedy:

Introduce one render/cell profile catalog that owns:

```rust
render_id
mesh_flags
texture paths by face
tint by face
material layer
emission factor
fluid render behavior
```

The rest of the code should consume that catalog or per-palette cached copies.

### 11. Water Descriptor Decoration Is Duplicated

References: `src/world/chunk/mesh/vertex_pulling.rs:165-187`,
`src/world/chunk/mesh/binary.rs` (`build_descriptors_hybrid` water decoration)

The scalar and binary paths duplicate water descriptor decoration: corner
heights, below-water data, and flowing top-face flag.

Remedy:

Extract a shared helper, for example:

```rust
fn decorate_fluid_descriptor(
    desc: FaceDescriptor,
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
    side_index: usize,
) -> FaceDescriptor
```

Longer term, this should be driven by a cell render profile rather than `is_water`.

### 12. Render IDs Are Still A Descriptor Limit

References: `src/world/chunk/mesh/vertex_pulling.rs:71-75`,
`assets/shaders/vertex_pulling.wgsl:127-129` (`FaceDescriptor`/shader render ID packing)

`HotBlockStateMeta.render_id` is `u16`, but the face descriptor packs it into
the low 8 bits of `info`. If render/material IDs exceed 255, descriptor decode
and table lookup corrupt silently.

Remedy:

Either enforce a hard `render_id <= 255` invariant with tests/assertions, or
change the descriptor/table layout before adding many render states.

## Public API And Semantic Leaks

### 13. `Chunk` Exposes Too Many Raw Storage Variants

Reference: `src/world/chunk/mod.rs:561-656` (`Chunk::palette`, `Chunk::cell_storage`,
`Chunk::palette_index*`, `Chunk::state_id*`, `Chunk::hot_meta*`)

The public API exposes cells, palette index, state ID, hot meta, linear index,
xyz index, and storage internals. This leaks implementation detail and invites
duplicated traversal logic.

Remedy:

Make storage/palette/linear APIs `pub(crate)` unless required externally. Add a
small canonical scanning API for hot loops and a typed local-index helper.

### 14. Several `BlockType` APIs Are Now Tautologies

References: `src/block/mod.rs:153-156` (`BlockType::is_rendered`),
`src/block/mod.rs:212-239` (`BlockType::render_profile`, `BlockType::is_solid`,
`BlockType::is_placeable`)

After removing air and water from `BlockType`, methods like `is_rendered()`,
`is_solid()`, `is_placeable()`, and `render_profile() -> Option<_>` are mostly
tautological.

Remedy:

Delete or collapse tautological APIs. Put nontrivial behavior on the block-state
metadata/render profile boundary rather than on `BlockType`.

### 15. `BlockUpdateKind` Is Still Block-Only

References: `src/player/interaction/mod.rs:226-232` (`BlockUpdateMessage` emission),
`src/block/mod.rs:490-493` (`BlockUpdateKind`)

Water placement mutates the world but emits no placement update because update
events only carry `BlockType`.

Remedy:

Make update events carry `ChunkCell`, `BlockStateId`, or `CellDelta`. `CellDelta`
is likely the cleanest because consumers can decide what changed.

## Tests, Benchmarks, And Commit Safety

### 16. Tests Live Inside The Oversized Production Module

Reference: `src/world/chunk/mod.rs:1117-1341` (`mod tests`)

Tests add more weight to an already oversized module, and many storage tests are
self-roundtrips only.

Remedy:

Move tests to `src/world/chunk/tests.rs`. Add golden codec tests that assert the
chosen y-fastest order and malformed codec cases.

### 17. Perf Microbench Is A Normal Unit Test

Reference: `src/world/chunk/mesh/binary.rs:1009` (`perf_breakdown_binary`)

`perf_breakdown_binary` runs 50k iterations under `#[test]`. This slows and
destabilizes normal test runs.

Remedy:

Move it to `benches/`, use `examples/perf_binary_mesh.rs`, or mark it
`#[ignore]`.

### 18. New Modules Are Untracked But Referenced

References: `src/world/chunk/mod.rs:3`, `src/world/chunk/mesh/mod.rs:1`

`fluid.rs` and `mesh/binary.rs` are untracked but referenced by tracked files.
An implementation commit that omits them will break the build.

Remedy:

Ensure intended new modules are included in the implementation commit.

## Recommended Cleanup Sequence

### Phase 1: Commit-Safety And Correctness Fixes

1. Include referenced new modules in the implementation commit.
2. Bump `CHUNK_FORMAT_VERSION`.
3. Fix vertical boundary water so falling water and source water use distinct
   states consistently.
4. Update boundary fluid writes to maintain `ChunkBlockCounts`.
5. Move or ignore the perf microbench test.

### Phase 2: Split The Monolith

1. Extract cell/fluid semantic types to `cell.rs`.
2. Extract palette and storage to `palette.rs`.
3. Extract serialization to `codec.rs`.
4. Extract pure fluid stepping to `fluid_core.rs`.
5. Move tests to `tests.rs`.

This phase should be behavior-preserving.

### Phase 3: Establish The Real Metadata Boundary

1. Move `BlockStateId` mapping out of `world::chunk` or keep it private until real.
2. Create a block-state metadata table that does not round-trip through `ChunkCell`.
3. Make meshing consume cached metadata directly, including mesh flags.
4. Make lighting propagation consume opacity/emission metadata directly.
5. Centralize water/render profile behavior.

### Phase 4: Prepare For Slabs, Stairs, And Waterlogging

1. Add finite block-state definitions for one simple shape first, likely slabs.
2. Prove two states can share render material but differ in metadata.
3. Add persistence tests proving state identity survives round-trip.
4. Add mesh tests proving state-specific flags survive through `ChunkMeshBlocks`.

## Acceptance Criteria Before Building More States

- `Chunk` storage remains compact and mostly `U8` for normal terrain.
- `BlockStateId` does not collapse to `ChunkCell` in the hot/registry path.
- Mesh generation does not reconstruct behavior from render ID.
- Lighting does not reconstruct behavior from `ChunkCell` where hot metadata is available.
- Water behavior has one canonical profile/metadata source.
- Fluid simulation boundary behavior matches in-chunk behavior.
- `chunk/mod.rs` is back below the 1000-line smell threshold and mostly delegates.

## Bottom Line

The palette storage idea is correct, but the current implementation stops halfway.
It saves memory for today's blocks while preserving the old semantic bottlenecks.

Do not add slabs/stairs/waterlogging on top of this shape yet. First establish a
real block-state metadata boundary and split the monolith. That will make the
next features much smaller, safer, and more cache-friendly.
