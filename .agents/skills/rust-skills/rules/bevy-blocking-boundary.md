# bevy-blocking-boundary

> Isolate blocking IO and runtime ownership in the storage adapter, never in systems.

## Why It Matters

A system that blocks stalls the entire schedule, not just its own work. The `turso-store` feature deliberately owns a Tokio runtime inside `TursoChunkStore` and exposes a blocking `ChunkStore` trait; saves are dispatched to `ChunkTaskPool` jobs and their results are polled later. That containment is the design. Leaking the runtime, or calling `block_on` from gameplay code, reintroduces frame hitches and makes shutdown and error handling unpredictable.

## Bad

```rust
fn save(mut store: ResMut<ChunkRepository>) {
    // Blocks the frame on database IO.
    store.flush().expect("save failed");
}
```

## Good

```rust
fn start_save(
    store: Res<ChunkRepository>,
    pool: Res<ChunkTaskPool>,
    mut tasks: ResMut<SaveTasks>,
) {
    let store = store.clone();
    tasks.push(pool.spawn(async move { store.flush() }));
}
```

Then poll the task with `check_ready` in a later system and apply the result.

## Rules

- `Runtime::block_on` is acceptable during startup or teardown outside the game loop, for example while `TursoChunkStore::open` builds the database.
- Never call `block_on` in a system or in a future polled on the main thread.
- For genuinely blocking calls, prefer `IoTaskPool` or a dedicated thread over parking an `AsyncComputeTaskPool` worker.
- Keep `tokio` behind the `turso-store` feature; do not make it a default dependency.
- Report failures through the existing `ChunkStoreError` and `.context(...)` chain, and use the existing retry/backoff rather than panicking or silently dropping a save.

## Project Notes

- Boundary points: `src/world/storage/turso.rs` (runtime ownership) and `src/world/dimension/persistence.rs` (task dispatch and polling).
- The blocking `ChunkStore` trait is fine as an adapter boundary. Do not "fix" it by making every caller async.

## See Also

- [bevy-task-pools](bevy-task-pools.md) - which pool to use
- [err-context-chain](err-context-chain.md) - preserve the store error chain
- [anti-empty-catch](anti-empty-catch.md) - do not swallow save failures
