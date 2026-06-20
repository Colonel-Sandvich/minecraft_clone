# Sodium Async Occlusion Culling Notes

## TLDR

Sodium's recent occlusion work is CPU-side section-graph culling, not GPU occlusion queries and not exact per-block raycasting. It decides which 16x16x16 render sections are visible by walking a graph of neighboring sections from the camera, using precomputed per-section face-connectivity data. The new part is that the expensive graph traversal can run asynchronously, while the render thread builds render lists from compact tree/octree-like visibility results.

The practical win is smoother frame times when moving, turning, flying, or using high render distances, especially when render-list generation was the bottleneck.

## Core Idea

Each chunk section asks:

```text
If visibility enters this 16x16x16 section from face A, which other faces can it leave from?
```

The six faces are:

```text
DOWN, UP, NORTH, SOUTH, WEST, EAST
```

During meshing, Sodium builds a 16x16x16 opaque-block occupancy grid using solid-render blocks. It flood-fills through non-opaque cells. For every connected empty region, it records which section boundary faces that region touches. Faces touched by the same connected empty region are considered mutually visible.

Example:

```text
Connected air touches WEST and EAST -> WEST can see EAST, EAST can see WEST.
Connected air touches WEST only -> WEST does not create an exit to another face.
Disconnected air pockets do not connect their faces.
```

This is connectivity through empty cells, not literal straight-line visibility.

## Runtime Graph Traversal

At runtime, Sodium starts from the camera section and performs a BFS-like traversal across neighboring render sections.

For each visited section:

1. Read which directions visibility entered from.
2. Load that section's face-connectivity bit mask.
3. Convert incoming directions to allowed outgoing directions.
4. Apply outward-direction, distance, frustum, and angular/slope filters.
5. Visit neighboring sections through allowed faces.
6. Add visible sections into output trees.

This behaves like coarse portal traversal over 16x16x16 boxes. It can stop traversal behind sealed terrain, cave walls, or other opaque section arrangements.

## Async Culling

Sodium's new implementation decouples slow graph culling from render-list generation.

```text
Background cull thread:
  Run occlusion graph traversal.
  Produce visibility trees.

Render thread:
  Pick the narrowest valid tree.
  Traverse it quickly to build render lists.
```

It uses multiple cull result scopes:

- `LOCAL`: strictest, camera/frustum/fog dependent, includes local ray-like testing.
- `REGULAR`: normal occlusion graph result, less tied to exact frustum.
- `WIDE`: broadest fallback, more reusable across camera movement.

If the exact local result is stale or not ready, Sodium can often use a broader valid tree instead of stalling the frame. If the camera teleports or moves extremely fast, it can fall back to synchronous culling for correctness.

## Ray-Like Local Test

Sodium includes a ray-like check in `RayOcclusionSectionTree`, but it is not a Minecraft block raycast.

For a candidate section, it samples from the section center back toward the camera and checks whether those samples pass through sections already present in the traversed portal tree. If the rough line does not pass through already-visible section corridors, the section can be rejected from the strict local tree.

This checks section-tree presence, not block geometry or GPU depth.

## Frustum And AABBs

Sodium still uses frustum culling. Render sections are tested as padded section AABBs, with extra margin for models extending outside nominal block bounds and precision issues.

The visibility pipeline combines:

- render/search distance limits
- section-graph occlusion connectivity
- frustum tests for local results
- fog-distance limits where configured
- angular/slope path refinement
- local ray-like portal checks

## Important Distinction

This feature is not:

- GPU occlusion queries
- hierarchical Z-buffer culling
- exact per-block ray tracing
- entity culling
- model/leaf-specific culling

It is section-level CPU occlusion graph traversal plus async render-list generation.

## References

- Sodium PR #2887, Asynchronous Graph Culling and Frame-Independent Task Scheduling: https://github.com/CaffeineMC/sodium/pull/2887
- Main merge commit: https://github.com/CaffeineMC/sodium/commit/a327f636280015ae9103d9e605420f438c043703
- Sodium changelog on `dev`: https://github.com/CaffeineMC/sodium/blob/dev/CHANGELOG.md
- `OcclusionCuller`: https://github.com/CaffeineMC/sodium/blob/dev/common/src/main/java/net/caffeinemc/mods/sodium/client/render/chunk/occlusion/OcclusionCuller.java
- `CullTask`: https://github.com/CaffeineMC/sodium/blob/dev/common/src/main/java/net/caffeinemc/mods/sodium/client/render/chunk/async/CullTask.java
- `SectionTree`: https://github.com/CaffeineMC/sodium/blob/dev/common/src/main/java/net/caffeinemc/mods/sodium/client/render/chunk/occlusion/SectionTree.java
- `RayOcclusionSectionTree`: https://github.com/CaffeineMC/sodium/blob/dev/common/src/main/java/net/caffeinemc/mods/sodium/client/render/chunk/occlusion/RayOcclusionSectionTree.java
- `DirectionalVisGraph`: https://github.com/CaffeineMC/sodium/blob/dev/common/src/main/java/net/caffeinemc/mods/sodium/client/render/chunk/occlusion/DirectionalVisGraph.java
- `VisibilityEncoding`: https://github.com/CaffeineMC/sodium/blob/dev/common/src/main/java/net/caffeinemc/mods/sodium/client/render/chunk/occlusion/VisibilityEncoding.java
- `RenderSectionManager`: https://github.com/CaffeineMC/sodium/blob/dev/common/src/main/java/net/caffeinemc/mods/sodium/client/render/chunk/RenderSectionManager.java
- Related out-of-world culling fix PR #3484: https://github.com/CaffeineMC/sodium/pull/3484
- Perspective visibility PR #3297: https://github.com/CaffeineMC/sodium/pull/3297
- Slope refinement PR #3307: https://github.com/CaffeineMC/sodium/pull/3307
