# Multiplayer / Netcode Plan

Status: design notes for future work. Single-player only today; no network code exists.
Derived from how Minecraft Java's integrated server splits client and server.

## Rules To Live By

1. **One authority per subsystem.** Write the owner down (see table below). Every
   desync bug is usually a subsystem with an undocumented or double owner.
2. **Corrections replace, never merge.** Attach monotonic sequence/revision numbers
   to replicated state; discard stale updates. The server's value always wins.
3. **Prediction is a bounded overlay.** Only predict reversible actions. Keep
   unacked predicted changes in an overlay with a lifetime limit and roll them
   back on contradiction. Do not predict irreversible things (item consumption,
   scripted consequences).
4. **Interpolate, don't extrapolate.** Render remote entities at `now - delay`
   so you interpolate between received snapshots. Clamp any extrapolation to
   tens of milliseconds.
5. **State digests + resync.** Send periodic per-region hashes; on mismatch,
   resync that region. This turns permanent desync into a short hitch, which is
   the single biggest robustness win.
6. **Deterministic simulation math.** Fixed 20 Hz tick, never variable `dt`.
   Integers/fixed-point for authoritative state, floats for presentation only.
   Seeded RNG, sorted iteration order, no fast-math.
7. **Transactions with IDs and acks.** Inventory-like actions are idempotent
   transactions; replay unacked intents after a resync.
8. **All replicated mutations flow through intent -> validate -> event.** This is
   required for multiplayer, and pays for itself in single player via replay,
   tests, commands, and mob AI using one code path.
9. **Authoritative sim must not reference rendering types.** No `Handle<Image>`,
   meshes, or audio in server-side systems. This is what makes a headless
   server binary possible without forking logic.
10. **Threads are an implementation detail.** The contract is messages and state,
    not which OS thread runs what. Same-thread schedules are a valid split.
11. **Never let the client keep simulating from unacked state indefinitely.**
    That feedback loop is what players experience as constant rubberbanding.

## Authority Table

| Subsystem | Owner | Client behavior |
| --- | --- | --- |
| Block edits | Server | Predicted overlay; rollback on reject |
| Own player movement | Client, within server tolerances | Send position/velocity; server validates speed/reach |
| Mobs | Server | Interpolate; do not predict |
| Dropped items (avian dynamic) | Server | Interpolate; do not predict |
| Chunk generation / streaming | Deterministic from seed, computed locally | Server sends only dirty edits later |
| Lighting | Derived deterministically from blocks | Computed locally on both sides; never replicated |
| Block breaking progress | Client prediction, server confirm | Bounded overlay; clear on contradiction |
| Dimension switching | Server-ordered | Client follows with a load gate |
| Inventory / crafting (future) | Server | Requests only; client copy is read-only |

## Local Player Identity

Multiple player entities share one archetype; exactly one is controlled by this
client:

- `Player` (existing) means **any** player. `PlayerId`, `GameMode`, `Inventory`,
  and `PlayerDimension` live here, so remote players reuse the same components
  and shared systems.
- `LocalPlayer` is a marker component on the one entity this client submits
  inputs for — the same split as Minecraft Java's `LocalPlayer extends Player`.

Client-only systems (input collection, camera look, HUD, inventory UI, block
targeting, pause routing) select `With<LocalPlayer>` and keep their `Single<...>`
queries: a second local player is a bug and panicking is the correct failure.
Shared simulation systems (physics, persistence, pickup into the owning
player's inventory) query `Player` and work unchanged for remotes.

Do not rename `Player`: the base component already carries the right meaning,
and a marker expresses ownership without changing the archetype. `RemotePlayer`
only becomes necessary if remotes must be distinguishable from other
player-like entities.

## Path To Multiplayer

**Stage 0 - now (single player)**
- Route world mutations through a `Command`/`Intent` enum and validated events;
  derive `serde` on them (`serde` and `bincode` are already dependencies).
- Dimension scoping and multi-dimension simulation wait until this intent
  pipeline is done and capped off; dimension transfers must be intents. See
  `dimension_switching_plan.md` (Simulation Scoping And Dead Ends).
- Keep simulation in `FixedUpdate` at 20 Hz; presentation in `Update`.
- Split plugins into `ClientPlugin` / `ServerPlugin` groups. The sim must build
  without rendering.
- Add a headless test harness that steps the sim with scripted inputs and
  records a per-tick state hash.

**Stage 1 - loopback (still one process)**
- Define a `Transport` trait and a `LoopbackTransport` that moves typed messages
  with no serialization.
- Step two `App`s/`World`s manually in tests, behind a faulty-link wrapper that
  injects latency, jitter, loss, duplication, reordering, and stalls.
- Assert convergence: after quiescence, client hash == server hash.

**Stage 2 - server thread**
- Move the server to a headless `App` on a worker thread (`MinimalPlugins`;
  winit keeps the client on the main thread).
- Connect with channels; pump messages at tick boundaries.
- Interpolate remote entities to decouple render rate from 20 Hz state.

**Stage 3 - real network**
- TCP first (ordered and reliable, mirroring Java). Versioned envelope with
  sequence numbers and acks; `bincode` payloads replaceable later.
- Login/handshake, per-connection outbound queues, backpressure.
- Replicate chunks and entities; absolute teleports once divergence exceeds a
  threshold instead of accumulating relative deltas.

**Stage 4 - prediction and polish**
- Client-side prediction + reconciliation for own movement (already
  client-owned) and the bounded block-break overlay.
- Optional lag compensation and rollback; only now consider `lightyear`.
- Multi-client conflict tests, accelerated soak tests, and desync report dumps
  (recent event log from both sides).

## Packages And Ideas To Steal

- `serde` + `bincode`: already present; use for the wire envelope and add codec
  fuzz tests (corrupted/truncated/out-of-order messages fail gracefully).
- `bevy_replicon`: server-authoritative replication for Bevy.
- `lightyear`: prediction, rollback, interpolation, replication. Adopt at
  Stage 4; check Bevy 0.19 compatibility before committing.
- `renet` / `quinnet`: transports if not hand-rolling sockets.
- Ideas:
  - Valve's "Source Multiplayer Networking" article: prediction + server
    reconciliation with pending input replay.
  - Gaffer On Games networking series; Gabriel Gambetta's client-side
    prediction articles.
  - Overwatch GDC talk (Timothy Ford): logical clock vs render clock.
  - Factorio: per-tick state hashes and replayable desync reports.
  - GGPO: rollback concepts if fighting-game-grade responsiveness is ever needed.
  - Minecraft specifics: `LocalServerConnection` (in-memory packet queue),
    bounded block prediction overlay, ~3-tick interpolation delay, and
    client-owned player movement with server speed checks.

## Physics And Determinism (avian3d)

Current use: kinematic player and mobs with custom `collide_and_slide`
(Minecraft-style AABB movement), static chunk colliders, dynamic dropped items,
and `PhysicsPlugins::default()`.

- `default()` enables `parallel` and `f32`. Parallel solver iteration order and
  platform `libm` differences make avian not cross-platform deterministic by
  default.
- avian has an off-by-default `enhanced-determinism` feature (libm-based math)
  and an `f64` option.
- Multiplayer rule: the server owns the avian simulation. The client predicts
  only kinematic player movement through the same collide-and-slide code and
  interpolates everything else. Never predict dynamic bodies.
- For deterministic tests: same build and platform, fixed physics schedule, and
  consider a test profile without `parallel`. Still keep the correction path;
  never rely on bit-identical simulation across machines.

## Desync Testing Checklist

- Per-tick state hashing + recorded seed/input replays; every found desync
  becomes a permanent regression test.
- Faulty-link convergence tests (latency, jitter, loss, reorder, duplicate).
- Reconciliation unit tests: merge predicted state with a correction at
  sequence `n`; result must equal the server while replaying only inputs `> n`.
- Conflict tests: two or more clients act on the same block/entity; all must
  converge to one outcome.
- Accelerated soak tests with invariants (item conservation, no ghost entities).
- Production: digest mismatch triggers region resync; never hope drift heals.

## Open Questions

- Inventory/crafting model: server-authoritative lists vs stack transactions.
- Mob AI determinism budget: how much mob state to replicate, at what rate.
- Whether chunk generation needs server parity for seed sharing or anti-cheat.
- TCP vs QUIC/UDP if latency ever matters more than ordered reliability.
