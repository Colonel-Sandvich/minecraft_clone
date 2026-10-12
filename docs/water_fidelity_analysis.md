# Water Fidelity Analysis

Analysis of `src/world/chunk/fluid_sim.rs` (`simulate_fluid_step`) against
vanilla Java Edition water behaviour, using the ASCII scenario tests and
targeted probes. Scenario coverage lives in `docs/water_fidelity_tests.md`.

Status legend: **matches** = covered by a passing scenario; **suspected** =
probe shows behaviour believed non-vanilla, verification pending; **pending** =
oracle research not finished.

## Model summary

The solver is a batch pass over every fluid cell in the active chunks:

1. Clear every fluid cell in the processed chunks.
2. For each fluid cell in the snapshot:
   - a source stays a source;
   - if the cell below can accept fluid, write falling water (level 8) below and
     skip horizontal spread entirely;
   - otherwise write `level - 1` into every valid horizontal direction, where
     "valid" is filtered to the nearest reachable drop within 4 blocks when one
     exists.
3. Promote freshly written flowing cells to sources when supported and adjacent
   to 2+ sources, or to 1 source with a source above.
4. Any cleared cell that is not rewritten becomes air.

One pass runs every 5 game ticks (4 updates/second), the vanilla horizontal
cadence.

## Confirmed against scenarios (matches)

These final states are pinned by passing scenarios in both runners:

| Scenario | What it pins |
| --- | --- |
| `flat_floor_diamond` | levels are `8 - manhattan_distance`, stop at 1 |
| `channel_seven_block_range` | flat range is 7 blocks; one level per block |
| `infinite_source_gap_over_stone` | two sources with a gap over stone create a source |
| `nearest_drop_preference` | water ignores open flat ground for a nearby drop |
| `falling_water_landing_spread` | landing cell is level 8 and spreads level 7 |
| `cross_chunk_channel_flow` | levels continue across chunk borders |

## Probe results (current behaviour)

Temporary instrumented probes (not committed) produced the following.

### A. Three source neighbours with a hole below

```
y=1: #S7...      S at west/north/south, test cell centre, open east
y=0: ##.###      hole under the test cell
```

Current result: the test cell (7) only writes falling water into the hole;
the open cells to the east stay dry. `y=1` stays `#S7...` and `y=0` becomes
`##8###`.

**Suspected deviation:** vanilla `FlowingFluid.spread()` also spreads sideways
while falling when the cell has 3+ horizontally adjacent sources
(`sourceNeighborCount(level, pos) >= 3`). Here the test cell has 3 source
neighbours, so vanilla would wet the east cell too. Verification pending.

### B. Gap supported by a water source below

```
y=1: #S.S#  →  #SSS#   (gap promoted to source)
y=0: ##S##
```

Current result: promoted, because support accepts "solid block **or** water
source below".

**Suspected deviation:** vanilla's conversion test uses
`blockState.below().blocksMotion()`; water does not block motion, so vanilla
would keep the middle cell flowing. Verification pending.

### C. One source neighbour plus a source above

```
y=1: #S..#  →  #SS7#
y=2: ##S##
```

Current result: promoted to a source.

**Suspected deviation:** the one-source-plus-source-above rule may be
Bedrock-only; if Java lacks it the middle cell should stay flowing.
Verification pending.

### D. Draining after source removal

Channel `S7654321` settled, then the source removed:

| Step | Row |
| --- | --- |
| 0 | `#65654321..` |
| 1 | `#45454321..` |
| 2 | `#43434321..` |
| 3 | `#23232321..` |
| 4 | `#21212121..` |
| 5 | `#.1.1.1.1..` |
| 6 | `#..........` |

So the model *does* drain gradually (each pass, cells only receive
`neighbour_level - 1`, so a cell with no upstream support loses its level),
but the shape alternates in a way that looks model-specific. Vanilla
recomputes each cell's level from its neighbours each tick; the transient
pattern and drain timing need comparison. Verification pending.

### E. Falling column speed

Shaft with a source at the top; the column grows one block per fluid pass
(i.e. one block per 5 game ticks). Vanilla's falling cadence needs
confirmation; if falling is 1 block/tick, the clone is 5x slow for waterfalls.

## Architectural notes

- **Batch clear/recompute vs scheduled ticks.** Vanilla ticks individual fluid
  positions with per-block scheduling and recomputes a cell's level from its
  neighbours. The clone recomputes all active cells from the previous snapshot
  in world order. Final states agree on the scenarios above; timing/transient
  states can differ (probe D shape, falling cadence).
- **Source chunk processing.** `simulate_fluid_step` receives source chunks; the
  dimension system expands them to neighbours containing fluids and snapshots
  the source chunks plus all 26 neighbour chunks, so cross-chunk flow works.
  Single-chunk `Chunk::step_fluids` drops boundary writes and reports
  `boundary_changed` instead.
- **Budget.** The fluid system processes at most `FluidStepBudget` (default 64)
  source chunks per pass, sorted by position; overflow is deferred.
- **`spread_dirs` drop search** uses a BFS capped at 4 horizontal steps from
  each candidate and only spreads into directions tied for the nearest drop.
  It does not exclude the direction it came from, unlike a path search; this
  can matter in tight spirals (untested).
- **Level 8 ambiguity.** `water_flow(8)` (falling) and `water_source()` are
  distinct states but render as the same char only via `S`; flow level 8 and
  source level 8 are separated by form.

## Open questions for the vanilla oracle

1. Exact `sourceNeighborCount >= 3` sideways spread while falling (probe A),
   and what level the sideways flow gets.
2. Source conversion support rule: `blocksMotion()` only, or water below
   accepted (probe B).
3. Whether Java has the one-source + source-above rule (probe C).
4. Falling water cadence and how the landing cell's level is derived (probe E).
5. Level recomputation from neighbours when flows meet or when a source is
   removed; drain transient shape and timing (probe D).
