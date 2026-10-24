# bevy-commands-deferred

> Treat `Commands` as queued until the next sync point.

## Why It Matters

Structural changes issued through `Commands` — spawn, despawn, insert, remove — are recorded and applied when the system finishes and the schedule reaches a deferred sync point. The issuing system still sees the old world. Querying an entity you just spawned through `Commands` always fails in the same system. Bevy inserts sync points between chained systems and across ordered sets, so the fix is usually ordering, not an explicit flush.

## Bad

```rust
fn spawn_and_touch(mut commands: Commands, chunks: Query<&Chunk>) {
    let entity = commands.spawn_empty().id();
    // Still Err: the spawn is queued, not applied.
    let _ = chunks.get(entity);
}
```

## Good

Split the work and let set ordering create the sync point:

```rust
app.configure_sets(FixedUpdate, (ExternalMutation, FluidStep).chain())
    .add_systems(FixedUpdate, insert_marker.in_set(ExternalMutation))
    .add_systems(FixedUpdate, observe_marker.in_set(FluidStep));
```

Or make the structural change immediately in an exclusive system:

```rust
fn spawn_now(world: &mut World) {
    let entity = world.spawn_empty().id();
    // Visible immediately, but this system cannot run in parallel.
}
```

Commands targeting the same entity are applied in insertion order, so this is fine:

```rust
let entity = commands.spawn_empty().id();
commands.entity(entity).insert(ChunkPosition::default());
```

## Project Notes

- `src/world/mod.rs` contains the regression test `external_mutation_commands_are_visible_to_the_fluid_set`; keep that guarantee when changing `configure_chunk_simulation`.
- Prefer `.chain()` on sets over scattering `apply_deferred` calls.
- Use an exclusive system only for bulk or genuinely immediate structural work; routine spawning belongs in commands.

## See Also

- [bevy-schedule-sets](bevy-schedule-sets.md) - ordering is what makes commands visible
- [bevy-query-disjoint](bevy-query-disjoint.md) - avoid conflicting queries while restructuring
