# Water Fidelity Tests

Two runners exercise the same set of water scenarios:

- **Headless** (`cargo test water_scenarios`) — a pure solver driver. Fast, used
  in CI.
- **In-game** (`F8`) — builds the same scenes in the live world, lets the real
  `FixedUpdate` fluid pipeline settle them, and reports pass/fail in an overlay.
  Used by developers to watch a scene and inspect failures in place.

Both compare the settled state cell by cell against a hand-written
expectation. A scenario can also declare follow-up **phases**: once the
opening scene settles and matches, the runner applies an edit (like a player
breaking or placing a block), settles again, and compares again. All scenario
data lives in `src/world/chunk/water_scenarios.rs`; the in-game runner is in
`src/game_test/`.

## Writing a scenario

Scenes are layered text maps. `layers[y]` is the horizontal slice at height
`y`, `layers[y][z]` is the row at depth `z`, columns run along `x`.

| Char | Cell |
| --- | --- |
| `.` / ` ` | air |
| `#` | glass |
| `L` | oak log |
| `S` | water source (level 8) |
| `1`..`7` | flowing water level |
| `8` | falling water (flowing level 8) |

Example, a source at one end of a walled channel:

```rust
WaterScenario {
    name: "channel_seven_block_range",
    input: &[
        &["##########"; 3],
        &["##########", "#S........", "##########"],
    ],
    expected: &[
        &["##########"; 3],
        &["##########", "#S7654321.", "##########"],
    ],
    description: "...",
    ..Default::default() // origin at zero, 64-step budget, no phases
}
```

Every layer must have the same number of rows and every row the same width.
`max_steps` is the fluid-step budget: the scene must stop changing within it,
in both runners.

### Follow-up phases (steady -> edit -> settle)

A single build-and-settle scene cannot reach the steady state -> block break
-> recompute -> steady state path, because that path only wakes up when a
block changes after the water has stopped moving. Scenarios cover it with
`phases`:

```rust
WaterScenario {
    name: "channel_dam_after_settle",
    // ...the usual input / expected / max_steps...
    phases: &[ScenePhase {
        max_steps: 64,
        // `~` leaves a cell alone; `.` clears it (breaking a block); every
        // other character is the usual cell character (placing one).
        edit: &[
            &["~~~~~~~~~~"; 3],
            &["~~~~~~~~~~", "~~~~#~~~~~", "~~~~~~~~~~"],
        ],
        expected: &[
            &["##########"; 3],
            &["##########", "#S76#.....", "##########"],
        ],
    }],
    description: "...",
}
```

The runner settles `input`, compares `expected`, then for each phase in order:
applies the edit, settles for that phase's `max_steps`, and compares its
`expected`. In-game the edit goes through the same `ChunkEditor` and
invalidation planning as a player's block edit, so the fluid pipeline is
woken and the settle detection re-armed exactly like in normal play. The
overlay shows which phase a scenario is on, e.g. `[2/3]`.

A phase (or the opening scene) can also pin how long it takes with
`expect_steps: Some(n)`: the headless driver then requires the scene to reach
its fixpoint in exactly `n` fluid steps. This is for behaviours where the
*duration* is the point, not just the end state — the hanging waterfall
drain is pinned this way. The in-game runner cannot count fluid steps that
precisely (it waits for quiet plus a grace period, and later plots share the
fluid budget), so in-game the `max_steps` timeout is the timing guard; the
exact number lives in the headless test and is watched by eye in-game.

### Relationship to Minecraft's GameTest

Minecraft ships the GameTest *framework* (the `/test` command, checkpoint
sequences such as `thenExecute`/`thenIdle`), but no water or fluid tests: no
released version from 1.16 through 1.21 ships `data/minecraft/gametest`
structures, and the test content Mojang runs internally is not published. So
there are no vanilla fluid fixtures to copy; these scenarios are hand-authored
from the verified rules. The phase mechanism above is our equivalent of a
`GameTestSequence` checkpoint (`settle -> assert`, `edit -> settle -> assert`),
which is exactly how the Java API expresses the steady -> break -> steady
pattern.

### The glass shell is implicit

`layout_scene` walls the scene in with a one-block glass shell on every side
except the top. Authors should not draw outer walls; the shell stops water
escaping sideways along the open chunk and falling out of the world, which
otherwise shows up as an inscrutable test failure. Glass (rather than stone)
keeps every fixture see-through, so water can be watched from the pulled-back
camera angle as well as from directly overhead. The top stays open so the
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
| `F7` | Toggle the status overlay (shown automatically when a run starts) |
| `F8` | Rebuild and run every registered scenario, then fly the player to the field |
| `F9` | Fly to the next test plot: the first failure on a fresh cycle, then each plot in order, wrapping around. With nothing built yet, `F9` starts a run instead. |
| `F11` | Write every built plot to `saves/dev/test_plots.txt` as ASCII maps (scenario layout, extended legend for hand-placed blocks). Use it after hand-editing a plot to hand the drawing back for transcription. |
| `R` | The existing debug reset, which returns the player to the dimension arrival. |

Because plot chunks are stripped of `ChunkNeedsSave` (they must never pollute
the world save) and `F8` rebuilds every plot from the scenario code, hand edits
inside a plot are ephemeral: draw first, then press `F11` before anything else
rebuilds the field. The dump lists each plot with its name, status, phase,
origin, and size, then the cell grid; the legend adds `g/d/t/a/L/v/G/i` for
grass, dirt, stone, sand, log, leaves, glowstone, and ice so marker blocks
placed while drawing survive the round trip.

Plots are packed row by row, starting at world `(4096, 64, 4096)`. Each origin
sits one block past a chunk boundary so a scene and its glass shell occupy whole
chunk columns; the next plot starts in the chunk column after that, so the
fixtures sit a few blocks apart instead of spread over a wide field. Each plot
is an independently registered chunk column built from the same `SceneLayout` as
the headless runner. Chunks get a real light rebuild, mesh, and colliders, so
the developer sees actual rendered water.

The teleports enable flight and frame the scene: the player is placed a few
blocks back along a fixed 45° view ray through the scene centre, so the water is
watched from the same angle every time. `F9` moves between plots and the overlay
marks the plot currently being viewed with `>`.

Pressing `F8` while standing in the test field first moves the player back to
the spawn and waits a few frames. The streamer owns every column around the
player, and a plot chunk cannot be registered inside a streamed column, so the
field must be released before a rerun. The arrival teleport then carries the
player back out to the first plot.

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
- `F8`/`F9` teleport the player instead of `/test locate` + `/test runclosest`.
- Rotation sweeps, retries, environments/undo-logs, mock players, and barrier
  shells are intentionally not implemented.

## Manual testing

The default inventory stocks two items for poking at water by hand:

- **Water** in hotbar slot 9 (the ninth slot). Water is a non-block item;
  placing it seeds a water source at the targeted cell. It behaves like any
  other placeable: unlimited in creative, one per placement in survival, and it
  can be dropped.
- **Ice** in the last main-inventory slot, directly above the water slot. Handy
  for walling off or filling a channel quickly, since it is an ordinary block.

Water sources are permanent, so the quickest way to clear an area is to place a
block into the water cell (blocks replace water). Breaking the block *under* a
source lets the column drain away.

### Freezing and stepping the simulation

`P` freezes the simulation, `O` runs one fixed tick, `I` runs the ticks up to
the next fluid update, and `U` includes player movement in the freeze and its
steps. A banner lists the controls while frozen. This is the in-game equivalent
of Minecraft's `/tick freeze` and `/tick step`; see
[`tick_control.md`](tick_control.md) for the design and for what stays live.
`F8` does not resume the simulation: a run started while frozen waits for
steps, and the suite overlay says so. The suite clock shares the freeze, so
the run cannot time out while it waits.

The hand-stepping loop is the fastest way to read a pour: freeze, place a
source (edits still land while frozen), then press `I` to advance exactly one
fluid update and watch the front move one block at a time.

## Non-obvious implementation notes

- **Test chunks must never be saved.** The fluid solver routes every cell write
  through `ChunkEditor`, which marks chunks `ChunkNeedsSave`, and pending fluid
  ticks are captured even without that marker. Plot chunks therefore carry
  `ChunkSaveExempt`, which the save capture skips outright, and the runner also
  removes `ChunkNeedsSave` from them every `Update`. Without this, running the
  tests would overwrite parts of the player's save with test geometry.
- **The streamer ignores plot columns.** `register_published_chunk` is for
  chunks outside the streaming system's ownership. `start_column_loads` skips
  columns that already contain a loaded chunk, so the plot columns are never
  streamed, evicted, or split across load states. Do not run the suite inside a
  currently desirable view chunk; `build_plot` fails the scenario rather than
  panicking if a plot chunk is already loaded.
- **Settle detection needs a grace period.** Fluid steps happen only every
  `FLUID_STEP_PERIOD_TICKS` (5) fixed ticks, so a quiet plot is only trusted
  after 5 ticks, guaranteeing at least one fluid step happened after the scene
  was built or edited.
- **Timeout is in fixed ticks per phase**, `max_steps * 5 + 20` slack, not wall
  time; each phase gets its own budget. The suite clock shares the simulation
  gate, so freezing the simulation also freezes a run's timeout budget.
- **Phase edits use the production edit path.** In-game the runner writes
  through `ChunkEditor` and `apply_chunk_invalidations`, exactly like the
  player's block interactions, so halo fanout and `ChunkNeedsFluidStep`
  re-marking are exercised; the headless driver applies the same edit cells
  directly and re-solves with every fluid chunk as a source.
- **F7/F8/F9/F11** (runner) and **P/O/I/U** (tick control) are unbound keys
  elsewhere; there is no chat/command system to attach `/test`-style commands
  to yet.
- The headless driver passes **all chunks with fluids** as source chunks on
  every step, which is more generous than the running game (budgeted, active
  markers only). The in-game runner is the authority on pipeline behaviour.

## Fidelity status

The scenarios above cover the settled states, including edit-triggered
re-settling (phases that break or place a block after the scene has settled)
and one pinned duration — the hanging waterfall drain, at 17 fluid steps.
Per-tick behaviour is otherwise pinned by targeted unit tests (falling
cadence, drain completeness). The solver was rewritten to vanilla's two-phase
model (recompute from neighbours, then spread) after oracle research against
decompiled 1.20.1 plus live server measurements. Verified rules, the fixes
made, and the remaining structural differences (tick scheduling and timing,
slope-search shape, simplified replaceability) are
documented in `docs/water_fidelity_analysis.md`.

When adding a scenario for a behaviour you are unsure about, check that doc
first: the deviations found so far (one-source-plus-source-above conversion,
sideways spread while pouring with 3+ sources, sources above a hole, cells
losing their level when upstream spread is suppressed, and the falling check
running before the two-source conversion) are all fixed and covered.
