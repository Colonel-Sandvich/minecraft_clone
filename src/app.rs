use std::time::{Duration, Instant};

use avian3d::PhysicsPlugins;
#[cfg(debug_assertions)]
use bevy::render::settings::InstanceFlags;
use bevy::{
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    input::common_conditions::input_toggle_active,
    prelude::*,
    render::{
        RenderPlugin,
        settings::{RenderCreation, WgpuSettings},
    },
};
use bevy_inspector_egui::{bevy_egui::EguiPlugin, quick::WorldInspectorPlugin};
use bevy_settings::SettingsPlugin;

use crate::{
    audio::{GameAudioPlugin, GameAudioSettings},
    game_state::GameStatePlugin,
    input::GameInputPlugin,
    item::DroppedItemPlugin,
    light::LightPlugin,
    memory::{MemoryTrackingPlugin, memory_profiler_enabled},
    mob::MobControllerPlugin,
    player::{Player, PlayerPlugin, cam::MouseSettings},
    textures::BlockTexturePlugin,
    ui::UIPlugin,
    world::{
        WorldConfig, WorldMetadata, WorldPlugin,
        chunk::{
            Chunk, ChunkColumn, ChunkContentCounts, ChunkNeedsLightRebuild, ChunkPerfCounters,
            ChunkPos, ChunkPosition, mesh::ChunkMeshLayer,
        },
        dimension::{Active, DesiredColumnView, Dimension, ViewDistance},
    },
};

pub const FIXED_TICK_RATE_HZ: f64 = 20.0;

pub struct AppPlugin;

impl Plugin for AppPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Minecraft".to_string(),
                        ..default()
                    }),
                    ..default()
                })
                .set(RenderPlugin {
                    render_creation: RenderCreation::Automatic(Box::new(wgpu_settings())),
                    ..default()
                }),
        )
        .register_type::<ViewDistance>()
        .register_type::<MouseSettings>()
        .register_type::<GameAudioSettings>()
        .add_plugins(SettingsPlugin::new("io.github.matt.minecraft_clone"))
        .add_plugins(EguiPlugin::default())
        .insert_resource(ClearColor(Srgba::hex("74b3ff").unwrap().into()))
        .add_plugins(GameStatePlugin)
        .add_plugins(GameAudioPlugin)
        .add_plugins(LightPlugin)
        .add_plugins(MobControllerPlugin)
        .add_plugins(PlayerPlugin)
        .add_plugins(GameInputPlugin)
        .add_plugins(DroppedItemPlugin)
        .add_plugins(BlockTexturePlugin)
        .insert_resource({
            let metadata = WorldMetadata::default();
            #[cfg(feature = "turso-store")]
            {
                WorldConfig::development_turso(metadata)
            }
            #[cfg(not(feature = "turso-store"))]
            {
                WorldConfig::development_sqlite(metadata)
            }
        })
        .add_plugins(WorldPlugin)
        .add_plugins(UIPlugin)
        .insert_resource(Time::<Fixed>::from_hz(FIXED_TICK_RATE_HZ))
        .insert_resource(Time::<Virtual>::from_max_delta(Duration::from_secs_f64(
            1.0 / FIXED_TICK_RATE_HZ,
        )))
        .add_plugins(PhysicsPlugins::default())
        // .add_plugins(PhysicsDebugPlugin::default())
        .add_plugins((FrameTimeDiagnosticsPlugin::default(),))
        .add_systems(Update, log_frame_perf)
        .add_plugins(WorldInspectorPlugin::new().run_if(input_toggle_active(false, KeyCode::F5)));
        if memory_profiler_enabled() {
            app.add_plugins(MemoryTrackingPlugin);
        }
        if std::env::var_os("MINECRAFT_CLONE_STREAM_TIMINGS").is_some() {
            app.init_resource::<StreamingStartupTrace>()
                .add_systems(First, begin_streaming_startup_frame)
                .add_systems(
                    Last,
                    (
                        log_streaming_startup_milestones,
                        finish_streaming_startup_frame,
                    )
                        .chain(),
                );
        }
        // .insert_resource(FramepaceSettings {
        //     limiter: Limiter::from_framerate(60.0),
        // })
    }
}

#[derive(Resource, Default)]
struct StreamingStartupTrace {
    started: Option<Instant>,
    frame_started: Option<Instant>,
    previous_frame_finished: Option<Instant>,
    between_updates: Duration,
    updates: u64,
    completed: u16,
}

fn begin_streaming_startup_frame(mut trace: ResMut<StreamingStartupTrace>) {
    let now = Instant::now();
    trace.started.get_or_insert(now);
    trace.between_updates = trace
        .previous_frame_finished
        .map_or(Duration::ZERO, |finished| now.duration_since(finished));
    trace.frame_started = Some(now);
    trace.updates += 1;
}

fn finish_streaming_startup_frame(mut trace: ResMut<StreamingStartupTrace>) {
    trace.previous_frame_finished = Some(Instant::now());
}

fn log_streaming_startup_milestones(
    dimension: Option<Single<(&Dimension, &DesiredColumnView), With<Active>>>,
    chunk_contents: Query<&ChunkContentCounts>,
    player: Option<Single<&Transform, With<Player>>>,
    perf: Option<Res<ChunkPerfCounters>>,
    mut trace: ResMut<StreamingStartupTrace>,
) {
    const MILESTONE_COUNT: usize = 10;
    const ALL_MILESTONES: u16 = (1 << MILESTONE_COUNT) - 1;

    if trace.completed == ALL_MILESTONES {
        return;
    }
    let Some(dimension) = dimension else {
        return;
    };
    let (dimension, desired_view) = dimension.into_inner();
    let Some(center) = desired_view.center() else {
        return;
    };
    let center_loaded = dimension.has_complete_loaded_column(center);
    let dependencies_loaded = dimension.has_complete_resident_light_neighborhood(center);
    let center_light_submitted = dimension
        .resident_column_state(center)
        .is_some_and(|state| state.light_patch_ticket().is_some() || state.is_lit());
    let center_published = column_is_published(dimension, center);
    let center_cpu_meshed = column_has_finished_cpu_mesh(dimension, center);
    let player_chunk =
        player.map(|transform| ChunkPos::containing_translation(transform.translation));
    let player_chunk_cpu_meshed =
        player_chunk.is_some_and(|position| chunk_has_finished_cpu_mesh(dimension, position));
    let near_columns = center.chebyshev_neighborhood(1).collect::<Vec<_>>();
    let near_published = near_columns
        .iter()
        .all(|&column| column_is_published(dimension, column));
    let near_cpu_meshed = near_columns
        .iter()
        .all(|&column| column_has_finished_cpu_mesh(dimension, column));
    let local_columns = center.chebyshev_neighborhood(2).collect::<Vec<_>>();
    let local_published = local_columns
        .iter()
        .all(|&column| column_is_published(dimension, column));
    let local_cpu_meshed = local_columns
        .iter()
        .all(|&column| column_has_finished_cpu_mesh(dimension, column));

    for (index, (name, reached)) in [
        ("center_loaded", center_loaded),
        ("center_dependencies_loaded", dependencies_loaded),
        ("center_light_submitted", center_light_submitted),
        ("center_published", center_published),
        ("center_cpu_meshed", center_cpu_meshed),
        ("player_chunk_cpu_meshed", player_chunk_cpu_meshed),
        ("near_3x3_published", near_published),
        ("near_3x3_cpu_meshed", near_cpu_meshed),
        ("local_5x5_published", local_published),
        ("local_5x5_cpu_meshed", local_cpu_meshed),
    ]
    .into_iter()
    .enumerate()
    {
        let bit = 1 << index;
        if !reached || trace.completed & bit != 0 {
            continue;
        }
        trace.completed |= bit;
        info!(
            target: "perf",
            milestone = name,
            elapsed_ms = format_args!(
                "{:.3}",
                trace.started.expect("startup trace must be initialized").elapsed().as_secs_f64()
                    * 1_000.0
            ),
            updates = trace.updates,
            main_update_ms = format_args!(
                "{:.3}",
                trace
                    .frame_started
                    .expect("startup frame must begin in First")
                    .elapsed()
                    .as_secs_f64()
                    * 1_000.0
            ),
            between_updates_ms = format_args!(
                "{:.3}",
                trace.between_updates.as_secs_f64() * 1_000.0
            ),
            loaded_chunks = dimension.loaded_chunk_count(),
            published_chunks = dimension.published_chunk_count(),
            player_chunk = ?player_chunk,
            "streaming startup milestone"
        );
        if name == "center_dependencies_loaded"
            && let Some(perf) = perf.as_deref()
        {
            info!(
                target: "perf",
                completed_columns = perf.column_loads,
                worker_total_ms = format_args!(
                    "{:.3}",
                    perf.column_load_worker_elapsed.as_secs_f64() * 1_000.0
                ),
                worker_max_ms = format_args!(
                    "{:.3}",
                    perf.column_load_max_worker_elapsed.as_secs_f64() * 1_000.0
                ),
                queue_total_ms = format_args!(
                    "{:.3}",
                    perf.column_load_queue_elapsed.as_secs_f64() * 1_000.0
                ),
                queue_max_ms = format_args!(
                    "{:.3}",
                    perf.column_load_max_queue_elapsed.as_secs_f64() * 1_000.0
                ),
                pickup_total_ms = format_args!(
                    "{:.3}",
                    perf.column_load_pickup_lag.as_secs_f64() * 1_000.0
                ),
                pickup_max_ms = format_args!(
                    "{:.3}",
                    perf.column_load_max_pickup_lag.as_secs_f64() * 1_000.0
                ),
                latency_max_ms = format_args!(
                    "{:.3}",
                    perf.column_load_max_latency.as_secs_f64() * 1_000.0
                ),
                "center dependency load work"
            );
        }
        if name == "center_cpu_meshed"
            && let Some(perf) = perf.as_deref()
        {
            let rendered_cells_by_y = dimension
                .complete_loaded_column(center)
                .into_iter()
                .flatten()
                .filter_map(|(position, entity)| {
                    chunk_contents
                        .get(entity)
                        .ok()
                        .map(|contents| (position.y(), contents.rendered))
                })
                .collect::<Vec<_>>();
            info!(
                target: "perf",
                rendered_cells_by_y = ?rendered_cells_by_y,
                mesh_runs = perf.mesh_rebuild_runs,
                mesh_rebuilt_chunks = perf.mesh_rebuilds,
                mesh_total_ms = format_args!(
                    "{:.3}",
                    perf.mesh_rebuild_elapsed.as_secs_f64() * 1_000.0
                ),
                mesh_context_ms = format_args!(
                    "{:.3}",
                    perf.mesh_context_elapsed.as_secs_f64() * 1_000.0
                ),
                mesh_build_ms = format_args!(
                    "{:.3}",
                    perf.mesh_build_elapsed.as_secs_f64() * 1_000.0
                ),
                mesh_apply_ms = format_args!(
                    "{:.3}",
                    perf.mesh_apply_elapsed.as_secs_f64() * 1_000.0
                ),
                light_submitted = perf.light_patch_runs,
                light_accepted = perf.light_patch_accepted_results,
                light_task_ms = format_args!(
                    "{:.3}",
                    perf.light_patch_elapsed.as_secs_f64() * 1_000.0
                ),
                light_solve_ms = format_args!(
                    "{:.3}",
                    perf.light_patch_solve_elapsed.as_secs_f64() * 1_000.0
                ),
                light_prepare_ms = format_args!(
                    "{:.3}",
                    perf.light_patch_prepare_elapsed.as_secs_f64() * 1_000.0
                ),
                light_snapshot_ms = format_args!(
                    "{:.3}",
                    perf.light_patch_snapshot_elapsed.as_secs_f64() * 1_000.0
                ),
                light_collect_ms = format_args!(
                    "{:.3}",
                    perf.light_patch_collect_elapsed.as_secs_f64() * 1_000.0
                ),
                light_queue_ms = format_args!(
                    "{:.3}",
                    perf.light_patch_queue_elapsed.as_secs_f64() * 1_000.0
                ),
                light_pickup_lag_ms = format_args!(
                    "{:.3}",
                    perf.light_patch_pickup_lag.as_secs_f64() * 1_000.0
                ),
                "center startup CPU work"
            );
        }
    }
}

fn chunk_has_finished_cpu_mesh(dimension: &Dimension, position: ChunkPos) -> bool {
    if !dimension.contains_published_chunk(position) {
        return false;
    }
    !dimension.has_pending_mesh_rebuild(position)
}

fn column_is_published(dimension: &Dimension, column: ChunkColumn) -> bool {
    dimension.contains_published_chunk(column.chunk(0))
}

fn column_has_finished_cpu_mesh(dimension: &Dimension, column: ChunkColumn) -> bool {
    if !column_is_published(dimension, column) {
        return false;
    }
    let Some(chunks) = dimension.complete_loaded_column(column) else {
        return false;
    };
    for (position, _) in chunks {
        if dimension.has_pending_mesh_rebuild(position) {
            return false;
        }
    }
    true
}

pub fn run() {
    App::new().add_plugins(AppPlugin).run();
}

fn wgpu_settings() -> WgpuSettings {
    #[allow(unused_mut)]
    let mut settings = WgpuSettings::default();
    #[cfg(debug_assertions)]
    {
        settings.instance_flags.insert(InstanceFlags::debugging());
    }
    settings
}

fn log_frame_perf(
    time: Res<Time>,
    diagnostics: Res<DiagnosticsStore>,
    chunks: Query<(), With<Chunk>>,
    dimension: Option<Single<&Dimension, With<Active>>>,
    dirty_light_rebuild_chunks: Query<&ChunkPosition, (With<Chunk>, With<ChunkNeedsLightRebuild>)>,
    chunk_mesh_layers: Query<&ChunkMeshLayer>,
    mut chunk_perf: Option<ResMut<ChunkPerfCounters>>,
    mut timer: Local<Option<Timer>>,
) {
    let timer = timer.get_or_insert_with(|| Timer::from_seconds(5.0, TimerMode::Repeating));
    if !timer.tick(time.delta()).just_finished() {
        return;
    }

    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|diagnostic| diagnostic.smoothed())
        .unwrap_or_default();
    let frame_ms = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|diagnostic| diagnostic.smoothed())
        .unwrap_or_else(|| time.delta_secs_f64() * 1000.0);

    let mut mesh_layers = 0usize;
    let mut mesh_faces = 0u64;
    for layer in &chunk_mesh_layers {
        mesh_layers += 1;
        mesh_faces += u64::from(layer.face_count());
    }
    let dirty_mesh_positions = dimension
        .as_deref()
        .map(|dimension| {
            dimension
                .pending_mesh_rebuilds()
                .map(|work| work.position().as_ivec3())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let dirty_light_upload_positions = dimension
        .as_deref()
        .map(|dimension| {
            dimension
                .pending_render_light_uploads()
                .map(|work| work.position().as_ivec3())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let dirty_collider_positions = dimension
        .as_deref()
        .map(|dimension| {
            dimension
                .pending_collider_rebuilds()
                .map(|work| work.position().as_ivec3())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let dirty_light_rebuild_positions = dirty_light_rebuild_chunks
        .iter()
        .map(|pos| pos.as_ivec3())
        .collect::<Vec<_>>();
    let dirty_mesh_sample = format_position_sample(&dirty_mesh_positions);
    let dirty_light_upload_sample = format_position_sample(&dirty_light_upload_positions);
    let dirty_collider_sample = format_position_sample(&dirty_collider_positions);
    let dirty_light_rebuild_sample = format_position_sample(&dirty_light_rebuild_positions);
    let chunk_perf = chunk_perf
        .as_deref_mut()
        .map(ChunkPerfCounters::take)
        .unwrap_or_default();

    info!(
        target: "perf",
        fps = format_args!("{fps:.1}"),
        frame_ms = format_args!("{frame_ms:.3}"),
        chunks = chunks.iter().count(),
        dirty_mesh = dirty_mesh_positions.len(),
        dirty_mesh_sample = %dirty_mesh_sample,
        dirty_light_upload = dirty_light_upload_positions.len(),
        dirty_light_upload_sample = %dirty_light_upload_sample,
        dirty_collider = dirty_collider_positions.len(),
        dirty_collider_sample = %dirty_collider_sample,
        dirty_light_rebuild = dirty_light_rebuild_positions.len(),
        dirty_light_rebuild_sample = %dirty_light_rebuild_sample,
        column_loads_5s = chunk_perf.column_loads,
        column_load_worker_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.column_load_worker_elapsed.as_secs_f64() * 1_000.0
        ),
        column_load_max_worker_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.column_load_max_worker_elapsed.as_secs_f64() * 1_000.0
        ),
        column_load_queue_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.column_load_queue_elapsed.as_secs_f64() * 1_000.0
        ),
        column_load_max_queue_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.column_load_max_queue_elapsed.as_secs_f64() * 1_000.0
        ),
        column_load_pickup_lag_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.column_load_pickup_lag.as_secs_f64() * 1_000.0
        ),
        column_load_max_pickup_lag_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.column_load_max_pickup_lag.as_secs_f64() * 1_000.0
        ),
        column_load_latency_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.column_load_latency.as_secs_f64() * 1_000.0
        ),
        column_load_max_latency_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.column_load_max_latency.as_secs_f64() * 1_000.0
        ),
        mesh_rebuilds_5s = chunk_perf.mesh_rebuilds,
        mesh_rebuild_runs_5s = chunk_perf.mesh_rebuild_runs,
        mesh_rebuild_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.mesh_rebuild_elapsed.as_secs_f64() * 1_000.0
        ),
        mesh_rebuild_max_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.mesh_rebuild_max_elapsed.as_secs_f64() * 1_000.0
        ),
        mesh_context_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.mesh_context_elapsed.as_secs_f64() * 1_000.0
        ),
        mesh_context_max_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.mesh_context_max_elapsed.as_secs_f64() * 1_000.0
        ),
        mesh_build_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.mesh_build_elapsed.as_secs_f64() * 1_000.0
        ),
        mesh_build_max_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.mesh_build_max_elapsed.as_secs_f64() * 1_000.0
        ),
        mesh_apply_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.mesh_apply_elapsed.as_secs_f64() * 1_000.0
        ),
        mesh_apply_max_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.mesh_apply_max_elapsed.as_secs_f64() * 1_000.0
        ),
        light_rebuild_targets_5s = chunk_perf.light_rebuild_targets,
        light_patch_submitted_5s = chunk_perf.light_patch_runs,
        light_patch_accepted_5s = chunk_perf.light_patch_accepted_results,
        light_patch_calculation_chunks_5s = chunk_perf.light_patch_calculation_chunks,
        light_patch_max_calculation_chunks_5s = chunk_perf.light_patch_max_calculation_chunks,
        light_patch_scratch_chunks_5s = chunk_perf.light_patch_scratch_chunks,
        light_patch_committed_columns_5s = chunk_perf.light_patch_committed_columns,
        light_patch_elapsed_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_elapsed.as_secs_f64() * 1_000.0
        ),
        light_patch_max_elapsed_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_max_elapsed.as_secs_f64() * 1_000.0
        ),
        light_patch_solve_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_solve_elapsed.as_secs_f64() * 1_000.0
        ),
        light_patch_prepare_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_prepare_elapsed.as_secs_f64() * 1_000.0
        ),
        light_patch_plan_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_plan_elapsed.as_secs_f64() * 1_000.0
        ),
        light_patch_max_plan_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_max_plan_elapsed.as_secs_f64() * 1_000.0
        ),
        light_patch_snapshot_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_snapshot_elapsed.as_secs_f64() * 1_000.0
        ),
        light_patch_max_snapshot_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_max_snapshot_elapsed.as_secs_f64() * 1_000.0
        ),
        light_patch_collect_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_collect_elapsed.as_secs_f64() * 1_000.0
        ),
        light_patch_max_collect_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_max_collect_elapsed.as_secs_f64() * 1_000.0
        ),
        light_patch_queue_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_queue_elapsed.as_secs_f64() * 1_000.0
        ),
        light_patch_max_queue_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_max_queue_elapsed.as_secs_f64() * 1_000.0
        ),
        light_patch_pickup_lag_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_pickup_lag.as_secs_f64() * 1_000.0
        ),
        light_patch_max_pickup_lag_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_max_pickup_lag.as_secs_f64() * 1_000.0
        ),
        light_patch_latency_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_latency.as_secs_f64() * 1_000.0
        ),
        light_patch_max_latency_ms_5s = format_args!(
            "{:.3}",
            chunk_perf.light_patch_max_latency.as_secs_f64() * 1_000.0
        ),
        light_patch_stale_results_5s = chunk_perf.light_patch_stale_results,
        light_patch_cancelled_5s = chunk_perf.light_patch_cancelled,
        light_uploads_5s = chunk_perf.light_uploads,
        mesh_layers,
        mesh_faces,
        "frame perf"
    );
}

fn format_position_sample(positions: &[IVec3]) -> String {
    if positions.is_empty() {
        return "[]".to_string();
    }

    let mut sample = positions
        .iter()
        .take(8)
        .map(|pos| format!("{},{},{}", pos.x, pos.y, pos.z))
        .collect::<Vec<_>>()
        .join(" ");
    if positions.len() > 8 {
        sample.push_str(" ...");
    }
    format!("[{sample}]")
}
