# Water Fidelity Tests

Two runners exercise the same set of water scenarios:

- **Headless** (`cargo test water_scenarios`) — a pure solver driver. Fast, used
  in CI.
- **In-game** (`F8`) — builds the same scenes in the live world, lets the real
  `FixedUpdate` fluid pipeline settle them, and reports pass/fail in an overlay.
  Used by developers to watch a scene and inspect failures in place.

Both compare the *final settled state* cell by cell against a hand-written
expectation. All scenario data lives in
`src/world/chunk/water_scenarios.rs`; the in-game runner is in
`src/game_test/`.

## Writing a scenario

Scenes are layered text maps. `layers[y]` is the horizontal slice at height
`y`, `layers[y][z]` is the row at depth `z`, columns run along `x`.

| Char | Cell |
| --- | --- |
| `.` / ` ` | air |
| `#` | stone |
| `G` | glass |
| `L` | oak log |
| `S` | water source (level 8) |
| `1`..`7` | flowing water level |
| `8` | falling water (flowing level 8) |

Example, a source at one end of a walled channel:

```rust
WaterScenario {
    name: "channel_seven_block_range",
    origin: IVec3::ZERO,
    max_steps: 64,
    input: &[
        &["##########"; 3],
        &["##########", "#S........", "##########"],
    ],
    expected: &[
        &["##########"; 3],
        &["##########", "#S7654321.", "##########"],
    ],
    description: "...",
}
```

Every layer must have the same number of rows and every row the same width.
`max_steps` is the fluid-step budget: the scene must stop changing within it,
in both runners.

### The stone shell is implicit

`layout_scene` walls the scene in with a one-block stone shell on every side
except the top. Authors should not draw outer walls; the shell stops water
escaping sideways along the open chunk and falling out of the world, which
otherwise shows up as an inscrutable test failure. The top stays open so the
in-game runner can be inspected from above. The headless driver's `diff()`
also fails loudly if any fluid ends up outside the reviewed box, which catches
shell bugs and accidentally-unsealed scenes.

## Headless runner

`FluidSceneWorld` inside `water_scenarios.rs` is a minimal multi-chunk world
that only runs `simulate_fluid_step`. It steps until the returned update set is
empty and then compares. One driver step is one fluid update; in the running
game that is one update every 5 fixed ticks (0.25 s).

```bash
cargo test -p minecraft_clone water_scenarios
```

Failures print the scenario name, its description, every mismatching row with
world coordinates, and the full actual scene.

## In-game runner

| Key | Action |
| --- | --- |
| `F7` | Toggle the status overlay |
| `F8` | Build/rebuild and run every registered scenario |
| `F9` | Teleport above the first failed/timed-out plot (or the running one) |

Plots are laid out in a grid starting at world `(4096, 64, 4096)`, spaced 64
blocks apart. Each plot is an independently registered chunk column built from
the same `SceneLayout` as the headless runner. Chunks get a real light rebuild,
mesh, and colliders, so the developer sees actual rendered water.

The runner does not tick fluids itself. It builds the scene, marks chunks for
the normal `ChunkNeedsFluidStep` pipeline, and waits for every plot chunk to go
quiet (no pending fluid step) for at least one fluid period before comparing.
Comparison reads the live `Chunk` components and happens in `Update`, after
`FixedUpdate` commands have flushed, so markers observed are post-step.

Deliberate differences from Minecraft's GameTest framework, because this repo
has no command system, no structure files, and far fewer tests:

- Scenes are code/text, not `.nbt` templates or data packs.
- All plots run **in parallel** in one remote field instead of sequential
  batches. Scenes are independent, so this is safe; it is faster and lets the
  dev watch all of them settle at once.
- Failures are left standing in place (like Minecraft); `F8` rebuilds
  everything from scratch on the next run.
- Teleport/inspection replaces `/test locate` + `/test runclosest`.
- Rotation sweeps, retries, environments/undo-logs, mock players, and barrier
  shells are intentionally not implemented.

## Non-obvious implementation notes

- **Test chunks must never be saved.** The fluid solver routes every cell write
  through `ChunkEditor`, which marks chunks `ChunkNeedsSave`. The runner
  removes that marker from plot chunks every `Update`, before the save capture
  runs in `PostUpdate`. Without this, running the tests would overwrite parts
  of the player's save with test geometry.
- **The streamer ignores plot columns.** `register_published_chunk` is for
  chunks outside the streaming system's ownership. `start_column_loads` skips
  columns that already contain a loaded chunk, so the plot columns are never
  streamed, evicted, or split across load states. Do not run the suite inside a
  currently desirable view chunk; `build_plot` fails the scenario rather than
  panicking if a plot chunk is already loaded.
- **Settle detection needs a grace period.** Fluid steps happen only every
  fifth fixed tick, so a quiet plot is only trusted after
  `TICKS_PER_FLUID_STEP` (5) ticks, guaranteeing at least one fluid step
  happened after the scene was built.
- **Timeout is in fixed ticks**, `max_steps * 5 + 20` slack, not wall time.
- **F7/F8/F9** are unbound keys elsewhere; there is no chat/command system to
  attach `/test`-style commands to yet.
- The headless driver passes **all chunks with fluids** as source chunks on
  every step, which is more generous than the running game (budgeted, active
  markers only). The in-game runner is the authority on pipeline behaviour.

## Current fidelity gaps

These are known places where the solver's *model* differs from vanilla Java
Edition; the final-state scenarios above mostly cannot see them yet, because
they test where the state converges rather than how it gets there. They are
tracked here until scenario coverage and/or solver work lands:

- The solver is a batch "clear and re-derive from upstream" pass over active
  chunks, not per-block scheduled fluid ticks. When a source is removed, the
  whole stream disappears in one fluid update, where vanilla drains gradually.
- Falling water descends one block per fluid update (5 game ticks), so long
  waterfalls grow visibly slower than vanilla if vanilla falls at one block per
  tick.
- Water can spread sideways only when it cannot fall; vanilla also spreads
  sideways while falling when the cell has three or more adjacent source
  blocks.
- Source conversion checks "solid block **or water source** below", while
  vanilla's condition is that the block below blocks motion (a solid block);
  the one-source-plus-source-above rule also needs verification against Java
  Edition specifically.
