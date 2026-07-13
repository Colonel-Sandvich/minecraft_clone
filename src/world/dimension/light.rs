use std::time::{Duration, Instant};

use bevy::{platform::collections::HashSet, prelude::*};

use crate::world::{
    chunk::{
        Chunk, ChunkColumn, ChunkHeightmap, ChunkInvalidationPlan, ChunkLight,
        ChunkNeedsLightRebuild, ChunkNeedsRenderLightUpload, ChunkPerfCounters, ChunkPos,
        ChunkPosition, light::ChunkLightRegion, mesh::PreparedChunkMeshLight,
    },
    generation::WorldMetadata,
};

use super::{
    Active, ChunkTaskPool, ColumnLightBudget, ColumnLoadBudget, DesiredColumnView, Dimension,
    apply_chunk_invalidations,
    light_patch::{InitialLightColumnState, LightPatchPlan},
    light_task::{
        FinishedLightPatchTask, LightChunkInputStamp, LightColumnInputStamp, LightCommitBaseline,
        LightPatchTaskRequest, OwnedLightCalculationChunk, OwnedLightPatchInput,
    },
    streaming::{ColumnExposure, ColumnLighting},
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn rebuild_chunk_light(
    mut commands: Commands,
    mut perf: Option<ResMut<ChunkPerfCounters>>,
    needs_rebuild: Query<(Entity, &ChunkPosition), With<ChunkNeedsLightRebuild>>,
    all_chunks: Query<(&ChunkPosition, &Chunk, &ChunkLight, &ChunkHeightmap)>,
    dimension: Single<&mut Dimension, With<Active>>,
    metadata: Res<WorldMetadata>,
    desired_view: Option<Res<DesiredColumnView>>,
    patch_budget: Option<Res<ColumnLightBudget>>,
    load_budget: Option<Res<ColumnLoadBudget>>,
    task_pool: Option<Res<ChunkTaskPool>>,
) {
    let mut dimension = dimension.into_inner();
    if let (Some(desired_view), Some(task_pool)) = (desired_view, task_pool) {
        process_streamed_column_light(
            &mut commands,
            perf.as_deref_mut(),
            &mut dimension,
            &all_chunks,
            &desired_view,
            patch_budget
                .as_deref()
                .map_or(usize::MAX, |budget| budget.0),
            load_budget.as_deref().is_none_or(|budget| budget.0 == 0),
            &task_pool,
        );
    }

    if needs_rebuild.is_empty() {
        return;
    }
    let dirty_chunks = dimension
        .iter_published_chunks()
        .filter_map(|(registered_position, entity)| {
            if dimension
                .resident_column_state(registered_position.column())
                .is_some()
            {
                return None;
            }
            let (_, actual_position) = needs_rebuild.get(entity).ok()?;
            (actual_position.chunk_pos() == registered_position)
                .then_some((entity, registered_position))
        })
        .collect::<Vec<_>>();
    let dirty_positions = dirty_chunks
        .iter()
        .map(|(_, position)| *position)
        .collect::<Vec<_>>();

    let height_chunks = metadata.height_chunks();
    let mut region = ChunkLightRegion::new(height_chunks);
    let targets = light_rebuild_targets(&dirty_positions, &dimension, height_chunks);
    if let Some(perf) = perf.as_deref_mut() {
        perf.light_rebuild_targets += targets.len();
    }

    if targets.is_empty() {
        for (entity, _) in dirty_chunks {
            commands.entity(entity).remove::<ChunkNeedsLightRebuild>();
        }
        return;
    }

    for &position in &targets {
        let entity = dimension
            .loaded_chunk_entity(position)
            .expect("light rebuild target must belong to the active dimension");
        let (actual_position, chunk, light, heightmap) = all_chunks
            .get(entity)
            .expect("registered light rebuild target must have chunk lighting components");
        assert_eq!(
            actual_position.chunk_pos(),
            position,
            "dimension registry and ChunkPosition must agree"
        );
        region.insert_target(position, chunk, light, heightmap);
    }

    for position in region.required_boundary_positions() {
        let Some(entity) = dimension.loaded_chunk_entity(position) else {
            continue;
        };
        if dimension
            .resident_column_state(position.column())
            .is_some_and(|state| !state.is_lit())
        {
            continue;
        }
        let (actual_position, _, light, _) = all_chunks
            .get(entity)
            .expect("registered light boundary must have chunk lighting components");
        assert_eq!(
            actual_position.chunk_pos(),
            position,
            "dimension registry and ChunkPosition must agree"
        );
        region.insert_boundary_light(position, light);
    }

    let mut invalidations = ChunkInvalidationPlan::new();
    for rebuilt in region.rebuild() {
        let entity = dimension
            .loaded_chunk_entity(rebuilt.position)
            .expect("rebuilt light target must remain in the active dimension");
        let light_changed = rebuilt.light_changed();
        let heightmap_changed = rebuilt.heightmap_changed();

        if light_changed {
            commands.entity(entity).insert(rebuilt.light);
            invalidations.record_render_light_changed(rebuilt.position);
        }
        if heightmap_changed {
            commands.entity(entity).insert(rebuilt.heightmap);
        }
        commands.entity(entity).remove::<ChunkNeedsLightRebuild>();
    }

    apply_chunk_invalidations(&mut commands, &mut dimension, &invalidations);
}

pub(crate) fn cancel_inactive_dimension_light_tasks(
    mut dimensions: Query<&mut Dimension, Without<Active>>,
    mut perf: Option<ResMut<ChunkPerfCounters>>,
) {
    for mut dimension in &mut dimensions {
        if let Some(ticket) = dimension.light_tasks().active_ticket() {
            dimension.cancel_column_light_patch(ticket);
        }
        record_cancelled_light_tasks(&mut dimension, perf.as_deref_mut());
        if let Some(finished) = dimension.light_tasks_mut().take_ready() {
            let collect_started = Instant::now();
            debug_assert!(finished.cancel_requested);
            record_finished_light_patch_perf(
                perf.as_deref_mut(),
                &finished,
                FinishedLightPatchDisposition::Cancelled,
            );
            record_light_patch_collect_perf(perf.as_deref_mut(), collect_started.elapsed());
        }
    }
}

fn process_streamed_column_light(
    commands: &mut Commands,
    mut perf: Option<&mut ChunkPerfCounters>,
    dimension: &mut Dimension,
    all_chunks: &Query<(&ChunkPosition, &Chunk, &ChunkLight, &ChunkHeightmap)>,
    desired_view: &DesiredColumnView,
    target_budget: usize,
    load_admission_paused: bool,
    task_pool: &ChunkTaskPool,
) {
    cancel_unclaimed_light_task(dimension, perf.as_deref_mut());
    if let Some(finished) = dimension.light_tasks_mut().take_ready() {
        let collect_started = Instant::now();
        if finished.cancel_requested {
            record_finished_light_patch_perf(
                perf.as_deref_mut(),
                &finished,
                FinishedLightPatchDisposition::Cancelled,
            );
        } else {
            apply_finished_light_patch(
                commands,
                perf.as_deref_mut(),
                dimension,
                all_chunks,
                finished,
            );
        }
        record_light_patch_collect_perf(perf.as_deref_mut(), collect_started.elapsed());
    }

    if target_budget == 0 || !dimension.light_tasks().is_idle() {
        return;
    }

    let height_chunks = dimension.height().chunks();
    let plan_started = Instant::now();
    let plan = next_light_patch_plan(
        dimension,
        desired_view,
        height_chunks,
        target_budget,
        load_admission_paused,
    );
    if let Some(perf) = perf.as_deref_mut() {
        let elapsed = plan_started.elapsed();
        perf.light_patch_plan_elapsed += elapsed;
        perf.light_patch_max_plan_elapsed = perf.light_patch_max_plan_elapsed.max(elapsed);
    }
    if plan.is_empty() {
        return;
    }
    start_light_patch(perf, dimension, all_chunks, task_pool, plan, height_chunks);
}

fn cancel_unclaimed_light_task(
    dimension: &mut Dimension,
    mut perf: Option<&mut ChunkPerfCounters>,
) {
    record_cancelled_light_tasks(dimension, perf.as_deref_mut());
    let Some(ticket) = dimension.light_tasks().active_ticket() else {
        return;
    };
    if dimension.stream().light_patch_columns(ticket).is_some() {
        return;
    }
    dimension.light_tasks_mut().cancel(ticket);
    record_cancelled_light_tasks(dimension, perf);
}

fn record_cancelled_light_tasks(dimension: &mut Dimension, perf: Option<&mut ChunkPerfCounters>) {
    let cancelled = dimension.light_tasks_mut().take_cancelled_count();
    if let Some(perf) = perf {
        perf.light_patch_cancelled += cancelled;
    }
}

fn next_light_patch_plan(
    dimension: &Dimension,
    desired_view: &DesiredColumnView,
    height_chunks: usize,
    target_budget: usize,
    load_admission_paused: bool,
) -> LightPatchPlan {
    let runtime = LightPatchPlan::build(
        desired_view.visible_columns(),
        height_chunks,
        target_budget,
        |column| {
            dimension.column_lighting(column) == Some(ColumnLighting::Pending)
                && dimension.column_exposure(column) == Some(ColumnExposure::Published)
                && dimension.has_complete_resident_light_neighborhood(column)
        },
    );
    if !runtime.is_empty() {
        return runtime;
    }

    let Some(center) = desired_view.center() else {
        return LightPatchPlan::default();
    };
    let initial =
        LightPatchPlan::build_initial_patch(desired_view.visible_columns(), center, |column| {
            classify_initial_light_column(dimension, column)
        });
    if !initial.is_empty() {
        return initial;
    }

    let awaiting_resident_data = desired_view
        .resident_columns()
        .iter()
        .any(|&column| dimension.stream().awaits_load_progress(column));
    if !load_admission_paused && awaiting_resident_data {
        return LightPatchPlan::default();
    }

    // Failed loads and deliberately paused admission must not strand unrelated
    // ready cores behind an incomplete tile.
    LightPatchPlan::build(
        desired_view.visible_columns(),
        height_chunks,
        target_budget,
        |column| {
            dimension.column_lighting(column) == Some(ColumnLighting::Pending)
                && dimension.column_exposure(column) == Some(ColumnExposure::Staged)
                && dimension.has_complete_resident_light_neighborhood(column)
        },
    )
}

fn classify_initial_light_column(
    dimension: &Dimension,
    column: ChunkColumn,
) -> InitialLightColumnState {
    let Some(state) = dimension.resident_column_state(column) else {
        return InitialLightColumnState::Waiting;
    };
    if state.exposure() != ColumnExposure::Staged || !state.is_light_pending() {
        return InitialLightColumnState::Excluded;
    }
    if dimension.has_complete_resident_light_neighborhood(column) {
        InitialLightColumnState::Ready
    } else {
        InitialLightColumnState::Waiting
    }
}

fn start_light_patch(
    perf: Option<&mut ChunkPerfCounters>,
    dimension: &mut Dimension,
    all_chunks: &Query<(&ChunkPosition, &Chunk, &ChunkLight, &ChunkHeightmap)>,
    task_pool: &ChunkTaskPool,
    plan: LightPatchPlan,
    height_chunks: usize,
) {
    let snapshot_started = Instant::now();
    let mut column_inputs = Vec::with_capacity(plan.calculation_columns().len());
    let mut chunk_inputs = Vec::with_capacity(plan.calculation_chunk_count(height_chunks));
    let mut owned_chunks = Vec::with_capacity(plan.calculation_chunk_count(height_chunks));
    for &column in plan.calculation_columns() {
        let state = dimension
            .resident_column_state(column)
            .expect("lighting calculation column must remain resident");
        column_inputs.push(LightColumnInputStamp {
            column,
            incarnation: dimension
                .column_incarnation(column)
                .expect("lighting calculation column must retain its incarnation"),
            commit_light_revision: plan.commits(column).then(|| state.light_revision()),
        });

        for y in 0..height_chunks as i32 {
            let position = column.chunk(y);
            let entity = dimension
                .loaded_chunk_entity(position)
                .expect("lighting dependency must remain loaded");
            let (actual, chunk, light, heightmap) = all_chunks
                .get(entity)
                .expect("loaded lighting dependency must retain lighting components");
            assert_eq!(
                actual.chunk_pos(),
                position,
                "loaded lighting dependency position must match its registry key"
            );
            chunk_inputs.push(LightChunkInputStamp {
                position,
                entity,
                content_revision: chunk.content_revision(),
            });
            owned_chunks.push(OwnedLightCalculationChunk {
                position,
                chunk: chunk.clone(),
                commit_baseline: plan.commits(column).then(|| LightCommitBaseline {
                    light: light.clone(),
                    heightmap: *heightmap,
                }),
            });
        }
    }

    let snapshot_elapsed = snapshot_started.elapsed();
    let commit_columns = plan.commit_columns().to_vec();
    let calculation_chunks = plan.calculation_chunk_count(height_chunks);
    let scratch_chunks = plan.scratch_chunk_count(height_chunks);
    let ticket = dimension
        .begin_column_light_patch(&commit_columns)
        .expect("prevalidated pending light cores must be claimable atomically");
    dimension.light_tasks_mut().start(
        task_pool,
        LightPatchTaskRequest {
            ticket,
            commit_columns,
            column_inputs,
            chunk_inputs,
            input: OwnedLightPatchInput::new(height_chunks, owned_chunks),
        },
    );
    if let Some(perf) = perf {
        perf.light_patch_runs += 1;
        perf.light_patch_calculation_chunks += calculation_chunks;
        perf.light_patch_max_calculation_chunks = perf
            .light_patch_max_calculation_chunks
            .max(calculation_chunks);
        perf.light_patch_scratch_chunks += scratch_chunks;
        perf.light_patch_snapshot_elapsed += snapshot_elapsed;
        perf.light_patch_max_snapshot_elapsed =
            perf.light_patch_max_snapshot_elapsed.max(snapshot_elapsed);
    }
}

fn apply_finished_light_patch(
    commands: &mut Commands,
    perf: Option<&mut ChunkPerfCounters>,
    dimension: &mut Dimension,
    all_chunks: &Query<(&ChunkPosition, &Chunk, &ChunkLight, &ChunkHeightmap)>,
    finished: FinishedLightPatchTask,
) {
    let current = light_patch_inputs_are_current(dimension, all_chunks, &finished);
    if !current {
        dimension.cancel_column_light_patch(finished.ticket);
        record_finished_light_patch_perf(perf, &finished, FinishedLightPatchDisposition::Stale);
        return;
    }

    let committed = dimension
        .finish_column_light_patch(finished.ticket)
        .expect("validated light patch authority must commit atomically");
    assert_eq!(
        committed
            .iter()
            .map(|(column, _)| *column)
            .collect::<Vec<_>>(),
        finished.commit_columns,
        "lighting authority must preserve the planned commit set"
    );
    record_finished_light_patch_perf(perf, &finished, FinishedLightPatchDisposition::Accepted);

    for prepared in finished.result.prepared {
        let entity = dimension
            .loaded_chunk_entity(prepared.position)
            .expect("validated prepared-light target must remain loaded");
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert(PreparedChunkMeshLight::new(prepared.data));
        if dimension.contains_published_chunk(prepared.position) {
            entity_commands.insert(ChunkNeedsRenderLightUpload);
        }
    }

    for rebuilt in finished.result.rebuilt {
        let entity = dimension
            .loaded_chunk_entity(rebuilt.position)
            .expect("validated light commit target must remain loaded");
        let light_changed = rebuilt.light_changed();
        let heightmap_changed = rebuilt.heightmap_changed();
        let mut entity_commands = commands.entity(entity);
        if light_changed {
            entity_commands.insert(rebuilt.light);
        }
        if heightmap_changed {
            entity_commands.insert(rebuilt.heightmap);
        }
        entity_commands.remove::<ChunkNeedsLightRebuild>();
    }
}

fn light_patch_inputs_are_current(
    dimension: &Dimension,
    all_chunks: &Query<(&ChunkPosition, &Chunk, &ChunkLight, &ChunkHeightmap)>,
    finished: &FinishedLightPatchTask,
) -> bool {
    if finished.ticket.owner() != dimension.stream().owner()
        || dimension.stream().light_patch_columns(finished.ticket)
            != Some(finished.commit_columns.as_slice())
    {
        return false;
    }

    for input in &finished.column_inputs {
        let Some(state) = dimension.resident_column_state(input.column) else {
            return false;
        };
        if dimension.column_incarnation(input.column) != Some(input.incarnation) {
            return false;
        }
        if let Some(revision) = input.commit_light_revision
            && (state.light_revision() != revision
                || state.light_patch_ticket() != Some(finished.ticket))
        {
            return false;
        }
    }

    for input in &finished.chunk_inputs {
        if dimension.loaded_chunk_entity(input.position) != Some(input.entity) {
            return false;
        }
        let Ok((actual, chunk, _, _)) = all_chunks.get(input.entity) else {
            return false;
        };
        if actual.chunk_pos() != input.position
            || chunk.content_revision() != input.content_revision
        {
            return false;
        }
    }

    let expected_chunks = finished.commit_columns.len() * dimension.height().chunks();
    if finished.result.rebuilt.len() != expected_chunks
        || finished.result.prepared.len() != expected_chunks
    {
        return false;
    }
    let expected_positions = finished
        .commit_columns
        .iter()
        .flat_map(|column| (0..dimension.height().chunks_i32()).map(move |y| column.chunk(y)))
        .collect::<HashSet<_>>();
    let rebuilt = finished
        .result
        .rebuilt
        .iter()
        .map(|chunk| chunk.position)
        .collect::<HashSet<_>>();
    let prepared = finished
        .result
        .prepared
        .iter()
        .map(|chunk| chunk.position)
        .collect::<HashSet<_>>();
    rebuilt.len() == expected_chunks
        && prepared.len() == expected_chunks
        && rebuilt == prepared
        && rebuilt == expected_positions
}

fn record_finished_light_patch_perf(
    perf: Option<&mut ChunkPerfCounters>,
    finished: &FinishedLightPatchTask,
    disposition: FinishedLightPatchDisposition,
) {
    let Some(perf) = perf else {
        return;
    };
    perf.light_patch_elapsed += finished.result.elapsed;
    perf.light_patch_max_elapsed = perf.light_patch_max_elapsed.max(finished.result.elapsed);
    perf.light_patch_solve_elapsed += finished.result.solve_elapsed;
    perf.light_patch_prepare_elapsed += finished.result.prepare_elapsed;
    perf.light_patch_queue_elapsed += finished.result.queue_elapsed;
    perf.light_patch_max_queue_elapsed = perf
        .light_patch_max_queue_elapsed
        .max(finished.result.queue_elapsed);
    let pickup_lag = finished
        .latency
        .saturating_sub(finished.result.queue_elapsed + finished.result.elapsed);
    perf.light_patch_pickup_lag += pickup_lag;
    perf.light_patch_max_pickup_lag = perf.light_patch_max_pickup_lag.max(pickup_lag);
    perf.light_patch_latency += finished.latency;
    perf.light_patch_max_latency = perf.light_patch_max_latency.max(finished.latency);
    match disposition {
        FinishedLightPatchDisposition::Accepted => {
            perf.light_patch_accepted_results += 1;
            perf.light_patch_committed_columns += finished.commit_columns.len();
        }
        FinishedLightPatchDisposition::Stale => perf.light_patch_stale_results += 1,
        FinishedLightPatchDisposition::Cancelled => {}
    }
}

fn record_light_patch_collect_perf(perf: Option<&mut ChunkPerfCounters>, elapsed: Duration) {
    let Some(perf) = perf else {
        return;
    };
    perf.light_patch_collect_elapsed += elapsed;
    perf.light_patch_max_collect_elapsed = perf.light_patch_max_collect_elapsed.max(elapsed);
}

#[derive(Clone, Copy)]
enum FinishedLightPatchDisposition {
    Accepted,
    Stale,
    Cancelled,
}

fn light_rebuild_targets(
    dirty_positions: &[ChunkPos],
    dimension: &Dimension,
    height_chunks: usize,
) -> HashSet<ChunkPos> {
    let columns = dirty_positions
        .iter()
        .copied()
        .map(ChunkColumn::from)
        .collect::<HashSet<_>>();

    let mut targets = HashSet::new();
    for column in columns {
        for y in 0..height_chunks as i32 {
            let position = column.chunk(y);
            if dimension.contains_loaded_chunk(position) {
                targets.insert(position);
            }
        }
    }

    targets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        block::BlockType,
        world::chunk::{
            CHUNK_SIZE, ChunkCell, ChunkNeedsLightRebuild, ChunkNeedsMeshRebuild,
            ChunkNeedsRenderLightUpload, LocalBlockPos,
        },
    };

    #[derive(Resource)]
    struct TestDimension(Entity);

    fn app_with_light_system(height_chunks: usize) -> App {
        let metadata = WorldMetadata::with_seed(1)
            .with_height_chunks(height_chunks)
            .unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metadata)
            .add_systems(Update, rebuild_chunk_light);
        let dimension = app.world_mut().spawn((Dimension::default(), Active)).id();
        app.insert_resource(TestDimension(dimension));
        app
    }

    fn register_chunk(app: &mut App, position: IVec3, entity: Entity) {
        let dimension = app.world().resource::<TestDimension>().0;
        app.world_mut()
            .entity_mut(dimension)
            .get_mut::<Dimension>()
            .unwrap()
            .register_published_chunk(position.into(), entity);
    }

    fn solid_chunk(block: BlockType) -> Chunk {
        Chunk::filled(block.into())
    }

    fn heightmap_with(value: u8) -> ChunkHeightmap {
        ChunkHeightmap {
            heights: [[value; CHUNK_SIZE]; CHUNK_SIZE],
        }
    }

    #[test]
    fn rebuild_light_resolves_vertical_sky_occlusion_across_loaded_column() {
        let mut app = app_with_light_system(2);
        let lower_pos = ivec3(0, 0, 0);
        let upper_pos = ivec3(0, 1, 0);
        let lower_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(lower_pos),
                Chunk::default(),
                ChunkLight::default(),
                ChunkHeightmap::default(),
                ChunkNeedsLightRebuild,
            ))
            .id();

        let mut upper = Chunk::default();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                upper.set_cell_xyz(x, 0, z, BlockType::Stone.into());
            }
        }
        let upper_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(upper_pos),
                upper,
                ChunkLight::default(),
                ChunkHeightmap::default(),
            ))
            .id();
        register_chunk(&mut app, lower_pos, lower_entity);
        register_chunk(&mut app, upper_pos, upper_entity);

        app.update();

        let world = app.world();
        assert_eq!(
            world
                .get::<ChunkLight>(upper_entity)
                .unwrap()
                .sky_light(LocalBlockPos::new(8, 1, 8)),
            15
        );
        assert_eq!(
            world
                .get::<ChunkLight>(lower_entity)
                .unwrap()
                .sky_light(LocalBlockPos::new(8, 15, 8)),
            0
        );
        assert!(world.get::<ChunkNeedsLightRebuild>(lower_entity).is_none());
    }

    #[test]
    fn changed_light_marks_padded_neighbor_light_upload_dirty() {
        let mut app = app_with_light_system(1);
        let center_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(IVec3::ZERO),
                Chunk::default(),
                ChunkLight::default(),
                ChunkHeightmap::default(),
                ChunkNeedsLightRebuild,
            ))
            .id();
        let neighbor_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(IVec3::X),
                solid_chunk(BlockType::Stone),
                ChunkLight::default(),
                heightmap_with(15),
            ))
            .id();
        register_chunk(&mut app, IVec3::ZERO, center_entity);
        register_chunk(&mut app, IVec3::X, neighbor_entity);

        app.update();

        let world = app.world();
        assert_eq!(
            world
                .get::<ChunkLight>(center_entity)
                .unwrap()
                .sky_light(LocalBlockPos::new(8, 8, 8)),
            15
        );
        assert!(
            world
                .get::<ChunkNeedsRenderLightUpload>(neighbor_entity)
                .is_some()
        );
        assert!(
            world
                .get::<ChunkNeedsRenderLightUpload>(center_entity)
                .is_some()
        );
        assert!(world.get::<ChunkNeedsMeshRebuild>(center_entity).is_none());
        assert!(
            world
                .get::<ChunkNeedsMeshRebuild>(neighbor_entity)
                .is_none()
        );
    }

    #[test]
    fn rebuild_light_clears_neighbor_block_light_after_emitter_removed() {
        let mut app = app_with_light_system(1);
        let left_pos = IVec3::ZERO;
        let right_pos = IVec3::X;
        let left_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(left_pos),
                Chunk::default(),
                ChunkLight::default(),
                ChunkHeightmap::default(),
                ChunkNeedsLightRebuild,
            ))
            .id();

        let mut right_chunk = Chunk::default();
        right_chunk.set_cell_xyz(0, 8, 8, BlockType::Glowstone.into());
        let right_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(right_pos),
                right_chunk,
                ChunkLight::default(),
                ChunkHeightmap::default(),
                ChunkNeedsLightRebuild,
            ))
            .id();
        register_chunk(&mut app, left_pos, left_entity);
        register_chunk(&mut app, right_pos, right_entity);

        app.update();

        assert_eq!(
            app.world()
                .get::<ChunkLight>(left_entity)
                .unwrap()
                .block_light(LocalBlockPos::new(15, 8, 8)),
            14
        );

        app.world_mut()
            .entity_mut(right_entity)
            .get_mut::<Chunk>()
            .unwrap()
            .set_cell_xyz(0, 8, 8, ChunkCell::EMPTY);
        app.world_mut()
            .entity_mut(left_entity)
            .insert(ChunkNeedsLightRebuild);
        app.world_mut()
            .entity_mut(right_entity)
            .insert(ChunkNeedsLightRebuild);

        app.update();

        assert_eq!(
            app.world()
                .get::<ChunkLight>(right_entity)
                .unwrap()
                .block_light(LocalBlockPos::new(0, 8, 8)),
            0
        );
        assert_eq!(
            app.world()
                .get::<ChunkLight>(left_entity)
                .unwrap()
                .block_light(LocalBlockPos::new(15, 8, 8)),
            0
        );
    }

    #[test]
    fn light_rebuild_does_not_consume_markers_from_another_dimension() {
        let mut app = app_with_light_system(1);
        let foreign_position = ivec3(12, 0, -8);
        let foreign_entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(foreign_position),
                Chunk::default(),
                ChunkLight::default(),
                ChunkHeightmap::default(),
                ChunkNeedsLightRebuild,
            ))
            .id();
        let mut foreign_dimension = Dimension::default();
        foreign_dimension.register_published_chunk(foreign_position.into(), foreign_entity);
        app.world_mut().spawn(foreign_dimension);

        app.update();

        assert!(
            app.world()
                .get::<ChunkNeedsLightRebuild>(foreign_entity)
                .is_some()
        );
    }

    #[test]
    fn light_rebuild_targets_do_not_expand_dirty_column_set() {
        let mut dimension = Dimension::default();
        for x in -2..=2 {
            for z in -2..=2 {
                for y in 0..2 {
                    dimension.register_published_chunk(ChunkPos::new(x, y, z), Entity::PLACEHOLDER);
                }
            }
        }

        let dirty_positions = (-1..=1)
            .flat_map(|x| (-1..=1).map(move |z| ChunkPos::new(x, 0, z)))
            .collect::<Vec<_>>();
        let targets = light_rebuild_targets(&dirty_positions, &dimension, 2);

        assert_eq!(targets.len(), 18);
        for x in -1..=1 {
            for z in -1..=1 {
                for y in 0..2 {
                    assert!(targets.contains(&ChunkPos::new(x, y, z)));
                }
            }
        }
        assert!(!targets.contains(&ChunkPos::new(-2, 0, 0)));
        assert!(!targets.contains(&ChunkPos::new(2, 0, 0)));
    }

    #[test]
    fn light_rebuild_targets_keep_loaded_gaps_and_world_height_bounds() {
        let column = ChunkColumn::new(-8, 13);
        let mut dimension = Dimension::default();
        for y in [0, 2, 4] {
            dimension.register_published_chunk(column.chunk(y), Entity::PLACEHOLDER);
        }

        let targets = light_rebuild_targets(
            &[column.chunk(0), column.chunk(2), column.chunk(2)],
            &dimension,
            4,
        );

        assert_eq!(targets, HashSet::from([column.chunk(0), column.chunk(2)]));
    }
}
