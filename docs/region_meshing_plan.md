# Region-based greedy meshing plan

## Summary

Mesh N×N×N chunks together into a "region" to get larger quads and eliminate
interior faces between adjacent chunks within the region.

## Why

- **Fewer faces**: interior solid→solid faces between chunks inside the region
  cancel out (full_cube[c] & !full_cube[c±1] naturally culls them).  A 2×2×2
  solid region emits 6×32×32=6,144 surface faces instead of 8×1,536=12,288.
- **Larger quads**: greedy merging spans the full 32-wide plane instead of
  hitting chunk boundaries at 16.  One 32×32 quad replaces up to 4 separate
  16×16 quads (fewer vertices, fewer draw calls).

## Why not partitioned remains why not partitioned

The partitioned approach (binning faces by AO key before merging) adds a
second bitwalk over every emit plane.  The cost scales linearly with plane
size (16×16→256 bits, 32×32→1024 bits), same as the saving it tries to
achieve.  Greedy always wins.  The region mesher uses standard single-pass
greedy with the FullCubeMask + 1x1 dedup + trailing_ones optimizations.

## Region size

Pick 2×2×2 chunks = 32³ blocks as the starting point:

```
Region size:   32 × 32 × 32  (8 chunks, aligned to even chunk coords)
Padded size:   34 × 34 × 34  =  39,304 blocks
Mask layers:   34 per axis   = 102 layers total
Plane width:   34 × 34       = 1,156 bits/plane  →  19 u64s
Bitplane mem:  3 axes × 34 layers × 2 masks × 19 u64s × 8 bytes ≈ 31 KB
FullCubeMask:  39,304 bits   = 615 u64s ≈ 5 KB
Block array:   39,304 bytes  ≈ 38 KB  (1-byte BlockType)
Total mem:     ≈ 74 KB       (fits L2, borderline L1)
```

Alternative 3×3×3 = 48³: bitplanes ≈ 96 KB, block array ≈ 122 KB, total ≈ 223 KB.
Still fits L2 (1 MB on Zen 5) but starting to push it.

Alternative 4×4×4 = 64³: bitplanes ≈ 218 KB, block array ≈ 281 KB, total ≈ 500 KB.
Needs L3 or stream-from-RAM.

## Neighbor data

To properly compute AO and face culling at region boundaries, we need 1 block
of padding on every face.  The padded region is (N×chunk_size + 2)³ blocks.

For a 2×2×2 region at chunk coords (0,0,0)–(1,1,1):
```
Chunks needed for padding: min=(−1,−1,−1), max=(2,2,2)
That's 4×4×4 = 64 chunks total.
```

At the center of a render distance ≥ 4 chunks, all 64 are already in memory.
At render-distance boundaries, some padding chunks are absent — those faces
get full brightness (no AO occlusion from missing data) and the mesh is still
correct, just missing AO at region edges.  So the edge of the render distance
is handled naturally: treat missing neighbor blocks as air.

## Pipeline

```
make_region_mesh(region_origin: IVec3, chunks: &HashMap<IVec3, &Chunk>)
  → RegionMeshInput { padded_blocks: [BlockType; 34³], full_mask, ... }
  → build_region_bitmasks(blocks)
  → count_faces(data)
  → emit_faces(blocks, data, tables, builders)
  → Vec<(Layer, Mesh)>
```

Key differences from current greedy:

| Aspect | Current (1 chunk) | Region (2×2×2) |
|---|---|---|
| Padded volume | 18³ | 34³ |
| Planes per axis | 18 | 34 |
| Plane width | 18×18→6 u64s | 34×34→19 u64s |
| Block count | 5,832 | 39,304 |
| CHUNK_SIZE constant | 16 | 32 (region_size) |
| PADDED_CHUNK_SIZE | 18 | 34 |
| PADDED_CHUNK_VOLUME | 5,832 | 39,304 |

The algorithm is identical — just the constants change.  All the same functions
(build_bitmasks, emit_plane_opaque, single_vertex_ao, etc.) work unchanged if
they use the scaled constants.

## Implementation strategy

### Option A: Duplicate greedy.rs with new constants (simplest)

Copy `greedy.rs` → `greedy_region.rs`.  Replace `CHUNK_SIZE=16` with
`REGION_SIZE=32`, `PADDED_CHUNK_SIZE=18` with `PADDED_REGION_SIZE=34`, etc.
Everything else stays the same.  The mesher struct takes the region size as
a const generic or hardcoded constant.

Pros: Fast to implement, no risk of breaking current mesher.
Cons: Duplicated code.

### Option B: Make greedy.rs generic over chunk size (cleaner)

Add `const CHUNK_SIZE: usize` as a const generic parameter.  All functions
become generic, constants derived at compile time.  The Mesher struct becomes
`GreedyMesher<const N: usize>` where N is the chunk size in blocks.

Pros: No code duplication.  Can instantiate for 16, 32, 48, etc.
Cons: More complex, potentially longer compile times, harder to read.

### Option C: RegionMesher as a higher-level wrapper (moderate)

Keep 1-chunk mesher as-is.  Create a `RegionMesher` that:
1. Collects chunks for the region + padding
2. Builds the padded block array (flattening N² chunks into one array)
3. Calls a generic `make_greedy_region_mesh(input)` with the region-size constants
4. Returns the resulting mesh(es) keyed by the region position

This separates concerns: chunk collection is separate from meshing.

**Recommendation: Option A for prototype, then evaluate Option B.**

## Wiring into the engine

### Current flow:
```
rebuild_chunk_meshes()  // runs per dirty chunk
  → ChunkMeshBlocks::from_chunks(center_pos, &all_chunks)
  → mesher.mesh(input)  // 1 chunk
  → attach mesh to chunk entity
```

### Region flow:
```
rebuild_region_meshes()  // runs per dirty region
  → for each dirty region (groups of 2×2×2 chunks):
    → collect chunks for region + padding from all loaded chunks
    → RegionMeshBlocks::from_chunks(region_origin, &all_chunks)
    → mesher.mesh(input)  // N×N×N region
    → for each chunk in region:
      → split mesh into per-chunk geometry
      → attach to chunk entity
```

### The mesh-splitting problem

After meshing a region, we get ONE mesh (or one per material layer) covering
the entire 32×32×32 region.  But we need per-chunk meshes for Bevy's
chunk-entity rendering (culling, LOD, etc.).

Options:
1. **Split by geometry**: clip quads to chunk boundaries after meshing.
   Complex, adds vertices at boundaries.
2. **Per-chunk sub-meshes**: during region meshing, track which chunk each quad
   belongs to.  Emit into separate per-chunk buffers.  Quads that span chunks
   are split at chunk boundaries.
3. **Whole-region entities**: create a "region entity" instead of per-chunk
   entities.  One mesh per region, culled as a unit.  Simpler but coarser
   culling.

Option 2 is the most practical: during `push_merged_face`, check the quad's
world coordinates against chunk boundaries and clip/split as needed.  The
`push_merged_face` function already takes world coordinates — splitting into
per-chunk buffers just adds a chunk-index lookup and potentially 2 quads
instead of 1 for boundary-spanners.

## Schedule / dirty tracking

Currently: one dirty bit per chunk.  A block change marks the chunk dirty.

Region approach: one dirty COUNT per region (how many chunks in the region
are dirty).  When a block changes in a chunk:
- Mark the region containing that chunk as dirty
- When the count reaches a threshold (e.g., half the region), rebuild

Or simpler: rebuild the region whenever ANY chunk in it is dirty.  This means
up to 8 chunks get remeshed for a 1-block change, which is wasteful for
isolated changes but amortized over the region lifetime (most region rebuilds
happen at load time, not mid-gameplay).

## Render distance boundary handling

If the render distance is R chunks, regions that cross the boundary need
special handling:
- Skip regions entirely outside the render distance
- For regions partially inside: only mesh the visible chunks, or skip the
  whole region
- Simplest: only mesh complete regions (all N³ chunks must be in render
  distance).  Waste at most N−1 chunks at edges.

For R=8 and N=2 (2×2×2 regions): 8/2=4 regions per axis, no waste at
boundaries (8 is divisible by 2).

For R=8 and N=3: 2 regions per axis, waste 2 chunks at each edge.  Acceptable
if the interior savings outweigh the edge waste.  Or drop to N=2 at edges.

## Open questions

1. **What N?**  2×2×2 is safe.  3×3×3 has better merging at the cost of more
   memory and more edge waste.
2. **Mesh splitting**: split during emission (per-chunk buffers) or after
   (geometry clipping)?
3. **Dirty granularity**: rebuild whole region or accumulate multiple dirty
   chunks before rebuilding?
4. **Vertical**: mesh all vertical chunks in the column or subdivide
   vertically too?  Vertical subdivision by 2 is natural.
5. **AO at region edges**: with missing neighbor chunks, use "open" AO
   (no occlusion).  Is this visually acceptable?

## Benchmark results (2026-06-14)

Implemented 2x2x2 region mesher (`greedy_region.rs`) — 34³ padded, 19 u64s/bitplane,
~74 KB working set.  Compared vs 8× single-chunk greedy.

| Scenario | 8× Single | Region | Ratio | Notes |
|---|---|---|---|---|
| Full stone open | 142 µs | 83.8 µs | **0.59x** | 8x fewer quads (6 vs 48) |
| Generated surface | 226 µs | 101.8 µs | **0.45x** | Interior culling + bigger quads |
| Dense leaves | 1.47 ms | 1.38 ms | **0.94x** | Slightly faster |
| Checkerboard | 2.34 ms | 5.0 ms | **2.13x** | No merge possible, overhead dominates |
| Empty | 7.2 ns | 33 ns | 4.6x | Both trivial |
| Full stone buried | 3.3 µs | 52.5 µs | 15.9x | Region scans 64 chunks before skip |

**Files created:**
- `src/world/chunk/mesh/greedy_region.rs` — region mesher (746 lines)
- AO offsets generated dynamically from padded dimensions (72 triplets)

**Key finding:** Region wins on real-world terrain and solid blocks, only regresses when
no merging is possible.  The per-plane overhead (19 vs 6 u64s) is amortized by
quad reduction and interior culling in all practical scenarios.
