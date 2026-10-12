//! In-game water fidelity tests, modelled on Minecraft's GameTest runner.
//!
//! Press `F8` to build every registered water scenario in a remote test field,
//! let the real fluid simulation settle each scene, and compare the result cell
//! by cell against the expected ASCII maps. `F7` toggles the results overlay and
//! `F9` teleports the player to the first failure so the scene can be inspected
//! by eye. Scenes are sealed stone shells with an open top, so failures can be
//! read from above without digging in.
//!
//! Test chunks are registered as independent published chunks (the streaming
//! system ignores columns that already contain a loaded chunk). They are never
//! allowed to keep `ChunkNeedsSave`, so running tests cannot pollute the
//! player's world save.

use std::collections::HashMap;

use avian3d::prelude::Position;
use bevy::prelude::*;

use crate::{
    mob::controller::{Flying, Velocity},
    player::{Player, PlayerDimension, cam::MouseCam},
    world::{
        chunk::{
            Chunk, ChunkCell, ChunkHeightmap, ChunkInvalidationPlan, ChunkLight,
            ChunkNeedsFluidStep, ChunkNeedsLightRebuild, ChunkNeedsSave, ChunkPos, ChunkPosition,
            WATER_SCENARIOS, WaterScenario, WorldBlockPos, char_from_cell, layout_scene,
            scene_size,
        },
        dimension::{Active, Dimension, apply_chunk_invalidations},
    },
};

pub(crate) struct GameTestPlugin;

/// Where the plot grid starts. Far from any sane play area, and high enough to
/// float above average terrain.
const TEST_FIELD_ORIGIN: IVec3 = IVec3::new(4096, 64, 4096);
/// Distance between plot origins; comfortably wider than any scene.
const PLOT_PITCH: i32 = 64;
const PLOTS_PER_ROW: usize = 4;
/// Fluid steps happen every fifth fixed tick in the running game.
const TICKS_PER_FLUID_STEP: u64 = 5;
/// Grace period before a quiet plot is trusted to have settled, guaranteeing
/// at least one fluid step happened since the scene was built.
const SETTLE_GRACE_TICKS: u64 = TICKS_PER_FLUID_STEP;
/// Extra ticks over the scenario budget before a run counts as timed out.
const TIMEOUT_SLACK_TICKS: u64 = 20;
/// Player must be at least this far from the test field before plots are
/// rebuilt; the streamer owns chunks around the player, and plot chunks cannot
/// be registered inside a streamed column.
const FIELD_CLEAR_RADIUS: f32 = 256.0;
/// Frames to wait after moving the player clear, so the streamer can evict the
/// field's columns before registration.
const CLEAR_FIELD_DEFER_FRAMES: u32 = 3;
/// Cap on how many mismatching rows are reported per failed scenario.
const MAX_REPORTED_ROWS: usize = 8;

pub(crate) struct WaterTestEntry {
    pub(crate) scenario: &'static WaterScenario,
    pub(crate) status: TestStatus,
    pub(crate) detail: String,
    start_tick: u64,
    plot: Option<TestPlot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TestStatus {
    Pending,
    Running,
    Passed,
    Failed,
    TimedOut,
}

impl TestStatus {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Running => "RUNNING",
            Self::Passed => "PASS",
            Self::Failed => "FAIL",
            Self::TimedOut => "TIMEOUT",
        }
    }
}

struct TestPlot {
    scene_origin: IVec3,
    size: IVec3,
    chunks: Vec<(ChunkPos, Entity)>,
}

#[derive(Resource)]
pub(crate) struct WaterTestSuite {
    pub(crate) entries: Vec<WaterTestEntry>,
    pub(crate) overlay_visible: bool,
    fixed_clock: u64,
    request_run: bool,
    request_teleport: bool,
    clear_field_defer: u32,
    /// The entry the player is currently parked above, shown in the overlay.
    viewing: Option<usize>,
}

impl WaterTestSuite {
    pub(crate) fn new() -> Self {
        Self {
            entries: WATER_SCENARIOS
                .iter()
                .map(|scenario| WaterTestEntry {
                    scenario,
                    status: TestStatus::Pending,
                    detail: String::new(),
                    start_tick: 0,
                    plot: None,
                })
                .collect(),
            overlay_visible: false,
            fixed_clock: 0,
            request_run: false,
            request_teleport: false,
            clear_field_defer: 0,
            viewing: None,
        }
    }

    pub(crate) fn passed(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.status == TestStatus::Passed)
            .count()
    }

    pub(crate) fn failed(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| matches!(entry.status, TestStatus::Failed | TestStatus::TimedOut))
            .count()
    }

    /// The entry the player is parked above, shown with a marker by the
    /// overlay.
    pub(crate) fn viewing(&self) -> Option<usize> {
        self.viewing
    }
}

impl Default for WaterTestSuite {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for GameTestPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WaterTestSuite::new())
            .add_systems(FixedUpdate, tick_suite_clock)
            .add_systems(
                Update,
                (
                    handle_suite_hotkeys,
                    start_requested_suite,
                    handle_teleport_request,
                    strip_plot_save_markers,
                    advance_running_tests,
                )
                    .chain(),
            );
    }
}

fn tick_suite_clock(mut suite: ResMut<WaterTestSuite>) {
    suite.fixed_clock = suite.fixed_clock.wrapping_add(1);
}

fn handle_suite_hotkeys(keys: Res<ButtonInput<KeyCode>>, mut suite: ResMut<WaterTestSuite>) {
    if keys.just_pressed(KeyCode::F7) {
        suite.overlay_visible = !suite.overlay_visible;
    }
    if keys.just_pressed(KeyCode::F8) {
        suite.request_run = true;
    }
    if keys.just_pressed(KeyCode::F9) {
        suite.request_teleport = true;
    }
}

/// Builds or rebuilds every scenario when a run was requested, then flies the
/// player out to the field so there is something to see.
fn start_requested_suite(
    mut commands: Commands,
    mut suite: ResMut<WaterTestSuite>,
    dimension: Option<Single<&mut Dimension, With<Active>>>,
    mut player_q: Query<
        (
            Entity,
            &PlayerDimension,
            &mut Transform,
            &mut Position,
            &mut Velocity,
        ),
        With<Player>,
    >,
    mut camera_q: Query<&mut Transform, (With<MouseCam>, Without<Player>)>,
) {
    if !suite.request_run {
        return;
    }

    // Keep the request pending while no dimension is active (main menu).
    let Some(dimension) = dimension else {
        return;
    };
    let mut dimension = dimension.into_inner();

    // Plot chunks cannot be registered while the streamer owns their columns.
    // Running the suite usually means the dev is standing in the test field,
    // so move them clear and wait for the streamer to release the field.
    if player_is_in_test_field(&player_q, dimension.id()) {
        if let Some((_, _, mut transform, mut position, mut velocity)) =
            player_q.iter_mut().next()
        {
            let spawn = dimension.arrival();
            **velocity = Vec3::ZERO;
            position.0 = spawn;
            *transform = Transform::from_translation(spawn);
        }
        suite.clear_field_defer = CLEAR_FIELD_DEFER_FRAMES;
        return;
    }
    if suite.clear_field_defer > 0 {
        suite.clear_field_defer -= 1;
        return;
    }

    suite.request_run = false;
    suite.overlay_visible = true;
    suite.viewing = None;

    for entry in &mut suite.entries {
        if let Some(plot) = entry.plot.take() {
            teardown_plot(&mut commands, &mut dimension, plot);
        }
        entry.status = TestStatus::Pending;
        entry.detail.clear();
    }

    let start_tick = suite.fixed_clock;
    for (index, entry) in suite.entries.iter_mut().enumerate() {
        match build_plot(&mut commands, &mut dimension, index, entry.scenario) {
            Ok(plot) => {
                entry.plot = Some(plot);
                entry.status = TestStatus::Running;
                entry.start_tick = start_tick;
            }
            Err(error) => {
                entry.status = TestStatus::Failed;
                entry.detail = error;
            }
        }
    }

    // Land on the first plot, but leave the cycle cursor unset so the first F9
    // press can jump straight to a failure if the run produced one.
    if let Some(plot) = suite.entries.first().and_then(|entry| entry.plot.as_ref()) {
        fly_player_to_plot(&mut commands, &dimension, &mut player_q, &mut camera_q, plot);
    }
}

fn player_is_in_test_field(
    player_q: &Query<
        (
            Entity,
            &PlayerDimension,
            &mut Transform,
            &mut Position,
            &mut Velocity,
        ),
        With<Player>,
    >,
    dimension_id: crate::world::DimensionId,
) -> bool {
    let field_center = TEST_FIELD_ORIGIN.as_vec3() + Vec3::new(96.0, 8.0, 64.0);
    player_q.iter().any(|(_, player_dimension, transform, _, _)| {
        player_dimension.id() == dimension_id
            && transform.translation.distance(field_center) <= FIELD_CLEAR_RADIUS
    })
}

fn teardown_plot(commands: &mut Commands, dimension: &mut Dimension, plot: TestPlot) {
    for (position, entity) in plot.chunks {
        dimension.unregister_published_chunk(position);
        commands.entity(entity).despawn();
    }
}

fn plot_origin(index: usize) -> IVec3 {
    TEST_FIELD_ORIGIN
        + IVec3::new(
            PLOT_PITCH * (index % PLOTS_PER_ROW) as i32,
            0,
            PLOT_PITCH * (index / PLOTS_PER_ROW) as i32,
        )
}

fn build_plot(
    commands: &mut Commands,
    dimension: &mut Dimension,
    index: usize,
    scenario: &WaterScenario,
) -> Result<TestPlot, String> {
    let scene_origin = plot_origin(index) + scenario.origin;
    let layout = layout_scene(scene_origin, scenario.input);

    let mut chunks: HashMap<ChunkPos, Chunk> = HashMap::new();
    for &(position, cell) in &layout.cells {
        let address = WorldBlockPos::from_ivec3(position).split();
        chunks
            .entry(address.chunk())
            .or_default()
            .set_cell(address.local().as_uvec3(), cell);
    }

    let mut ordered = chunks.into_iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(position, _)| (position.x(), position.y(), position.z()));

    for (position, _) in &ordered {
        if dimension.contains_loaded_chunk(*position) {
            return Err(format!(
                "test field collision: `{}` needs chunk {position:?} but it is already loaded",
                scenario.name
            ));
        }
    }

    let mut plot = TestPlot {
        scene_origin: layout.origin,
        size: layout.size,
        chunks: Vec::with_capacity(ordered.len()),
    };
    let mut plan = ChunkInvalidationPlan::new();
    for (position, chunk) in ordered {
        let counts = chunk.compute_content_counts();
        let entity = commands
            .spawn((
                ChunkPosition::from(position),
                chunk,
                counts,
                ChunkLight::default(),
                ChunkHeightmap::default(),
                Transform::from_translation(position.origin_translation()),
                Visibility::Inherited,
            ))
            .id();
        dimension.register_published_chunk(position, entity);
        plan.record_chunk_published(position, counts);
        commands.entity(entity).insert(ChunkNeedsLightRebuild);
        plot.chunks.push((position, entity));
    }
    apply_chunk_invalidations(commands, dimension, &plan);

    Ok(plot)
}

/// Compares each running plot against its expected state once every plot chunk
/// has gone quiet.
fn advance_running_tests(
    mut suite: ResMut<WaterTestSuite>,
    chunk_q: Query<(&ChunkPosition, &Chunk)>,
    pending_fluid: Query<(), With<ChunkNeedsFluidStep>>,
) {
    let fixed_clock = suite.fixed_clock;
    for entry in &mut suite.entries {
        if entry.status != TestStatus::Running {
            continue;
        }
        let elapsed = fixed_clock.saturating_sub(entry.start_tick);

        let decision = {
            let Some(plot) = &entry.plot else {
                continue;
            };
            let busy = plot
                .chunks
                .iter()
                .any(|&(_, entity)| pending_fluid.get(entity).is_ok());

            if !busy && elapsed >= SETTLE_GRACE_TICKS {
                match compare_plot(plot, entry.scenario, &chunk_q) {
                    Ok(()) => TestDecision::Passed,
                    Err(problem) => TestDecision::Failed(problem),
                }
            } else if elapsed > timeout_ticks(entry.scenario) {
                TestDecision::TimedOut(describe_plot(plot, &chunk_q))
            } else {
                TestDecision::Waiting
            }
        };

        match decision {
            TestDecision::Waiting => {}
            TestDecision::Passed => {
                entry.status = TestStatus::Passed;
                entry.detail.clear();
            }
            TestDecision::Failed(problem) => {
                entry.status = TestStatus::Failed;
                entry.detail = problem;
            }
            TestDecision::TimedOut(detail) => {
                entry.status = TestStatus::TimedOut;
                entry.detail = detail;
            }
        }
    }
}

enum TestDecision {
    Waiting,
    Passed,
    Failed(String),
    TimedOut(String),
}

fn timeout_ticks(scenario: &WaterScenario) -> u64 {
    u64::from(scenario.max_steps) * TICKS_PER_FLUID_STEP + TIMEOUT_SLACK_TICKS
}

fn compare_plot(
    plot: &TestPlot,
    scenario: &WaterScenario,
    chunk_q: &Query<(&ChunkPosition, &Chunk)>,
) -> Result<(), String> {
    let size = plot.size;
    debug_assert_eq!(
        size,
        scene_size(scenario.expected),
        "plot and expected scene shapes must match"
    );
    let chunks = plot_chunks(plot, chunk_q);

    let mut problems = Vec::new();
    for (y, layer) in scenario.expected.iter().enumerate() {
        for (z, row) in layer.iter().enumerate() {
            let actual = actual_row(plot, &chunks, size.x, y, z);
            if actual != *row {
                problems.push(format!(
                    "y={y} z={z} (world {:?}..):\n  actual  : {actual}\n  expected: {row}",
                    plot.scene_origin + IVec3::new(0, y as i32, z as i32)
                ));
                if problems.len() >= MAX_REPORTED_ROWS {
                    break;
                }
            }
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("{}\n{}", scenario.description, problems.join("\n")))
    }
}

/// Renders the current plot state for timeout reports.
fn describe_plot(plot: &TestPlot, chunk_q: &Query<(&ChunkPosition, &Chunk)>) -> String {
    let size = plot.size;
    let chunks = plot_chunks(plot, chunk_q);

    let mut rows = Vec::new();
    'layers: for y in 0..size.y as usize {
        for z in 0..size.z as usize {
            rows.push(format!("y={y} z={z}: {}", actual_row(plot, &chunks, size.x, y, z)));
            if rows.len() >= MAX_REPORTED_ROWS {
                break 'layers;
            }
        }
    }

    format!("timed out; current state:\n{}", rows.join("\n"))
}

fn plot_chunks<'a>(
    plot: &TestPlot,
    chunk_q: &'a Query<(&ChunkPosition, &Chunk)>,
) -> HashMap<ChunkPos, &'a Chunk> {
    let mut chunks = HashMap::new();
    for &(position, entity) in &plot.chunks {
        if let Ok((actual_position, chunk)) = chunk_q.get(entity)
            && actual_position.chunk_pos() == position
        {
            chunks.insert(position, chunk);
        }
    }
    chunks
}

fn actual_row(
    plot: &TestPlot,
    chunks: &HashMap<ChunkPos, &Chunk>,
    width: i32,
    y: usize,
    z: usize,
) -> String {
    (0..width as usize)
        .map(|x| {
            let world =
                plot.scene_origin + IVec3::new(x as i32, y as i32, z as i32);
            let address = WorldBlockPos::from_ivec3(world).split();
            let cell = chunks
                .get(&address.chunk())
                .map_or(ChunkCell::EMPTY, |chunk| {
                    chunk.get_cell(address.local().as_uvec3())
                });
            char_from_cell(cell)
        })
        .collect()
}

/// Cycles the player through the test plots. The first press with nothing
/// viewed yet starts at the first failure if there is one, otherwise the first
/// scenario; later presses move to the next built plot and wrap around. With
/// nothing built at all, asking to teleport starts a run, which also flies the
/// player out to the field.
fn handle_teleport_request(
    mut commands: Commands,
    mut suite: ResMut<WaterTestSuite>,
    dimension: Option<Single<&Dimension, With<Active>>>,
    mut player_q: Query<
        (
            Entity,
            &PlayerDimension,
            &mut Transform,
            &mut Position,
            &mut Velocity,
        ),
        With<Player>,
    >,
    mut camera_q: Query<&mut Transform, (With<MouseCam>, Without<Player>)>,
) {
    if !std::mem::take(&mut suite.request_teleport) {
        return;
    }
    let Some(dimension) = dimension else {
        return;
    };

    let Some(index) = next_plotted_entry(&suite.entries, suite.viewing) else {
        suite.request_run = true;
        return;
    };
    let Some(plot) = suite.entries[index].plot.as_ref() else {
        return;
    };

    if fly_player_to_plot(
        &mut commands,
        &dimension,
        &mut player_q,
        &mut camera_q,
        plot,
    ) {
        suite.viewing = Some(index);
    }
}

/// The next entry with a built plot: the first failure when the cycle starts
/// fresh, then the following entries in order, wrapping and skipping unbuilt
/// ones.
fn next_plotted_entry(entries: &[WaterTestEntry], viewing: Option<usize>) -> Option<usize> {
    if entries.is_empty() {
        return None;
    }

    let start = match viewing {
        Some(viewed) => (viewed + 1) % entries.len(),
        None => entries
            .iter()
            .position(|entry| matches!(entry.status, TestStatus::Failed | TestStatus::TimedOut))
            .unwrap_or(0),
    };

    (0..entries.len())
        .map(|offset| (start + offset) % entries.len())
        .find(|&index| entries[index].plot.is_some())
}

/// Places the player above a plot, looking down into it, and turns on flight
/// so the dev can hover and move between plots. Returns whether the player was
/// moved.
fn fly_player_to_plot(
    commands: &mut Commands,
    dimension: &Dimension,
    player_q: &mut Query<
        (
            Entity,
            &PlayerDimension,
            &mut Transform,
            &mut Position,
            &mut Velocity,
        ),
        With<Player>,
    >,
    camera_q: &mut Query<&mut Transform, (With<MouseCam>, Without<Player>)>,
    plot: &TestPlot,
) -> bool {
    let Some((entity, player_dimension, mut transform, mut position, mut velocity)) =
        player_q.iter_mut().next()
    else {
        return false;
    };
    if player_dimension.id() != dimension.id() {
        return false;
    }

    let destination =
        plot.scene_origin + IVec3::new(plot.size.x / 2, plot.size.y + 4, plot.size.z / 2);
    let destination = destination.as_vec3();
    **velocity = Vec3::ZERO;
    position.0 = destination;
    *transform = Transform::from_translation(destination);
    if let Some(mut camera) = camera_q.iter_mut().next() {
        // Tilted rather than straight down so WASD still moves sensibly.
        camera.rotation = Transform::default()
            .looking_to(Vec3::new(0.0, -1.0, -1.0), Vec3::Y)
            .rotation;
    }
    commands.entity(entity).insert(Flying);
    true
}

/// Test chunks must never be captured by the save system; the fluid solver
/// marks every change as save-worthy, so scrub the marker every frame before
/// the save capture runs in `PostUpdate`.
fn strip_plot_save_markers(
    mut commands: Commands,
    suite: Res<WaterTestSuite>,
    dirty: Query<(), With<ChunkNeedsSave>>,
) {
    for entry in &suite.entries {
        let Some(plot) = &entry.plot else {
            continue;
        };
        for &(_, entity) in &plot.chunks {
            if dirty.get(entity).is_ok() {
                commands.entity(entity).remove::<ChunkNeedsSave>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::dimension::fluid::{FluidStepBudget, FluidTickCounter, step_chunk_fluids, tick_counter};

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(FluidStepBudget(16))
            .insert_resource(FluidTickCounter(0))
            .add_plugins(GameTestPlugin)
            .add_systems(Update, (tick_counter, step_chunk_fluids).chain());
        app.world_mut().spawn((Dimension::default(), Active));
        app
    }

    #[test]
    fn suite_settles_every_scenario_in_the_live_pipeline() {
        let mut app = test_app();
        app.world_mut().resource_mut::<WaterTestSuite>().request_run = true;
        app.update();

        let built = app
            .world()
            .resource::<WaterTestSuite>()
            .entries
            .iter()
            .filter(|entry| entry.plot.is_some())
            .count();
        assert_eq!(
            built,
            WATER_SCENARIOS.len(),
            "every registered scenario should build a plot"
        );

        for _ in 0..2_000 {
            app.world_mut().run_schedule(FixedUpdate);
            app.update();
            let done = app
                .world()
                .resource::<WaterTestSuite>()
                .entries
                .iter()
                .all(|entry| {
                    matches!(
                        entry.status,
                        TestStatus::Passed | TestStatus::Failed | TestStatus::TimedOut
                    )
                });
            if done {
                break;
            }
        }

        let suite = app.world().resource::<WaterTestSuite>();
        for entry in &suite.entries {
            assert_eq!(
                entry.status,
                TestStatus::Passed,
                "scenario `{}` did not pass in the live pipeline: {}",
                entry.scenario.name,
                entry.detail
            );
        }
    }

    fn entry_with_plot(scenario_index: usize, status: TestStatus, plotted: bool) -> WaterTestEntry {
        WaterTestEntry {
            scenario: &WATER_SCENARIOS[scenario_index],
            status,
            detail: String::new(),
            start_tick: 0,
            plot: plotted.then(|| TestPlot {
                scene_origin: IVec3::ZERO,
                size: IVec3::ONE,
                chunks: Vec::new(),
            }),
        }
    }

    #[test]
    fn teleport_cycle_starts_at_the_first_failure_then_advances() {
        let entries = vec![
            entry_with_plot(0, TestStatus::Passed, true),
            entry_with_plot(1, TestStatus::Failed, true),
            entry_with_plot(2, TestStatus::Passed, true),
        ];

        assert_eq!(next_plotted_entry(&entries, None), Some(1));
        assert_eq!(next_plotted_entry(&entries, Some(1)), Some(2));
        assert_eq!(
            next_plotted_entry(&entries, Some(2)),
            Some(0),
            "the cycle wraps around"
        );
    }

    #[test]
    fn teleport_cycle_skips_entries_without_plots() {
        let entries = vec![
            entry_with_plot(0, TestStatus::Passed, false),
            entry_with_plot(1, TestStatus::Passed, true),
            entry_with_plot(2, TestStatus::Passed, false),
        ];

        assert_eq!(next_plotted_entry(&entries, None), Some(1));
        assert_eq!(
            next_plotted_entry(&entries, Some(1)),
            Some(1),
            "a lone built plot must stay reachable"
        );
    }

    #[test]
    fn teleport_cycle_is_empty_without_any_plot() {
        let entries = vec![
            entry_with_plot(0, TestStatus::Pending, false),
            entry_with_plot(1, TestStatus::Pending, false),
        ];
        assert_eq!(next_plotted_entry(&entries, None), None);
    }

    #[test]
    fn teleport_with_nothing_built_requests_a_run() {
        let mut app = test_app();
        app.world_mut().resource_mut::<WaterTestSuite>().request_teleport = true;

        // The teleport handler runs after the start system in the chain, so the
        // run it requests is picked up on the following update.
        app.update();
        app.update();

        assert!(
            app.world()
                .resource::<WaterTestSuite>()
                .entries
                .iter()
                .any(|entry| entry.plot.is_some()),
            "F9 with nothing built should start a run"
        );
    }
}

