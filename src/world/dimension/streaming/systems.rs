use std::time::Instant;

use avian3d::prelude::Collider;
use bevy::prelude::*;

use crate::{
    player::{Player, PlayerDimension},
    world::{
        chunk::{
            ChunkColumn, ChunkContentCounts, ChunkInvalidationPlan, ChunkLight,
            ChunkNeedsColliderRebuild, ChunkNeedsFluidStep, ChunkNeedsLightRebuild, ChunkNeedsSave,
            ChunkPerfCounters, ChunkPos, ChunkPosition,
        },
        definition::ColumnAddress,
        loading::load_or_generate_column,
        storage::ChunkRepository,
    },
};

use super::{
    ColumnActivationBudget, ColumnExposure, ColumnLighting, ColumnLoadBudget, ColumnStagingBudget,
    CompletedColumnLoad,
};
use crate::world::dimension::{
    Active, ChunkTaskPool, DesiredColumnView, Dimension, ViewDistance, apply_chunk_invalidations,
};

#[derive(Component)]
struct LoadedColumnRoot;

pub(crate) fn refresh_desired_column_view(
    maybe_player: Option<Single<(&Transform, &PlayerDimension), With<Player>>>,
    view_distance: Res<ViewDistance>,
    dimension: Single<(&Dimension, &mut DesiredColumnView), With<Active>>,
) {
    let (dimension, mut desired_view) = dimension.into_inner();
    let translation = maybe_player
        .filter(|player| player.1.id() == dimension.id())
        .map_or(dimension.arrival(), |player| player.0.translation);
    let center = ChunkColumn::from(ChunkPos::containing_translation(translation));
    if desired_view
        .bypass_change_detection()
        .refresh(center, *view_distance, dimension.height())
    {
        desired_view.set_changed();
    }
}

pub(crate) fn maintain_column_residency(
    mut commands: Commands,
    dimension: Single<(&mut Dimension, &DesiredColumnView, Entity), With<Active>>,
    dirty_chunks: Query<(), With<ChunkNeedsSave>>,
    children: Query<&Children>,
    colliders: Query<(), With<Collider>>,
) {
    let (mut dimension, desired_view, owner) = dimension.into_inner();
    dimension.assert_stream_owner(owner);
    dimension.stream_mut().tick_backoffs();

    for &column in desired_view.resident_columns() {
        dimension.stream_mut().mark_desired(column);
    }

    let published_columns = dimension
        .stream()
        .columns()
        .filter(|&column| dimension.column_exposure(column) == Some(ColumnExposure::Published))
        .collect::<Vec<_>>();
    for column in published_columns {
        if desired_view.contains_visible_column(column) {
            continue;
        }
        assert!(dimension.unpublish_column(column));
        let incarnation = dimension
            .column_incarnation(column)
            .expect("unpublished column must retain its incarnation");
        commands.entity(incarnation).insert(Visibility::Hidden);

        let chunks = dimension
            .complete_loaded_column(column)
            .expect("unpublished column must remain complete");
        for (_, entity) in chunks {
            commands.entity(entity).remove::<(
                ChunkNeedsColliderRebuild,
                ChunkNeedsFluidStep,
                ChunkNeedsLightRebuild,
            )>();
            if let Ok(children) = children.get(entity) {
                for child in children {
                    if colliders.get(*child).is_ok() {
                        commands.entity(*child).despawn();
                    }
                }
            }
        }
    }

    let tracked_columns = dimension.stream().columns().collect::<Vec<_>>();
    for column in tracked_columns {
        if !desired_view.contains_resident_column(column) {
            dimension.cancel_light_task_depending_on(column);
            dimension.stream_mut().mark_undesired(column);
        }
    }

    let eviction_tickets = dimension.stream().eviction_tickets().collect::<Vec<_>>();
    for ticket in eviction_tickets {
        let entities = dimension
            .complete_loaded_column(ticket.column())
            .expect("resident column must contain every configured Y chunk");
        if entities
            .iter()
            .any(|(_, entity)| dirty_chunks.get(*entity).is_ok())
        {
            continue;
        }

        let removed = dimension
            .evict_column(ticket)
            .expect("current clean eviction ticket must commit");
        commands.entity(removed.incarnation).despawn();
    }
}

pub(crate) fn finish_column_loads(
    mut commands: Commands,
    mut perf: Option<ResMut<ChunkPerfCounters>>,
    dimension: Single<(&mut Dimension, &DesiredColumnView, Entity), With<Active>>,
    staging_budget: Res<ColumnStagingBudget>,
) {
    if staging_budget.0 == 0 {
        return;
    }

    let (mut dimension, desired_view, owner) = dimension.into_inner();
    dimension.assert_stream_owner(owner);
    let mut completed = Vec::new();
    for &column in desired_view.resident_columns() {
        if completed.len() >= staging_budget.0 {
            break;
        }
        if let Some(ready) = dimension.stream_mut().take_ready_load(column) {
            completed.push(ready);
        }
    }

    for (ticket, completed) in completed {
        record_column_load_perf(perf.as_deref_mut(), &completed);
        if ticket.view_revision() != desired_view.revision() {
            trace!(
                column = ?ticket.column(),
                started_revision = ticket.view_revision(),
                current_revision = desired_view.revision(),
                "Accepting a still-desired column from an older view revision"
            );
        }
        let loaded = match completed.result {
            Ok(loaded) => loaded,
            Err(error) => {
                warn!(%error, column = ?ticket.column(), "Failed to load chunk column");
                assert!(dimension.stream_mut().fail_load(ticket, error));
                continue;
            }
        };

        assert_eq!(loaded.position(), ticket.column());
        assert_eq!(loaded.address.dimension(), dimension.id());
        assert_eq!(loaded.height, dimension.height());
        if !dimension.stream_mut().accept_load(ticket) {
            continue;
        }

        let heightmap = loaded.heightmap;
        let incarnation = commands
            .spawn((
                ChildOf(owner),
                LoadedColumnRoot,
                Transform::default(),
                Visibility::Hidden,
            ))
            .id();
        let mut entities = Vec::with_capacity(loaded.height.chunks());
        for loaded_chunk in loaded.into_chunks() {
            let position = loaded_chunk.position;
            let counts = loaded_chunk.contents;
            let entity = commands
                .spawn((
                    ChildOf(incarnation),
                    ChunkPosition::from(position),
                    loaded_chunk.chunk,
                    ChunkLight::default(),
                    heightmap,
                    counts,
                    Transform::from_translation(position.origin_translation()),
                    Visibility::Inherited,
                ))
                .id();
            entities.push(entity);
        }

        dimension.install_accepted_column(ticket, incarnation, entities);
    }
}

pub(crate) fn publish_lit_columns(
    mut commands: Commands,
    dimension: Single<(&mut Dimension, &DesiredColumnView), With<Active>>,
    activation_budget: Res<ColumnActivationBudget>,
    contents: Query<&ChunkContentCounts>,
) {
    if activation_budget.0 == 0 {
        return;
    }
    let (mut dimension, desired_view) = dimension.into_inner();
    let mut invalidations = ChunkInvalidationPlan::new();

    let mut activated = 0;
    for &column in desired_view.visible_columns() {
        if activated == activation_budget.0 {
            break;
        }
        if dimension.column_lighting(column) != Some(ColumnLighting::Lit)
            || dimension.column_exposure(column) != Some(ColumnExposure::Staged)
            || !dimension.has_complete_resident_light_neighborhood(column)
        {
            continue;
        }
        assert!(dimension.publish_lit_column(column));
        activated += 1;
        let incarnation = dimension
            .column_incarnation(column)
            .expect("published column must retain its incarnation");
        commands.entity(incarnation).insert(Visibility::Inherited);

        for (position, entity) in dimension
            .complete_loaded_column(column)
            .expect("published column must remain complete")
        {
            let counts = *contents
                .get(entity)
                .expect("published chunk must retain its content counts");
            invalidations.record_chunk_published(position, counts);
        }
    }

    apply_chunk_invalidations(&mut commands, &mut dimension, &invalidations);
}

pub(crate) fn start_column_loads(
    dimension: Single<(&mut Dimension, &DesiredColumnView, Entity), With<Active>>,
    repository: Res<ChunkRepository>,
    load_budget: Res<ColumnLoadBudget>,
    task_pool: Res<ChunkTaskPool>,
) {
    let (mut dimension, desired_view, owner) = dimension.into_inner();
    dimension.assert_stream_owner(owner);
    assert_eq!(
        repository.catalog().get(dimension.id()).copied(),
        Some(dimension.definition()),
        "dimension root definition must match the repository catalog"
    );
    if load_budget.0 == 0 {
        return;
    }

    // The center cannot begin lighting until this exact H1 closure is resident.
    // Admit it as one dependency group instead of serializing it over several
    // steady-state load windows.
    let bootstrap_columns = desired_view
        .center()
        .filter(|&center| !dimension.has_complete_resident_light_neighborhood(center))
        .map(|_| desired_view.center_light_dependencies().collect::<Vec<_>>())
        .unwrap_or_default();
    for &column in &bootstrap_columns {
        try_start_column_load(
            &mut dimension,
            column,
            desired_view.revision(),
            &repository,
            &task_pool,
        );
    }

    let available = load_budget
        .0
        .saturating_sub(dimension.stream().loading_count());
    if available == 0 {
        return;
    }

    let mut started = 0;
    for &column in desired_view.resident_columns() {
        if started >= available {
            break;
        }
        if bootstrap_columns.contains(&column) || dimension.has_any_loaded_chunk_in_column(column) {
            continue;
        }

        if try_start_column_load(
            &mut dimension,
            column,
            desired_view.revision(),
            &repository,
            &task_pool,
        ) {
            started += 1;
        }
    }
}

fn try_start_column_load(
    dimension: &mut Dimension,
    column: ChunkColumn,
    view_revision: u64,
    repository: &ChunkRepository,
    task_pool: &ChunkTaskPool,
) -> bool {
    if dimension.has_any_loaded_chunk_in_column(column) {
        return false;
    }
    let address = ColumnAddress::new(dimension.id(), column);
    dimension
        .stream_mut()
        .start_load(column, view_revision, || {
            let repository = repository.clone();
            let submitted_at = Instant::now();
            task_pool.spawn(async move {
                let queue_elapsed = submitted_at.elapsed();
                let worker_started = Instant::now();
                let result = load_or_generate_column(address, repository);
                let completed_at = Instant::now();
                CompletedColumnLoad {
                    result,
                    submitted_at,
                    completed_at,
                    queue_elapsed,
                    worker_elapsed: completed_at.duration_since(worker_started),
                }
            })
        })
        .is_some()
}

fn record_column_load_perf(perf: Option<&mut ChunkPerfCounters>, completed: &CompletedColumnLoad) {
    let Some(perf) = perf else { return };
    let picked_up_at = Instant::now();
    let pickup_lag = picked_up_at.duration_since(completed.completed_at);
    let latency = picked_up_at.duration_since(completed.submitted_at);
    perf.column_loads += 1;
    perf.column_load_worker_elapsed += completed.worker_elapsed;
    perf.column_load_max_worker_elapsed = perf
        .column_load_max_worker_elapsed
        .max(completed.worker_elapsed);
    perf.column_load_queue_elapsed += completed.queue_elapsed;
    perf.column_load_max_queue_elapsed = perf
        .column_load_max_queue_elapsed
        .max(completed.queue_elapsed);
    perf.column_load_pickup_lag += pickup_lag;
    perf.column_load_max_pickup_lag = perf.column_load_max_pickup_lag.max(pickup_lag);
    perf.column_load_latency += latency;
    perf.column_load_max_latency = perf.column_load_max_latency.max(latency);
}
