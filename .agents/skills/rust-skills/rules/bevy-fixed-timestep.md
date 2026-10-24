# bevy-fixed-timestep

> Simulate on `FixedUpdate` and `Time<Fixed>`; keep input and presentation in `Update`.

## Why It Matters

Variable frame delta makes fluid, physics, and mob simulation non-deterministic and enables tunneling. Bevy runs fixed schedules zero or more times per frame, so gameplay written against `Time` in `Update` changes behavior with frame rate. This repo sets `Time<Virtual>` max delta, drives Avian physics, and separates variable-rate input collection from fixed-rate input dispatch.

## Bad

```rust
fn fluid_step(time: Res<Time>, /* ... */) {
    // In Update: dt depends on frame rate.
    let dt = time.delta_secs();
    advance_fluid(dt);
}
```

## Good

```rust
app.add_systems(FixedUpdate, (fluid_step, mob_step).in_set(SimulationSet))
    .add_systems(Update, (collect_input, interpolate_visuals));
```

```rust
fn fluid_step(time: Res<Time<Fixed>>, /* ... */) {
    let dt = time.delta_secs();
    advance_fluid(dt);
}
```

Use `Time<Fixed>` for simulation, `Time`/`Time<Virtual>` for camera, UI, and animation. Buffer input until the next fixed tick, and keep once-per-frame work out of `FixedUpdate`. In tests, run the fixed schedule a known number of ticks instead of sleeping.

## Project Notes

- `FIXED_TICK_RATE_HZ`, `Time::<Virtual>::from_max_delta`, and the physics schedules are configured in `src/app.rs`; do not add a second game clock.
- `InputCollectionSystems::Collect` runs in `Update` and `FixedInputSystems::Dispatch` runs at the fixed rate. Keep that split when adding input.
- Visual interpolation belongs in `Update`; simulation state must not depend on how many frames were rendered.

## See Also

- [bevy-schedule-sets](bevy-schedule-sets.md) - name the fixed phases
- [num-float-compare](num-float-compare.md) - compare accumulated time carefully
- [bevy-change-detection](bevy-change-detection.md) - avoid waking consumers for unchanged simulation state
