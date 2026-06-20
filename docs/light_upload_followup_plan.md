# Light Upload Follow-Up Plan

## Context

The current vertex-pulling render path now separates geometry remeshing from lighting upload:

- `ChunkNeedsMeshRebuild` rebuilds CPU face descriptors and updates `VertexPullingMesh` children.
- `ChunkNeedsLightUpload` rebuilds padded 18^3 render light data and updates `VertexPullingLight` children.
- Render prep can rebuild descriptor/origin buffers without rebuilding light buffers.
- Render prep can rebuild light buffers without remeshing descriptors.

This is the correct first split. The remaining rough edge is that light data is still stored and prepared per material-layer child, so opaque/cutout children for the same chunk can duplicate CPU light blobs and GPU light buffers.

## Goals

- Preserve the invariant that lighting changes do not trigger remeshing.
- Preserve the invariant that remeshing does not upload lighting again for existing layer children.
- Reduce CPU copies of padded light data across layer children.
- Reduce GPU light-buffer allocation/upload duplication across layer children.
- Keep the implementation incremental and easy to validate.

## Non-Goals

- Do not redesign the light propagation algorithm here.
- Do not move chunk rendering to a completely new entity hierarchy.
- Do not change shader bindings unless profiling proves the current bind-group shape is the problem.
- Do not add broad compatibility layers for abandoned component layouts.

## Current Pain Points

1. `VertexPullingLight` owns `Box<[u32]>`, so light upload clones the padded 18^3 data into every layer child.
2. Light-only render prep creates a GPU light buffer per changed child, even when opaque and cutout children use identical light data.
3. `ChunkNeedsLightUpload` is accurate enough, but the marker semantically means "rebuild padded render light data and upload it" rather than only "upload".
4. The render prep split is subtle: `VertexPullingMesh` and `VertexPullingLight` are extracted separately and then recombined into `PreparedChunkVp`.

## Proposed Phases

### Phase 1: Clarify Naming And Invariants

Optional but low risk.

- Add comments on `ChunkNeedsLightUpload`, `VertexPullingMesh`, and `VertexPullingLight` documenting ownership and invalidation causes.
- Consider renaming `ChunkNeedsLightUpload` to `ChunkNeedsRenderLightUpload` if the existing name becomes ambiguous.
- Keep the current committed behavior unchanged.

Validation:

- `cargo test --lib`
- Confirm tests still assert light rebuilds do not set `ChunkNeedsMeshRebuild`.

### Phase 2: Share CPU Light Data Between Layer Children

Change `VertexPullingLight` from owning `Box<[u32]>` to owning shared immutable light data:

```rust
pub struct VertexPullingLight {
    pub light_data: Arc<[u32]>,
}
```

Implementation sketch:

- Convert `ChunkLight::build_padded_light_data` output into `Arc<[u32]>` in mesh creation and light upload systems.
- Assign the same `Arc` clone to all material-layer children for a chunk.
- When a mesh rebuild adds a new layer child to a chunk that already has another layer child, clone the existing sibling's `VertexPullingLight` `Arc` instead of rebuilding padded light data.
- Only call `ChunkLight::build_padded_light_data` during mesh rebuild if no existing child light data is available.
- Keep `VertexPullingLight` on children for now. This avoids parent/child render-world lookup complexity.

Why this phase is attractive:

- Very small data-model change.
- Avoids copying the same padded light blob for opaque/cutout children.
- Does not require shader changes.
- Does not require render entity parent lookups.

Validation:

- Add a test for a chunk with both opaque and cutout layers, asserting both children receive equivalent light data after `upload_chunk_lights`.
- If practical, assert `Arc::ptr_eq` between children after upload.
- Add a test or extend the mesh rebuild test so adding a new material layer reuses existing sibling light data.
- `cargo test --lib`
- `cargo check --bins`

### Phase 3: Deduplicate GPU Light Buffer Creation Per Shared CPU Blob

After Phase 2, changed layer children for the same chunk should usually reference the same `Arc<[u32]>`. Render prep can exploit that during a single prepare pass.

This phase is opportunistic same-frame dedupe, not a permanent guarantee of exactly one GPU light buffer per chunk. It dedupes siblings whose `VertexPullingLight` components are prepared together. Reusing an unchanged sibling's existing GPU buffer would require a persistent render-world cache or a parent-level prepared-light model.

Implementation sketch:

- Inside `prepare_gpu_data`, keep a short-lived map from shared light-data pointer to `Buffer` for the current frame.
- When preparing changed `VertexPullingLight`, create one GPU light buffer per unique `Arc<[u32]>` and clone the `Buffer` handle for sibling layer children.
- Keep one bind group per render child because descriptors differ by layer.

Possible key:

```rust
let key = (light.light_data.as_ptr() as usize, light.light_data.len());
```

Validation:

- Hard to unit test directly without `RenderDevice`.
- Add a small render-world debug counter/resource for light-buffer creates and dedupe hits, reset per prepare pass.
- Exercise a chunk containing both opaque and cutout faces.
- `cargo test --lib`
- `cargo check --bins`

Memory-reporting note:

- If memory debug/profiling code reports per-child light bytes, update it to count unique `Arc` pointers or do not use that metric as Phase 2 proof. Otherwise shared CPU storage will still look duplicated in reports.

### Phase 4: Reuse Existing Fixed-Size GPU Light Buffers

Only do this if profiling shows light-buffer allocation is measurable.

Padded light data has a fixed size, so existing buffers can be updated in place with `RenderQueue::write_buffer` rather than replaced.

Implementation sketch:

- Keep `PreparedChunkVp.light_buf` as the persistent buffer handle.
- Create light buffers with `BufferUsages::STORAGE | BufferUsages::COPY_DST`; `RenderQueue::write_buffer` requires `COPY_DST`.
- On light-only changes, if a prepared chunk already has a light buffer of the right fixed size, write new contents into it.
- Recreate the bind group only when descriptor/origin/light buffer identity changes.
- If multiple children share the same light buffer, write once per unique buffer.

Risks:

- In-place writes are simple for fixed-size data, but sharing a buffer across children means duplicate writes must be avoided.
- Bind-group identity must remain valid when buffers are reused.
- This adds complexity that is not justified unless light uploads are frequent enough to matter.

Validation:

- Add debug counters for GPU buffer creates vs `write_buffer` updates.
- Profile block edits that trigger light-only updates across multiple layer children.
- `cargo test --lib`
- `cargo check --bins`

## Rejected For Now: Parent-Level Render Light Component

An alternative is to store render light data only on the chunk parent and have layer children reference it.

This is probably cleaner conceptually, but it creates render-world lookup complexity:

- Queueing and drawing operate on visible layer child entities.
- `PreparedChunkVp` is queried from the child render entity.
- Parent-child relationships and main/render entity mapping would need careful handling during extraction/preparation.

Recommendation: avoid parent-level render light storage until duplicated child components or buffers are proven expensive enough to justify the added complexity.

## Acceptance Criteria

- A pure light rebuild does not add `ChunkNeedsMeshRebuild` to the changed chunk or padded-light neighbors.
- A pure mesh rebuild of existing children does not modify `VertexPullingLight`.
- A chunk with multiple material-layer children shares CPU light data after light upload.
- GPU light buffers are created no more than once per shared light-data blob during one prepare pass, if Phase 3 is implemented.
- Existing mesh/light tests pass.
- No shader-visible lighting behavior changes.

## Recommended Next Step

Do Phase 1 only if names/comments feel unclear. Otherwise start with Phase 2, because it is the smallest meaningful improvement and does not commit us to a more complex parent-level render-data design.
