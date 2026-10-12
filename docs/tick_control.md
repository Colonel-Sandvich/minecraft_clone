# Tick control: freezing and stepping the simulation

Minecraft can freeze and step its server ticks with `/tick freeze` and
`/tick step`. This game has the same dev controls, bound to keys instead of
commands:

| Key | Action |
| --- | --- |
| `P` | Freeze / resume the simulation |
| `O` | Step one fixed tick of the frozen systems |
| `I` | Step up to the next fluid update (at most 5 fixed ticks) |
| `U` | Include player movement in the freeze and its steps |

While frozen, a banner lists the controls and shows how many fixed ticks remain
until the next fluid update.

## What freezes

The fixed tick is 20 Hz (`FIXED_TICK_RATE_HZ`) and fluids step every
`FLUID_STEP_PERIOD_TICKS` (5) fixed ticks, so the frozen set is:

- the fluid pipeline (`tick_counter`, `step_chunk_fluids`), and
- the water test suite's clock, so a frozen run does not time out while the
  developer inspects it.

Everything gated declares membership in a `TickControlSet`. A `TickGate`
resource is resolved once per fixed tick in `TickControlSet::Decide`, ordered
before the `Simulation` and `Player` sets, and the run conditions read it.
Systems in apps without `TickControlPlugin` (most unit tests) do not carry the
run conditions and keep ticking; the conditions also treat a missing gate as
"run".

Player movement (input, drag, gravity, `collide_and_slide`) is in the `Player`
set and stays live by default, so the world holds still while the developer
flies around it. `U` moves the player into the freeze, making each step advance
movement one tick: that is how to watch `collide_and_slide` resolve tick by
tick.

`F8` does not touch the freeze: starting a run while frozen builds the plots
and holds them until they are stepped (`O`/`I`) or the simulation is resumed
(`P`). The suite clock shares the freeze, so a held run never times out. That
makes "freeze, start a run, step one fluid update at a time" the intended way
to read an in-game scenario.

## What deliberately stays live

- **Block edits** land while frozen (place a source, then step the water by
  hand). Only the fluid pipeline is held.
- **Dropped-item physics** (avian) keeps running. Gating the drop request
  producers and consumers apart would lose the requests, and the feature is
  scoped to the fluid simulation for now.
- Camera look, inventory/UI, streaming, meshing, lighting, and saving are all
  unaffected (they live in `Update`/`PostUpdate`).

## Why not pause `Time`?

Esc already pauses `Time<Virtual>`, which stops every fixed schedule including
the player. The freeze here must keep the player moving while the simulation
holds, and both live in the same `FixedUpdate`/`FixedPostUpdate` schedules, so
the only workable approach is gating systems with run conditions. The Esc pause
and this freeze are independent: pausing the game also holds the simulation,
but freezing the simulation does not pause the game.

## Stepping and the fluid period

`step_chunk_fluids` runs when the incrementing counter is a multiple of
`FLUID_STEP_PERIOD_TICKS`, so one press of `O` may run a fixed tick without
touching water. `I` computes the remaining ticks from the current counter phase
(`period - counter % period`) and queues exactly that many, so one press always
produces exactly one fluid update. The gate consumes one pending step per fixed
tick, so a frame that runs several fixed ticks (catch-up) cannot run multiple
steps per press.
