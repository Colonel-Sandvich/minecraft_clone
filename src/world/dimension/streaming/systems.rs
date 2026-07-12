use bevy::prelude::*;

use crate::{
    player::Player,
    world::{
        chunk::{
            ChunkColumn, ChunkInvalidationPlan, ChunkLight, ChunkNeedsSave, ChunkPos, ChunkPosition,
        },
        generation::WorldMetadata,
        loading::load_or_generate_column,
        storage::ChunkRepository,
    },
};

use super::{ColumnActivationBudget, ColumnLoadBudget};
use crate::world::dimension::{
    Active, ChunkTaskPool, DesiredColumnView, Dimension, ViewDistance, apply_chunk_invalidations,
};

pub(crate) fn refresh_desired_column_view(
    maybe_player: Option<Single<&Transform, With<Player>>>,
    metadata: Res<WorldMetadata>,
    view_distance: Res<ViewDistance>,
    mut desired_view: ResMut<DesiredColumnView>,
) {
    let translation = maybe_player.map_or(Vec3::ZERO, |player| player.translation);
    let center = ChunkColumn::from(ChunkPos::containing_translation(translation));
    if desired_view
        .bypass_change_detection()
        .refresh(center, *view_distance, metadata.height())
    {
        desired_view.set_changed();
    }
}

pub(crate) fn maintain_column_residency(
    mut commands: Commands,
    dimension: Single<(&mut Dimension, Entity), With<Active>>,
    desired_view: Res<DesiredColumnView>,
    dirty_chunks: Query<(), With<ChunkNeedsSave>>,
) {
    let (mut dimension, owner) = dimension.into_inner();
    dimension.assert_stream_owner(owner);
    dimension.stream_mut().tick_backoffs();

    for &column in desired_view.columns() {
        dimension.stream_mut().mark_desired(column);
    }

    let tracked_columns = dimension.stream().columns().collect::<Vec<_>>();
    for column in tracked_columns {
        if !desired_view.contains_column(column) {
            dimension.stream_mut().mark_undesired(column);
        }
    }

    let eviction_tickets = dimension.stream().eviction_tickets().collect::<Vec<_>>();
    let mut invalidations = ChunkInvalidationPlan::new();
    for ticket in eviction_tickets {
        let entities = dimension
            .complete_column(ticket.column())
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
        for (position, entity) in removed {
            invalidations.record_chunk_unloaded(position);
            commands.entity(entity).despawn();
        }
    }

    apply_chunk_invalidations(&mut commands, &dimension, &invalidations);
}

pub(crate) fn finish_column_loads(
    mut commands: Commands,
    dimension: Single<(&mut Dimension, Entity), With<Active>>,
    desired_view: Res<DesiredColumnView>,
    activation_budget: Res<ColumnActivationBudget>,
) {
    if activation_budget.0 == 0 {
        return;
    }

    let (mut dimension, owner) = dimension.into_inner();
    dimension.assert_stream_owner(owner);
    let mut completed = Vec::new();
    for &column in desired_view.columns() {
        if completed.len() >= activation_budget.0 {
            break;
        }
        if let Some(ready) = dimension.stream_mut().take_ready_load(column) {
            completed.push(ready);
        }
    }

    let mut invalidations = ChunkInvalidationPlan::new();
    for (ticket, result) in completed {
        if ticket.view_revision() != desired_view.revision() {
            trace!(
                column = ?ticket.column(),
                started_revision = ticket.view_revision(),
                current_revision = desired_view.revision(),
                "Accepting a still-desired column from an older view revision"
            );
        }
        let loaded = match result {
            Ok(loaded) => loaded,
            Err(error) => {
                warn!(%error, column = ?ticket.column(), "Failed to load chunk column");
                assert!(dimension.stream_mut().fail_load(ticket, error));
                continue;
            }
        };

        assert_eq!(loaded.position, ticket.column());
        assert_eq!(loaded.height, dimension.height());
        if !dimension.stream_mut().accept_load(ticket) {
            continue;
        }

        let heightmap = loaded.heightmap;
        let mut entities = Vec::with_capacity(loaded.height.chunks());
        for loaded_chunk in loaded.into_chunks() {
            let position = loaded_chunk.position;
            let counts = loaded_chunk.chunk.compute_content_counts();
            let entity = commands
                .spawn((
                    ChildOf(owner),
                    ChunkPosition::from(position),
                    loaded_chunk.chunk,
                    ChunkLight::default(),
                    heightmap,
                    counts,
                    Transform::from_translation(position.origin_translation()),
                    Visibility::default(),
                ))
                .id();
            entities.push(entity);
            invalidations.record_chunk_loaded(position, counts);
        }

        dimension.publish_accepted_column(ticket, entities);
    }

    apply_chunk_invalidations(&mut commands, &dimension, &invalidations);
}

pub(crate) fn start_column_loads(
    dimension: Single<(&mut Dimension, Entity), With<Active>>,
    desired_view: Res<DesiredColumnView>,
    repository: Res<ChunkRepository>,
    load_budget: Res<ColumnLoadBudget>,
    task_pool: Res<ChunkTaskPool>,
) {
    let (mut dimension, owner) = dimension.into_inner();
    dimension.assert_stream_owner(owner);
    let available = load_budget
        .0
        .saturating_sub(dimension.stream().loading_count());
    if available == 0 {
        return;
    }

    let mut started = 0;
    for &column in desired_view.columns() {
        if started >= available {
            break;
        }
        if dimension.has_any_chunk_in_column(column) {
            continue;
        }

        let repository = repository.clone();
        if dimension
            .stream_mut()
            .start_load(column, desired_view.revision(), || {
                task_pool.spawn(async move { load_or_generate_column(column, repository) })
            })
            .is_some()
        {
            started += 1;
        }
    }
}
