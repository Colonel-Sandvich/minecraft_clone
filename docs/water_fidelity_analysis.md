# Water Fidelity Analysis

How `src/world/chunk/fluid_sim.rs` compares to vanilla Java Edition water, and
what this branch changed. Scenario coverage is described in
`docs/water_fidelity_tests.md`; run `cargo test water_scenarios` or press `F8`
in game.

## Method

The comparison used vanilla Java Edition 1.20.1 as the oracle:

- Decompiled `FlowingFluid`/`WaterFluid` from the official server jar with
  Mojang mappings, and diffed the fluid classes against 1.14.4, 1.16.5 and
  1.21.1 (the algorithm is unchanged across those versions).
- Cross-checked against live measurements on a vanilla 1.20.1 dedicated server
  (tick-accurate datapack): falling cadence, drain order, direction choice,
  source conversion matrix.
- Cross-checked with the [Water](https://minecraft.wiki/w/Water),
  [Fluid](https://minecraft.wiki/w/Fluid) and [Tick](https://minecraft.wiki/w/Tick)
  wiki pages.

## The solver today

One fluid step runs every fifth game tick (4 steps/second), matching vanilla's
5-tick spread delay. A step has two phases that mirror vanilla's
`FlowingFluid.tick`:

**Phase 1 — recompute (`getNewLiquid`).** Every existing fluid cell computes
its own next state, in this order:

1. A source stays a source.
2. If the same fluid sits directly above, the cell is falling (flowing level 8)
   — regardless of how it was fed.
3. Else, if two or more horizontal neighbours are sources **and** the block
   below is a solid block or a water source, the cell becomes a source.
4. Else its level is one below the fullest horizontal same-fluid neighbour
   (sources counting as level 8). With no such neighbour the cell dries up.

**Phase 2 — spread (`FlowingFluid.spread`).** Each cell then spreads using its
recomputed state:

- If the cell below is empty, place falling water there. Additionally spread
  sideways when the cell has three or more horizontal source neighbours (the
  wide-pool lip rule).
- Otherwise, a source always spreads sideways; flowing water spreads sideways
  unless the cell below is a water hole (not a solid block), which keeps a
  stream above a hole one block wide. A cell whose below is **water** also
  counts as a hole: vanilla `FlowingFluid.spread` fails the downward
  `canSpreadTo` (water below is not replaceable by water) and `isWaterHole`
  short-circuits true for same-type fluid below, so a non-source cell never
  spreads sideways over water — only sources keep spreading. Re-verified
  against a fresh 1.20.1 client decompile (`spread`, `isWaterHole`) and a
  live-server two-deep trench (top layer stays one cell wide over the lower
  layer).

Sideways directions come from a nearest-drop search: a direction whose cell
below is a hole (empty or water) wins outright; otherwise the nearest hole
within four horizontal steps is chosen, with ties spreading in all tied
directions. Spreads only ever fill empty cells — vanilla never overwrites
water with water, so existing cells change only through their own recompute.

## Verified behaviours and the tests that pin them

| Vanilla rule | Scenario / test |
| --- | --- |
| Levels are `8 - manhattan distance`, stopping at 1 | `flat_floor_diamond` |
| Flat range is 7 blocks from a source, one level per block | `channel_seven_block_range` |
| Two sources with a gap over stone create a source | `infinite_source_gap_over_stone` |
| Conversion support also accepts a **water source** below | `gap_over_source_becomes_source` |
| Java has **no** one-source-plus-source-above rule; that cell is falling | `source_above_is_falling_not_source` |
| Three or more source neighbours spread sideways while pouring | `three_sources_spread_while_falling` |
| A source pours first; once the pour fails it spreads sideways | `source_over_hole_spreads_sideways` |
| Flowing water above a hole never spreads sideways | `flow_into_hole_stays_narrow` |
| Falling water lands at level 8 and spreads 7 | `falling_water_landing_spread` |
| Falling descends exactly one block per step (5 ticks) | `falling_column_descends_one_cell_per_step` |
| Water ignores flat ground to reach a nearby drop | `nearest_drop_preference` |
| Flow continues across chunk borders | `cross_chunk_channel_flow` |
| Damming a settled channel drains everything downstream | `channel_dam_after_settle` |
| Breaking a wall beside a source fills the pocket at level 7 | `channel_wall_break_after_settle` |
| Removing a source drains its settled range completely | `channel_source_removed_after_settle` |
| A hanging waterfall drains completely, taking its time | `hanging_waterfall_dissipates_after_source_removed` |

The last two rows are multi-phase scenarios: they settle first, then apply a
block edit through the production edit path and settle again, so the
edit-triggered invalidation and recomputation path is pinned too.

Already correct before this branch (now locked in by the tests above): the
5-tick horizontal cadence, downward-first priority, nearest-drop direction
choice, level decay, infinite-water conversion over stone, landing spread, and
falling cadence.

## Fixes in this branch

The probes in the previous revision of this document found four deviations
from Java; all four are fixed by the two-phase rewrite:

1. **One-source-plus-source-above conversion** existed in the solver but is a
   Bedrock-only rule (verified live on 1.20.1: the cell becomes falling water,
   not a source). Removed; the recompute phase turns that cell falling.
2. **Sideways spread while pouring with 3+ source neighbours** was missing
   entirely. Added to the spread phase.
3. **A source above a hole never spread sideways**, because the old
   clear-and-rewrite model kept "pouring" into water below forever. The spread
   phase now stops pouring into non-empty cells, so sources spread sideways
   from the next step, and flowing water correctly stays narrow above a hole.
4. **Existing water was only updated by upstream writes**, so a cell whose
   upstream spread was suppressed (the fix in 3) would vanish. The recompute
   phase now gives every existing cell its own vanilla `getNewLiquid` update.

One timing change fell out of this: a gap between two sources now converts on
its *second* step instead of its first, because the placed cell gets its own
update one step after it is created — exactly what vanilla's placed-block
scheduling (`onPlace` → `scheduleTick(..., 5)`) produces.

## Remaining known differences

These are structural or untested corners, not observed mismatches:

- **Scheduling model.** Vanilla schedules per-block fluid ticks with
  coalescing; this solver batch-processes active chunks in a fixed order every
  fifth tick. Final states match the scenarios; transient shapes during
  rearrangements (e.g. the exact drain order of a channel) can differ, and a
  `setBlock`-heavy tick may behave differently from vanilla's priority queue.
- **Slope search shape.** Vanilla's hole search is a depth-limited DFS that
  excludes stepping back through the direction it came from; this solver uses a
  BFS with a visited set, which can path back through the origin and cannot
  revisit cells. It agrees on all covered scenes, but tight spirals and
  dead-end geometry may choose different directions.
- **No scheduled-tick persistence.** Vanilla saves pending fluid ticks per
  chunk; the batch model instead re-derives state from cells each step, so
  saving mid-propagation resumes from the cell grid rather than exact tick
  timing. Steady states are unaffected.
- **Can-place checks are simplified.** Vanilla's `canPassThroughWall`,
  `canHoldFluid`, replaceable plants and waterlogging are reduced to "full
  solid blocks block everything else". The clone has no slabs, plants or
  waterloggable blocks yet.
- **World edges.** Cells outside the snapshot count as non-holders; there is no
  void-pour behaviour for an unloaded world bottom.

## Files

- Solver: `src/world/chunk/fluid_sim.rs` (two phases: `recompute_fluid`,
  `spread_fluid`; direction search: `spread_dirs`, `nearest_drop_distance`).
- Scheduling: `src/world/dimension/fluid.rs` (5-tick counter, chunk budget,
  invalidation-driven activity).
- Scenarios: `src/world/chunk/water_scenarios.rs`.
- Runners: headless (`FluidSceneWorld` in the scenario module) and in-game
  (`src/game_test/`, overlay in `src/ui/game_test.rs`).
