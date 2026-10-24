# err-custom-type

> Define custom error types per boundary instead of stringifying failures.

## Why It Matters

Generic errors (`String`, `Box<dyn Error>`) erase what can actually go wrong. A caller cannot match on a failure kind, decide whether it is transient, or recover. Typed variants document the failure modes and preserve the data needed for display and retry logic. This repo's `ChunkStoreError` is the model: typed IO/SQLite/Turso variants, a `Context` variant that wraps a source, plus `root_cause()` and `is_transient()` used by the save retry path.

## Bad

```rust
fn load(path: &Path) -> Result<Chunk, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    decode(&bytes).map_err(|error| format!("decode failed: {error}"))
}
```

The caller can only display the string. It cannot tell a missing file from corrupt data, and retry logic has to guess.

## Good

```rust
#[derive(Debug)]
pub enum StoreError {
    Io { path: PathBuf, source: std::io::Error },
    Corrupt { path: PathBuf, detail: String },
    Context { context: String, source: Box<StoreError> },
}

impl StoreError {
    pub fn context(self, context: impl Into<String>) -> Self {
        Self::Context { context: context.into(), source: Box::new(self) }
    }

    pub fn root_cause(&self) -> &Self {
        match self {
            Self::Context { source, .. } => source.root_cause(),
            other => other,
        }
    }

    pub fn is_transient(&self) -> bool {
        match self {
            Self::Context { source, .. } => source.is_transient(),
            Self::Io { source, .. } => matches!(
                source.kind(),
                std::io::ErrorKind::Interrupted
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::WouldBlock
            ),
            Self::Corrupt { .. } => false,
        }
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "io error at {}: {source}", path.display())
            }
            Self::Corrupt { path, detail } => {
                write!(f, "corrupt data at {}: {detail}", path.display())
            }
            Self::Context { context, source } => write!(f, "{context}: {source}"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Context { source, .. } => Some(source),
            Self::Corrupt { .. } => None,
        }
    }
}
```

Callers can now branch on the failure kind:

```rust
match load(path) {
    Ok(chunk) => chunk,
    Err(error) if error.is_transient() => retry_later(error),
    Err(error) => return Err(error.context("failed to load chunk")),
}
```

## Guidelines

- One enum per boundary (storage, settings, world loading), not one global error type.
- Preserve `source` and the data needed to display a useful message; do not stringify at the boundary.
- Expose semantic queries such as `is_transient()` / `root_cause()` instead of making callers match deep variants.
- Implement `Display` and `std::error::Error` by hand. This repo does not depend on `thiserror` or `anyhow`, and adding one for syntax sugar is not worth it.
- `#[non_exhaustive]` exists for published libraries. This is a binary crate; it is not needed.

## Project Notes

- `ChunkStoreError` in `src/world/storage/mod.rs` is the reference implementation. Extend it rather than creating a parallel error type.
- Keep `is_transient`, `root_cause`, and the `Context` wrapping correct when adding variants; the save retry/backoff path depends on them.
- `From<std::io::Error>`, `From<rusqlite::Error>`, and the feature-gated Turso conversion already exist. Add a `From` impl only when a conversion is genuinely needed.

## See Also

- [err-context-chain](err-context-chain.md) - attach context without losing the typed cause
- [err-source-chain](err-source-chain.md) - preserve the source chain
- [err-from-impl](err-from-impl.md) - when to add `From` conversions
- [anti-empty-catch](anti-empty-catch.md) - do not discard the failure
