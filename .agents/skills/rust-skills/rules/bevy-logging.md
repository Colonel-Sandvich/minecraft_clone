# bevy-logging

> Log through `bevy::log`/`tracing`; never install a second subscriber.

## Why It Matters

Bevy's `LogPlugin` initializes tracing and owns level filtering and formatting. Adding `tracing_subscriber::fmt::init()` installs a competing global subscriber, which either panics or is silently ignored, and can drop Bevy's filters. `println!` bypasses levels, targets, and `RUST_LOG` entirely. This repo already uses `info!`, `warn!`, and `error!` with env-gated diagnostics.

## Bad

```rust
fn tick() {
    println!("frame");
}

fn main() {
    tracing_subscriber::fmt::init(); // Bevy already initialized tracing.
    App::new().add_plugins(DefaultPlugins).run();
}
```

## Good

```rust
use bevy::log::{debug, info, warn};

fn finish_column(chunks: usize, elapsed_ms: f32) {
    info!(chunks, elapsed_ms, "column published");
    debug!(target: "streaming", chunks, "streaming detail");
}
```

Configure the existing plugin instead of adding a subscriber:

```rust
DefaultPlugins.set(LogPlugin {
    level: bevy::log::Level::INFO,
    filter: "info,minecraft_clone=debug".to_string(),
    ..default()
})
```

## Project Notes

- Use structured fields (`info!(chunks, elapsed_ms, "...")`) instead of `format!` in the message; this keeps logs queryable and avoids an allocation.
- Keep per-frame logging behind an environment flag, as `MINECRAFT_CLONE_STREAM_TIMINGS` does. `log_frame_perf` is the existing example.
- `warn!` is for recoverable degradation; `error!` is for an operation that lost data or cannot continue. Do not log the same error at both levels.
- Do not log save-file paths, world seeds, or user data unless the diagnostic is explicitly opt-in.

## See Also

- [obs-structured-fields](obs-structured-fields.md) - structured fields over interpolated strings
- [obs-levels-filter](obs-levels-filter.md) - choose levels deliberately
- [obs-no-sensitive-data](obs-no-sensitive-data.md) - redact user data
