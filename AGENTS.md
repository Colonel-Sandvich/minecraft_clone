# Agent Guidelines

## Required checks

Before committing, run:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

A tracked pre-commit hook enforces both. Enable it once per clone with:

```sh
git config core.hooksPath .githooks
```

## Clippy suppressions

Do not add `#[allow(clippy::...)]` or `#[expect(clippy::...)]` without explicit
approval from the repository owner in the current conversation. Fix the lint
instead. Existing suppressions are grandfathered.

If the owner has explicitly approved a new suppression, the pre-commit hook
allows the commit with:

```sh
CLIPPY_ALLOW_APPROVED=1 git commit ...
```
