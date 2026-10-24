use std::thread;

use bevy::prelude::*;

use super::super::*;
use crate::{
    player::{Player, PlayerDimension},
    world::{
        chunk::ChunkColumn,
        definition::{DimensionCatalog, DimensionId},
        dimension::{
            Active, ChunkSaveTasks, ChunkTaskPool, ColumnLightBudget, DesiredColumnView, Dimension,
            ViewDistance,
            light::{cancel_inactive_dimension_light_tasks, rebuild_chunk_light},
        },
        generation::WorldMetadata,
        storage::{ChunkRepository, NoopChunkStore},
    },
};

pub(super) fn metadata(height_chunks: usize) -> WorldMetadata {
    WorldMetadata::with_seed(7)
        .with_height_chunks(height_chunks)
        .unwrap()
}

pub(super) fn spawn_dimension(
    app: &mut App,
    height: crate::world::WorldHeight,
    active: bool,
) -> Entity {
    let metadata = app.world().resource::<WorldMetadata>();
    assert_eq!(metadata.height(), height);
    let definition = *DimensionCatalog::for_world(metadata)
        .get(DimensionId::OVERWORLD)
        .unwrap();
    let entity = app.world_mut().spawn_empty().id();
    let mut entity_mut = app.world_mut().entity_mut(entity);
    entity_mut.insert((
        Dimension::new(entity, definition),
        DesiredColumnView::default(),
    ));
    if active {
        entity_mut.insert(Active);
    }
    entity
}

pub(super) fn spawn_defined_dimension(
    app: &mut App,
    definition: crate::world::DimensionDefinition,
    active: bool,
) -> Entity {
    let entity = app.world_mut().spawn_empty().id();
    let mut entity_mut = app.world_mut().entity_mut(entity);
    entity_mut.insert((
        Dimension::new(entity, definition),
        DesiredColumnView::default(),
    ));
    if active {
        entity_mut.insert(Active);
    }
    entity
}

pub(super) fn streaming_app(height_chunks: usize) -> (App, Entity, Entity) {
    let metadata = metadata(height_chunks);
    let repository = ChunkRepository::new(NoopChunkStore::new(metadata.clone()));
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(metadata.clone())
        .insert_resource(repository)
        .insert_resource(ChunkTaskPool::new_for_test())
        .init_resource::<ChunkSaveTasks>()
        .insert_resource(ColumnLoadBudget(1))
        .insert_resource(ColumnStagingBudget(1))
        .insert_resource(ColumnActivationBudget(1))
        .insert_resource(ViewDistance::new(1))
        .add_systems(
            Update,
            (
                refresh_desired_column_view,
                maintain_column_residency,
                finish_column_loads,
                start_column_loads,
            )
                .chain(),
        );
    let dimension = spawn_dimension(&mut app, metadata.height(), true);
    let player = app
        .world_mut()
        .spawn((
            Player::default(),
            PlayerDimension::new(DimensionId::OVERWORLD),
            Transform::default(),
        ))
        .id();
    (app, dimension, player)
}

pub(super) fn staged_lighting_app(height_chunks: usize) -> (App, Entity, Entity) {
    let metadata = metadata(height_chunks);
    let repository = ChunkRepository::new(NoopChunkStore::new(metadata.clone()));
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(metadata.clone())
        .insert_resource(repository)
        .insert_resource(ChunkTaskPool::new_for_test())
        .init_resource::<ChunkSaveTasks>()
        .insert_resource(ColumnLoadBudget(9))
        .insert_resource(ColumnStagingBudget(1))
        .insert_resource(ColumnActivationBudget(9))
        .insert_resource(ColumnLightBudget(100))
        .insert_resource(ViewDistance::new(1))
        .init_resource::<crate::world::chunk::ChunkPerfCounters>()
        .add_systems(
            Update,
            (
                cancel_inactive_dimension_light_tasks,
                refresh_desired_column_view,
                maintain_column_residency,
                finish_column_loads,
                rebuild_chunk_light,
                publish_lit_columns,
                start_column_loads,
            )
                .chain(),
        );
    let dimension = spawn_dimension(&mut app, metadata.height(), true);
    let player = app
        .world_mut()
        .spawn((
            Player::default(),
            PlayerDimension::new(DimensionId::OVERWORLD),
            Transform::default(),
        ))
        .id();
    (app, dimension, player)
}

pub(super) fn update_until(app: &mut App, mut predicate: impl FnMut(&World) -> bool) {
    for _ in 0..1_000 {
        app.update();
        if predicate(app.world()) {
            return;
        }
        thread::yield_now();
    }
    panic!("streaming condition did not become true");
}

pub(super) fn loaded_in_column(world: &World, dimension: Entity, column: ChunkColumn) -> usize {
    let dimension = world.get::<Dimension>(dimension).unwrap();
    (0..dimension.height().chunks_i32())
        .filter(|&y| dimension.contains_loaded_chunk(column.chunk(y)))
        .count()
}
