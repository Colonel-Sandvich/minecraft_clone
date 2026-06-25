# CellId Storage And Binary Meshing Plan

## Goals

- Keep one occupant per chunk cube.
- Keep runtime storage clean and flexible.
- Keep `BlockType` block-only: no `Air`, no `Water`.
- Avoid split block/fluid arrays.
- Make hot meshing and lighting paths table-driven instead of match-heavy.
- Prototype binary face-mask meshing for full-cube face occluders.
- Preserve water, ice, cutout, translucent, lighting, AO, and palette storage behavior.

## Current Problem

The semantic `Cell` model is nice, but using rich enum-like classification directly in hot loops is costly. The current scalar meshing path scans every cell, checks every neighbor for rendered cells, and classifies cells repeatedly.

Current meshing shape:

1. Build a padded `18^3` neighborhood buffer.
2. Iterate all `16^3` center cells.
3. For each rendered cell, check all six neighbors.
4. Emit visible faces one by one.
5. Compute AO per emitted face by sampling nearby cells.

This is simple and correct, but not ideal for full-cube terrain where face visibility can be computed with bit operations.

## Target Runtime Model

Use compact dense cell IDs in chunks and keep semantic decoding at the API boundary.

```rust
pub struct Chunk {
    cells: [CellId; CHUNK_VOLUME],
}

#[repr(transparent)]
pub struct CellId(u8);

pub enum Cell {
    Empty,
    Block(BlockType),
    Fluid(FluidState),
}
```

`CellId` is the storage and hot-path type. `Cell` is the semantic type used by gameplay, interaction, tests, storage names, and UI.

## CellId Layout

Use stable runtime IDs with table-driven properties.

Initial layout:

```text
0        Empty
1..127   Blocks
128..135 Water levels
136..143 Lava levels, later
144..255 Reserved
```

Water source is derived from fluid type and level:

- `Water + level 8` means source.
- `Water + level 1..7` means flowing.

No source bool is stored.

If block count pressure becomes real, we can move to `u16 CellId` later. The public semantic API should not care.

## Tables

All hot classification should be table lookups by `CellId.0`.

Candidate tables:

```rust
CELL_FLAGS: [u16; 256]
CELL_LIGHT_OPACITY: [u8; 256]
CELL_LIGHT_EMISSION: [u8; 256]
CELL_RENDER_LAYER: [BlockMaterialLayer; 256]
CELL_VISUAL_KIND: [CellVisual; 256]
CELL_FLUID_LEVEL: [u8; 256]
CELL_FLUID_TYPE: [Option<FluidType>; 256]
```

Important flags:

```rust
CELL_FLAG_RENDERED
CELL_FLAG_FACE_OCCLUDER_FULL_CUBE
CELL_FLAG_CUTOUT
CELL_FLAG_TRANSLUCENT
CELL_FLAG_FLUID
CELL_FLAG_EMITS_INTERNAL_FACES
```

Prefer `face_occluder_full_cube` over `opaque`. The behavior needed by face culling is geometry/culling behavior, not render material.

## Public API Shape

Chunk API should remain ergonomic:

```rust
impl Chunk {
    pub fn cell(&self, pos: UVec3) -> Cell;
    pub fn cell_id(&self, pos: UVec3) -> CellId;
    pub fn set_cell(&mut self, pos: UVec3, cell: Cell) -> CellDelta;
    pub fn set_cell_id(&mut self, pos: UVec3, id: CellId) -> CellDelta;
    pub fn set_block(&mut self, pos: UVec3, block: BlockType) -> CellDelta;
    pub fn set_fluid(&mut self, pos: UVec3, fluid: FluidState) -> CellDelta;
    pub fn set_empty(&mut self, pos: UVec3) -> CellDelta;
}
```

Conversions:

```rust
impl From<BlockType> for CellId;
impl From<FluidState> for CellId;
impl From<Cell> for CellId;
impl CellId {
    pub fn decode(self) -> Cell;
}
```

Use `BlockType::Stone.into()` where the target type is clearly `CellId` or `Cell`.

## Storage Serialization

Keep palette serialization semantic, not tied to runtime ID numeric values.

Names:

- `air`
- `stone`
- `oak_leaves`
- `water_8`
- `water_7`
- etc.

Runtime IDs may be versioned internally, but saved worlds should remain name-based unless we explicitly design a stable binary registry.

## Meshing Plan

### Phase 1: Table-Driven Scalar Meshing

Keep the current scalar algorithm, but make classification table-driven from `CellId`.

Expected outcome:

- Same behavior.
- Cleaner than compact `u16` bit packing.
- No repeated semantic enum matching in hot loops.

Validation:

- `cargo fmt --check`
- `cargo check --benches`
- `cargo test`
- `cargo bench --bench vertex_pulling -- --baseline pre_fluids`
- `cargo bench --bench light_propagation -- --baseline pre_fluids`

Commit after this phase if tests pass and benchmarks are no worse than current perf-fixed branch outside noise.

Suggested commit message:

```text
Use compact CellId chunk storage
```

### Phase 2: Full-Cube Face Masks

Build transient masks during mesh generation.

Mask meaning:

- `face_occluder_full_cube`: cells that fully occlude neighboring full-cube faces.
- Possibly `rendered_full_cube`: full-cube cells that should emit opaque/cutout/translucent full-cube faces.

Start simple. Build one padded occupancy representation per chunk mesh rebuild. Do not store it as a chunk component yet.

Possible shape:

```text
axis-major masks over padded 18x18 slices
u32 lanes because 18 bits do not fit in u16
```

Do not over-optimize memory first. Transient memory around 3-4KB per active mesh build is acceptable for the experiment.

Face masks:

```text
left_faces  = rendered & !neighbor_shift_left(face_occluder_full_cube)
right_faces = rendered & !neighbor_shift_right(face_occluder_full_cube)
down_faces  = rendered & !neighbor_shift_down(face_occluder_full_cube)
up_faces    = rendered & !neighbor_shift_up(face_occluder_full_cube)
front_faces = rendered & !neighbor_shift_front(face_occluder_full_cube)
back_faces  = rendered & !neighbor_shift_back(face_occluder_full_cube)
```

Iterate set bits with `trailing_zeros()` and emit descriptors.

AO can remain scalar per emitted face at first. Binary meshing removes the expensive face-visibility scan; AO is only paid for faces that survive.

Validation:

- Add targeted tests comparing scalar and binary face descriptors for existing scenarios.
- Keep existing AO tests.
- `cargo test world::chunk::mesh`
- `cargo test`
- `cargo bench --bench vertex_pulling -- --baseline pre_fluids`

Commit after binary full-cube face generation is correct and benchmarked.

Suggested commit message:

```text
Add binary full-cube face meshing
```

### Phase 3: Keep Non-Full-Cube Scalar Pass

Do not force everything through binary masks immediately.

Use hybrid meshing:

- Binary pass for full-cube face occluder blocks.
- Scalar pass for cutout/non-full-cube/internal-face blocks.
- Scalar pass for fluids/translucents initially.

This keeps complexity bounded and avoids mixing leaves, glass, water, and future shaped blocks into the first binary implementation.

Validation:

- Descriptor count tests for all scenarios.
- Visual smoke test if practical.
- `cargo test`
- `cargo bench --bench vertex_pulling -- --baseline pre_fluids`

Commit when hybrid behavior matches current output.

Suggested commit message:

```text
Split full-cube and special-case mesh passes
```

### Phase 4: Dedicated Fluid/Translucent Pass

Keep storage unified, split only mesh generation logic.

Fluid pass rules:

- Emit translucent descriptors.
- Cull faces against same fluid type where appropriate.
- Cull faces against full-cube face occluders.
- Keep water full-block visual for now.

This gives fluids different face rules without reintroducing split chunk storage.

Validation:

- Water basin face-count tests.
- Existing water/ice mesh tests.
- `cargo test world::chunk::mesh`
- `cargo test`
- `cargo bench --bench vertex_pulling -- --baseline pre_fluids`

Commit after fluid-specific output is correct.

Suggested commit message:

```text
Mesh fluids in a dedicated translucent pass
```

### Phase 5: Optional AO Optimization

Only do this after binary face masks prove worthwhile.

Options:

- Keep scalar AO, likely good enough.
- Precompute AO sample masks from `face_occluder_full_cube`.
- Batch AO by direction and slice.

Do not optimize AO until benchmark output shows AO is the remaining bottleneck.

Validation:

- Existing AO tests.
- New reference tests comparing old scalar AO and optimized AO.
- `cargo bench --bench vertex_pulling -- --baseline pre_fluids`

Commit only after correctness tests cover all six directions.

## Benchmark Cadence

Use quick validation often and full benchmarks at phase boundaries.

After small edits:

```bash
cargo fmt --check
cargo check --benches
```

After functional changes:

```bash
cargo test world::chunk::mesh
cargo test world::chunk::light
```

Before each commit:

```bash
cargo fmt --check
cargo check --benches
cargo test
```

Before and after each performance phase:

```bash
cargo bench --bench vertex_pulling -- --baseline pre_fluids
cargo bench --bench light_propagation -- --baseline pre_fluids
```

Use `light_propagation` after storage/classification changes. Use `vertex_pulling` after mesh changes.

## Clippy Cadence

Run Clippy before merging/committing larger phase work, not after every small edit.

Command:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

If existing third-party or feature-specific warnings block this, record them and run the closest useful command without hiding new warnings.

## Commit Strategy

Commit at stable, reviewable boundaries:

1. `CellId` storage and semantic API compiles/tests/benchmarks.
2. Table-driven scalar mesh/light classification complete.
3. Binary full-cube face mask prototype passes scalar comparison tests.
4. Hybrid mesh passes replace scalar full-cube path.
5. Dedicated fluid/translucent pass if it stays clean.
6. AO optimization only if needed.

Avoid committing giant mixed changes that combine storage, mesh algorithm, and rendering pass behavior unless the intermediate state cannot compile.

## Stash / Fallback

Before starting this experiment, preserve the current perf-fixed implementation:

```bash
git stash push --include-untracked -m "compact-cell-storage-perf-fixed"
```

A branch is safer than a stash if the experiment will take more than one session:

```bash
git switch -c experiment/cell-id-binary-meshing
```

Do not drop the stash until the `CellId` plus binary meshing path has passed tests and benchmark comparison.

## Open Questions

- Is `u8 CellId` enough long-term, or should we start at `u16` for registry headroom?
- Should `CellId` numeric values be stable enough for direct save storage, or remain runtime-only with semantic palette names?
- Should water source be max-level-only for all fluids, or should source behavior live in fluid definitions?
- Should ice be handled in the translucent scalar pass or remain a block pass with translucent material?
- How much descriptor ordering stability do we need between scalar and binary mesh generation?
