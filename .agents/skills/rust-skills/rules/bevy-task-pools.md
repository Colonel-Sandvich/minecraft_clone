# bevy-task-pools

> Use Bevy's task pools for parallel and background work; do not add rayon or Tokio, and do not spawn a thread per job.

## Why It Matters

Bevy sizes its pools to the machine, integrates them with the frame, and lets tests substitute a deterministic single-thread pool. Extra executors oversubscribe cores and escape task accounting, so profiling and frame budgets stop meaning what they say. This repo already wraps `AsyncComputeTaskPool` in `ChunkTaskPool` and meshes with `ComputeTaskPool`. One long-lived thread is still the right tool for a blocking socket/DB loop; see [bevy-io-networking](bevy-io-networking.md).

## Bad

```rust
// One thread per chunk/request: oversubscribes cores and escapes task accounting.
std::thread::spawn(move || build_chunk(snapshot));
rayon::par_iter().for_each(|chunk| mesh(chunk));
tokio::spawn(load_column(address));
```

None of these belongs to an executor the game controls.

## Good

```rust
// Background work that may span frames.
let task = task_pool.spawn(async move { load_column(address, &repository) });

// CPU work that must finish this frame.
let builds = dirty.par_splat_map(ComputeTaskPool::get(), None, |_, slice| {
    slice.iter().map(|&(entity, position)| build(entity, position)).collect::<Vec<_>>()
});
```

Poll background tasks from a normal system without blocking:

```rust
use bevy::tasks::futures::check_ready;

if let Some(output) = check_ready(&mut task) {
    // Apply the finished result.
}
```

## Project Notes

- `ChunkTaskPool` in `src/world/dimension/mod.rs` wraps `AsyncComputeTaskPool` in production and a one-thread `TaskPool` under `cfg(test)`; keep that seam so tests stay deterministic.
- `ComputeTaskPool` is for CPU work that should complete within the frame. `AsyncComputeTaskPool` is for work that may span frames and is polled with `check_ready`.
- Never call `block_on`, `join`, or a blocking channel receive inside a system. See [bevy-blocking-boundary](bevy-blocking-boundary.md).
- Use `IoTaskPool` for async file/asset IO. For a genuinely blocking socket/DB loop, use one long-lived dedicated thread plus a bounded channel, not a pool worker.

## See Also

- [bevy-io-networking](bevy-io-networking.md) - pool selection for files, sockets, and packets
- [bevy-blocking-boundary](bevy-blocking-boundary.md) - where blocking work is allowed
- [bevy-schedule-sets](bevy-schedule-sets.md) - when to collect task results
- [perf-profile-first](perf-profile-first.md) - measure before adding parallelism
