use std::thread;

use bevy::prelude::*;

use super::*;
use crate::{
    player::Player,
    world::{
        chunk::{CHUNK_SIZE, ChunkColumn, ChunkNeedsLightRebuild, ChunkNeedsSave},
        dimension::{Active, ChunkTaskPool, DesiredColumnView, Dimension, ViewDistance},
        generation::WorldMetadata,
        storage::{ChunkRepository, NoopChunkStore},
    },
};

fn metadata(height_chunks: usize) -> WorldMetadata {
    WorldMetadata::with_seed(7)
        .with_height_chunks(height_chunks)
        .unwrap()
}

fn spawn_dimension(app: &mut App, height: crate::world::WorldHeight, active: bool) -> Entity {
    let entity = app.world_mut().spawn_empty().id();
    let mut entity_mut = app.world_mut().entity_mut(entity);
    entity_mut.insert(Dimension::new(entity, height));
    if active {
        entity_mut.insert(Active);
    }
    entity
}

fn streaming_app(height_chunks: usize) -> (App, Entity, Entity) {
    let metadata = metadata(height_chunks);
    let repository = ChunkRepository::new(NoopChunkStore::new(metadata.clone()));
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(metadata.clone())
        .insert_resource(repository)
        .insert_resource(ChunkTaskPool::new_for_test())
        .insert_resource(ColumnLoadBudget(1))
        .insert_resource(ColumnActivationBudget(1))
        .insert_resource(ViewDistance::new(1))
        .init_resource::<DesiredColumnView>()
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
        .spawn((Player::default(), Transform::default()))
        .id();
    (app, dimension, player)
}

fn update_until(app: &mut App, mut predicate: impl FnMut(&World) -> bool) {
    for _ in 0..1_000 {
        app.update();
        if predicate(app.world()) {
            return;
        }
        thread::yield_now();
    }
    panic!("streaming condition did not become true");
}

fn loaded_in_column(world: &World, dimension: Entity, column: ChunkColumn) -> usize {
    let dimension = world.get::<Dimension>(dimension).unwrap();
    (0..dimension.height().chunks_i32())
        .filter(|&y| dimension.contains_loaded_chunk(column.chunk(y)))
        .count()
}

#[test]
fn activation_publishes_a_complete_column_in_one_update() {
    let height_chunks = 3;
    let center = ChunkColumn::new(0, 0);
    let (mut app, dimension, _) = streaming_app(height_chunks);

    app.update();
    assert_eq!(loaded_in_column(app.world(), dimension, center), 0);

    for _ in 0..1_000 {
        app.update();
        let loaded = loaded_in_column(app.world(), dimension, center);
        assert!(loaded == 0 || loaded == height_chunks);
        if loaded == height_chunks {
            break;
        }
        thread::yield_now();
    }

    let world = app.world();
    let dimension_ref = world.get::<Dimension>(dimension).unwrap();
    assert_eq!(loaded_in_column(world, dimension, center), height_chunks);
    for (_, entity) in dimension_ref.complete_loaded_column(center).unwrap() {
        assert!(world.get::<ChunkNeedsLightRebuild>(entity).is_some());
    }
}

#[test]
fn load_completion_remains_bound_to_the_dimension_that_started_it() {
    let height_chunks = 2;
    let center = ChunkColumn::new(0, 0);
    let (mut app, first, _) = streaming_app(height_chunks);
    let height = app.world().resource::<WorldMetadata>().height();
    let second = spawn_dimension(&mut app, height, false);

    app.update();
    assert_eq!(
        app.world()
            .get::<Dimension>(first)
            .unwrap()
            .stream()
            .loading_count(),
        1
    );

    app.world_mut().entity_mut(first).remove::<Active>();
    app.world_mut().entity_mut(second).insert(Active);
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 0;
    for _ in 0..20 {
        app.update();
        thread::yield_now();
    }

    assert_eq!(loaded_in_column(app.world(), second, center), 0);
    assert_eq!(loaded_in_column(app.world(), first, center), 0);

    app.world_mut().entity_mut(second).remove::<Active>();
    app.world_mut().entity_mut(first).insert(Active);
    update_until(&mut app, |world| {
        loaded_in_column(world, first, center) == height_chunks
    });

    assert_eq!(loaded_in_column(app.world(), first, center), height_chunks);
    assert_eq!(loaded_in_column(app.world(), second, center), 0);
}

#[test]
fn one_dirty_chunk_blocks_eviction_of_the_whole_column() {
    let height_chunks = 3;
    let center = ChunkColumn::new(0, 0);
    let (mut app, dimension, player) = streaming_app(height_chunks);
    update_until(&mut app, |world| {
        loaded_in_column(world, dimension, center) == height_chunks
    });

    let dirty = app
        .world()
        .get::<Dimension>(dimension)
        .unwrap()
        .loaded_chunk_entity(center.chunk(1))
        .unwrap();
    app.world_mut().entity_mut(dirty).insert(ChunkNeedsSave);
    app.world_mut()
        .entity_mut(player)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::X * (CHUNK_SIZE as f32 * 10.0);

    app.update();
    assert_eq!(
        loaded_in_column(app.world(), dimension, center),
        height_chunks
    );
    assert!(matches!(
        app.world()
            .get::<Dimension>(dimension)
            .unwrap()
            .stream()
            .state(center),
        Some(ColumnResidency::Evicting { .. })
    ));

    app.world_mut().entity_mut(dirty).remove::<ChunkNeedsSave>();
    app.update();
    assert_eq!(loaded_in_column(app.world(), dimension, center), 0);
}

#[test]
fn column_budgets_count_columns_not_subchunks() {
    let height_chunks = 4;
    let (mut app, dimension, _) = streaming_app(height_chunks);

    app.update();

    let dimension = app.world().get::<Dimension>(dimension).unwrap();
    assert_eq!(dimension.stream().loading_count(), 1);
    assert_eq!(dimension.loaded_chunk_count(), 0);
}
