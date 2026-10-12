//! Dev controls for freezing and stepping the world simulation.
//!
//! Minecraft can freeze and step its server ticks (`/tick freeze`,
//! `/tick step`); this is the same idea for this game's fixed tick:
//!
//! - `P` freezes the simulation. Fluid updates stop, and the water test
//!   suite's clock stops with them, so a run can be inspected by hand.
//! - `O` runs exactly one fixed tick of the frozen systems.
//! - `I` runs the ticks needed to reach the next fluid update (at most
//!   [`FLUID_STEP_PERIOD_TICKS`]).
//! - `U` moves the player into the frozen set, so steps advance player
//!   movement too (handy when debugging `collide_and_slide`).
//!
//! Player movement is excluded by default, so the world holds still while the
//! developer flies around it.
//!
//! The freeze gates systems; it does not pause `Time<Virtual>`. The fixed
//! schedules are shared between the simulation and player movement, so pausing
//! time would freeze the player along with the water. Instead, systems declare
//! which [`TickControlSet`] they belong to, and the per-tick [`TickGate`]
//! decides what runs.
//!
//! Block edits and dropped-item physics deliberately stay outside the freeze:
//! edits must still land while frozen (place a source, then step the water by
//! hand), and gating the drop request producers and consumers apart would
//! drop the requests before they were consumed.

use bevy::prelude::*;

use crate::{
    game_state::gameplay_input_active,
    world::dimension::fluid::{FLUID_STEP_PERIOD_TICKS, FluidTickCounter},
};

pub(crate) struct TickControlPlugin;

/// Freeze/step state owned by the developer.
#[derive(Resource, Default)]
pub(crate) struct TickControl {
    frozen: bool,
    /// Player movement joins the freeze and its steps.
    player_included: bool,
    /// Frozen fixed ticks still owed; each runs one simulation tick.
    steps: u32,
    /// Flag: expand the next fixed tick into the ticks up to a fluid update.
    step_to_fluid: bool,
}

impl TickControl {
    /// Whether the simulation is being stepped by hand right now.
    pub(crate) fn frozen(&self) -> bool {
        self.frozen
    }

    /// Whether player movement follows the freeze instead of staying live.
    pub(crate) fn player_included(&self) -> bool {
        self.player_included
    }

    /// Resumes normal ticking, e.g. from the freeze toggle.
    fn resume(&mut self) {
        self.frozen = false;
        self.steps = 0;
        self.step_to_fluid = false;
    }

    /// Freezes if needed, then toggles between frozen and running.
    fn toggle_freeze(&mut self) {
        if self.frozen {
            self.resume();
        } else {
            self.frozen = true;
        }
    }

    /// Freezes if needed and owes one explicit step.
    fn step_once(&mut self) {
        self.frozen = true;
        self.steps = self.steps.saturating_add(1);
    }

    /// Freezes if needed and owes the ticks up to the next fluid update.
    fn step_to_fluid(&mut self) {
        self.frozen = true;
        self.step_to_fluid = true;
    }

    /// Resolves what one fixed tick may run, consuming a pending step while
    /// frozen.
    fn begin_fixed_tick(&mut self, fluid_counter: u32) -> TickGate {
        if std::mem::take(&mut self.step_to_fluid) && self.frozen {
            self.steps = self
                .steps
                .saturating_add(ticks_until_fluid_update(fluid_counter));
        }

        let simulation = !self.frozen || self.steps > 0;
        if self.frozen && self.steps > 0 {
            self.steps -= 1;
        }

        let player = simulation || !(self.frozen && self.player_included);
        TickGate { simulation, player }
    }
}

/// What the current fixed tick is allowed to run, resolved by
/// [`decide_tick_gate`] before any gated system runs.
#[derive(Resource)]
pub(crate) struct TickGate {
    pub(crate) simulation: bool,
    pub(crate) player: bool,
}

impl Default for TickGate {
    /// Permissive until a gate decides, so apps that add gated systems
    /// without [`TickControlPlugin`] (most tests) keep running normally.
    fn default() -> Self {
        Self {
            simulation: true,
            player: true,
        }
    }
}

/// Fixed-tick ordering for [`TickControlPlugin`].
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TickControlSet {
    /// Resolves [`TickGate`] for this tick; everything gated runs after it.
    Decide,
    /// Simulation systems that follow the freeze and its steps.
    Simulation,
    /// Player systems that stay live unless the player is included.
    Player,
}

impl Plugin for TickControlPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TickControl>()
            .init_resource::<TickGate>()
            .configure_sets(
                FixedUpdate,
                TickControlSet::Decide
                    .before(TickControlSet::Simulation)
                    .before(TickControlSet::Player),
            )
            .add_systems(FixedUpdate, decide_tick_gate.in_set(TickControlSet::Decide))
            .add_systems(
                Update,
                handle_tick_control_keys.run_if(gameplay_input_active),
            );
    }
}

fn decide_tick_gate(
    mut control: ResMut<TickControl>,
    counter: Option<Res<FluidTickCounter>>,
    mut gate: ResMut<TickGate>,
) {
    let counter = counter.map_or(0, |counter| counter.0);
    *gate = control.begin_fixed_tick(counter);
}

fn handle_tick_control_keys(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut control: ResMut<TickControl>,
) {
    let Some(keys) = keys else {
        return;
    };
    if keys.just_pressed(KeyCode::KeyP) {
        control.toggle_freeze();
    }
    if keys.just_pressed(KeyCode::KeyO) {
        control.step_once();
    }
    if keys.just_pressed(KeyCode::KeyI) {
        control.step_to_fluid();
    }
    if keys.just_pressed(KeyCode::KeyU) {
        control.player_included = !control.player_included;
    }
}

/// Fixed ticks before the next fluid update, counting the tick that performs
/// it. The counter is incremented before the period check, so a counter
/// already on the period waits a whole new period.
pub(crate) fn ticks_until_fluid_update(counter: u32) -> u32 {
    FLUID_STEP_PERIOD_TICKS - counter % FLUID_STEP_PERIOD_TICKS
}

/// Run condition for simulation systems gated by the freeze.
pub(crate) fn simulation_ticks_allowed(gate: Option<Res<TickGate>>) -> bool {
    gate.is_none_or(|gate| gate.simulation)
}

/// Run condition for player systems that stay live unless the player was
/// included in the freeze.
pub(crate) fn player_ticks_allowed(gate: Option<Res<TickGate>>) -> bool {
    gate.is_none_or(|gate| gate.player)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_ticks_run_only_queued_steps() {
        let mut control = TickControl::default();
        control.toggle_freeze();

        for _ in 0..3 {
            assert!(!control.begin_fixed_tick(0).simulation);
        }

        control.step_once();
        assert!(control.begin_fixed_tick(0).simulation);
        assert!(!control.begin_fixed_tick(0).simulation);
        assert!(control.frozen, "stepping must not unfreeze");
    }

    #[test]
    fn player_follows_the_freeze_only_when_included() {
        let mut control = TickControl::default();
        control.toggle_freeze();
        assert!(
            control.begin_fixed_tick(0).player,
            "player stays live by default"
        );

        control.player_included = true;
        assert!(!control.begin_fixed_tick(0).player);

        control.step_once();
        let gate = control.begin_fixed_tick(0);
        assert!(
            gate.simulation && gate.player,
            "a step advances the player too"
        );
    }

    #[test]
    fn resuming_clears_pending_steps() {
        let mut control = TickControl::default();
        control.step_once();
        control.resume();
        assert!(!control.frozen);

        control.toggle_freeze();
        assert!(
            !control.begin_fixed_tick(0).simulation,
            "the step queued before the resume must be gone"
        );
    }

    #[test]
    fn stepping_to_fluid_queues_the_ticks_to_the_update() {
        // The counter is incremented before the modulo check, so ticks-to-go
        // is `period - counter % period`: from phase 4 it is the next tick,
        // and from phase 0 the whole period.
        assert_eq!(ticks_until_fluid_update(0), 5);
        assert_eq!(ticks_until_fluid_update(1), 4);
        assert_eq!(ticks_until_fluid_update(4), 1);
        assert_eq!(ticks_until_fluid_update(5), 5);

        let mut control = TickControl::default();
        control.step_to_fluid();
        let allowed = (0..FLUID_STEP_PERIOD_TICKS * 2)
            .filter(|_| control.begin_fixed_tick(4).simulation)
            .count();
        assert_eq!(allowed, 1, "exactly the ticks needed for the next update");
    }

    #[test]
    fn absent_gate_keeps_systems_running() {
        #[derive(Resource, Default)]
        struct Probe(u32);

        fn probe(mut probe: ResMut<Probe>) {
            probe.0 += 1;
        }

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Probe>()
            .add_systems(Update, probe.run_if(simulation_ticks_allowed));

        app.update();
        assert_eq!(app.world().resource::<Probe>().0, 1);

        app.init_resource::<TickGate>();
        app.world_mut().resource_mut::<TickGate>().simulation = false;
        app.update();
        assert_eq!(
            app.world().resource::<Probe>().0,
            1,
            "a frozen gate must block the system"
        );

        app.world_mut().resource_mut::<TickGate>().simulation = true;
        app.update();
        assert_eq!(app.world().resource::<Probe>().0, 2);
    }

    #[test]
    fn frozen_ticks_skip_sets_until_stepped() {
        #[derive(Resource, Default)]
        struct Probe {
            simulation: u32,
            player: u32,
        }

        fn simulation_probe(mut probe: ResMut<Probe>) {
            probe.simulation += 1;
        }

        fn player_probe(mut probe: ResMut<Probe>) {
            probe.player += 1;
        }

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Probe>()
            .add_plugins(TickControlPlugin)
            .add_systems(
                FixedUpdate,
                simulation_probe
                    .in_set(TickControlSet::Simulation)
                    .run_if(simulation_ticks_allowed),
            )
            .add_systems(
                FixedUpdate,
                player_probe
                    .in_set(TickControlSet::Player)
                    .run_if(player_ticks_allowed),
            );

        for _ in 0..3 {
            app.world_mut().run_schedule(FixedUpdate);
        }
        let probe = app.world().resource::<Probe>();
        assert_eq!(
            (probe.simulation, probe.player),
            (3, 3),
            "while running, both sets tick every fixed tick"
        );

        app.world_mut()
            .resource_mut::<TickControl>()
            .toggle_freeze();
        for _ in 0..3 {
            app.world_mut().run_schedule(FixedUpdate);
        }
        let probe = app.world().resource::<Probe>();
        assert_eq!(
            (probe.simulation, probe.player),
            (3, 6),
            "frozen stops the simulation but leaves the player live"
        );

        let control = &mut app.world_mut().resource_mut::<TickControl>();
        control.player_included = true;
        control.step_once();
        app.world_mut().run_schedule(FixedUpdate);
        let probe = app.world().resource::<Probe>();
        assert_eq!(
            (probe.simulation, probe.player),
            (4, 7),
            "a step with the player included advances both sets by one tick"
        );
    }
}
