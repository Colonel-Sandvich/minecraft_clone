# Dimension Switching Plan

Status: active architecture work.

## Goal

Add three switchable dimensions while making chunk generation, persistence,
streaming, and derived work explicitly dimension-owned.

- Dimension 0 preserves the current terrain generator exactly.
- Dimension 1 contains grass blocks only on world block layer `y = 0`.
- Dimension 2 contains one 16x16, one-block-thick glass platform in the centre
  chunk and is empty elsewhere.
- A debug key cycles the player's primary dimension.

The first implementation unloads inactive chunk data. That is a residency
policy, not part of dimension identity, so inactive dimensions can be retained
or rendered later without replacing the core model.

## Identity And Ownership

Logical identity, runtime incarnation, and local coordinates are separate:

```rust
struct DimensionId(u32);

struct DimensionDefinition {
    id: DimensionId,
    height: WorldHeight,
    generator: GeneratorProfile,
    arrival: Vec3,
}

enum GeneratorProfile {
    OverworldV1,
    GrassFloorV1,
    CenterGlassPlatformV1,
}
```

Each logical dimension has its own ECS root. The root owns loaded and published
chunk registries, streaming and lighting tasks, the desired column view, and the
derived-work queue. `Active` identifies the player's primary simulation
dimension; it does not mean that every other dimension must be absent.

`ChunkPos` and `ChunkColumn` remain dimension-local. Persistence and other
cross-dimension boundaries use qualified addresses:

```rust
struct ChunkAddress {
    dimension: DimensionId,
    position: ChunkPos,
}
```

This prevents equal chunk coordinates in different dimensions from sharing
storage, cache, or asynchronous-work identity.

## Generation

Generation is selected through the immutable dimension definition captured by
each column-load task. Code must not match directly on numeric dimension IDs or
read a mutable global current-dimension value after a task has started.

The current generator becomes the `OverworldV1` profile with golden tests that
preserve its output. Simple profiles still use normal full-height column
residency; implicit sparse chunks are a separate future optimization.

## Switching Lifecycle

Switching is an explicit transition rather than mutating one `Dimension` into
another:

1. Resolve a switch request and pause player mutation.
2. Stop scheduling the outgoing dimension and hide its column roots.
3. Disable or remove its colliders immediately.
4. Cancel load/light authority and clear disposable derived work.
5. Move dirty chunk snapshots into persistence before destroying authoritative
   block entities.
6. Drain and despawn outgoing column roots.
7. Activate the target root and move the player to its arrival point.
8. Refresh the target's desired view and run ordinary column streaming.
9. Let column publication enqueue mesh, collider, and render-light work through
   the normal derived-work path.
10. Resume the player once the centre column is published and collision-ready.

The player is not owned by the disposable dimension hierarchy. It carries
explicit dimension membership and keeps its camera as a child.

## Derived Work

Every dimension owns a coalescing derived-work queue. Entries retain both the
local position and expected chunk entity, so Bevy entity generations reject
work for an evicted or replaced chunk.

Switching clears outgoing disposable work. It does not enqueue a special global
rebuild: publishing chunks in the incoming dimension remains the only path that
creates initial mesh, collider, render-light, and fluid work.

Durable save state remains separate from disposable derived work.

## Future Compatibility

The design leaves room for warm inactive caches, portals, and multiple rendered
dimensions. Those features can add residency, rendering, and simulation
interest independently of the player's primary `Active` dimension.

Avoid these limiting shortcuts:

- reusing one dimension entity and swapping its identity;
- treating `ChunkPos` as globally unique;
- using one unqualified repository namespace;
- making `Active` synonymous with resident or renderable;
- destroying dirty authoritative data before persistence owns a retryable
  snapshot.

## Atomic Implementation Order

1. Add stable dimension IDs, definitions, generator profiles, and exact tests.
2. Qualify storage and load/save requests with dimension identity.
3. Move desired-view and runtime load context ownership onto each dimension.
4. Add the dimension-owned derived-work queue and migrate visual/collider work.
5. Add the switch coordinator, teardown/readiness tests, player membership, and
   debug keybind.
6. Migrate runtime lighting and fluid activation, then remove obsolete markers.

