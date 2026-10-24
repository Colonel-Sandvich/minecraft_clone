# obs-levels-filter

> Use log levels meaningfully; filter through Bevy's `LogPlugin` (`RUST_LOG`).

## Why It Matters

Levels communicate urgency and let an operator tune verbosity without rebuilding. Bevy's `LogPlugin` builds an `EnvFilter` from its `level` and `filter` fields, then layers `RUST_LOG` directives on top. Emitting everything at `info!`, or leaving `debug!` in a per-item loop, hides the real signal and makes frame diagnostics unreadable.

## Bad

```rust
use bevy::log::info;

fn step(chunks: &[Chunk]) {
    info!(count = chunks.len(), "fluid step");       // every tick
    info!("entered fluid step");                      // trace-level lifecycle noise
    for chunk in chunks {
        info!(?chunk, "chunk");                       // per-item spam
    }
}
```

## Good

```rust
use bevy::log::{debug, info, trace, warn};

fn step(chunks: &[Chunk]) {
    trace!("entered fluid step");
    debug!(count = chunks.len(), "fluid step");

    for chunk in chunks {
        trace!(id = chunk.id, "chunk");
    }

    if chunks.is_empty() {
        warn!("fluid step with no active chunks");
    }
}
```

Reserve `info!` for lifecycle events (startup, dimension switch, save complete), use `debug!` for per-frame aggregates, and `trace!` for per-item detail. Use structured fields, not `format!` in the message.

## Configuring Through Bevy

```rust
DefaultPlugins.set(LogPlugin {
    level: bevy::log::Level::INFO,
    filter: "info,minecraft_clone=debug".to_string(),
    ..default()
})
```

`RUST_LOG` extends and overrides those defaults, which is what you want for a one-off investigation:

```sh
RUST_LOG="warn,minecraft_clone::world::chunk::mesh=debug" cargo run
```

Do not build an `EnvFilter` or install a subscriber yourself; `LogPlugin` already owns that. See [bevy-logging](bevy-logging.md).

## Level Guidelines

| Level | Use for | Example here |
| --- | --- | --- |
| `error!` | Data loss or an operation that cannot continue | chunk save exhausted retries |
| `warn!` | Recoverable anomaly or degraded state | malformed save falls back to empty chunk |
| `info!` | High-level lifecycle | world opened, dimension switched |
| `debug!` | Diagnostic detail for development | chunks meshed this frame, queue depth |
| `trace!` | Per-item / per-iteration detail | per-chunk timing, raw indices |

## Project Notes

- Keep per-frame diagnostics behind an environment flag, as `MINECRAFT_CLONE_STREAM_TIMINGS` does; `log_frame_perf` is the existing model.
- Use `target:` and the module path (`minecraft_clone::world::chunk::mesh`) to filter noisy subsystems from `RUST_LOG`.
- Do not add a direct `tracing` or `tracing-subscriber` dependency; use `bevy::log`.
- Avoid interpolating large structures into hot-path messages; log ids and counts, then drill in at `trace!`.

## See Also

- [bevy-logging](bevy-logging.md) - `LogPlugin` ownership and structured fields
- [obs-structured-fields](obs-structured-fields.md) - fields over interpolated strings
- [obs-error-chain](obs-error-chain.md) - log each failure once, with its cause
- [obs-no-sensitive-data](obs-no-sensitive-data.md) - do not log save paths or user data
