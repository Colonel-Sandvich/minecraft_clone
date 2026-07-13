# Dimension Switching Plan

Status: implemented with a cold-switch residency policy.

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

## Implemented Shape

- The catalog creates persistent ECS roots for all three logical dimensions.
- Dimension 0 retains the versioned overworld generator, Dimension 1 generates
  one grass layer at `y = 0`, and Dimension 2 generates one glass layer in the
  origin chunk only.
- `F6` cycles the player's primary dimension.
- Streaming, lighting, visual work, collider work, and persistence are scoped
  to a dimension root. Durable addresses include `DimensionId`.
- Switching captures dirty data into owned, eviction-priority save snapshots,
  drains and despawns outgoing column incarnations, activates the target root,
  and lets normal streaming publish the target.
- Player movement and interaction are suspended until the exact target root's
  arrival column is published and collision-ready. Collider-disabled runs use
  publication as the readiness boundary and discard unused collider work.
- A full-cycle integration test switches through all three dimensions and
  verifies that an overworld mutation survives teardown, persistence, and
  reload.

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

## Deferred Extensions

- Keep inactive dimensions warm by changing residency policy, without changing
  logical identity or qualified storage.
- Replace the debug key with explicit transition requests used by portals,
  commands, or UI while keeping the same coordinator.
- Add a loading presentation that waits for a configurable published radius;
  correctness currently waits only for the arrival column and colliders.
- Render more than one dimension by separating simulation interest, render
  interest, and the player's primary `Active` membership.
- Rebuild fluid scheduling around typed regions and a dimension-owned frontier.
- Move persistence from ordered subchunk writes to an atomic column commit.
