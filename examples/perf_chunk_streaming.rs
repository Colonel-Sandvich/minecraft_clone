//! Renderer-free profile of initial column generation, lighting, and publication.
//!
//! The arguments are view radius, world height in chunks, lighting subchunk
//! budget, timeout in seconds, load budget, staging budget, and activation
//! budget, followed by the simulated frame interval in microseconds. Defaults
//! match production except for the smaller radius, which keeps quick runs
//! short. Pass a zero frame interval for an unpaced CPU-throughput profile.
//!
//! ```text
//! cargo run --release --example perf_chunk_streaming -- 8 5 80 120
//! perf stat -d target/release/examples/perf_chunk_streaming 24 5 80 120 4 8 8 0
//! ```

use std::time::{Duration, Instant};

use bevy::{prelude::*, state::app::StatesPlugin};
use minecraft_clone::{
    game_state::GameStatePlugin,
    world::{
        DimensionCatalog, WorldMetadata,
        chunk::ChunkPerfCounters,
        dimension::{
            Active, ColumnActivationBudget, ColumnLightBudget, ColumnLoadBudget,
            ColumnStagingBudget, DesiredColumnView, Dimension, DimensionPlugin, ViewDistance,
        },
        storage::{ChunkRepository, NoopChunkStore},
    },
};

const DEFAULT_RADIUS: i32 = 8;
const DEFAULT_HEIGHT_CHUNKS: usize = 5;
const DEFAULT_LIGHT_BUDGET: usize = 80;
const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const DEFAULT_LOAD_BUDGET: usize = 4;
const DEFAULT_STAGING_BUDGET: usize = 8;
const DEFAULT_ACTIVATION_BUDGET: usize = 8;
const DEFAULT_FRAME_INTERVAL_MICROS: u64 = 16_667;

fn main() {
    let radius = argument(1, DEFAULT_RADIUS);
    let height_chunks = argument(2, DEFAULT_HEIGHT_CHUNKS);
    let light_budget = argument(3, DEFAULT_LIGHT_BUDGET);
    let timeout_seconds = argument(4, DEFAULT_TIMEOUT_SECONDS);
    let load_budget = argument(5, DEFAULT_LOAD_BUDGET);
    let staging_budget = argument(6, DEFAULT_STAGING_BUDGET);
    let activation_budget = argument(7, DEFAULT_ACTIVATION_BUDGET);
    let frame_interval = Duration::from_micros(argument(8, DEFAULT_FRAME_INTERVAL_MICROS));
    let metadata = WorldMetadata::default()
        .with_height_chunks(height_chunks)
        .expect("profile world height must be valid");
    let repository = ChunkRepository::new(NoopChunkStore::new(metadata.clone()));
    let catalog = DimensionCatalog::for_world(&metadata);

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(StatesPlugin)
        .add_plugins(GameStatePlugin)
        .insert_resource(metadata)
        .insert_resource(catalog)
        .insert_resource(repository)
        .insert_resource(ViewDistance::new(radius))
        .insert_resource(ColumnLoadBudget(load_budget))
        .insert_resource(ColumnStagingBudget(staging_budget))
        .insert_resource(ColumnActivationBudget(activation_budget))
        .insert_resource(ColumnLightBudget(light_budget))
        .init_resource::<ChunkPerfCounters>()
        .add_plugins(DimensionPlugin);

    let timeout = Duration::from_secs(timeout_seconds);
    let started = Instant::now();
    let mut update_times = Vec::new();
    let (visible_chunks, resident_chunks, loaded_chunks, published_chunks) = loop {
        let frame_started = Instant::now();
        app.update();
        update_times.push(frame_started.elapsed());

        let (visible_chunks, resident_chunks, loaded_chunks, published_chunks) =
            active_dimension_counts(app.world_mut());
        if visible_chunks > 0 && published_chunks == visible_chunks {
            break (
                visible_chunks,
                resident_chunks,
                loaded_chunks,
                published_chunks,
            );
        }
        assert!(
            started.elapsed() < timeout,
            "streaming profile timed out after {timeout_seconds}s: \
             {published_chunks}/{visible_chunks} visible chunks published, \
             {loaded_chunks}/{resident_chunks} resident chunks loaded"
        );
        let remaining = frame_interval.saturating_sub(frame_started.elapsed());
        if remaining.is_zero() {
            std::thread::yield_now();
        } else {
            std::thread::sleep(remaining);
        }
    };
    update_times.sort_unstable();
    let perf = app.world().resource::<ChunkPerfCounters>();
    let committed_chunks = perf.light_patch_committed_columns * height_chunks;
    let amplification = ratio(perf.light_patch_calculation_chunks, committed_chunks);
    let scratch_percent = percent(
        perf.light_patch_scratch_chunks,
        perf.light_patch_calculation_chunks,
    );
    let solve_percent = duration_percent(perf.light_patch_solve_elapsed, perf.light_patch_elapsed);
    let prepare_percent =
        duration_percent(perf.light_patch_prepare_elapsed, perf.light_patch_elapsed);

    println!("streaming profile complete");
    println!(
        "view radius={radius}, height={height_chunks}, light={light_budget} subchunks, \
         load={load_budget}, staging={staging_budget}, activation={activation_budget} columns, \
         frame_interval={:.3}ms",
        millis(frame_interval),
    );
    println!(
        "world visible={visible_chunks}, resident={resident_chunks}, loaded={loaded_chunks}, \
         published={published_chunks} chunks"
    );
    println!(
        "updates={} elapsed={:.3}s p50={:.3}ms p95={:.3}ms p99={:.3}ms max={:.3}ms",
        update_times.len(),
        started.elapsed().as_secs_f64(),
        millis(percentile(&update_times, 50)),
        millis(percentile(&update_times, 95)),
        millis(percentile(&update_times, 99)),
        millis(*update_times.last().unwrap_or(&Duration::ZERO)),
    );
    println!(
        "lighting submitted={} accepted={} committed_columns={} calculated_chunks={} \
         scratch_chunks={} \
         amplification={amplification:.3}x scratch={scratch_percent:.1}%",
        perf.light_patch_runs,
        perf.light_patch_accepted_results,
        perf.light_patch_committed_columns,
        perf.light_patch_calculation_chunks,
        perf.light_patch_scratch_chunks,
    );
    println!(
        "lighting elapsed={:.3}ms max_patch={:.3}ms solve={:.3}ms ({solve_percent:.1}%) \
         prepare={:.3}ms ({prepare_percent:.1}%)",
        millis(perf.light_patch_elapsed),
        millis(perf.light_patch_max_elapsed),
        millis(perf.light_patch_solve_elapsed),
        millis(perf.light_patch_prepare_elapsed),
    );
    println!(
        "main plan={:.3}ms max_plan={:.3}ms snapshot={:.3}ms max_snapshot={:.3}ms \
         collect={:.3}ms max_collect={:.3}ms",
        millis(perf.light_patch_plan_elapsed),
        millis(perf.light_patch_max_plan_elapsed),
        millis(perf.light_patch_snapshot_elapsed),
        millis(perf.light_patch_max_snapshot_elapsed),
        millis(perf.light_patch_collect_elapsed),
        millis(perf.light_patch_max_collect_elapsed),
    );
    println!(
        "task queue={:.3}ms max_queue={:.3}ms pickup_lag={:.3}ms max_pickup_lag={:.3}ms \
         latency={:.3}ms max_latency={:.3}ms stale={} cancelled={}",
        millis(perf.light_patch_queue_elapsed),
        millis(perf.light_patch_max_queue_elapsed),
        millis(perf.light_patch_pickup_lag),
        millis(perf.light_patch_max_pickup_lag),
        millis(perf.light_patch_latency),
        millis(perf.light_patch_max_latency),
        perf.light_patch_stale_results,
        perf.light_patch_cancelled,
    );
}

fn active_dimension_counts(world: &mut World) -> (usize, usize, usize, usize) {
    let mut query = world.query_filtered::<(&Dimension, &DesiredColumnView), With<Active>>();
    let (dimension, desired) = query
        .iter(world)
        .next()
        .expect("dimension setup must create one active dimension");
    (
        desired.visible_chunk_count(),
        desired.resident_chunk_count(),
        dimension.loaded_chunk_count(),
        dimension.published_chunk_count(),
    )
}

fn argument<T>(index: usize, default: T) -> T
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    std::env::args()
        .nth(index)
        .map(|value| {
            value
                .parse::<T>()
                .unwrap_or_else(|error| panic!("invalid argument {index}: {error}"))
        })
        .unwrap_or(default)
}

fn percentile(sorted: &[Duration], percentile: usize) -> Duration {
    let index = sorted.len().saturating_sub(1).saturating_mul(percentile) / 100;
    sorted.get(index).copied().unwrap_or_default()
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn percent(numerator: usize, denominator: usize) -> f64 {
    ratio(numerator, denominator) * 100.0
}

fn duration_percent(part: Duration, total: Duration) -> f64 {
    if total.is_zero() {
        0.0
    } else {
        part.as_secs_f64() / total.as_secs_f64() * 100.0
    }
}
