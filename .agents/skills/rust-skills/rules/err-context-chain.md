# err-context-chain

> Attach context as an error crosses a boundary; preserve the typed source.

## Why It Matters

A raw error says what failed, not which operation was attempted. This repo's `ChunkStoreError::context` wraps a source in a `Context { context, source }` variant, so `root_cause()` still finds the typed cause and `is_transient()` still sees the IO/SQLite/Turso kind. Context belongs at boundaries — file load, decode, database call — not at every `?`.

## Bad

```rust
fn load(path: &Path) -> Result<Chunk, StoreError> {
    let bytes = std::fs::read(path).map_err(|source| StoreError::Io {
        path: path.to_owned(),
        source,
    })?;
    decode(&bytes) // "unexpected EOF" — which chunk? which stage?
}
```

## Good

```rust
fn load(path: &Path) -> Result<Chunk, StoreError> {
    let bytes = std::fs::read(path)
        .map_err(|source| StoreError::Io { path: path.to_owned(), source })
        .context("failed to read chunk file")?;

    decode(&bytes).context("failed to decode chunk")?;
    Ok(chunk)
}
```

The chain now reads:

```text
failed to decode chunk
  failed to read chunk file
    io error at saves/world/chunk.mca: unexpected end of file
```

## Where to Add Context

Add it where the operation changes meaning:

- Reading or writing a file/database
- Decoding or parsing bytes into a domain type
- Wrapping a retryable operation before scheduling a retry

Do not add it to every `?` in internal helpers. Duplicated context makes the chain noisy and allocates on the success path: this repo's `.context()` takes `impl Into<String>` eagerly. Use a static `&str` when it is enough; reach for `format!` only when the extra data changes what an operator can diagnose.

## Project Notes

- `ChunkStoreError` implements `std::error::Error::source()` only for its `Context` variant. Use `root_cause()` when you need the innermost typed error; do not assume `Error::source()` walks every variant.
- Context messages in this repo are lowercase and have no trailing period. Keep that style.
- The storage adapter adds context; gameplay code should propagate the resulting error rather than wrapping it again.

## See Also

- [err-custom-type](err-custom-type.md) - the typed error the context wraps
- [err-source-chain](err-source-chain.md) - preserve the cause
- [err-lowercase-msg](err-lowercase-msg.md) - message style
- [bevy-blocking-boundary](bevy-blocking-boundary.md) - where storage errors surface
