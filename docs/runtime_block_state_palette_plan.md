# Runtime Block-State Palette Plan

## Summary

Use a chunk-local runtime palette instead of storing rich per-cell data or fixed
global cell IDs directly in every voxel.

The target shape is:

```text
Chunk cells      = compact local palette indices, usually u8
Chunk palette    = states used by this chunk + hot metadata cached beside them
Global registry  = canonical block-state definitions
Side storage     = sparse arbitrary block-entity data only
```

This keeps simple chunks near the old 4 KiB cell footprint while still allowing
stairs, slabs, waterlogging, growth stages, orientation, and future block-state
data without putting NBT-like payloads in every cell.

This supersedes the older fixed `CellId` idea in `cell_id_binary_meshing_plan.md`
for long-term block-state growth. The binary meshing ideas from that document
still apply, but the cell identity source should become a local palette index.

## Current Context

The current model is roughly:

```rust
pub enum ChunkCell {
    Empty,
    Block(BlockType),
    Fluid(FluidState),
}
```

This is simple and readable, but it has two major limits:

- A cell cannot naturally represent finite block states like `slab[top]`,
  `stairs[facing=north,half=bottom]`, or `leaves[waterlogged=true]`.
- Water is partly simulation state and partly a special render ID, which leaks
  into meshing, texture lookup, UI icons, shader descriptor packing, and tests.

The next feature pressure is not arbitrary NBT. It is finite block-state
variation. That should not live in a per-cell NBT blob.

## Goals

- Keep the common case compact: simple terrain chunks should usually store one
  byte per cell.
- Avoid a uniform `u32` cost for every voxel unless a chunk genuinely needs it.
- Avoid per-cell dynamic data, strings, maps, or optional payloads in hot loops.
- Keep meshing, lighting, and collider loops cache-friendly.
- Make states like slabs, stairs, orientation, waterlogging, and crop age finite
  block states.
- Keep arbitrary block-entity data sparse and off the hot path.
- Let chunk storage grow only when local palette complexity requires it.

## Non-Goals

- Do not add `collision_shape` to the first design. Current collision is still
  voxel-based and works well with Avian's voxel collider path.
- Do not build a full NBT system for normal block states.
- Do not bit-pack custom meanings into each chunk's cell byte.
- Do not optimize below one byte per cell at runtime. Sub-byte packing is better
  left to disk/network serialization if it ever matters.

## Core Model

### Global Block State ID

`BlockStateId` is the canonical identity for a finite block state.

```rust
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockStateId(pub u32);
```

Examples of states:

```text
air
stone
oak_log[axis=y]
oak_leaves[waterlogged=false]
oak_leaves[waterlogged=true]
stone_slab[type=bottom,waterlogged=false]
stone_slab[type=bottom,waterlogged=true]
stone_slab[type=top,waterlogged=false]
stone_slab[type=top,waterlogged=true]
stone_slab[type=double]
oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]
oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=true]
```

The registry should only contain valid state combinations. It should not blindly
materialize impossible cross-products.

### Hot Metadata

Hot systems should not decode strings or generic properties in their inner
loops. They should read compact metadata.

```rust
#[derive(Clone, Copy, Debug)]
pub struct HotBlockStateMeta {
    pub render_id: u16,
    pub mesh_flags: u8,
    pub light_opacity: u8,
    pub light_emission: u8,
    pub fluid_level: u8,
}
```

Notes:

- `fluid_level == 0` means no fluid contribution.
- Waterlogged states expose their water through this metadata.
- A later fluid system may want `fluid_type`; add it only when water is no
  longer the only fluid.
- Collision shape is deliberately omitted for now.

### Chunk Palette

Each chunk stores only the states it actually uses.

```rust
pub struct ChunkPalette {
    entries: Vec<PaletteEntry>,
}

#[derive(Clone, Copy, Debug)]
pub struct PaletteEntry {
    pub state: BlockStateId,
    pub hot: HotBlockStateMeta,
}
```

The `hot` field duplicates global registry data intentionally. The palette is
small and chunk-local, so meshing and lighting stay in cache and do not chase a
large global registry for every cell.

### Cell Storage

Cells store local palette indices. Most chunks should stay in `U8`.

```rust
pub enum CellStorage {
    U8(Box<[u8; CHUNK_VOLUME]>),
    U16(Box<[u16; CHUNK_VOLUME]>),
    U32(Box<[u32; CHUNK_VOLUME]>),
}
```

Promotion rules:

- `U8`: up to 256 local states.
- `U16`: up to 65,536 local states.
- `U32`: fallback for pathological/modded chunks.

Runtime demotion should not happen during normal edits. Promote when needed,
then optionally compact/demote during save, unload, or explicit maintenance.

### Chunk Shape

```rust
pub struct Chunk {
    palette: ChunkPalette,
    cells: CellStorage,
    block_entities: BlockEntityMap,
}
```

`block_entities` is for true arbitrary data only: chest inventory, sign text,
furnace progress, etc. It should not store slab type, stair orientation,
waterlogged, crop age, or other finite state.

## Why Not Custom Bit Layouts Per Chunk?

A tempting design is to reinterpret each cell byte based on what the chunk
contains:

```text
7 bits block type + 1 bit waterlogged
5 bits block type + 3 bits stair orientation
larger layout only when needed
```

This optimizes for byte density, but makes the meaning of a cell depend on a
chunk-specific schema. That complicates:

- random access
- mutation
- neighbor comparisons
- meshing
- save/load
- debugging
- tests
- future tooling

A palette byte has one stable meaning:

```text
cell byte 17 => chunk.palette[17]
```

The palette entry owns the semantic meaning. That is simpler and usually just
as compact at runtime.

## Registry Size Concern

A large global state registry is acceptable if hot loops do not scan it.

Meshing should not do this per voxel:

```rust
let meta = global_registry[state_id];
```

Meshing should do this:

```rust
let local = cells.get(i) as usize;
let meta = chunk.palette.entries[local].hot;
```

The chunk palette is tiny compared to the global registry. A complex global
registry does not matter much if each chunk only touches the entries it uses.

## Expected Runtime Memory

Current pre-fluid ideal was roughly:

```text
4096 cells * 1 byte = 4 KiB
```

A common runtime-palette chunk remains:

```text
4096 cells * 1 byte = 4 KiB
palette entries      = usually tiny, often under 1 KiB
```

Only chunks with more than 256 distinct local block states promote to `U16`:

```text
4096 cells * 2 bytes = 8 KiB
```

`U32` should be rare. If it becomes common, the block-state design or modded
content assumptions need review.

## Hot Path Access

Provide helpers that make the common path direct and boring.

```rust
impl Chunk {
    pub fn state_id(&self, pos: LocalChunkPos) -> BlockStateId;
    pub fn palette_index(&self, pos: LocalChunkPos) -> u32;
    pub fn hot_meta(&self, pos: LocalChunkPos) -> HotBlockStateMeta;
    pub fn set_state(&mut self, pos: LocalChunkPos, state: BlockStateId, registry: &BlockRegistry) -> CellDelta;
}
```

For full scans, avoid constructing `LocalChunkPos` for every cell unless needed:

```rust
for i in 0..CHUNK_VOLUME {
    let local = chunk.cells.get_linear(i) as usize;
    let meta = chunk.palette.entries[local].hot;
}
```

Meshing can build its padded buffers from local palette metadata:

```rust
padded_render_ids[i] = entry.hot.render_id;
padded_mesh_flags[i] = entry.hot.mesh_flags;
padded_fluid_levels[i] = entry.hot.fluid_level;
```

This keeps the current binary/hybrid meshing architecture viable.

## Mutation Path

When setting a block state:

1. Look for the state in the chunk palette.
2. If present, write that local index into `cells`.
3. If absent, fetch `HotBlockStateMeta` from the global registry and append a
   new `PaletteEntry`.
4. Promote `CellStorage` only if the local palette outgrows the current width.
5. Return a `CellDelta` containing old/new state or old/new hot metadata needed
   for counts and dirty marking.

Optional implementation detail:

```rust
pub struct ChunkPalette {
    entries: Vec<PaletteEntry>,
    lookup: HashMap<BlockStateId, u32>,
}
```

The lookup is not used by meshing. It is only for edits and loading. If palette
sizes stay tiny, a linear scan may be enough initially and simpler.

## Block Entities And Local Positions

Use sparse side storage for arbitrary per-cell data.

```rust
pub type BlockEntityMap = HashMap<LocalChunkPos, BlockEntityData>;
```

This should probably not be `HashMap<u16, _>` in the public API. A packed index
is fine internally, but a typed local position is safer and clearer.

Recommended follow-up doc: design `LocalChunkPos` / `LocalBlockIndex` utilities.

Open questions for that follow-up:

- Should public sparse maps key by `LocalChunkPos` or `LocalBlockIndex`?
- Should `LocalChunkPos` validate `0..16` on construction?
- Should linear scan APIs use `LocalBlockIndex` to avoid repeated x/y/z math?
- Should conversion order be explicitly `y + 16 * (z + 16 * x)` to keep local
  linear scans y-fastest?

Do not block the palette design on this. The palette design only needs a stable
local key type before block entities become important.

## Serialization

Disk storage should remain palette-based and semantic.

Save chunk palette entries as stable state names/properties, not raw runtime
local indices:

```text
palette:
  0: air
  1: stone
  2: oak_leaves[waterlogged=true]
  3: oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]
cells:
  bit-packed palette indices
block_entities:
  sparse local positions + data
```

Runtime local palette order can differ from disk order if needed, but keeping
them aligned on load is simpler.

Disk can bit-pack indices to the minimum number of bits. Runtime should usually
stay byte/word-addressable for fast random access.

## Migration Plan

### Phase 1: Add Types Without Rewriting Hot Loops

- Add `BlockStateId` as an opaque type.
- Add `HotBlockStateMeta` without `collision_shape`.
- Add a tiny initial registry mapping current blocks, air, and water states.
- Keep existing `ChunkCell` APIs as compatibility wrappers during transition.

### Phase 2: Add Chunk Palette Storage

- Introduce `ChunkPalette` and `CellStorage::U8` first.
- Convert loaded/generated chunks into palette chunks.
- Keep save/load semantic and palette-based.
- Add promotion to `U16` only when needed.

### Phase 3: Move Meshing To Palette Metadata

- Build padded mesh buffers from `PaletteEntry.hot`.
- Stop using water as a globally special render ID except as a render material.
- Keep current binary full-cube path and scalar special-shape path.

### Phase 4: Convert Block/Fluid Features Into States

- Replace `ChunkCell::Fluid(FluidState)` with water states where appropriate.
- Model waterlogged blocks as finite block states.
- Add slab/stair finite state sets.
- Keep true arbitrary payloads in `block_entities`.

### Phase 5: Clean Up Old Semantic Leaks

- Rename render-facing helpers so `kind` does not mean render ID.
- Remove special `WATER_RENDER_ID` conditionals where registry metadata can own
  the behavior.
- Split chunk storage from fluid simulation if `chunk/mod.rs` keeps growing.

## Performance Checks

Benchmark after each phase:

- chunk mesh rebuild on simple terrain
- chunk mesh rebuild with water
- chunk mesh rebuild with many palette states
- light propagation
- chunk load/decode
- block placement in chunks near the 256-entry promotion boundary

Success criteria:

- Common chunks stay at `U8` runtime storage.
- Meshing does not perform global registry lookups per voxel.
- Palette promotion is rare and measurable.
- Palette metadata remains small enough to stay cache-hot.

## Bottom Line

The right long-term model is not rich per-cell NBT and not custom semantic bits
inside each cell byte.

Use dense chunk-local palette indices for cells, cache hot metadata in the local
palette, keep finite block variation in `BlockStateId`, and reserve sparse side
storage for true block entities.

This gives the flexibility needed for slabs, stairs, waterlogging, and future
stateful blocks while preserving the cache behavior that made the current simple
chunk representation fast.
