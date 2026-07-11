# Contiguous Query Access Candidates

This tracks Bevy 0.19 `contiguous_iter` / `contiguous_iter_mut` candidates. Do one candidate at a time: add or update a benchmark first, compare normal iteration against contiguous access, then only apply the production change if the result is convincing.

## Protocol

- Benchmark before changing production code.
- Prefer targeted system-level benchmarks over raw query scans once a loop performs real work. Build realistic worlds, run a Bevy `Schedule`, and use batched setup so entity insertion is not part of the measured operation.
- Use raw query microbenchmarks only for narrow setup work where the production code truly is just collecting or scanning query data.
- Prefer `expect(...)` over fallback paths when the query shape is fixed and should always be dense.
- Keep `par_iter` where benchmarks show parallel overhead is worth it.
- Record benchmark command, result, decision, and production files changed under the candidate.

## Todo

- [x] Candidate 1: meshing setup position-map scans in `src/world/chunk/mesh/mod.rs`.
- [x] Candidate 2: dirty mesh rebuild loop threshold: contiguous serial path for small batches, `par_iter` for large batches.
- [x] Candidate 3: light upload light-map build in `src/world/chunk/mesh/mod.rs`.
- [x] Candidate 3b: light upload dirty loop in `src/world/chunk/mesh/mod.rs`.
- [x] Candidate 4: dimension light rebuild setup scans in `src/world/dimension/light.rs`.
- [x] Revalidate Candidate 1 and Candidate 3 under targeted system-level benchmarks.
- [x] Candidate 5: fluid active chunk stepping and boundary write lookup in `src/world/chunk/fluid.rs`.
- [x] Candidate 6: collider rebuild dirty loop in `src/world/chunk/collider.rs`.
- [x] Candidate 7: render extraction/prep redesign away from `Changed<T>` queries, only if GPU/render benchmarks justify it.

## Candidate 1: Meshing Setup Position-Map Scans

Production target: `src/world/chunk/mesh/systems.rs`

Current loops:

- `all_chunks_q.iter().map(|(pos, chunk)| (pos.0, chunk)).collect::<HashMap<_, _>>()`
- `light_q.iter().map(|(pos, light)| (pos.0, light)).collect::<HashMap<_, _>>()`

Hypothesis: `contiguous_iter().expect(...)` may reduce ECS iteration overhead while building the per-system position maps. Expected payoff is modest because `HashMap` insertion and later mesh generation likely dominate.

Benchmark command: `cargo bench --bench ecs_queries`

Benchmark added: `ecs_query_meshing_setup` in `benches/ecs_queries.rs`.

Results for 4096 chunk entities after adding fair preallocated normal-iterator variants:

- `chunk_map_iter`: 18.090-18.307 us
- `chunk_map_iter_prealloc`: 18.231-18.745 us
- `chunk_map_contiguous`: 11.457-11.798 us
- `light_map_iter`: 17.797-18.024 us
- `light_map_iter_prealloc`: 17.931-18.162 us
- `light_map_contiguous`: 11.346-11.696 us

Decision: apply. Contiguous map builds are about 35-37% faster than the normal iterator variants in this benchmark. The change is low risk because the query uses table components and only archetypal filters.

Production change: `rebuild_chunk_meshes` now builds `chunks_by_pos` and `lights_by_pos` with `contiguous_iter().expect(...)`.

System-level revalidation command: `cargo bench --bench ecs_queries -- ecs_system_mesh_rebuild_maps`

System-level revalidation benchmark: `ecs_system_mesh_rebuild_maps` in `benches/ecs_queries.rs`. This builds a 16x16x4 generated chunk world, runs a Bevy `Schedule`, initializes the schedule outside the measured closure, and includes map builds, descriptor generation, and padded light-data construction. It compares normal iterator map builds against contiguous map builds while keeping the dirty mesh loop shape the same.

Revalidated results:

- 1 dirty chunk: iter 242.40-287.38 us, contiguous 218.93-238.79 us
- 4 dirty chunks: iter 278.06-338.12 us, contiguous 276.19-477.73 us
- 16 dirty chunks: iter 463.54-528.49 us, contiguous 421.59-469.69 us
- 64 dirty chunks: iter 1.0281-1.1026 ms, contiguous 1.0162-1.1348 ms

Revalidated decision: keep. The full system-level benchmark is noisier because mesh generation dominates, but contiguous remains clearly better at 1 and 16 dirty chunks and effectively tied at 64. The 4-dirty result has a noisy high outlier, not a convincing regression. The original narrow map-build result still supports the low-risk production change for the actual setup scans.

Verification:

- `cargo fmt --check`
- `cargo check`
- `cargo test`
- `cargo bench --bench ecs_queries`

## Candidate 2: Dirty Mesh Rebuild Serial/Parallel Threshold

Production target: `src/world/chunk/mesh/mod.rs`

Current loop: `dirty_chunks_q.par_iter().for_each_init(...)`

Hypothesis: for small dirty batches, a serial contiguous path could beat `par_iter` overhead. For large dirty batches, `par_iter` should still win because descriptor generation is CPU-heavy.

Benchmark command: `cargo bench --bench chunk_meshing`

Benchmark added: `chunk_mesh_dirty_loop` in `benches/chunk_meshing.rs`.

Results using realistic-terrain chunks, `ChunkMeshBlocks::from_chunks`, and `mesher::build`:

- 1 dirty chunk: serial contiguous 14.762-15.046 us, parallel 16.915-17.289 us
- 4 dirty chunks: serial contiguous 60.894-61.273 us, parallel 23.293-24.770 us
- 16 dirty chunks: serial contiguous 242.04-244.42 us, parallel 53.599-55.542 us
- 64 dirty chunks: serial contiguous 983.61 us-1.0055 ms, parallel 135.68-147.05 us
- 256 dirty chunks: serial contiguous 3.9566-3.9951 ms, parallel 397.51-407.31 us

Decision: no production change. Serial contiguous only wins for exactly one dirty chunk, by about 2 us. At 4+ dirty chunks, `par_iter` is already much faster. A threshold branch would duplicate the mesh-build loop for a small single-chunk gain and risks making the system harder to maintain.

Production change: none.

## Candidate 3: Light Upload Light-Map Build

Production target: `src/world/chunk/mesh/mod.rs`

Current loops:

- `light_q.iter().map(|(pos, light)| ...).collect()`
- `for (chunk_pos, chunk_entity) in &dirty_chunks_q { ... }`

Hypothesis: contiguous access may help the light map scan; dirty loop payoff depends on dirty count and child/light update cost.

Benchmark command: `cargo bench --bench ecs_queries`

Benchmark used: `ecs_query_meshing_setup/light_map_*` in `benches/ecs_queries.rs`, which has the same `(&ChunkPosition, &ChunkLight)` map-build shape.

Results for 4096 chunk-light entities:

- `light_map_iter`: 17.760-17.983 us
- `light_map_iter_prealloc`: 17.900-18.020 us
- `light_map_contiguous`: 11.119-11.375 us

Decision: apply only the light-map build. This is the same low-risk dense table query as Candidate 1 and remains about 36-38% faster than normal iteration.

Production change: `upload_chunk_lights` now builds `lights_by_pos` with `contiguous_iter().expect(...)`.

System-level revalidation command: `cargo bench --bench ecs_queries -- ecs_system_light_upload_map_build`

System-level revalidation benchmark: `ecs_system_light_upload_map_build` in `benches/ecs_queries.rs`. This runs the real dirty light upload loop, including padded-light construction, child traversal, mutable `ChunkMeshLight` writes, and command removal. Only the light-map build changes between variants.

Revalidated results with 4096 chunk-light entities and 3 chunk mesh layer children per dirty chunk:

- 1 dirty chunk: iter 241.13-331.28 us, contiguous 169.57-176.10 us
- 4 dirty chunks: iter 189.09-206.63 us, contiguous 184.23-190.49 us
- 16 dirty chunks: iter 231.93-255.95 us, contiguous 230.30-302.27 us
- 64 dirty chunks: iter 398.31-429.23 us, contiguous 387.87-429.85 us
- 256 dirty chunks: iter 1.1144-1.1677 ms, contiguous 1.0519-1.1125 ms

Revalidated decision: keep. The contiguous map build is a clear win for 1, 4, and 256 dirty chunks and ties for 16 and 64 dirty chunks. Since production already keeps the dirty loop on normal iteration, this preserves the measured win without changing the heavier loop body.

Verification:

- `cargo fmt --check`
- `cargo check`
- `cargo test`
- `cargo bench --bench ecs_queries`

Dirty loop decision: deferred to Candidate 3b. It includes padded-light construction, child traversal, mutable light writes, and command removal, so it needs a dedicated benchmark instead of borrowing the map-build result.

## Candidate 3b: Light Upload Dirty Loop

Production target: `src/world/chunk/mesh/systems.rs`

Current loop: `for (chunk_pos, chunk_entity) in &dirty_chunks_q { ... }`

Hypothesis: contiguous access may help small dirty batches, but padded-light construction and child/component writes probably dominate.

Benchmark command: `cargo bench --bench ecs_queries`

Benchmark added: `ecs_query_light_upload_dirty_loop` in `benches/ecs_queries.rs`.

The benchmark keeps the already-applied contiguous light-map build identical in both variants and compares only the dirty upload loop shape. It models padded-light construction, child traversal, and mutable `ChunkMeshLight` writes. It does not remove `ChunkNeedsLightUpload`, because repeating archetype mutation would measure marker churn more than the loop shape.

Results with 4096 chunk-light entities and 3 chunk mesh layer children per dirty chunk:

- 1 dirty chunk: iter 17.730-18.553 us, contiguous 17.736-18.112 us
- 4 dirty chunks: iter 24.789-25.110 us, contiguous 24.576-25.045 us
- 16 dirty chunks: iter 52.046-52.584 us, contiguous 51.990-53.274 us
- 64 dirty chunks: iter 162.95-165.43 us, contiguous 165.46-168.80 us
- 256 dirty chunks: iter 658.20-662.73 us, contiguous 663.88-668.19 us

Decision: no production change. The contiguous dirty loop is a tie for small batches and slower for larger batches. Padded-light construction and child updates dominate enough that changing the production loop would add complexity without a clear win.

Production change: none.

Revalidation benchmark command: `cargo bench --bench ecs_queries`

Revalidation benchmark: `ecs_system_light_upload_dirty_loop` in `benches/ecs_queries.rs`. This uses batched realistic worlds, initializes the schedule outside the measured closure, and includes `Commands` marker removal in the timed schedule run.

Revalidated results with 4096 chunk-light entities and 3 vertex-pulling children per dirty chunk:

- 1 dirty chunk: iter 151.41-158.18 us, contiguous 152.57-159.46 us
- 4 dirty chunks: iter 166.00-170.54 us, contiguous 166.82-174.18 us
- 16 dirty chunks: iter 205.21-215.73 us, contiguous 204.24-212.01 us
- 64 dirty chunks: iter 382.21-394.89 us, contiguous 379.21-401.35 us
- 256 dirty chunks: iter 1.1325-1.1563 ms, contiguous 1.1088-1.1625 ms

Revalidated decision: still no production change. Once measured as steady-state schedule execution with command removal included, the variants are effectively tied across the tested dirty counts.

## Candidate 4: Dimension Light Rebuild Setup

Production target: `src/world/dimension/light.rs`

Current loops scan dirty positions and sometimes all chunks to build fallback maps.

Hypothesis: setup scans may get a small improvement, but sparse `get(entity)` lookups and `ChunkLightRegion::rebuild` probably dominate.

Benchmark command: `cargo bench --bench ecs_queries`

Benchmark added: `ecs_system_light_rebuild` in `benches/ecs_queries.rs`.

The benchmark builds 32x32x4 chunk worlds, marks one chunk per dirty column, runs a Bevy `Schedule`, and includes command application/marker removal. It measures two scenarios:

- `active_dimension`: production's normal path using `Dimension.chunks` as the loaded chunk map.
- `fallback_map`: no active dimension, so the system builds a loaded chunk map from `all_chunks`.

Variants:

- `iter`: current production-style dirty-position scan and fallback map build.
- `contiguous`: `contiguous_iter().expect(...)` for the dirty-position scan and fallback map build.

Results for `active_dimension`, where production uses the maintained `Dimension.chunks` map:

- 1 dirty column: iter 530.91-560.34 us, contiguous 533.51-569.93 us
- 4 dirty columns: iter 952.07 us-1.0024 ms, contiguous 986.51 us-1.0260 ms
- 16 dirty columns: iter 2.6549-2.7587 ms, contiguous 2.6130-2.6383 ms
- 64 dirty columns: iter 9.0181-9.2118 ms, contiguous 8.9115-9.1062 ms

Results for `fallback_map`, where no active dimension exists and the system builds a loaded chunk map from `all_chunks`:

- 1 dirty column: iter 576.29-602.39 us, contiguous 573.06-610.05 us
- 4 dirty columns: iter 986.31 us-1.0209 ms, contiguous 975.55 us-1.0100 ms
- 16 dirty columns: iter 2.7225-2.8610 ms, contiguous 2.7132-2.7857 ms
- 64 dirty columns: iter 9.0144-9.1047 ms, contiguous 8.9152-9.0746 ms

Decision: no production change. The active-dimension path is mixed and mostly within noise; the fallback path trends slightly better for larger dirty sets but is not the normal runtime path and does not justify complicating `rebuild_chunk_light`. The expensive work is target expansion, region construction, entity lookups, `ChunkLightRegion::rebuild`, and command writes, not the initial dirty/fallback query scans.

Production change: none.

## Candidate 5: Fluid Active Chunk Stepping

Production target: `src/world/chunk/fluid.rs`

Current loops:

- active-fluid chunk stepping over `With<ChunkHasActiveFluids>`
- boundary-flow second pass scans all chunks for each boundary flow

Hypothesis: replacing the boundary-flow linear scan with a position/entity lookup is likely a larger win than contiguous access. Contiguous mutable stepping may still help small per-tick batches.

Benchmark command: `cargo bench --bench ecs_queries -- ecs_system_fluid_boundary_lookup`

Benchmark added: `ecs_system_fluid_boundary_lookup` in `benches/ecs_queries.rs`.

The benchmark builds 32x32 chunk worlds, seeds active chunks with water that produces boundary flows, runs a Bevy `Schedule`, initializes the schedule outside the measured closure, and includes command writes in the timed run.

Variants:

- `scan`: current production-style second pass, scanning all chunks for every boundary flow.
- `lookup`: build one `ChunkPosition -> Entity` map, then use `Query::get_mut(entity)` per boundary flow.

Results:

- 1 active chunk: scan 179.39-195.36 us, lookup 199.36-208.77 us
- 4 active chunks: scan 653.53-676.60 us, lookup 408.62-420.31 us
- 16 active chunks: scan 2.5825-2.6710 ms, lookup 1.2738-1.2821 ms
- 64 active chunks: scan 9.9280-10.007 ms, lookup 4.3082-4.3896 ms

Decision: apply lookup. It loses for exactly one active boundary chunk because the map build overhead is not repaid, but the default fluid budget is 64 and the lookup path is much faster for multi-chunk boundary work. Avoided a threshold branch for now; it would add complexity to save roughly 15-20 us in the one-active-chunk boundary case.

Production change: `step_chunk_fluids` now builds `chunks_by_pos` once before applying boundary flows and uses `param_set.p1().get_mut(entity)` for neighbor writes.

## Candidate 6: Collider Rebuild Dirty Loop

Production target: `src/world/chunk/collider.rs`

Current loop scans chunks with `With<ChunkNeedsColliderRebuild>`.

Hypothesis: query iteration could be faster, but collider voxel extraction, despawns, command writes, and collider creation probably dominate.

Benchmark command: `cargo bench --bench ecs_queries -- ecs_system_collider_rebuild`

Benchmark added: `ecs_system_collider_rebuild` in `benches/ecs_queries.rs`. It builds 4096 chunk entities, marks a variable dirty prefix, gives dirty chunks an existing collider child to despawn, runs a Bevy `Schedule`, and includes voxel extraction, `Collider::voxels`, despawn/spawn commands, and marker removal. The contiguous variant uses a split `children_q.get(entity)` lookup because that is the practical production shape if the dirty chunk scan moved to `contiguous_iter()`.

Results:

- 1 dirty chunk: iter 401.53-466.47 us, contiguous 397.41-438.47 us
- 4 dirty chunks: iter 639.21-698.06 us, contiguous 632.09-704.07 us
- 16 dirty chunks: iter 1.4768-1.5888 ms, contiguous 1.4792-1.5543 ms
- 64 dirty chunks: iter 4.8270-4.9318 ms, contiguous 4.9160-5.2417 ms

Decision: no production change. The contiguous version is effectively tied for 1-16 dirty chunks and slower at 64 dirty chunks. It also requires splitting child lookup out of the primary query, so the production code would get more complex without a reliable win.

Production change: none.

## Candidate 7: Render Extract/Prepare Redesign

Production target: `src/world/chunk/mesh/render/vertex_pulling/prepare.rs`

Current queries use `Changed<T>`/`Ref<T>`, which blocks contiguous iteration because `Changed<T>` is not an archetypal filter.

Hypothesis: explicit dirty markers or queues could enable contiguous scans, but buffer creation and bind group work likely dominate. Needs GPU/render-specific benchmarking before changes.

Benchmark: not added. Existing CPU benches do not measure render extraction/prep or GPU bind-group behavior well enough to justify a renderer architecture change.

Decision: no production change. Defer any redesign away from `Changed<T>`/`Ref<T>` until there is render-specific or GPU benchmark coverage that can detect improvements and regressions in extraction, buffer writes, bind group creation, and draw preparation.
