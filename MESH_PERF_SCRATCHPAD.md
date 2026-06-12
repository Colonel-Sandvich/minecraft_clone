# Mesh Perf Scratchpad

## Baseline

Command: `cargo bench --bench chunk_meshing -- --warm-up-time 1 --measurement-time 2`

Reference full mesh times from first short run:

| Scenario | Time |
| --- | ---: |
| empty | ~1.32 us |
| full_stone_buried | ~29.2 us |
| generated_underground | ~29.6 us |
| generated_surface | ~59.1 us |
| full_stone_open | ~97.3 us |
| mixed_transparency | ~187.6 us |
| checkerboard_stone | ~544.6 us |
| dense_leaves | ~590.2 us |

Reference input build:

| Scenario family | Time |
| --- | ---: |
| single center chunk | ~1.80 us |
| full 27-neighbor halo | ~3.05 us |

## Hypotheses

- Biggest exposed-face costs are intermediate `QuadGroups`, repeated texture map lookup, repeated color lookup, and AO helper/block-profile calls.
- Biggest buried/full-solid cost is scanning and six neighbor/profile checks per visible block even though no faces emit.
- Padded block copy is already small, so defer optimizing it until meshing hot paths are improved.

## Iterations

- v1 target: direct table-driven mesh emission from `ChunkMeshBlocks`.

### v1: `DirectChunkMesher`

Command: `cargo bench --bench chunk_meshing -- --warm-up-time 1 --measurement-time 2`

| Scenario | Reference | Direct | Result |
| --- | ---: | ---: | ---: |
| empty | ~1.32 us | ~2.24 us | slower, table setup dominates |
| full_stone_open | ~96.2 us | ~44.1 us | ~2.2x faster |
| full_stone_buried | ~28.2 us | ~18.7 us | ~1.5x faster |
| checkerboard_stone | ~554.9 us | ~222.9 us | ~2.5x faster |
| dense_leaves | ~611.0 us | ~249.6 us | ~2.4x faster |
| mixed_transparency | ~188.4 us | ~77.5 us | ~2.4x faster |
| generated_underground | ~28.0 us | ~18.6 us | ~1.5x faster |
| generated_surface | ~59.1 us | ~25.1 us | ~2.4x faster |

Notes:

- Direct emission is a clear win once faces are emitted.
- Empty chunks regress because `BlockMeshTables` is rebuilt per call.
- Next: prepared direct mesher with one-time table build.

### v2: `PreparedDirectChunkMesher` by reference

Command: `cargo bench --bench chunk_meshing -- --warm-up-time 1 --measurement-time 2`

| Scenario | Prepared Direct |
| --- | ---: |
| empty | ~1.54 us |
| full_stone_open | ~55.4 us |
| full_stone_buried | ~19.4 us |
| checkerboard_stone | ~287.6 us |
| dense_leaves | ~321.3 us |
| mixed_transparency | ~99.6 us |
| generated_underground | ~19.7 us |
| generated_surface | ~30.1 us |

Notes:

- Empty improves versus v1 direct, but high-face scenarios regress badly versus v1.
- Likely cause: hot loop table access through reference plus missed scalarization/inlining.
- Next: pass/copy the small table by value into the direct hot loop.

### v3: zero-output skip and counted direct

Changes:

- `ChunkMeshBlocks` tracks center rendered/full-cube counts.
- Empty center chunks return no meshes immediately.
- Fully full-cube center chunks with full-cube face-neighbor shells return no meshes immediately.
- `CountedDirectChunkMesher` pre-counts faces and reserves exact mesh buffer capacities.

Findings:

- Zero-output mesh-only time drops to single-digit ns because it returns `Vec::new()`.
- Input-build summaries initially cost too much when counts updated through struct fields in the inner loop.
- Splitting center-copy from neighbor-copy and accumulating counts in locals fixed that.

Input build after center-copy split:

| Scenario family | Time |
| --- | ---: |
| single center chunk | ~1.0 us |
| full 27-neighbor halo | ~1.65-1.70 us |

### v4: adaptive direct/count path

Current adaptive heuristic:

- Skip if empty or fully buried.
- Use reference path for fewer than 32 rendered center blocks.
- Use direct path for all-full-cube center chunks.
- Otherwise use counted direct path.

Short end-to-end run (`cargo bench --bench chunk_meshing end_to_end -- --warm-up-time 0.5 --measurement-time 1` plus focused adaptive rerun):

| Scenario | Reference E2E | Direct E2E | Counted E2E | Adaptive E2E |
| --- | ---: | ---: | ---: | ---: |
| empty | ~1.00 us | ~2.83 us before split | ~2.85 us before split | ~1.00 us |
| single_stone | ~2.89 us | ~5.28 us before split | ~6.05 us before split | ~2.87 us |
| sparse_stone | ~33.5 us | ~21.2 us before split | ~19.8 us before split | ~18.4 us |
| full_stone_open | ~97.6 us | ~57.4 us before split | ~61.6 us before split | ~55-57 us |
| full_stone_buried | ~2.07 us | ~4.60 us before split | ~4.54 us before split | ~2.07 us |
| checkerboard_stone | ~560 us | ~295 us before split | ~256-258 us before split | ~252 us |
| dense_leaves | ~599 us | ~327 us before split | ~295 us before split | ~294 us |
| mixed_transparency | ~188.5 us | ~102 us before split | ~93.8 us before split | ~91.4 us |
| generated_underground | ~2.14 us | ~4.50 us before split | ~4.57 us before split | ~2.06 us |
| generated_surface | ~62.1 us | ~35 us before split | ~34.4 us before split | ~31.6 us |

Notes:

- End-to-end adaptive is currently the best general candidate.
- Counted direct reduces allocation/capacity churn on high-face cases.
- Direct still wins for all-full-cube open chunks and ultra-sparse chunks.
- A real allocation profiler is still needed; Criterion times strongly imply less allocation pressure for counted high-face paths, but does not measure allocation counts directly.
- Runtime `rebuild_chunk_meshes` now uses `AdaptiveDirectChunkMesher`; public `make_chunk_meshes` remains the reference path.

### v5: full-cube shell mesher

Changes:

- `FullCubeShellChunkMesher` scans only six boundary planes for all-full-cube center chunks.
- Adaptive path routes all-full-cube centers to shell mesher.
- Center summaries (`center_rendered_blocks`, `center_full_cube_blocks`) computed during center-copy.

Short end-to-end run (`cargo bench --bench chunk_meshing ...`):

| Scenario | Adaptive E2E (pre-shell) | Adaptive E2E (with shell) |
| --- | ---: | ---: |
| empty | ~1.00 us | ~1.09 us |
| single_stone | ~2.87 us | ~3.20 us |
| sparse_stone | ~18.4 us | ~19.9 us |
| full_stone_open | ~55-57 us | ~36.8 us |
| full_stone_buried | ~2.07 us | ~2.35 us |
| checkerboard_stone | ~252 us | ~285 us |
| dense_leaves | ~294 us | ~328 us |
| mixed_transparency | ~91.4 us | ~102 us |
| generated_underground | ~2.06 us | ~2.34 us |
| generated_surface | ~31.6 us | ~35.8 us |

Notes:

- Shell path is ~1.5x faster for all-full-cube open centers.
- Other scenarios slightly regressed due to center-summary overhead.
- Adaptive heuristic still uses reference for <32 rendered blocks, shell for all-full-cube, counted direct otherwise.

### v6: static block property arrays

Changes:

- Replaced runtime `BlockMeshProperty` table with static const arrays (`BLOCK_IS_RENDERED`, `BLOCK_IS_FULL_CUBE`, `BLOCK_IS_DOUBLE_SIDED`, `BLOCK_MATERIAL_LAYER_INDEX`).
- Hot-loop functions (`should_emit_face_from_indices`, `face_ao_from_indices`, `block_occludes_ambient_light_from_index`) now use static arrays instead of passed-by-reference property tables.
- `BlockMeshProperty` type and `block_mesh_properties()` removed entirely.
- All 84 tests pass.

Impact:

- Eliminates table construction and passing overhead in direct/count/shell paths.
- Static arrays are trivially constant-folded and inlined by the compiler.
