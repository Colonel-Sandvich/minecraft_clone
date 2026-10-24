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

#[derive(Resource, Default)]
struct ChangeLog(Vec<bool>);

fn record_suite_change(suite: Res<WaterTestSuite>, mut log: ResMut<ChangeLog>) {
    log.0.push(suite.is_changed());
}

#[test]
fn ticking_the_suite_clock_does_not_mark_the_suite_changed() {
    let mut app = App::new();
    app.init_resource::<WaterTestSuite>()
        .init_resource::<WaterTestClock>()
        .init_resource::<ChangeLog>()
        .add_systems(FixedUpdate, tick_suite_clock)
        .add_systems(Update, record_suite_change);

    app.world_mut().run_schedule(FixedUpdate);
    app.world_mut().run_schedule(Update);
    app.world_mut().run_schedule(FixedUpdate);
    app.world_mut().run_schedule(Update);

    assert_eq!(app.world().resource::<WaterTestClock>().0, 2);
    assert_eq!(
        app.world().resource::<ChangeLog>().0,
        vec![true, false],
        "only the initial suite insertion should be visible as a change"
    );
}

#[test]
fn idle_frames_do_not_mark_the_suite_changed() {
    let mut app = test_app();
    app.init_resource::<ChangeLog>()
        .add_systems(Update, record_suite_change);

    // Warm up the observer's last-run tick, then check both measured frames.
    app.update();
    app.world_mut().resource_mut::<ChangeLog>().0.clear();
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<ChangeLog>().0,
        vec![false, false],
        "idle frames with no requests or running tests should not rebuild the overlay"
    );
}

#[test]
fn waiting_running_tests_do_not_mark_the_suite_changed() {
    let mut app = test_app();
    app.init_resource::<ChangeLog>()
        .add_systems(Update, record_suite_change);
    app.world_mut().resource_mut::<WaterTestSuite>().request_run = true;
    app.update();
    assert!(
        app.world()
            .resource::<WaterTestSuite>()
            .entries
            .iter()
            .any(|entry| entry.status == TestStatus::Running)
    );

    // Starting the run changes the suite; let that frame pass before
    // measuring the waiting frames.
    app.update();
    app.world_mut().resource_mut::<ChangeLog>().0.clear();
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<ChangeLog>().0,
        vec![false, false],
        "running tests with no decision should not rebuild the overlay"
    );
}

#[test]
fn teleport_request_waits_for_an_active_dimension() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(GameTestPlugin);
    app.world_mut()
        .resource_mut::<WaterTestSuite>()
        .request_teleport = true;

    app.update();

    assert!(
        app.world().resource::<WaterTestSuite>().request_teleport,
        "a menu-time teleport request should survive until a dimension is active"
    );
}

#[test]
fn teleport_request_clears_once_a_dimension_is_active() {
    let mut app = test_app();
    app.world_mut()
        .resource_mut::<WaterTestSuite>()
        .request_teleport = true;

    app.update();

    let suite = app.world().resource::<WaterTestSuite>();
    assert!(!suite.request_teleport);
    assert!(
        suite.request_run,
        "with no plots built, a teleport request should start a run"
    );
}

#[test]
fn starting_a_run_unregisters_stale_plot_markers() {
    let mut app = test_app();
    let dimension_entity = app
        .world_mut()
        .query_filtered::<Entity, With<Active>>()
        .single(app.world())
        .unwrap();
    let position = ChunkPos::ZERO;
    let orphan = app
        .world_mut()
        .spawn((ChunkPosition::from(position), TestPlotChunk))
        .id();
    app.world_mut()
        .get_mut::<Dimension>(dimension_entity)
        .unwrap()
        .register_published_chunk(position, orphan);
    app.world_mut().resource_mut::<WaterTestSuite>().request_run = true;

    app.update();

    assert!(app.world().get_entity(orphan).is_err());
    assert!(
        !app.world()
            .get::<Dimension>(dimension_entity)
            .unwrap()
            .contains_loaded_chunk(position)
    );
}

#[test]
fn field_defer_keeps_plots_registered_until_the_run_is_committed() {
    let mut app = test_app();
    let dimension_entity = app
        .world_mut()
        .query_filtered::<Entity, With<Active>>()
        .single(app.world())
        .unwrap();
    let dimension_id = app.world().get::<Dimension>(dimension_entity).unwrap().id();
    let field = field_center();
    app.world_mut().spawn((
        Player::default(),
        PlayerDimension::new(dimension_id),
        Transform::from_translation(field),
        Position::new(field),
        Velocity::default(),
    ));
    let position = ChunkPos::ZERO;
    let marker = app
        .world_mut()
        .spawn((ChunkPosition::from(position), TestPlotChunk))
        .id();
    app.world_mut()
        .get_mut::<Dimension>(dimension_entity)
        .unwrap()
        .register_published_chunk(position, marker);
    app.world_mut().resource_mut::<WaterTestSuite>().request_run = true;

    app.update();

    assert!(
        app.world().resource::<WaterTestSuite>().request_run,
        "the run must stay pending while the field is cleared"
    );
    assert_eq!(
        app.world()
            .get::<Dimension>(dimension_entity)
            .unwrap()
            .published_chunk_entity(position),
        Some(marker),
        "deferred cleanup must not unregister plots before the run commits"
    );
}

#[test]
fn stale_marker_does_not_unregister_a_replaced_chunk() {
    let mut app = test_app();
    let dimension_entity = app
        .world_mut()
        .query_filtered::<Entity, With<Active>>()
        .single(app.world())
        .unwrap();
    let position = ChunkPos::ZERO;
    let marker = app
        .world_mut()
        .spawn((ChunkPosition::from(position), TestPlotChunk))
        .id();
    let replacement = app.world_mut().spawn(ChunkPosition::from(position)).id();
    app.world_mut()
        .get_mut::<Dimension>(dimension_entity)
        .unwrap()
        .register_published_chunk(position, replacement);
    app.world_mut().resource_mut::<WaterTestSuite>().request_run = true;

    app.update();

    assert!(app.world().get_entity(marker).is_err());
    assert_eq!(
        app.world()
            .get::<Dimension>(dimension_entity)
            .unwrap()
            .published_chunk_entity(position),
        Some(replacement),
        "cleanup must not unregister a chunk that replaced the stale marker"
    );
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
