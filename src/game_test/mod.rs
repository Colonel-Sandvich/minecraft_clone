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
    mob::controller::Velocity,
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

/// Builds or rebuilds every scenario when a run was requested.
fn start_requested_suite(
    mut commands: Commands,
    mut suite: ResMut<WaterTestSuite>,
    dimension: Option<Single<&mut Dimension, With<Active>>>,
) {
    if !suite.request_run {
        return;
    }

    // Keep the request pending while no dimension is active (main menu).
    let Some(dimension) = dimension else {
        return;
    };
    suite.request_run = false;
    let mut dimension = dimension.into_inner();

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

/// Teleports the player above the first failed plot, or the current run.
fn handle_teleport_request(
    mut suite: ResMut<WaterTestSuite>,
    dimension: Option<Single<&Dimension, With<Active>>>,
    mut player_q: Query<(&PlayerDimension, &mut Transform, &mut Position, &mut Velocity), With<Player>>,
    mut camera_q: Query<&mut Transform, (With<MouseCam>, Without<Player>)>,
) {
    if !std::mem::take(&mut suite.request_teleport) {
        return;
    }
    let Some(dimension) = dimension else {
        return;
    };

    let target_entry = suite
        .entries
        .iter()
        .find(|entry| {
            entry.plot.is_some()
                && matches!(entry.status, TestStatus::Failed | TestStatus::TimedOut)
        })
        .or_else(|| {
            suite
                .entries
                .iter()
                .find(|entry| entry.plot.is_some() && entry.status == TestStatus::Running)
        });
    let Some(entry) = target_entry else {
        return;
    };
    let Some(plot) = &entry.plot else {
        return;
    };

    let Some((player_dimension, mut transform, mut position, mut velocity)) =
        player_q.iter_mut().next()
    else {
        return;
    };
    if player_dimension.id() != dimension.id() {
        return;
    }

    let size = plot.size;
    let destination = plot.scene_origin + IVec3::new(size.x / 2, size.y + 2, size.z / 2);
    let destination = destination.as_vec3();
    **velocity = Vec3::ZERO;
    position.0 = destination;
    *transform = Transform::from_translation(destination);
    if let Some(mut camera) = camera_q.iter_mut().next() {
        camera.rotation = Transform::default().looking_to(Vec3::NEG_Y, Vec3::Z).rotation;
    }
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
}

