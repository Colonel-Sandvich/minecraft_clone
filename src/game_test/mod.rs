//! In-game water fidelity tests, modelled on Minecraft's GameTest runner.
//!
//! Press `F8` to build every registered water scenario in a remote test field,
//! let the real fluid simulation settle each scene, and compare the result cell
//! by cell against the expected ASCII maps. `F7` toggles the results overlay and
//! `F9` teleports the player to the first failure so the scene can be inspected
//! by eye. Scenes are sealed glass shells with an open top, so failures can be
//! read from above or through the walls without digging in.
//!
//! Test chunks are registered as independent published chunks (the streaming
//! system ignores columns that already contain a loaded chunk). They are never
//! allowed to keep `ChunkNeedsSave`, so running tests cannot pollute the
//! player's world save.

use std::collections::HashMap;
use std::f32::consts::FRAC_1_SQRT_2;

use avian3d::prelude::Position;
use bevy::prelude::*;

use crate::{
    item::Item,
    mob::controller::{Flying, Velocity},
    player::{Player, PlayerDimension, cam::MouseCam},
    tick_control::{TickControlSet, simulation_ticks_allowed},
    world::{
        chunk::{
            CHUNK_SIZE, Chunk, ChunkCell, ChunkContentCounts, ChunkEditor, ChunkHeightmap,
            ChunkInvalidationPlan, ChunkLight, ChunkNeedsFluidStep, ChunkNeedsLightRebuild,
            ChunkNeedsSave, ChunkPos, ChunkPosition, SceneLayers, WATER_SCENARIOS, WaterScenario,
            WorldBlockPos, char_from_cell, for_each_edit_cell, layout_scene, phase_expected,
            phase_max_steps, scene_size,
        },
        dimension::{Active, Dimension, apply_chunk_invalidations, fluid::FLUID_STEP_PERIOD_TICKS},
    },
};

pub(crate) struct GameTestPlugin;

/// Where the plot field starts. Far from any sane play area, and high enough to
/// float above average terrain.
const TEST_FIELD_ORIGIN: IVec3 = IVec3::new(4096, 64, 4096);
const PLOTS_PER_ROW: usize = 4;
/// Plot origins sit one block past a chunk boundary so a scene's shell (one
/// cell outside the scene) starts exactly on that boundary.
const PLOT_ORIGIN_OFFSET: i32 = 1;
/// Grace period before a quiet plot is trusted to have settled, guaranteeing
/// at least one fluid step happened since the scene was built.
const SETTLE_GRACE_TICKS: u64 = FLUID_STEP_PERIOD_TICKS as u64;
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
/// Fixed direction the inspection camera looks: 45° down and towards -Z, so
/// every plot is read from the same angle.
const INSPECTION_VIEW_DIR: Vec3 = Vec3::new(0.0, -FRAC_1_SQRT_2, -FRAC_1_SQRT_2);
/// Distance from the scene centre along the view ray when teleporting to a
/// plot; wider scenes get more room so they still fit the frame.
const INSPECTION_BASE_DISTANCE: f32 = 10.0;
/// Where `F11` writes the current plot field as ASCII maps, so a plot the
/// developer hand-edited in the world can be handed back for transcription.
const PLOT_DUMP_PATH: &str = "saves/dev/test_plots.txt";

pub(crate) struct WaterTestEntry {
    pub(crate) scenario: &'static WaterScenario,
    pub(crate) status: TestStatus,
    pub(crate) detail: String,
    /// Settle-and-compare phases already completed; 0 is the opening scene.
    pub(crate) phase: usize,
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
    request_dump: bool,
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
                    phase: 0,
                    start_tick: 0,
                    plot: None,
                })
                .collect(),
            overlay_visible: false,
            fixed_clock: 0,
            request_run: false,
            request_teleport: false,
            request_dump: false,
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
            .add_systems(
                FixedUpdate,
                tick_suite_clock
                    .in_set(TickControlSet::Simulation)
                    .run_if(simulation_ticks_allowed),
            )
            .add_systems(
                Update,
                (
                    handle_suite_hotkeys,
                    start_requested_suite,
                    handle_teleport_request,
                    strip_plot_save_markers,
                    advance_running_tests,
                    dump_test_plots,
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
    if keys.just_pressed(KeyCode::F11) {
        suite.request_dump = true;
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
        if let Some((_, _, mut transform, mut position, mut velocity)) = player_q.iter_mut().next()
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
        entry.phase = 0;
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
        fly_player_to_plot(
            &mut commands,
            &dimension,
            &mut player_q,
            &mut camera_q,
            plot,
        );
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
    let field_center = field_center();
    player_q
        .iter()
        .any(|(_, player_dimension, transform, _, _)| {
            player_dimension.id() == dimension_id
                && transform.translation.distance(field_center) <= FIELD_CLEAR_RADIUS
        })
}

/// Centre of the packed plot field, used to decide whether the player is close
/// enough to the field to interfere with plot chunk registration.
fn field_center() -> Vec3 {
    let mut min = IVec3::MAX;
    let mut max = IVec3::MIN;
    for (origin, scenario) in plot_origins().into_iter().zip(WATER_SCENARIOS) {
        let size = scene_size(scenario.input);
        min = min.min(origin - IVec3::ONE);
        max = max.max(origin + size);
    }
    (min.as_vec3() + max.as_vec3()) * 0.5
}

fn teardown_plot(commands: &mut Commands, dimension: &mut Dimension, plot: TestPlot) {
    for (position, entity) in plot.chunks {
        dimension.unregister_published_chunk(position);
        commands.entity(entity).despawn();
    }
}

/// Steps from one plot origin to the next along an axis: the scene's cells
/// plus its shell span `extent + 2` blocks from the (chunk-aligned) shell
/// edge, where `extent` is the scene's interior size, so round that up to
/// whole chunks.
///
/// Chunk positions are addressed in 3D and every plot occupies the same y
/// layers, so two plots packed into one chunk column would collide when the
/// runner registers them.
fn plot_advance(extent: i32) -> i32 {
    let chunk = CHUNK_SIZE as i32;
    let columns = (extent + 1).div_euclid(chunk) + 1;
    chunk * columns
}

/// Row-major plot origins, packed against the scenes' real extents so the
/// fixtures sit a few blocks apart instead of spread over a wide field.
fn plot_origins() -> Vec<IVec3> {
    let mut origins = Vec::with_capacity(WATER_SCENARIOS.len());
    let mut cursor = TEST_FIELD_ORIGIN + IVec3::new(PLOT_ORIGIN_OFFSET, 0, PLOT_ORIGIN_OFFSET);
    let mut row_depth = 0;
    for (index, scenario) in WATER_SCENARIOS.iter().enumerate() {
        if index > 0 && index.is_multiple_of(PLOTS_PER_ROW) {
            cursor.x = TEST_FIELD_ORIGIN.x + PLOT_ORIGIN_OFFSET;
            cursor.z += plot_advance(row_depth);
            row_depth = 0;
        }
        let size = scene_size(scenario.input);
        origins.push(cursor);
        cursor.x += plot_advance(size.x);
        row_depth = row_depth.max(size.z);
    }
    origins
}

fn plot_origin(index: usize) -> IVec3 {
    plot_origins()[index]
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

/// Read-only view of a plot chunk for comparisons.
type ChunkRead = (&'static ChunkPosition, &'static Chunk);
/// Mutable view of a plot chunk for applying phase edits.
type ChunkEdit = (
    &'static ChunkPosition,
    &'static mut Chunk,
    &'static mut ChunkContentCounts,
);

/// Compares each running plot against the expected state for its current
/// phase once every plot chunk has gone quiet. A phase that passes with a
/// follow-up edit applies the edit and keeps running, so scenarios can cover
/// "steady state -> block edit -> settle again".
fn advance_running_tests(
    mut commands: Commands,
    mut suite: ResMut<WaterTestSuite>,
    mut dimension: Option<Single<&mut Dimension, With<Active>>>,
    mut chunks: ParamSet<(Query<ChunkRead>, Query<ChunkEdit>)>,
    pending_fluid: Query<(), With<ChunkNeedsFluidStep>>,
) {
    let fixed_clock = suite.fixed_clock;
    for entry in &mut suite.entries {
        if entry.status != TestStatus::Running {
            continue;
        }
        let elapsed = fixed_clock.saturating_sub(entry.start_tick);
        let phase = entry.phase;
        let expected = phase_expected(entry.scenario, phase);

        let decision = {
            let Some(plot) = &entry.plot else {
                continue;
            };
            let busy = plot
                .chunks
                .iter()
                .any(|&(_, entity)| pending_fluid.get(entity).is_ok());

            if !busy && elapsed >= SETTLE_GRACE_TICKS {
                match compare_plot(plot, expected, entry.scenario.description, &chunks.p0()) {
                    Ok(()) => TestDecision::PhaseSettled,
                    Err(problem) => TestDecision::Failed(problem),
                }
            } else if elapsed > timeout_ticks(phase_max_steps(entry.scenario, phase)) {
                TestDecision::TimedOut(describe_plot(plot, &chunks.p0()))
            } else {
                TestDecision::Waiting
            }
        };

        match decision {
            TestDecision::Waiting => {}
            TestDecision::PhaseSettled => {
                entry.detail.clear();
                let next_edit = entry.scenario.phases.get(phase).map(|phase| phase.edit);
                let Some(edit) = next_edit else {
                    entry.status = TestStatus::Passed;
                    continue;
                };
                // Without an active dimension (dimension switch) the edit is
                // retried next frame; the plot stays quiet until it lands.
                if let (Some(plot), Some(dimension)) =
                    (entry.plot.as_ref(), dimension.as_deref_mut())
                {
                    apply_plot_edit(&mut commands, dimension, plot, edit, &mut chunks.p1());
                    entry.phase = phase + 1;
                    entry.start_tick = fixed_clock;
                }
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

/// Applies a phase edit to a live plot through the same chunk editor and
/// invalidation planning a player's block edit uses, so the fluid pipeline is
/// woken (and the settle detection re-armed) exactly like in normal play.
fn apply_plot_edit(
    commands: &mut Commands,
    dimension: &mut Dimension,
    plot: &TestPlot,
    edit: &SceneLayers,
    chunks: &mut Query<ChunkEdit>,
) {
    let mut invalidations = ChunkInvalidationPlan::new();
    for_each_edit_cell(plot.scene_origin, plot.size, edit, |position, cell| {
        let address = WorldBlockPos::from_ivec3(position).split();
        let chunk_pos = address.chunk();
        let Some(&(_, entity)) = plot.chunks.iter().find(|(pos, _)| *pos == chunk_pos) else {
            return;
        };
        let Ok((_, mut chunk, mut counts)) = chunks.get_mut(entity) else {
            return;
        };
        let mut editor = ChunkEditor::new(chunk_pos, &mut chunk, &mut counts, &mut invalidations);
        editor.set_cell(address.local(), cell);
    });
    apply_chunk_invalidations(commands, dimension, &invalidations);
}

enum TestDecision {
    Waiting,
    PhaseSettled,
    Failed(String),
    TimedOut(String),
}

fn timeout_ticks(max_steps: u32) -> u64 {
    u64::from(max_steps) * u64::from(FLUID_STEP_PERIOD_TICKS) + TIMEOUT_SLACK_TICKS
}

fn compare_plot(
    plot: &TestPlot,
    expected: &SceneLayers,
    description: &str,
    chunk_q: &Query<(&ChunkPosition, &Chunk)>,
) -> Result<(), String> {
    let size = plot.size;
    debug_assert_eq!(
        size,
        scene_size(expected),
        "plot and expected scene shapes must match"
    );
    let chunks = plot_chunks(plot, chunk_q);

    let mut problems = Vec::new();
    for (y, layer) in expected.iter().enumerate() {
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
        Err(format!("{description}\n{}", problems.join("\n")))
    }
}

/// Renders the current plot state for timeout reports.
fn describe_plot(plot: &TestPlot, chunk_q: &Query<(&ChunkPosition, &Chunk)>) -> String {
    let size = plot.size;
    let chunks = plot_chunks(plot, chunk_q);

    let mut rows = Vec::new();
    'layers: for y in 0..size.y as usize {
        for z in 0..size.z as usize {
            rows.push(format!(
                "y={y} z={z}: {}",
                actual_row(plot, &chunks, size.x, y, z)
            ));
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
            let world = plot.scene_origin + IVec3::new(x as i32, y as i32, z as i32);
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

/// Where the player hovers to inspect `plot`, and which way the camera looks.
///
/// The camera keeps one fixed angle and the position is pulled back along the
/// view ray from the scene centre, so the scene sits in the middle of the
/// frame instead of below it.
fn inspection_viewpoint(plot: &TestPlot) -> (Vec3, Vec3) {
    let anchor = plot.scene_origin.as_vec3() + plot.size.as_vec3() * 0.5;
    let distance = INSPECTION_BASE_DISTANCE + plot.size.x.max(plot.size.z) as f32 * 0.5;
    (anchor - INSPECTION_VIEW_DIR * distance, INSPECTION_VIEW_DIR)
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

    let (destination, view) = inspection_viewpoint(plot);
    **velocity = Vec3::ZERO;
    position.0 = destination;
    *transform = Transform::from_translation(destination);
    if let Some(mut camera) = camera_q.iter_mut().next() {
        // Tilted rather than straight down so WASD still moves sensibly.
        camera.rotation = Transform::default().looking_to(view, Vec3::Y).rotation;
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

/// `F11`: writes every built plot back out in the scenario map layout so a plot
/// the developer hand-edited (for example stone left where water should reach)
/// can be handed back for transcription. Unlike [`char_from_cell`] the dump has
/// a character for every block, so marker blocks never panic.
fn dump_test_plots(mut suite: ResMut<WaterTestSuite>, chunks: Query<&Chunk>) {
    if !suite.request_dump {
        return;
    }
    suite.request_dump = false;

    let mut out = String::from(
        "# water test plot dump\n\
         # legend: . air  # glass  S source  1-8 water level  g grass  d dirt  t stone\n\
         #         a sand  L oak log  v leaves  G glowstone  i ice  ? missing chunk\n\
         # one section per plot; y=0 is the bottom layer, rows are z, characters are x\n",
    );

    let mut dumped = 0;
    for entry in &suite.entries {
        let Some(plot) = &entry.plot else {
            continue;
        };
        let by_position = plot.chunks.iter().copied().collect::<HashMap<_, _>>();
        render_plot_dump(&mut out, entry, plot, |world| {
            let address = WorldBlockPos::from_ivec3(world).split();
            let Some(&entity) = by_position.get(&address.chunk()) else {
                return '?';
            };
            match chunks.get(entity) {
                Ok(chunk) => dump_cell_char(chunk.get_cell(address.local().as_uvec3())),
                Err(_) => '?',
            }
        });
        dumped += 1;
    }

    if dumped == 0 {
        info!("water test dump: no plots built yet; press F8 first");
        return;
    }

    let result =
        std::fs::create_dir_all("saves/dev").and_then(|()| std::fs::write(PLOT_DUMP_PATH, &out));
    match result {
        Ok(()) => info!("water test dump: wrote {dumped} plots to {PLOT_DUMP_PATH}"),
        Err(error) => warn!(%error, "water test dump: could not write {PLOT_DUMP_PATH}"),
    }
}

/// Renders one plot in the same layout as the scenario maps (`layers[y][z]`,
/// characters along `x`), reading the live cells through `cell_at`.
fn render_plot_dump(
    out: &mut String,
    entry: &WaterTestEntry,
    plot: &TestPlot,
    mut cell_at: impl FnMut(IVec3) -> char,
) {
    use std::fmt::Write as _;

    let _ = writeln!(
        out,
        "## {} [{}] phase {}  origin {:?}  size {}x{}x{}",
        entry.scenario.name,
        entry.status.label(),
        entry.phase,
        plot.scene_origin,
        plot.size.x,
        plot.size.y,
        plot.size.z,
    );
    for y in 0..plot.size.y {
        let _ = writeln!(out, "y={y}:");
        for z in 0..plot.size.z {
            let mut row = String::with_capacity(plot.size.x as usize);
            for x in 0..plot.size.x {
                row.push(cell_at(plot.scene_origin + IVec3::new(x, y, z)));
            }
            let _ = writeln!(out, "  z={z}: {row}");
        }
    }
}

/// Character for one dumped cell. Extends the scenario legend with the
/// remaining blocks so a developer can mark expectations with any block.
fn dump_cell_char(cell: ChunkCell) -> char {
    match cell {
        ChunkCell::Empty => '.',
        ChunkCell::Fluid(fluid) if fluid.is_source() => 'S',
        ChunkCell::Fluid(fluid) => char::from(b'0' + fluid.level().get()),
        ChunkCell::Block(Item::Glass) => '#',
        ChunkCell::Block(Item::OakLog) => 'L',
        ChunkCell::Block(Item::Grass) => 'g',
        ChunkCell::Block(Item::Dirt) => 'd',
        ChunkCell::Block(Item::Stone) => 't',
        ChunkCell::Block(Item::Sand) => 'a',
        ChunkCell::Block(Item::OakLeaves) => 'v',
        ChunkCell::Block(Item::Glowstone) => 'G',
        ChunkCell::Block(Item::Ice) => 'i',
        ChunkCell::Block(Item::Water) => '?',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::dimension::fluid::{
        FluidStepBudget, FluidTickCounter, step_chunk_fluids, tick_counter,
    };

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
            phase: 0,
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
        app.world_mut()
            .resource_mut::<WaterTestSuite>()
            .request_teleport = true;

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

    #[test]
    fn plot_layout_keeps_every_plot_on_its_own_chunks() {
        let mut seen = std::collections::HashSet::new();
        for (origin, scenario) in plot_origins().into_iter().zip(WATER_SCENARIOS) {
            let mut chunks = std::collections::HashSet::new();
            for &(position, _) in &layout_scene(origin, scenario.input).cells {
                chunks.insert(WorldBlockPos::from_ivec3(position).split().chunk());
            }
            for chunk in chunks {
                assert!(
                    seen.insert(chunk),
                    "scenario `{}` shares chunk {chunk:?} with an earlier plot",
                    scenario.name
                );
            }
        }
    }

    #[test]
    fn inspection_viewpoint_looks_at_the_scene_centre_from_one_fixed_angle() {
        let plot = TestPlot {
            scene_origin: IVec3::new(4097, 64, 4097),
            size: IVec3::new(24, 3, 9),
            chunks: Vec::new(),
        };
        let anchor = plot.scene_origin.as_vec3() + plot.size.as_vec3() * 0.5;
        let (destination, view) = inspection_viewpoint(&plot);

        let to_scene = (anchor - destination).normalize();
        assert!(
            to_scene.distance(view) < 1e-4,
            "the scene centre must sit on the view ray: {to_scene} vs {view}"
        );
        assert!(
            destination.z > anchor.z,
            "the player stands back from the scene"
        );
        assert!(
            destination.y > anchor.y,
            "the player looks down into the scene"
        );
    }

    #[test]
    fn f11_requests_a_plot_dump() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(WaterTestSuite::new())
            .add_systems(Update, handle_suite_hotkeys);

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F11);
        app.update();

        assert!(
            app.world().resource::<WaterTestSuite>().request_dump,
            "F11 must request a plot dump"
        );
    }

    #[test]
    fn plot_dump_renders_hand_edits_in_scenario_layout() {
        let entry = WaterTestEntry {
            scenario: &WATER_SCENARIOS[0],
            status: TestStatus::Failed,
            detail: String::new(),
            phase: 1,
            start_tick: 0,
            plot: None,
        };
        let origin = IVec3::new(4097, 64, 4097);
        let plot = TestPlot {
            scene_origin: origin,
            size: IVec3::new(3, 2, 1),
            chunks: Vec::new(),
        };

        let mut out = String::new();
        render_plot_dump(&mut out, &entry, &plot, |world| {
            let local = world - origin;
            match (local.x, local.y) {
                (0, 0) => 't',
                (1, 0) => '3',
                (0, 1) => '#',
                _ => '.',
            }
        });

        assert!(out.contains(WATER_SCENARIOS[0].name));
        assert!(out.contains("[FAIL]"), "status must be reported: {out}");
        assert!(out.contains("phase 1"), "phase must be reported: {out}");
        assert!(out.contains("y=0:"));
        assert!(
            out.contains("z=0: t3."),
            "hand-placed markers must survive the dump: {out}"
        );
        assert!(
            out.contains("z=0: #.."),
            "upper layers must be dumped too: {out}"
        );
    }

    #[test]
    fn dump_legend_covers_every_storable_cell() {
        assert_eq!(dump_cell_char(ChunkCell::EMPTY), '.');
        assert_eq!(dump_cell_char(ChunkCell::water_source()), 'S');
        assert_eq!(dump_cell_char(ChunkCell::water_flow(5)), '5');
        assert_eq!(dump_cell_char(Item::Stone.into()), 't');
        assert_eq!(dump_cell_char(Item::Glass.into()), '#');
        assert_eq!(dump_cell_char(Item::Grass.into()), 'g');
    }
}
