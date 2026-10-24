---
name: rust-skills
description: >
  Curated Rust and Bevy rules for this voxel game. Use when writing, reviewing, or refactoring Rust in this repo: ownership and borrowing, Bevy queries, deferred commands, change detection, schedule ordering, task pools, asset handles, unsafe meshing, error handling, serde persistence, numeric safety, performance, and clippy policy. Project-tuned subset of leonardomso/rust-skills plus local Bevy rules.
license: MIT
metadata:
  author: leonardomso (upstream), curated for minecraft_clone
  upstream: https://github.com/leonardomso/rust-skills
  upstream-commit: fd2a861
  curated: "2026-10-24"
---

# Rust + Bevy Skill (curated for minecraft_clone)

This is a project-tuned subset of [leonardomso/rust-skills](https://github.com/leonardomso/rust-skills) (upstream commit `fd2a861`): **188 rules**, 178 kept from upstream and 10 Bevy rules written for this codebase.
See [README.md](README.md) for provenance and what was dropped.

Use it when writing, reviewing, or refactoring Rust here. The rule set is weighted
toward the hot paths: ECS scheduling and queries, chunk meshing and lighting,
streaming, persistence, and rendering. Library-publishing, async-Tokio, rayon, and
platform-abstraction advice was removed as noise.

## When to Apply

- Writing new systems, queries, resources, or schedule ordering
- Changing chunk meshing, lighting, streaming, or persistence
- Reviewing `unsafe` blocks and raw buffer indexing
- Adding or changing save/settings formats
- Performance work, profiling reviews, and allocation reductions
- Reviewing a diff or refactoring a touched module

## Project Overrides (these win)

- `AGENTS.md` is authoritative: `cargo fmt --all -- --check` and
  `cargo clippy --all-targets --all-features -- -D warnings` must pass. Do not add
  `#[allow(clippy::...)]` or `#[expect(clippy::...)]` without owner approval in the
  current conversation.
- Toolchain is rolling nightly, edition 2024, Bevy 0.20. Do not suggest an MSRV or
  stable-only constraints.
- This is a single binary crate, not a published library. Skip semver,
  `#[non_exhaustive]`, crate metadata, and public-API documentation advice.
- Do not add dependencies without profiling evidence and owner approval (SmallVec,
  arrayvec, ahash, anyhow, thiserror, rayon, Tokio, tracing-subscriber, ...). Bevy
  already provides task pools, hashers, logging, and math types.
- `unwrap`/`expect` are accepted in tests, tooling, and startup invariants. Do not
  mass-rewrite existing call sites; apply `anti-unwrap-abuse` to new fallible runtime
  paths only.
- Async/Tokio is confined to the optional `turso-store` feature. Gameplay systems stay
  synchronous and use Bevy task pools.
- Bevy owns the tracing subscriber (`LogPlugin`) and the scheduler. Never install a
  second subscriber. Use the task pools; one long-lived thread with a bounded channel
  is fine for a blocking IO/socket loop, never a thread per frame or per job.
- Apply rules to the code you touch; do not drive-by refactor unrelated modules.
- New Bevy work goes into the existing schedules and named sets. Do not invent a
  parallel ordering scheme.

## How to Use

1. Start here. Open only the handful of rule files relevant to the code in front of you.
2. Each rule file has a one-line summary, why it matters, a bad example, and a good example.
3. Prefer the project overrides above when a general rule conflicts with this repo.
4. Apply rules to new or touched code. Do not use them as a mandate for unrelated rewrites.

## Categories

| Category | Rules | Focus |
| --- | ---: | --- |
| Bevy ECS & Game Loop | 10 | Queries, commands, change detection, schedule order, task pools, assets, fixed tick |
| Ownership & Borrowing | 12 | Borrow vs clone, smart pointers, lifetimes |
| Error Handling | 10 | Result propagation, context, panic policy |
| Memory Optimization | 13 | Allocation reuse, compact data, drop behavior |
| Unsafe Code | 5 | Sound `get_unchecked`, SAFETY comments, Miri limits |
| API Design | 12 | Builders, newtypes, conversions, internal API shape |
| Compiler Optimization | 10 | Inlining, LTO, codegen, bounds checks, cache layout |
| Numeric & Arithmetic Safety | 5 | Overflow, casts, float comparison, clamping |
| Type Safety | 11 | Newtypes, enums for states, validated types |
| Trait & Generics Design | 6 | Static vs dynamic dispatch, coherence, defaults |
| Conversions | 3 | `TryFrom`, `FromStr`, `AsMut` |
| Const & Compile-Time | 4 | `const fn`, const generics, const blocks |
| Serde | 8 | Save/settings compatibility and validation |
| Pattern Matching | 5 | `let-else`, `matches!`, exhaustive enums |
| Macros | 3 | When to use `macro_rules!` and how to keep it hygienic |
| Closures | 5 | Fn traits, captures, returning closures |
| Collections | 4 | Map/sequence choice, sets, heaps |
| Naming Conventions | 11 | Rust API naming conventions |
| Testing | 8 | Unit/integration layout, criterion, determinism |
| Documentation | 2 | Module docs and intra-doc links |
| Observability | 4 | Structured fields, levels, error chains |
| Performance | 13 | Iterators, entry API, allocation and IO patterns |
| Project Structure | 7 | Module layout, visibility, features |
| Linting | 3 | Clippy config that composes with the repo hook |
| Anti-patterns | 14 | Common mistakes and review checklist |
| **Total** | **188** | |

## Rule Application by Task

| Task | Primary rules |
| --- | --- |
| New system, query, or schedule | `bevy-query-disjoint`, `bevy-schedule-sets`, `bevy-change-detection` |
| Structural spawn/despawn/insert | `bevy-commands-deferred` |
| Streaming, meshing, background work | `bevy-task-pools`, `bevy-blocking-boundary`, `perf-*`, `mem-*` |
| File writes, sockets, networking | `bevy-io-networking`, `bevy-task-pools`, `bevy-blocking-boundary` |
| Assets, generated meshes | `bevy-asset-handles`, `bevy-change-detection` |
| Gameplay tick / physics / fluid | `bevy-fixed-timestep`, `bevy-schedule-sets`, `num-*` |
| Logging and diagnostics | `bevy-logging`, `obs-*` |
| Unsafe mesher/light code | `unsafe-*`, `opt-bounds-check` |
| Save/settings format | `serde-*`, `err-*` |
| Code review | `anti-*`, `lint-*`, relevant `bevy-*` rules |
| Module/API refactor | `api-*`, `type-*`, `trait-*`, `proj-*` |

## Quick Reference

### 1. Bevy ECS & Game Loop (10)

- [`bevy-asset-handles`](rules/bevy-asset-handles.md) - Keep `Handle<T>` values in long-lived state; do not reload assets per frame.
- [`bevy-blocking-boundary`](rules/bevy-blocking-boundary.md) - Isolate blocking IO and runtime ownership in the storage adapter, never in systems.
- [`bevy-change-detection`](rules/bevy-change-detection.md) - Mutate ECS data only when the value actually changes.
- [`bevy-commands-deferred`](rules/bevy-commands-deferred.md) - Treat `Commands` as queued until the next sync point.
- [`bevy-fixed-timestep`](rules/bevy-fixed-timestep.md) - Simulate on `FixedUpdate` and `Time<Fixed>`; keep input and presentation in `Update`.
- [`bevy-io-networking`](rules/bevy-io-networking.md) - Pick the pool by workload: Compute for this frame, AsyncCompute for later frames, Io for async IO, one dedicated thread for blocking sockets/DB.
- [`bevy-logging`](rules/bevy-logging.md) - Log through `bevy::log`/`tracing`; never install a second subscriber.
- [`bevy-query-disjoint`](rules/bevy-query-disjoint.md) - Keep query access disjoint; resolve overlap with filters, `ParamSet`, or separate systems.
- [`bevy-schedule-sets`](rules/bevy-schedule-sets.md) - Encode system order in named sets; never rely on registration order.
- [`bevy-task-pools`](rules/bevy-task-pools.md) - Use Bevy's task pools for parallel and background work; do not add rayon or Tokio, and do not spawn a thread per job.

### 2. Ownership & Borrowing (12)

- [`own-arc-shared`](rules/own-arc-shared.md) - Use `Arc<T>` for thread-safe shared ownership
- [`own-borrow-over-clone`](rules/own-borrow-over-clone.md) - Prefer `&T` borrowing over `.clone()`
- [`own-clone-explicit`](rules/own-clone-explicit.md) - Use explicit `Clone` for types where copying has meaningful cost
- [`own-copy-small`](rules/own-copy-small.md) - Implement `Copy` for small, simple types
- [`own-cow-conditional`](rules/own-cow-conditional.md) - Use `Cow<'a, T>` for conditional ownership
- [`own-lifetime-elision`](rules/own-lifetime-elision.md) - Rely on lifetime elision rules; add explicit lifetimes only when required
- [`own-move-large`](rules/own-move-large.md) - Move large types instead of copying; use `Box` if moves are expensive
- [`own-mutex-interior`](rules/own-mutex-interior.md) - Use `Mutex<T>` for interior mutability across threads
- [`own-rc-single-thread`](rules/own-rc-single-thread.md) - Use `Rc<T>` for shared ownership in single-threaded contexts
- [`own-refcell-interior`](rules/own-refcell-interior.md) - Use `RefCell<T>` for interior mutability in single-threaded code
- [`own-rwlock-readers`](rules/own-rwlock-readers.md) - Use `RwLock<T>` when reads significantly outnumber writes
- [`own-slice-over-vec`](rules/own-slice-over-vec.md) - Accept `&[T]` not `&Vec<T>`, `&str` not `&String`

### 3. Error Handling (10)

- [`err-context-chain`](rules/err-context-chain.md) - Attach context as an error crosses a boundary; preserve the typed source.
- [`err-custom-type`](rules/err-custom-type.md) - Define custom error types per boundary instead of stringifying failures.
- [`err-doc-errors`](rules/err-doc-errors.md) - Document error conditions with `# Errors` section in doc comments
- [`err-expect-bugs-only`](rules/err-expect-bugs-only.md) - Use `expect()` only for invariants that indicate bugs, not user errors
- [`err-from-impl`](rules/err-from-impl.md) - Implement `From<E>` for error conversions to enable `?` operator
- [`err-lowercase-msg`](rules/err-lowercase-msg.md) - Start error messages lowercase, no trailing punctuation
- [`err-no-unwrap-prod`](rules/err-no-unwrap-prod.md) - Avoid `unwrap()` in production code; use `?`, `expect()`, or handle errors
- [`err-question-mark`](rules/err-question-mark.md) - Use `?` operator for clean propagation
- [`err-result-over-panic`](rules/err-result-over-panic.md) - Return `Result<T, E>` instead of panicking for recoverable errors
- [`err-source-chain`](rules/err-source-chain.md) - Preserve error chains with `#[source]` or `source()` method

### 4. Memory Optimization (13)

- [`mem-arena-allocator`](rules/mem-arena-allocator.md) - Use arena allocators for batch allocations
- [`mem-arrayvec`](rules/mem-arrayvec.md) - Use `ArrayVec<T, N>` for fixed-capacity collections that never heap-allocate
- [`mem-avoid-format`](rules/mem-avoid-format.md) - Avoid `format!()` when string literals work
- [`mem-box-large-variant`](rules/mem-box-large-variant.md) - Box large enum variants to reduce overall enum size
- [`mem-boxed-slice`](rules/mem-boxed-slice.md) - Use `Box<[T]>` instead of `Vec<T>` for fixed-size heap data
- [`mem-clone-from`](rules/mem-clone-from.md) - Use `clone_from()` to reuse allocations when repeatedly cloning
- [`mem-drop-order`](rules/mem-drop-order.md) - Know and control drop order: struct fields drop top-to-bottom, locals in reverse
- [`mem-reuse-collections`](rules/mem-reuse-collections.md) - Clear and reuse collections instead of creating new ones in loops
- [`mem-smallvec`](rules/mem-smallvec.md) - Use `SmallVec` for usually-small collections
- [`mem-take-replace`](rules/mem-take-replace.md) - Use `mem::take` / `mem::replace` to move a value out of a `&mut` without cloning
- [`mem-with-capacity`](rules/mem-with-capacity.md) - Use `with_capacity()` when size is known
- [`mem-write-over-format`](rules/mem-write-over-format.md) - Use `write!()` into existing buffers instead of `format!()` allocations
- [`mem-zero-copy`](rules/mem-zero-copy.md) - Use zero-copy patterns with slices and `Bytes`

### 5. Unsafe Code (5)

- [`unsafe-maybeuninit`](rules/unsafe-maybeuninit.md) - Use `MaybeUninit<T>` for uninitialized memory; never use `mem::uninitialized()` or `mem::zeroed()` for types with validity invariants.
- [`unsafe-minimize-scope`](rules/unsafe-minimize-scope.md) - Keep `unsafe` blocks as small as possible — mark only the operation that requires unsafety, not the surrounding safe code.
- [`unsafe-miri-ci`](rules/unsafe-miri-ci.md) - Run `cargo miri test` in CI for every crate that contains `unsafe` code.
- [`unsafe-safety-comment`](rules/unsafe-safety-comment.md) - Write a `// SAFETY:` comment above every `unsafe` block and a `# Safety` section in every `unsafe fn`.
- [`unsafe-send-sync-manual`](rules/unsafe-send-sync-manual.md) - Document the invariants when manually implementing `Send` or `Sync`; prefer letting the compiler derive them automatically.

### 6. API Design (12)

- [`api-builder-must-use`](rules/api-builder-must-use.md) - Mark builder methods with `#[must_use]` to prevent silent drops
- [`api-builder-pattern`](rules/api-builder-pattern.md) - Use Builder pattern for complex construction
- [`api-common-traits`](rules/api-common-traits.md) - Implement standard traits (Debug, Clone, PartialEq, etc.) for public types
- [`api-default-impl`](rules/api-default-impl.md) - Implement `Default` for types with sensible default values
- [`api-extension-trait`](rules/api-extension-trait.md) - Use extension traits to add methods to external types
- [`api-from-not-into`](rules/api-from-not-into.md) - Implement `From<T>`, not `Into<U>` - From gives you Into for free
- [`api-impl-asref`](rules/api-impl-asref.md) - Use `AsRef<T>` when you only need to borrow the inner data
- [`api-impl-into`](rules/api-impl-into.md) - Accept `impl Into<T>` for flexible APIs, implement `From<T>` for conversions
- [`api-must-use`](rules/api-must-use.md) - Mark types and functions with `#[must_use]` when ignoring results is likely a bug
- [`api-newtype-safety`](rules/api-newtype-safety.md) - Use newtypes to prevent mixing semantically different values
- [`api-operator-overload`](rules/api-operator-overload.md) - Overload operators only when the semantics are natural and unsurprising
- [`api-parse-dont-validate`](rules/api-parse-dont-validate.md) - Parse into validated types at boundaries

### 7. Compiler Optimization (10)

- [`opt-bounds-check`](rules/opt-bounds-check.md) - Use iterators and patterns that eliminate bounds checks in hot paths
- [`opt-cache-friendly`](rules/opt-cache-friendly.md) - Organize data for cache-efficient access patterns
- [`opt-codegen-units`](rules/opt-codegen-units.md) - Set `codegen-units = 1` for maximum optimization in release builds
- [`opt-cold-unlikely`](rules/opt-cold-unlikely.md) - Mark unlikely code paths with `#[cold]` to help compiler optimization
- [`opt-inline-always-rare`](rules/opt-inline-always-rare.md) - Use `#[inline(always)]` sparingly—only for critical hot paths proven by profiling
- [`opt-inline-never-cold`](rules/opt-inline-never-cold.md) - Use `#[inline(never)]` and `#[cold]` for error paths and rarely-executed code
- [`opt-inline-small`](rules/opt-inline-small.md) - Use `#[inline]` for small hot functions
- [`opt-lto-release`](rules/opt-lto-release.md) - Enable LTO in release builds
- [`opt-pgo-profile`](rules/opt-pgo-profile.md) - Use Profile-Guided Optimization (PGO) for maximum performance
- [`opt-target-cpu`](rules/opt-target-cpu.md) - Use `target-cpu=native` for maximum performance on known deployment targets

### 8. Numeric & Arithmetic Safety (5)

- [`num-cast-try-from`](rules/num-cast-try-from.md) - Avoid `as` for narrowing casts; use `From` for widening and `TryFrom` for narrowing
- [`num-float-compare`](rules/num-float-compare.md) - Don't compare floats with `==`; use a tolerance, and `total_cmp` for ordering
- [`num-nonzero`](rules/num-nonzero.md) - Use `NonZero*` types to forbid zero and unlock the niche optimization
- [`num-overflow-explicit`](rules/num-overflow-explicit.md) - Handle integer overflow explicitly: `checked_`/`saturating_`/`wrapping_`/`overflowing_`
- [`num-saturating-clamp`](rules/num-saturating-clamp.md) - Bound values with `clamp` and saturating arithmetic

### 9. Type Safety (11)

- [`type-deref-coercion`](rules/type-deref-coercion.md) - Implement `Deref`/`DerefMut` only for smart-pointer and transparent wrapper types
- [`type-display-vs-debug`](rules/type-display-vs-debug.md) - Use `Display` for user-facing output and `Debug` for diagnostics; never swap them
- [`type-enum-states`](rules/type-enum-states.md) - Use enums for mutually exclusive states
- [`type-generic-bounds`](rules/type-generic-bounds.md) - Add trait bounds only where needed, prefer where clauses for readability
- [`type-never-diverge`](rules/type-never-diverge.md) - Use `!` (never type) for functions that never return
- [`type-newtype-ids`](rules/type-newtype-ids.md) - Wrap IDs in newtypes: `UserId(u64)`
- [`type-newtype-validated`](rules/type-newtype-validated.md) - Use newtypes to enforce validation at construction time
- [`type-no-stringly`](rules/type-no-stringly.md) - Avoid stringly-typed APIs; use enums, newtypes, or validated types
- [`type-option-nullable`](rules/type-option-nullable.md) - Use `Option<T>` for values that might not exist
- [`type-phantom-marker`](rules/type-phantom-marker.md) - Use `PhantomData` to express type relationships without runtime cost
- [`type-result-fallible`](rules/type-result-fallible.md) - Use `Result<T, E>` for operations that can fail

### 10. Trait & Generics Design (6)

- [`trait-associated-type-vs-generic`](rules/trait-associated-type-vs-generic.md) - Use an associated type when each impl has exactly one output type; use a generic parameter when a type can implement the trait for many input types
- [`trait-blanket-impl`](rules/trait-blanket-impl.md) - Use a blanket impl `impl<T: Bound> Trait for T` to give behaviour to every type that satisfies a bound
- [`trait-coherence-newtype`](rules/trait-coherence-newtype.md) - Respect the orphan rule; wrap a foreign type in a newtype to implement a foreign trait on it
- [`trait-default-methods`](rules/trait-default-methods.md) - Define a trait in terms of a few required methods plus defaulted ones built on top of them
- [`trait-dyn-vs-generic`](rules/trait-dyn-vs-generic.md) - Choose static dispatch (generics / `impl Trait`) vs dynamic dispatch (`dyn Trait`) deliberately
- [`trait-object-safety`](rules/trait-object-safety.md) - Keep a trait dyn-compatible (object-safe) when you need `dyn Trait`

### 11. Conversions (3)

- [`conv-asmut-mutable`](rules/conv-asmut-mutable.md) - Accept `impl AsMut<T>` for flexible mutable borrowed inputs instead of concrete mutable references
- [`conv-fromstr-parsing`](rules/conv-fromstr-parsing.md) - Implement `FromStr` to enable `str::parse` for string-to-type conversions
- [`conv-tryfrom-fallible`](rules/conv-tryfrom-fallible.md) - Implement `TryFrom` for fallible conversions instead of ad-hoc conversion functions

### 12. Const & Compile-Time (4)

- [`const-block`](rules/const-block.md) - Use inline `const { }` blocks for compile-time evaluation and assertions
- [`const-fn`](rules/const-fn.md) - Make functions `const fn` when they can run at compile time
- [`const-generics`](rules/const-generics.md) - Parameterize over values with const generics `<const N: usize>`
- [`const-vs-static`](rules/const-vs-static.md) - Use `const` for an inlined value and `static` for a single addressed instance

### 13. Serde (8)

- [`serde-custom-with`](rules/serde-custom-with.md) - Customize a field's (de)serialization with `with` / `serialize_with` / `deserialize_with`
- [`serde-default-compat`](rules/serde-default-compat.md) - Use `#[serde(default)]` for optional and backward-compatible fields
- [`serde-deny-unknown-fields`](rules/serde-deny-unknown-fields.md) - Reject unexpected keys with `#[serde(deny_unknown_fields)]`
- [`serde-enum-representation`](rules/serde-enum-representation.md) - Choose enum tagging deliberately: externally, internally, adjacently tagged, or untagged
- [`serde-flatten`](rules/serde-flatten.md) - Inline nested structs or capture extra keys with `#[serde(flatten)]`
- [`serde-rename-all`](rules/serde-rename-all.md) - Match the external naming convention with `#[serde(rename_all = ...)]`
- [`serde-skip-empty`](rules/serde-skip-empty.md) - Omit empty fields with `skip_serializing_if`
- [`serde-try-from-validate`](rules/serde-try-from-validate.md) - Validate while deserializing with `#[serde(try_from = "Raw")]`

### 14. Pattern Matching (5)

- [`pat-at-bindings`](rules/pat-at-bindings.md) - Use `@` bindings to capture a value while matching it against a pattern
- [`pat-exhaustive-enum`](rules/pat-exhaustive-enum.md) - Match owned enums exhaustively; avoid catch-all `_` that hides new variants
- [`pat-if-let-chains`](rules/pat-if-let-chains.md) - Use `if let` chains to combine pattern bindings and conditions
- [`pat-let-else`](rules/pat-let-else.md) - Use `let ... else` for early-return pattern extraction
- [`pat-matches-macro`](rules/pat-matches-macro.md) - Use `matches!()` for boolean pattern tests

### 15. Macros (3)

- [`macro-fragment-specifiers`](rules/macro-fragment-specifiers.md) - Capture with precise fragment specifiers, not raw `:tt`, where you can
- [`macro-prefer-functions`](rules/macro-prefer-functions.md) - Reach for a macro only when a function or generic cannot express it
- [`macro-rules-hygiene`](rules/macro-rules-hygiene.md) - Rely on `macro_rules!` hygiene and use `$crate` for paths to your crate's items

### 16. Closures (5)

- [`closure-disjoint-capture`](rules/closure-disjoint-capture.md) - Capture only what you use; lean on edition-2021 disjoint closure captures
- [`closure-fn-trait-bounds`](rules/closure-fn-trait-bounds.md) - Require the least restrictive `Fn` trait a callback needs (`FnOnce` ⊇ `FnMut` ⊇ `Fn`)
- [`closure-impl-fn-return`](rules/closure-impl-fn-return.md) - Return closures as `impl Fn`/`FnMut`/`FnOnce`, not `Box<dyn Fn>`
- [`closure-move-capture`](rules/closure-move-capture.md) - Use `move` for closures that outlive the current scope; clone before `move` to keep the original
- [`closure-static-vs-dyn`](rules/closure-static-vs-dyn.md) - Accept `impl Fn` (generic) for hot callbacks; use `&dyn Fn`/`Box<dyn Fn>` to cut code size or to store them

### 17. Collections (4)

- [`coll-binaryheap`](rules/coll-binaryheap.md) - Use `BinaryHeap` for a priority queue or repeated max-extraction
- [`coll-map-choice`](rules/coll-map-choice.md) - Pick the map by access pattern: `HashMap` (fast, unordered), `BTreeMap` (sorted / range queries), `IndexMap` (insertion order)
- [`coll-seq-choice`](rules/coll-seq-choice.md) - Default to `Vec`; use `VecDeque` for queue/deque behaviour; avoid `LinkedList`
- [`coll-set-membership`](rules/coll-set-membership.md) - Use `HashSet`/`BTreeSet` for membership tests and dedup, not linear `Vec::contains`

### 18. Naming Conventions (11)

- [`name-acronym-word`](rules/name-acronym-word.md) - Treat acronyms as words in identifiers: `HttpServer`, not `HTTPServer`
- [`name-as-free`](rules/name-as-free.md) - `as_` prefix: free reference conversion
- [`name-into-ownership`](rules/name-into-ownership.md) - Use `into_` prefix for ownership-consuming conversions
- [`name-is-has-bool`](rules/name-is-has-bool.md) - Use `is_`, `has_`, `can_`, `should_` prefixes for boolean-returning methods
- [`name-iter-convention`](rules/name-iter-convention.md) - Use iter/iter_mut/into_iter for iterator methods
- [`name-iter-method`](rules/name-iter-method.md) - Name iterator methods `iter()`, `iter_mut()`, and `into_iter()` consistently
- [`name-iter-type-match`](rules/name-iter-type-match.md) - Name iterator types after their source method
- [`name-lifetime-short`](rules/name-lifetime-short.md) - Use short, conventional lifetime names: `'a`, `'b`, `'de`, `'src`
- [`name-no-get-prefix`](rules/name-no-get-prefix.md) - Omit get_ prefix for simple getters
- [`name-to-expensive`](rules/name-to-expensive.md) - Use `to_` prefix for expensive conversions that allocate or compute
- [`name-type-param-single`](rules/name-type-param-single.md) - Use single uppercase letters for type parameters: `T`, `E`, `K`, `V`

### 19. Testing (8)

- [`test-arrange-act-assert`](rules/test-arrange-act-assert.md) - Structure tests with clear Arrange, Act, Assert sections
- [`test-cfg-test-module`](rules/test-cfg-test-module.md) - Put unit tests in `#[cfg(test)] mod tests { }` within each module
- [`test-criterion-bench`](rules/test-criterion-bench.md) - Use `criterion` for benchmarking
- [`test-descriptive-names`](rules/test-descriptive-names.md) - Use descriptive test names that explain what is being tested
- [`test-fixture-raii`](rules/test-fixture-raii.md) - Use RAII pattern (Drop trait) for automatic test cleanup
- [`test-integration-dir`](rules/test-integration-dir.md) - Put integration tests in the `tests/` directory
- [`test-should-panic`](rules/test-should-panic.md) - Use `#[should_panic]` to test that code panics as expected
- [`test-use-super`](rules/test-use-super.md) - Use `use super::*;` in test modules to access parent module items

### 20. Documentation (2)

- [`doc-intra-links`](rules/doc-intra-links.md) - Use intra-doc links to reference types and items
- [`doc-module-inner`](rules/doc-module-inner.md) - Use `//!` for module-level documentation

### 21. Observability (4)

- [`obs-error-chain`](rules/obs-error-chain.md) - Log errors with their full source chain, and log each error exactly once
- [`obs-levels-filter`](rules/obs-levels-filter.md) - Use log levels meaningfully; filter through Bevy's `LogPlugin` (`RUST_LOG`).
- [`obs-no-sensitive-data`](rules/obs-no-sensitive-data.md) - Never log secrets or PII; redact or skip them
- [`obs-structured-fields`](rules/obs-structured-fields.md) - Record structured key-value fields, not values interpolated into the message string

### 22. Performance (13)

- [`perf-ahash`](rules/perf-ahash.md) - Use a faster hasher (`ahash` / `FxHashMap`) when DoS resistance is not needed
- [`perf-black-box-bench`](rules/perf-black-box-bench.md) - Use black_box in benchmarks
- [`perf-chain-avoid`](rules/perf-chain-avoid.md) - Avoid chain in hot loops
- [`perf-collect-into`](rules/perf-collect-into.md) - Use collect_into for reusing containers
- [`perf-collect-once`](rules/perf-collect-once.md) - Don't collect intermediate iterators
- [`perf-drain-reuse`](rules/perf-drain-reuse.md) - Use drain to reuse allocations
- [`perf-entry-api`](rules/perf-entry-api.md) - Use entry API for map insert-or-update
- [`perf-extend-batch`](rules/perf-extend-batch.md) - Use extend for batch insertions
- [`perf-io-buffering`](rules/perf-io-buffering.md) - Wrap `Read`/`Write` in `BufReader`/`BufWriter` for many small operations
- [`perf-iter-lazy`](rules/perf-iter-lazy.md) - Keep iterators lazy, collect only when needed
- [`perf-iter-over-index`](rules/perf-iter-over-index.md) - Prefer iterators over manual indexing
- [`perf-profile-first`](rules/perf-profile-first.md) - Profile before optimizing
- [`perf-release-profile`](rules/perf-release-profile.md) - Optimize release profile settings

### 23. Project Structure (7)

- [`proj-feature-additive`](rules/proj-feature-additive.md) - Design Cargo features to be strictly additive
- [`proj-flat-small`](rules/proj-flat-small.md) - Keep small projects flat
- [`proj-mod-by-feature`](rules/proj-mod-by-feature.md) - Organize modules by feature, not type
- [`proj-mod-rs-dir`](rules/proj-mod-rs-dir.md) - Use mod.rs for multi-file modules
- [`proj-pub-crate-internal`](rules/proj-pub-crate-internal.md) - Use pub(crate) for internal APIs
- [`proj-pub-super-parent`](rules/proj-pub-super-parent.md) - Use pub(super) for parent-only visibility
- [`proj-pub-use-reexport`](rules/proj-pub-use-reexport.md) - Use pub use for clean public API

### 24. Linting (3)

- [`lint-cfg-check`](rules/lint-cfg-check.md) - Enable `unexpected_cfgs` and declare known cfgs to catch feature-gate typos
- [`lint-clippy-nursery-selected`](rules/lint-clippy-nursery-selected.md) - Enable high-value `clippy::nursery` lints selectively, not the whole group
- [`lint-pedantic-selective`](rules/lint-pedantic-selective.md) - Enable clippy::pedantic selectively

### 25. Anti-patterns (14)

- [`anti-clone-excessive`](rules/anti-clone-excessive.md) - Don't clone when borrowing works
- [`anti-collect-intermediate`](rules/anti-collect-intermediate.md) - Don't collect intermediate iterators
- [`anti-empty-catch`](rules/anti-empty-catch.md) - Don't silently ignore errors
- [`anti-expect-lazy`](rules/anti-expect-lazy.md) - Don't use expect for recoverable errors
- [`anti-format-hot-path`](rules/anti-format-hot-path.md) - Don't use format! in hot paths
- [`anti-index-over-iter`](rules/anti-index-over-iter.md) - Don't use indexing when iterators work
- [`anti-over-abstraction`](rules/anti-over-abstraction.md) - Don't over-abstract with excessive generics
- [`anti-panic-expected`](rules/anti-panic-expected.md) - Don't panic on expected or recoverable errors
- [`anti-premature-optimize`](rules/anti-premature-optimize.md) - Don't optimize before profiling
- [`anti-string-for-str`](rules/anti-string-for-str.md) - Don't accept &String when &str works
- [`anti-stringly-typed`](rules/anti-stringly-typed.md) - Don't use strings where enums or newtypes would provide type safety
- [`anti-type-erasure`](rules/anti-type-erasure.md) - Don't use Box<dyn Trait> when impl Trait works
- [`anti-unwrap-abuse`](rules/anti-unwrap-abuse.md) - Don't use `.unwrap()` in production code
- [`anti-vec-for-slice`](rules/anti-vec-for-slice.md) - Don't accept &Vec<T> when &[T] works

## Provenance

- Upstream: [https://github.com/leonardomso/rust-skills](https://github.com/leonardomso/rust-skills) at `fd2a861` (MIT).
- Curated 2026-10-24: 178/265 upstream rules kept,
  10 local `bevy-*` rules added.
- Removed categories: async (18) and concurrency (4), plus selective removals from
  API design, docs, project structure, linting, testing, and macros. See README.md.
- Rule text is adapted for this project; upstream content remains MIT-licensed.
