# Deferred Architecture Work

Status: the column-streaming, lighting, derived-work, and dimension-identity
foundations are implemented. This file tracks the remaining architectural work
without making it a requirement for the current runtime.

## Completed Foundation

- XZ columns are the dimension-owned unit of residency.
- Visible columns publish only after dependency-complete lighting.
- Initial lighting uses off-thread patches with separate calculation and commit
  sets plus entity, content, light, and ticket validation.
- Mesh, render-light, and collider work use coalescing per-dimension queues.
- Stable dimension IDs qualify storage and asynchronous work.
- Persistent dimension roots support save-safe cold switching between three
  dimension-specific generators.

## Fluid Simulation

Fluid stepping still copies multi-chunk snapshots, carries raw coordinates
through the solver, and selects active chunks with a position-biased budget.
Replace it with a typed fluid region that separates writable simulation targets
from readable context. Feed regions from a fair, bounded dimension-owned
frontier and apply deterministic results through `ChunkEditor`.

## Persistence

Detached snapshots are owned and retryable, and save/load addresses are
dimension-qualified. The remaining weakness is that a column's subchunks are
written separately while sharing one heightmap. Introduce one atomic column
commit and track the last durable column revision so eviction can prove what
storage contains.

## Render Ownership

Material-layer children still mirror shared chunk render state, and GPU sharing
is inferred from allocation identity. Extract an explicit chunk render context,
keep immutable face geometry and material identity on layers, and make camera
uniforms and pipeline specialization view-specific before multi-dimension
rendering is attempted.

## Generation

Generator profiles are stable and versioned, but overworld terrain, noise,
features, and the development landmark still share one module. Split these
behind a `WorldGenerator` facade while retaining the existing golden outputs.
Column generation should compute shared surface and feature inputs once.

## Lighting Follow-ups

Profile delayed storage and moving views in the full client before changing
budgets or adding worker concurrency. If real traces show head-of-line blocking,
allow old incomplete initial-light tiles to yield. Runtime block edits can later
propagate changed light boundaries instead of conservatively relighting their
connected region.

## Dimension Follow-ups

The current switch policy unloads inactive chunk data. Warm caches, portals,
loading presentation, and multiple rendered dimensions should add independent
residency, transition, and render-interest policies without replacing logical
dimension roots or treating `Active` as global existence.

## Suggested Order

1. Rebuild fluid scheduling around typed regions and a fair frontier.
2. Add atomic column persistence and durable column revisions.
3. Consolidate render ownership before multi-view rendering.
4. Split generation behind its versioned facade.
5. Revisit lighting admission only with delayed-load client traces.
