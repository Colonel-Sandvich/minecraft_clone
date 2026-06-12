# Perf / Memory Profile Results

## CPU: `perf stat -d -r 3` on Ryzen 9 9950X3D

### Direct `full_stone_open`
| Metric | Value |
|---|---|
| Instructions | 182,911,420 |
| Cycles | 62,012,896 |
| IPC | 2.6 |
| L1-dcache misses | 180,796 (0.4%) |
| Branch misses | 243,780 (1.1%) |
| Page faults | 2,126 |
| Task-clock | 15.8 ms |

### Reference `full_stone_open`
| Metric | Value |
|---|---|
| Instructions | 323,428,210,597 |
| Cycles | 76,407,315,171 |
| IPC | 4.2 |
| L1-dcache misses | 357,468,090 (0.4%) |
| Branch misses | 170,468,079 (0.3%) |
| Page faults | 9,327 |
| Task-clock | 14.8 s |

**Takeaways:**
- IPC is high on both (2.6–4.2), no major pipeline stalls
- L1-dcache miss rate is extremely low (0.4%) — data access pattern is cache-friendly
- Branch miss rate is very low (0.3–1.1%) — predictable branching
- Bottleneck is raw instruction throughput, not cache/mispredictions

---

## CPU: `perf record -F 1999 -g` (direct_full_meshes, 279K samples)

### Function breakdown (self % of total cycles)

| Function | Self % | Notes |
|---|---|---|
| `DirectChunkMesher::mesh` | 20.95% | Entry point: counting pass + main loop |
| `MeshBufferBuilder::push_face` | 17.38% | Vertex emission (positions, normals, uvs, colors, indices) |
| `ChunkMeshBlocks::can_skip_mesh` | 12.58% | Early-exit check (empty/all-full-cube) |
| `Vec::reserve` [f32;4] | 0.85% | Allocation |
| `Vec::reserve` [f32;2] | 0.70% | Allocation |
| `Vec::reserve` [f32;3] | 0.51% | Allocation |
| `BlockMeshTables::from_texture_map` | 0.30% | Table construction |

**Plus Criterion overhead** (excluded from mesher hot path):
| Function | Self % |
|---|---|
| Criterion KDE (rayon map) | 17.35% |
| `__ieee754_exp_fma` (libm) | 8.98% |
| `exp@@GLIBC_2.29` (libm) | 5.99% |
| Other libm | ~1.5% |
| Rayon sort helpers | ~1% |

### Within actual meshing (~50% of total samples):
| Activity | Share of meshing |
|---|---|
| `mesh` traversal + counting | ~41% |
| `push_face` emission | ~34% |
| `can_skip_mesh` | ~25% |

**Notes:** `can_skip_mesh` is expensive on `full_stone_buried` case (6x16x16 block lookups for neighbor shell check). On most real workloads the two cheap field comparisons dominate.

---

## Reference breakdown (`perf record -F 1999 -g`, 292K samples)

| Function | Self % |
|---|---|
| `make_layered_quad_groups_from_blocks` | 40.16% |
| `face_ao` (via Drain closure) | 19.63% |
| Criterion KDE (rayon) | 15.71% |
| `__ieee754_exp_fma` (libm) | 8.10% |
| `exp@@GLIBC_2.29` (libm) | 5.60% |
| `__memcmp_evex_movbe` (libc) | 2.72% |
| `foldhash::hash_bytes_long` | 1.63% |
| `realloc` (libc) | 1.14% |

**Key differences vs Direct:**
- Reference has `face_ao` at **19.63%** — Direct bakes AO into `push_face` via `face_ao_from_indices`, eliminating separate AO traversal
- Reference has `realloc` at **1.14%** — Direct pre-counts faces, avoiding reallocation
- Reference has `foldhash` at **1.63%** + `memcmp` at **2.72%** — Direct resolves texture paths once in `BlockMeshTables`, avoiding per-face hash lookups and string matching
- Reference has no `push_face` equivalent because it collects `Quad` structs in a separate phase before Mesh conversion

## Checkerboard direct (`perf stat -d -r 3`)

| Metric | Value |
|---|---|
| Instructions | 147,849,427,810 |
| Cycles | 52,377,803,362 |
| IPC | 2.8 |
| L1-dcache misses | 972,319,989 |
| Cache misses | 28,104,114 |
| Branch misses | 165,102,653 |
| Page faults | 11,210,897 |
| Time elapsed | 8.6 s |

IPC stays high (2.6–4.2 range across all measured cases). No evidence of pipeline stalls or memory bandwidth limits.

---

## Allocation pattern

Both meshers allocate into growable buffers. Key difference:

- **Reference** collects `Quad` structs in `Vec<Quad>` per layer (amortized growth shown as `realloc` at 1.14%), then converts to `Mesh` in a separate pass
- **Direct** pre-counts faces per layer (`count_direct_faces` at ~1.5% of samples), reserves exact capacities in `MeshBufferBuilder`, then emits directly. No `realloc` observed in direct profiles
- **Vec::reserve** calls show up at ~3% combined in direct profiles: `[f32;4]` colours (0.85%), `[f32;2]` UVs (0.70%), `[f32;3]` positions/normals (0.51% each)

---

## Flamegraphs

Files in project root:
- [`flamegraph_direct_full.svg`](flamegraph_direct_full.svg) — direct_full_meshes (all scenarios, 279K samples)
- [`flamegraph_reference_full.svg`](flamegraph_reference_full.svg) — reference_quad_groups (all scenarios, 292K samples)

Open in browser to explore call stacks interactively. Wide bars = hot functions.

---

## Recommendations

### 1. Profile actual game runtime (not benchmarks)
Criterion's statistical analysis adds ~40% overhead (KDE, rayon, libm exp). A real game loop with `perf record` would give cleaner profiles of the mesher in isolation.

### 2. Push `can_skip_mesh` into callers when possible
The full neighbor shell check (`neighbor_face_shells_are_full_cube`) loops 6×16×16 = 1536 block lookups. Only useful for all-full-cube center chunks. Runtime already has `ChunkMeshBlocks` counters — can skip the expensive shell check when `center_is_all_full_cube()` is false.

### 3. Face counting pass is cheap
`count_direct_faces` is ~1.5% of samples. Its benefit (exact Vec capacity, avoiding all reallocation) is already realized. Worth keeping.

### 4. Measure at higher sample rates for the actual mesher
At 1999 Hz the mesher-specific samples are diluted by Criterion analysis. A game-loop harness with `perf record -F 9973` would isolate the hot path better.

### 5. `push_face` is the remaining hot spot
At 34% of meshing time, optimizing vertex emission further requires either fewer faces (better culling) or cheaper per-face operations (e.g. using SIMD for color/lit computation, or writing directly to GPU buffers).
