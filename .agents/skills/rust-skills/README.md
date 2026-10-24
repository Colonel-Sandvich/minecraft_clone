# Curated Rust + Bevy skill

This directory is a **manual, project-tuned install** of
[leonardomso/rust-skills](https://github.com/leonardomso/rust-skills) at commit `fd2a861`.
It is not managed by `npx skills`; there is intentionally no `skills-lock.json`.

OpenCode discovers it as `.agents/skills/rust-skills/SKILL.md`.

## What was kept

- 178 of the upstream 265 rules, copied into `rules/`.
- 10 new `bevy-*` rules written for this codebase.
- All `async-*` (18) and `conc-*` (4) rules were removed: the game uses Bevy task pools,
  and Tokio exists only behind the optional `turso-store` feature.
- Published-library advice was removed: workspace/MSRV/crate metadata, semver,
  `#[non_exhaustive]`, doc-for-all-public-items, mockall/loom/insta/proptest/tokio tests,
  proc-macro crates, and FFI rules.
- Dependency-first suggestions were removed or annotated (`thiserror`, `anyhow`, `ahash`,
  `smallvec`, `arrayvec`, `tracing-subscriber`). This repo has its own error type and
  already uses Bevy's re-exports.
- `See Also` links to removed rules were pruned, so every remaining link resolves.

## Local Bevy rules

| Rule | Covers |
| --- | --- |
| `bevy-query-disjoint` | overlapping `Query` access, `ParamSet`, `Single` |
| `bevy-commands-deferred` | `Commands` sync points and set ordering |
| `bevy-change-detection` | write-on-change, `Changed`/`Added`/`Ref`, dirty markers |
| `bevy-schedule-sets` | named sets, `.chain()`, schedule-shuffle testing |
| `bevy-task-pools` | `ComputeTaskPool`/`AsyncComputeTaskPool`, `check_ready` |
| `bevy-io-networking` | pool selection, file writes, socket threads, packet channels |
| `bevy-blocking-boundary` | Turso runtime containment and blocking IO |
| `bevy-asset-handles` | handle lifetime and generated assets |
| `bevy-logging` | `LogPlugin`/tracing, no competing subscriber |
| `bevy-fixed-timestep` | `FixedUpdate`, `Time<Fixed>`, input/presentation split |

## Updating

1. Clone upstream at a new commit.
2. Diff `rules/` against this directory.
3. Re-apply the keep/drop policy: drop async/concurrency and published-library advice;
   keep rules that affect ECS, meshing, streaming, persistence, rendering, and unsafe data access.
4. Re-check that `See Also` links resolve and that `SKILL.md` has one entry per rule file.
5. Update the commit hash in `SKILL.md` and this README.

## License

Upstream rules are MIT-licensed; `LICENSE` is included. Local `bevy-*` rules follow the
same license.
