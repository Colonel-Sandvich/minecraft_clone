# bevy-schedule-sets

> Encode system order in named sets; never rely on registration order.

## Why It Matters

The scheduler runs unordered systems in parallel and may reorder them between builds. Correctness must be explicit, and sets also document phase boundaries: input, simulation, streaming, presentation. They give you one place to define the edges and one name to attach systems to, instead of a web of pairwise `.before`/`.after` calls.

## Bad

```rust
app.add_systems(Update, collect_input);
app.add_systems(Update, apply_input); // no edge: either order is legal
```

## Good

```rust
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum InputSystems {
    Collect,
    Apply,
}

app.configure_sets(Update, (InputSystems::Collect, InputSystems::Apply).chain())
    .add_systems(Update, collect_input.in_set(InputSystems::Collect))
    .add_systems(Update, apply_input.in_set(InputSystems::Apply));
```

`.chain()` gives you ordering and a deferred sync point between the sets, which matters when the first set uses `Commands`. Prefer `.before`/`.after` across sets, not between individual systems, and use `.chain()` for a short local pipeline inside one set.

## Project Notes

- This repo already models phases with `ChunkSimulationSet`, `DimensionStreamingSet`, `InputCollectionSystems`, `FixedInputSystems`, `MobPhysicsSystems`, and `PlayerCameraSystems`. Add new work to an existing set instead of inventing parallel ordering edges.
- `configure_chunk_simulation` in `src/world/mod.rs` chains `ExternalMutation` before `FluidStep`; that edge is load-bearing, not cosmetic.
- After changing ordering, run the `schedule-shuffle` feature with `MINECRAFT_CLONE_SHUFFLE_SEED` to shuffle `Update`, `FixedUpdate`, and the physics schedules. A failure there is a real hidden dependency; do not "fix" it by pinning order unless the dependency is intentional.
- Once-per-frame work belongs in `Update`; simulation belongs in `FixedUpdate` (see [bevy-fixed-timestep](bevy-fixed-timestep.md)).

## See Also

- [bevy-commands-deferred](bevy-commands-deferred.md) - ordered sets create the sync points commands need
- [bevy-fixed-timestep](bevy-fixed-timestep.md) - choose the right schedule as well as the right order
